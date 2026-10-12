# `from_utf8_lossy` が正しい UTF-8 でも `utf8_chunks` で 1 バイトずつ検証し、`LeanStr` 版は `realloc` も足される

- 種別: perf
- 対象: `src/lib.rs` の `LeanString::from_utf8_lossy`、`LeanStr::from_utf8_lossy`
- 影響: 正しい UTF-8 (実際の入力の大半) を `from_utf8_lossy` で変換するすべての呼び出し。ASCII 1000 バイトで、
  先に `str::from_utf8` で検証する場合の約 8〜10 倍の命令数になる。`LeanStr` 版は結果が heap に載る長さ
  (64-bit で 17 バイト以上) のとき、確保に加えて `realloc` 1 回と文字列全体の `ptr::copy` が毎回入る
- 優先度: 中

## 問題

```rust
// LeanString
pub fn from_utf8_lossy(buf: &[u8]) -> Self {
    let mut ret = LeanString::with_capacity(buf.len());
    for chunk in buf.utf8_chunks() {
        ret.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            ret.push(char::REPLACEMENT_CHARACTER);
        }
    }
    ret
}

// LeanStr
pub fn from_utf8_lossy(buf: &[u8]) -> Self {
    LeanString::from_utf8_lossy(buf).into_lean_str()
}
```

入力が正しいかどうかに関係なく、検証はすべて `Utf8Chunks::next` が行う。

### 命令数

`5dd94ea`、rustc 1.97.0、x86-64、LTO なしの下流クレートで、callgrind の命令数を 2 通りの方法で数えた。
「fast」は下の修正例と同じこと (`str::from_utf8` が通れば `LeanString::from` / `LeanStr::from`) を公開 API で書いたもの。

測定 1: `#[inline(never)]` のラッパ関数の中だけを `--toggle-collect` で数えた (1 回あたり、確保を含み、結果の解放は含まない)。

| 入力 | `LeanString::from_utf8_lossy` | fast | `LeanStr::from_utf8_lossy` | fast (`LeanStr`) | `String::from_utf8_lossy(..).into_owned()` |
| --- | --: | --: | --: | --: | --: |
| ASCII 10 B | 198 | 132 | 247 | 132 | 223 |
| ASCII 40 B | 487 | 195 | 660 | 194 | 432 |
| ASCII 1000 B | 7320 | 734 | 7606 | 733 | 7265 |
| かな 30 B | 546 | 433 | 817 | 433 | 491 |
| かな 1000 B | 11642 | 10570 | 11928 | 10569 | 11587 |

(かな = 3 バイト文字だけの入力。非 ASCII では `from_utf8` にも速い経路が無いので差は小さい。)

測定 2: 入力をループの外で作り、呼び出しを繰り返す driver の 1 回あたり。測り方が違うので測定 1 とは絶対値が
130〜150 Ir ほどずれる (どの入力でもほぼ一定なので、結果の解放が含まれる分と思われる。未確認)。

| 入力 | `LeanString::from_utf8_lossy` | `LeanStr::from_utf8_lossy` | 参考 |
| --- | ---: | ---: | --- |
| ASCII 40 バイト | 636 | 803 | `LeanString::from_utf8`: 343、`LeanStr::from_utf8`: 335 |
| ASCII 1000 バイト | 7,457 | 7,737 | `String::from_utf8_lossy(..).into_owned()`: 7,386 |
| ASCII 1000 バイト、900 バイト目だけ不正 | 8,005 | - | `String` 版: 7,906 |
| 日本語 1000 文字 (3000 バイト) | 34,837 | - | `String` 版: 34,767 |

どちらの測定でも、ASCII 1000 バイトで fast / `from_utf8` の約 8〜10 倍になる。`String::from_utf8_lossy` も
`utf8_chunks` を使うので std と同程度ではあるが、`LeanString::from_utf8` と比べると正しい入力に対して余計に払っている。

同じバイナリの中で交互に時間も測った (`Instant`、負荷のある共有マシンなので参考値): ASCII 1000 バイトで
`LeanString::from_utf8_lossy` 約 650 ns に対し fast 約 70 ns、10 バイトで約 20 ns に対し約 14 ns。命令数の差とおおむね一致する。

### 確保回数

確保を数える `#[global_allocator]` で数えた。

| 入力 | 関数 | alloc | realloc |
| --- | --- | ---: | ---: |
| ASCII 55 バイト | `LeanString::from_utf8_lossy` | 1 | 0 |
| ASCII 55 バイト | `LeanStr::from_utf8_lossy` | 1 | 1 |
| 正しい 48 バイト | `LeanStr::from_utf8` (比較用) | 1 | 0 |
| 正しい 48 / 100 バイト | `LeanStr::from_utf8_lossy` | 1 | 1 |

## 原因

1. `Utf8Chunks::next` (core の `core/src/str/lossy.rs`、nightly の rust-src で確認) は 1 バイトずつ先頭バイトを見て
   文字の長さを判定するループで、`str::from_utf8` (`core::str::validations::run_utf8_validation`) にある
   ワード単位で ASCII を読み飛ばす処理が無い。ASCII の長い入力ほど差が開く (core のソースと命令数で確認)。
2. 正しい入力でも `push_str` を経由するので、`from` のレジスタだけの inline 経路 (`InlineBuffer::new`) や
   長さで分けた固定長コピー (`HeapBuffer::new`) を使えない。
3. `LeanStr` 版は `GrowableHeader` の heap バッファを作ってから `into_lean_str` → `HeapBuffer::into_exact` で
   `ExactHeader` に変換する。ヘッダの大きさが違うので、容量と長さが同じでも文字列を前へ `ptr::copy` してから
   `realloc` で縮める (確保回数で確認。この仕組み自体は [#14](./14-lean-str-built-via-lean-string.md) を参照)。
   正しい入力なら最初から `Repr::<Immutable>::from_str` で作れば済む。

## 修正例

先に `str::from_utf8` で検証し、通ればそのまま `from` にする。失敗した場合も `Utf8Error::valid_up_to()` までは
検証済みなので、そこまでをそのまま追記し、残りだけを `utf8_chunks` にかける (正しい先頭部分を 2 回検証しない)。
`LeanStr` 版は正しい入力を `LeanStr::from` で直接作るので、`into_lean_str` の `realloc` が無くなる。
不正な入力のときだけ従来どおり `LeanString` で作って変換する (まれな経路なので realloc は残してよい)。
遅い経路を `valid_up_to` を受け取る関数に分けて両方から呼べば、`LeanStr` 版でも `from_utf8` は 1 回で済む。

```rust
impl LeanString {
    pub fn from_utf8_lossy(buf: &[u8]) -> Self {
        match str::from_utf8(buf) {
            Ok(s) => LeanString::from(s),
            Err(e) => Self::from_utf8_lossy_slow(buf, e.valid_up_to()),
        }
    }

    fn from_utf8_lossy_slow(buf: &[u8], valid_up_to: usize) -> Self {
        let (valid, rest) = buf.split_at(valid_up_to);
        let mut ret = LeanString::with_capacity(buf.len());
        // SAFETY: `from_utf8` validated `buf[..valid_up_to]`.
        ret.push_str(unsafe { str::from_utf8_unchecked(valid) });
        for chunk in rest.utf8_chunks() {
            ret.push_str(chunk.valid());
            if !chunk.invalid().is_empty() {
                ret.push(char::REPLACEMENT_CHARACTER);
            }
        }
        ret
    }
}

impl LeanStr {
    pub fn from_utf8_lossy(buf: &[u8]) -> Self {
        match str::from_utf8(buf) {
            Ok(s) => LeanStr::from(s),
            Err(e) => LeanString::from_utf8_lossy_slow(buf, e.valid_up_to()).into_lean_str(),
        }
    }
}
```

試した結果:

- 測定 1 の「fast」列がこの形 (の公開 API で書いた同等品) で、ASCII 1000 バイトで約 1/10 になる。
- crate 内に入れた同じ方針の実装で、測定 2 の条件では次のようになった。

  | 入力 | 現状 | 修正例 |
  | --- | ---: | ---: |
  | `LeanString`、ASCII 40 バイト | 636 | 357 |
  | `LeanStr`、ASCII 40 バイト | 803 | 347 |
  | `LeanString`、ASCII 1000 バイト | 7,457 | 884 |
  | `LeanString`、900 バイト目だけ不正 | 8,005 | 2,190 |
  | `LeanString`、日本語 3000 バイト | 34,837 | 31,777 |

  正しい入力では `from_utf8` とほぼ同じになる。不正なバイトを含む入力も、その手前までは速い経路で検証されるので減る。
- `LeanStr::from_utf8_lossy` は正しい入力で alloc 1 / realloc 0 になる。
- `cargo +nightly test` (`tests/alloc_string.rs` の `test_from_utf8_lossy` と `tests/property.rs` の
  `create_from_utf8_bytes` を含む) は全件通過した。
- criterion (LTO あり、1 回の計測で値は粗い) では 40 バイトで 62.0 ns (現状 70.6 ns、std 93.9 ns)、
  1000 バイトで 204 ns (現状 827 ns、std 452 ns) だった。

効かなかった対処: 最初の `Utf8Chunk` の `invalid()` が空ならそれが入力全体なので `from(chunk.valid())` で返す方法
(std の `String::from_utf8_lossy` が `Cow::Borrowed` を返すのと同じ判定)。`LeanStr` 版の `realloc` は無くなる
(正しい 48 バイトで alloc 1 / realloc 0) が、検証が `Utf8Chunks::next` のままなので、命令数は
ASCII 40 バイトで 636 → 573、1000 バイトで 7,457 → 7,400 (測定 2) にしかならない。

## 確認方法

- テスト: `tests/property.rs` の `create_from_utf8_bytes` が `String::from_utf8_lossy` との一致を見ているが、
  `Vec<u8>` の一様な乱数はほぼ必ず先頭付近で不正になるので、正しい入力の経路と長い正しい部分の後の不正な
  バイトの経路をほとんど通らない。`any::<String>().prop_map(String::into_bytes)` を入力にした版と、
  正しい文字列の途中・末尾に不正なバイトや不完全な多バイト列を差し込んだ入力を足す。空の入力も含める。
- 確保回数: `LeanStr::from_utf8_lossy(&[b'a'; 100])` で `realloc` が 0 回になること (確保を数える
  `#[global_allocator]` を持つテスト。`tests/out_of_memory.rs` の確保を失敗させる仕組みでも、
  `realloc` が起きないことを確かめられる)。
- ベンチ: `bench/benches/apis.rs` に足す (ASCII / 非 ASCII、不正なバイトを含む入力も足すとよい)。

  ```rust
  fn from_utf8_lossy(c: &mut Criterion) {
      let mut group = c.benchmark_group("from_utf8_lossy");
      for len in [40usize, 1000] {
          let text = ascii(len);
          duplicate! {
              [
                  label         StrTy;
                  ["current"]   [lean_string::LeanString];
                  ["prev"]      [lean_string_prev::LeanString];
              ]
              group.bench_with_input(BenchmarkId::new(label, len), &len, |b, _| {
                  b.iter(|| StrTy::from_utf8_lossy(black_box(text.as_bytes())))
              });
          }
          group.bench_with_input(BenchmarkId::new("std", len), &len, |b, _| {
              b.iter(|| String::from_utf8_lossy(black_box(text.as_bytes())).into_owned())
          });
      }
      group.finish();
  }
  ```

## 関連

- [#14](./14-lean-str-built-via-lean-string.md): `LeanStr` の他の構築関数も `LeanString` + `into_lean_str` で同じ `realloc` を払う
- [#47](./47-fallible-decoders.md): `from_utf8_lossy` などに `try_` 版が無い
