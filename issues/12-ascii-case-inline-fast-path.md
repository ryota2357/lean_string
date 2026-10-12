# inline の文字列の大文字・小文字変換が、確保しないのに `String` (確保あり) より遅い

- 種別: perf
- 対象: `src/repr.rs` の `Repr::to_ascii_case`, `Repr::<Mutable>::make_ascii_case`, `Repr::<Mutable>::to_case`、`src/case.rs`
- 影響: 16 バイト (64-bit) 以下の文字列への `to_ascii_lowercase` / `to_ascii_uppercase` / `to_lowercase` / `to_uppercase`
  (`LeanString` と `LeanStr` の両方)。識別子やキーの正規化のように短い文字列で多用される操作で、
  inline の利点 (確保しない) が走査とコピーの重さで相殺されている。共有バッファへの `make_ascii_*` で何も変わらないときにも、
  参照カウントの増減が 1 組余計に起きる。
- 優先度: 中

## 問題

`Repr::to_ascii_case` は inline でも heap と同じ手順をとる。`split_at_first_ascii_change` (非インラインの関数呼び出し、
16 バイトのチャンク判定のあと最初に変わる位置を 1 バイトずつ数える) で分割し、`Repr::new_with` の中で
変わらない部分を `ptr::copy_nonoverlapping` (実行時長なので `memcpy` の呼び出し) し、残りを 1 バイトずつ変換する。
`to_lowercase` / `to_uppercase` (`Repr::to_case`) も、ASCII だけの inline の文字列に対して、走査を 3 回
(`split_at_first_change`、`split_ascii_prefix`、変換) と `memcpy` 1 回を行う。

`5dd94ea`、rustc 1.97.0、LTO off の probe の x86-64 asm: `to_ascii_case` (`LeanString` の実体、268 命令) は
`split_at_first_ascii_change` の呼び出しと `memcpy` の呼び出しを 2 か所含む。

cachegrind の Ir (1 回あたり、ループと `black_box` を含む):

| 入力 | 操作 | `LeanString` 現状 | 修正例 | `String` |
|---|---|---:|---:|---:|
| `"HELLO WORLD ABCD"` (16 バイト) | `to_ascii_lowercase` | 238 (確保 0) | 48 (確保 0) | 218 (確保 1) |
| ランダムな印字可能 ASCII 16 バイト | `to_lowercase` | 316 (確保 0) | 81 (確保 0) | 225 (確保 1) |

共有バッファへの `make_ascii_lowercase` で何も変わらない場合、`make_ascii_case` は `to_ascii_case` を呼び、
`to_ascii_case` は変化が無いので `make_shallow_clone` (`lock incq`) を返し、それを `replace_inner` で
元と入れ替える (`lock decq`)。結果は同じバッファで、atomic な RMW が 2 回余計になる (asm で確認。
Ir には現れないが、`lock` 付き命令は 1 回あたり数十サイクルかかり、他スレッドと共有しているときはキャッシュラインも奪い合う)。

## 原因

- inline の結果は値のコピーなので、「変化が無ければ共有を保つ」ための事前走査が inline では意味を持たない。
  変化が無くても、inline 全体をコピーして変換した結果は元と同じになる。
- inline の 16 バイトは、長さの位置を気にせずまとめて変換できる。長さ未満の末尾は 0、最後のバイトは長さのタグ
  (`0xC0..=0xCF`) で、どちらも ASCII 英字ではないので ASCII の大文字・小文字変換で変わらない。今のコードは長さで区切った
  スライスとして扱うので、`memcpy` と長さ依存のループが残る (asm で確認)。
- `make_ascii_case` は「変化が無い」ことを `to_ascii_case` が返す shallow clone でしか知らない。

## 修正例

inline のときは 2 ワードをそのまま SWAR で変換する。ワード単位の判定は桁上がりがバイトをまたがない形にする。

```rust
// src/case.rs
/// Maps the ASCII letters of every byte of `word` to `case`, leaving other bytes as they are.
#[inline(always)]
pub(crate) fn swar_ascii_case(word: usize, case: Case) -> usize {
    const fn splat(b: u8) -> usize {
        usize::from_ne_bytes([b; size_of::<usize>()])
    }
    let (first, last) = match case {
        Case::Lower => (b'A', b'Z'),
        Case::Upper => (b'a', b'z'),
    };
    let heptets = word & splat(0x7F);
    // The top bit of each byte is set where the byte (without its top bit) is >= `first`,
    let ge_first = heptets.wrapping_add(splat(0x80 - first));
    // and where it is > `last`. `heptets <= 0x7F`, so neither addition carries across bytes.
    let gt_last = heptets.wrapping_add(splat(0x7F - last));
    let is_letter = (ge_first ^ gt_last) & !word & splat(0x80);
    // 0x80 >> 2 == 0x20, the bit that differs between the cases.
    word ^ (is_letter >> 2)
}

// src/repr.rs の Repr::to_ascii_case の先頭
        if self.is_inline_buffer() {
            // An inline buffer is copied by value anyway. Map all its bytes: the padding is zero and
            // the last byte is the length tag (0xC0..=0xCF), neither of which is an ASCII letter.
            // SAFETY: `Repr` is two `usize`s, and the mapping keeps it a valid inline buffer.
            let words: [usize; 2] = unsafe { mem::transmute_copy(self) };
            let words = words.map(|w| case::swar_ascii_case(w, case));
            return Ok(unsafe { mem::transmute::<[usize; 2], Self>(words) });
        }
```

`to_lowercase` / `to_uppercase` は、inline で全バイトが ASCII なら上と同じ変換で済む (ASCII の文字に対する Unicode の大文字・小文字変換は
ASCII の変換と一致する)。

```rust
        // LeanString::to_lowercase / LeanStr::to_lowercase などの先頭 (Repr にメソッドとして置く)
        if self.is_inline_buffer() && self.as_bytes().is_ascii() {
            return self.to_ascii_case(case);
        }
```

`make_ascii_case` の非 modifiable 経路は、先に `split_at_first_ascii_change` で変化の有無を見て、無ければ何もしないで返す。
(inline は modifiable なので、in-place の経路も同じ SWAR にすれば長さ依存のループが消える。)

先に試した「`[u8; 16]` に `transmute_copy` して `<[u8]>::make_ascii_lowercase()`」は、LLVM が
1 バイトずつのシフトと比較に展開し (asm で 150 命令ほど。2 ワードとして読んだ値を分解したためと思われるが未検証)、
66 Ir だった。SWAR のほうが速い。

(`make_ascii_case` の 2 点は未実装。`to_ascii_case` の inline 分岐と `to_lowercase` の分岐だけを試した。)

試した結果: inline の分岐は x86-64 で SSE の 16 命令ほどになった (`movdqu` / `pand` / `paddq` / `pxor` / `psrlq` / `movdqu`)。
Ir は上の表のとおり。`tests/property.rs` (`to_lowercase` / `to_uppercase` / `to_ascii_*` / `make_ascii_*` の property テスト)、
`tests/lean_string.rs`、`tests/lean_str.rs` が通り、短い入力の `to_ascii_*` を `LeanString` / `LeanStr` で確かめるテストは
Miri (`-Zmiri-strict-provenance`) の x86_64、i686、powerpc (32-bit big-endian) で問題なし。

## 確認方法

- `bench/benches/apis.rs` に `to_ascii_lowercase` のベンチが無い。inline (16 バイト以下) と heap の入力で、`String` と並べて足す。
  既存の `to_lowercase` / `to_uppercase` のベンチは `ascii/16` がこのケースにあたる。
- probe の asm で、`to_ascii_case` の inline 分岐に `memcpy` と `split_at_first_ascii_change` の呼び出しが無いこと。
- 長さ 0〜16 のすべての長さで、英字・記号・境界の文字 (`@`, `[`, `` ` ``, `{`) と 2 バイト文字を含む入力を `str` の結果と比べるテスト
  (SWAR の境界条件を押さえる)。32-bit と big-endian のために Miri の i686 / powerpc でも回す。
