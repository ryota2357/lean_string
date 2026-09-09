# 13: テストと unsafe 契約の穴を埋める

種別: テスト + doc / 実装量: 小〜中 / 設計判断: 小

## A. realloc 失敗の経路がまったくテストされていない

`tests/out_of_memory.rs` には確保と再確保を 1 回だけ失敗させる仕組みがある。

```rust
static FAIL_NEXT_ALLOCATION: Cell<bool> = const { Cell::new(false) };
static FAIL_NEXT_REALLOCATION: Cell<bool> = const { Cell::new(false) };
```

しかし `FAIL_NEXT_REALLOCATION` を `true` にするのは `without_allocating` ヘルパ
(tests/out_of_memory.rs:56-66) だけで、そこは「再確保が起きなかったこと」を確認する用途。
つまり**再確保が失敗する経路を通るテストが 1 つも無い**。

未検証のまま残っているもの:

| 場所 | 到達経路 | 内容 |
| --- | --- | --- |
| `HeapBuffer::into_exact` の失敗時復元 (heap_buffer.rs:507-522) | unique な heap の `LeanString` に対する `try_into_lean_str()` | このクレートでもっとも入り組んだ unsafe。`ptr::copy` でヘッダを潰した後に元へ戻し、長さ prefix を書き直して `GrowableHeader` を再構成する。誤っていても即座にクラッシュせず、壊れたバッファが生き残る |
| `HeapBuffer::into_growable` の失敗時 (heap_buffer.rs:614-617) | unique な heap の `LeanStr` に対する `try_into_lean_string()` | 同種 |
| `Repr::reserve` の unique heap grow (repr.rs:479-486) | unique な heap に対する `try_reserve` / `try_push_str` | `realloc` が失敗したとき |
| `Repr::shrink_to` のその場縮小 (repr.rs:561-564) | unique な heap に対する `try_shrink_to` | 同上 |

4 つとも到達可能で、いま書けば通ることをスクラッチのコピーで確認した
(通常の `cargo test` と `cargo +nightly miri test` の両方。Miri は UB もリークも報告しない)。

**ただし観測できる範囲が 2 つに分かれる**。

`try_reserve` と `try_shrink_to` は `&mut self` を取って返るので、失敗後の状態を
そのまま検査できる。

```rust
// unique heap、capacity 20 に 19 バイト
FAIL_NEXT_REALLOCATION.set(true);
assert!(s.try_reserve(1000).is_err());
assert_eq!(s.as_ptr(), before);      // ポインタが変わっていない
assert_eq!(s.capacity(), 20);        // 容量も元のまま
assert_eq!(s, "0123456789abcdefgh"); // 内容も無事
assert!(s.is_heap_allocated());
```

これは `GlobalAlloc::realloc` の「失敗したら元のブロックはそのまま」という契約が
このクレートの経路で守られていることの、端から端までの確認になる。

一方 `try_into_lean_str` / `try_into_lean_string` は `self` を値で取り、失敗時には
`ReserveError` しか返さない (lib.rs:1243-1254、lib.rs:1733-1744 の `mem::replace` +
復元)。つまり呼び出し側からは**文字列の内容を検査できず、「1 回だけ drop されて
リークしない」ことしか観測できない**。`into_exact` の失敗復元がもっとも入り組んだ
unsafe なのに、そこがいちばん検査しにくい。

- 短期的には「リークしないこと」をカウントするアロケータで固定する。
  グローバルなカウンタになるので `--test-threads=1` が要る (並行するテストの
  確保が混ざることを確認した)。
- [plans/12](./12-mutability-conversion.md) の A で `&mut self` を取る形に変えれば、
  `try_reserve` と同じように内容も検査できるようになる。**これは 12-A を採る理由の
  1 つとして数えてよい**。

`into_exact` の失敗復元は 32-bit で長さ prefix の有無が変わるので、
Miri の i686 / powerpc ターゲットでも通ることを確認したい。

このテストは [plans/12](./12-mutability-conversion.md) の A で
`into_exact` / `into_growable` の形を変えるときの安全網になるので、先に入れる。

## B. `fmt::Write::write_fmt` の fast path が一度も実行されていない

`impl fmt::Write for LeanString` の `write_fmt` (lib.rs:2508-2522) には
「引数の無いリテラルなら static バッファに差し替える」経路がある。

```rust
match args.as_str() {
    Some(s) => {
        if self.is_empty() && !self.is_heap_allocated() {
            *self = LeanString::from_static_str(s);
        } else {
            self.push_str(s);
        }
        Ok(())
    }
    None => fmt::write(self, args),
}
```

`tests/` に `write!` / `writeln!` が 1 つも無いので、`args.as_str()` が `Some` になる
ケースがテストで一度も通っていない。

足すべきケース:

- 空の inline 文字列に `write!(s, "literal")` → static バッファになること
- 空の static 文字列に対して同じことをした場合
- 空でない文字列に対して (fast path に入らず追記されること)
- **確保済みの空の heap 文字列に対して** — `!self.is_heap_allocated()` のガードが
  効いて容量が保たれること。このガードを外すと確保済み容量が捨てられるので、
  退行テストとして価値がある

`src/traits.rs` の `try_from_fmt` にも同じ `args.as_str()` の fast path があるが、
**こちらは到達不能**であることを確認した。

`fmt::Arguments::as_str()` が `Some` を返すのは、`format_args!` 全体がコンパイル時に
固定文字列へ畳める場合に限る。リテラルを直接埋めた `format_args!("{}", "literal")` は
`Some` になるが、変数を埋めた `format_args!("{s}")` は `s` の実行時の値が
`&'static str` であっても `None` になる。`try_from_fmt` の 2 つの呼び出し元
(traits.rs:77 と traits.rs:159) はどちらも `match_type!` の catch-all arm にあり、
generic な `T: fmt::Display` の変数を埋めるので、常に `None` になる。

スクラッチのコピーで `Some(str)` の arm に `panic!()` を置き、動くテスト全部 (70 以上) と
doctest 71 件を走らせても一度も発火しなかった。`tests/out_of_memory.rs` は
独自の `Display` 実装 3 種でこの関数を通しているが、それでも到達しない。

**残しておく問題は、到達したときに `from_static_str` が panic しうること**。
fallible な API の中にある以上これは契約違反になる。到達不能なら消し、
残すなら `try_from_static_str` 相当を使う。どちらにするか決める。

## C. `LeanStr` の trait 実装にテストが無い

`tests/lean_str.rs` はテストが 5 つしかない (`new_empty` / `from_char` /
`from_around_inline_limit` / `from_static_str_around_inline_limit` /
`clone_shares_heap_buffer`)。

- `Hash` / `Borrow<str>` のテストが無い。`HashMap<LeanStr, _>` に `&str` で引ける
  (`Borrow<str>` + `Hash` + `Eq` の整合) ことは実装上は正しいが、固定するテストが無い。
  リポジトリ全体で `HashMap` / `HashSet` / `BTreeMap` / `BTreeSet` を使ったテストは 1 つも無い。
- **`Ord` / `PartialOrd` は `LeanString` と `LeanStr` のどちらもまったくテストされていない**。
  `.cmp(` や `.sort()` を使うテストがリポジトリに存在しない。
  [plans/07](./07-equality-policy.md) の案1 を採ると `Ord` の実装を差し替えることになり、
  「ポインタ一致だが長さが違う ⇒ 長さが順序を決める」という間違えやすい規則が入るので、
  その前に網を張っておきたい。

## D. `Extend` の大半にテストが無い

`tests/` で実行されている `extend` は 3 種類だけ。

| impl | テスト |
| --- | --- |
| `Extend<char> for LeanString` | あり (tests/lean_string.rs:757、tests/alloc_string.rs:574) |
| `Extend<&char> for LeanString` | あり (tests/alloc_string.rs:775) |
| `Extend<&str> for LeanString` | あり (tests/alloc_string.rs:581、`vec![u]` の `u` は `&str`) |
| `Extend<LeanString> for LeanString` | 間接的にあり (`FromIterator<LeanString> for LeanString` が使う) |
| `Extend<Box<str>> for LeanString` | **無し** |
| `Extend<Cow<str>> for LeanString` | **無し** |
| `Extend<String> for LeanString` | **無し** |
| `Extend<LeanStr> for LeanString` | **無し** |
| `Extend<LeanString> for String` | **無し** |
| `Extend<LeanStr> for String` | **無し** |

[plans/04](./04-append-writer.md) がこれらの実装をまとめて書き換えるので、
先に網を張っておきたい。

## C-2. main で足した API のテストが薄い

v0.7.0 以降に足した公開 API のテスト状況を調べた。

| 対象 | 統合テスト | doctest |
| --- | --- | --- |
| `from_utf16le` / `be` とその lossy (両型) | `tests/property.rs` のみ | あり (8 通りすべて) |
| `as_static_str` (両型) | **無し** | あり |
| `AsRef<Path>` (両型) | **無し** | **無し** (doc コメント自体が無い) |
| `FromIterator` の 14 impl | 一部のみ (下表) | 無し |
| `try_from_fmt` の OOM 処理 | あり (`tests/out_of_memory.rs` の 3 テスト) | — |

`from_utf16le` / `be` は `tests/property.rs` にしかテストが無く、そのファイルは
[plans/08](./08-test-toolchain.md) のとおり rustc 1.98.0 未満ではコンパイルできない。
つまり手元の安定版によっては、この 4 メソッドを実際に動かすテストが doctest だけになる。

`AsRef<Path>` は 1 行の実装だが、テストも doc コメントも無い。

`FromIterator` は 14 impl のうちテストがあるのは 3 つだけ。

| impl | テスト |
| --- | --- |
| `FromIterator<char> for LeanString` | あり |
| `FromIterator<&str> for LeanString` | あり |
| `FromIterator<LeanString> for LeanString` | あり (1 要素の再利用、共有の detach、空の 3 ケース) |
| `FromIterator<String> for LeanString` | `tests/property.rs` のみ |
| 残り 10 (`LeanStr` 向け 8 と `String` 向け 2、`Box<str>` / `Cow` 向け) | **無し** |

[plans/12](./12-mutability-conversion.md) の B が
`FromIterator<LeanStr> for LeanStr` を書き換えるので、そこは先にテストを足す。

## E. static バッファに対する `clear` のテストが無い

`tests/lean_string.rs` の `clear_cow` (737-752) は inline と heap しか見ていない。
static バッファでは `clear()` が容量を 0 にする ([plans/09](./09-doc-fixes.md) の A) ので、
doc の修正と合わせてテストを足す。

`pop` / `truncate` / `push` / `insert` / `shrink_to` については static バッファの
テストが既にある (`pop_from_static` / `pop_from_static_cow` / `truncate_from_static` /
`push_to_static` / `insert_to_static` / `shrink_to_static_buffer`)。`clear` だけが抜けている。

## F. `retain` が何も削らない場合のテストが無い

`retain_cow` (tests/lean_string.rs:535-549) は heap と static のどちらも
「実際に文字が削られる」ケースしか見ていない。
[plans/06](./06-retain-shared-fast-path.md) で追加するテストがここを埋める。

なお述語の呼び出し回数を数えるテスト (`retain_f_apply_count`, :516) は既にあるので、
06 の変更で回数が変わらないことはそれで守られる。

## G. `tests/const.rs` が static バッファを const 評価していない

`from_static_str` は 16 バイト以下を inline バッファにする (repr.rs:110-121) ので、
`tests/const.rs` が使っている "hello world" (11 バイト) と長さ 0..=16 のループは
すべて inline 経路を通る。

長さ 16 超の `from_static_str` から `as_str()` / `len()` を const で取り出す経路
(`StaticBuffer::new` と `tail_word()`、`slice::from_raw_parts` を const 評価で通る) が
固定されていない。`as_bytes` や `StaticBuffer` を将来触ったときに黙って壊れる。

## H. unsafe の契約が書かれていない箇所

安全性の問題は見つかっていないが、根拠が近くに書かれていない場所がある。
このクレートは他の場所では丁寧に書いているので、揃えておきたい。

### `# Safety` が無い `unsafe fn`

| 場所 | 内容 |
| --- | --- |
| `Repr::as_inline_buffer_mut` (repr.rs:371) | 呼び出し元がすべて `is_inline_buffer()` を確認済みであることが前提 |
| `Repr::as_heap_buffer` (repr.rs:380) / `as_heap_buffer_mut` (repr.rs:389) | 同じく `is_heap_buffer()` |
| `Repr::as_static_buffer` (repr.rs:398) / `as_static_buffer_mut` (repr.rs:407) | 同じく `is_static_buffer()` |
| `HeapBuffer::allocation` (heap_buffer.rs:301) | 確保の先頭アドレスを復元する関数。`dealloc` / `realloc` / `into_exact` / `into_growable` の 4 か所がその結果と `alloc_layout()` の一致に依存している。必要な不変条件 (「`self.ptr` は `allocate_ptr` が作った確保の、`header_offset` バイト (+ `has_len_prefix` なら `usize` 1 個) 先を指す」) を書き、`into_exact` と `into_growable` にある議論から参照する |

### `SAFETY:` コメントが無い、または対象がずれている箇所

| 場所 | 内容 |
| --- | --- |
| `HeapBuffer::header` (heap_buffer.rs:312) | **safe fn** の中で生ポインタを参照へ変換している。`&GrowableHeader` を作れるということは非 atomic な `capacity` フィールドに触れられるということで、それが安全なのは「`capacity` は unique なときにしか書かれない」から。その根拠がどこにも書かれていない |
| `Repr::from_inline` / `from_heap` / `from_static` の `transmute` (repr.rs:328/336/344) | 直後の `SAFETY:` コメントは `assert_unchecked` の根拠であって transmute の根拠ではない。とくに `from_inline` は任意の UTF-8 バイト列を `*const ()` フィールドに載せる。生ポインタには妥当性要件が無く、`as_bytes` / `as_mut_ptr` は `last_byte >= HeapMarker` のときしかフィールド 0 をポインタとして読まないので正しいが、その説明が無い |
| `Repr::make_shallow_clone` の `ref_count_overflow` (repr.rs:267-271) | `unsafe { ptr::read(repr) }.replace_inner(Repr::new())` に `SAFETY:` が無い。直前の `fetch_add` で作った参照を消費して、panic で巻き戻る前にカウントを釣り合わせる、というのが意図 |
| `HeapBuffer::dealloc` の本体 (heap_buffer.rs:233) | `SAFETY:` コメントが無い |
| `realloc` の 32-bit layout 変換 arm (heap_buffer.rs:401-405) | `realloc` の他の部分は丁寧に書かれているのに、この arm だけ `SAFETY:` が無い |
| `LeanString::from_utf8_unchecked` (lib.rs:208)、`LeanStr::from_utf8_unchecked` (lib.rs:1431) | 本体に "SAFETY: From `# Safety`, ..." の 1 行が無い。他の同種 API は書いている |
| `from_utf16le` / `_lossy` / `from_utf16be` / `_lossy` の `buf.align_to::<u16>()` (lib.rs:310, 338, 383, 411) | `SAFETY:` コメントが無い。`u16` には無効なビットパターンが無いので健全だが、その説明が無い |
| `Repr::from_char` (repr.rs:92-96) | `char::encode_utf8` の結果が必ず 4 バイト以下で `MAX_INLINE_SIZE` に収まる、という根拠が書かれていない |

### `LastByte` の網羅性

`Repr` はフィールド 2 を `LastByte` 型で持つ (repr.rs:52)。この enum の variant は
`0x00..=0xD1` しか無いので、バイト 15 が `0xD2..=0xFF` になった `Repr` は即座に UB になる。

書き手を追うと不変条件は保たれている。

- `InlineBuffer::new` / `empty` は `len | 0xC0` (= `0xC0..=0xCF`) を書く。
  ただし `len == MAX_INLINE_SIZE` のときはバイト 15 が文字列の最終バイトそのもので、
  有効な UTF-8 の末尾バイトは必ず `0xC0` 未満。
- `HeapBuffer` の `TextLen` と `StaticBuffer` の `len` は必ず
  `HeapMarker` / `StaticMarker` を OR する。
- `push_str` / `insert_str` / `remove` / `retain` は `as_mut_ptr` 経由で書くが、
  inline のときバイト 15 に届くのは `len == 16` の場合だけで、そこも UTF-8 の末尾バイト。

バイト 15 に書き込む経路を全部洗い出して、どれも `LastByte` の定義済みの値に
収まることを確認した。

| 書き手 | 書く値 |
| --- | --- |
| `InlineBuffer::new` (LE64 版と可搬版の両方) | `0xC0 + len`、`len == 16` のときは文字列の最終バイト |
| `InlineBuffer::empty` | `Length00` (`0xC0`) |
| `InlineBuffer::set_len` | `0xC0 + len` (`len < 16` のときだけ。16 なら no-op) |
| `TextLen::new` | 上位バイトに `HeapMarker` (`0xD0`) を OR。32-bit の `ON_THE_HEAP` 経路も同じ |
| `HeapBuffer::set_len` / `realloc` / `into_exact` / `into_growable` | 同じ `TextLen` を引き継ぐだけで、独自に作り直さない |
| `StaticBuffer::new` / `set_len` | 上位バイトに `StaticMarker` (`0xD1`) を OR |

`src/repr/last_byte.rs` の冒頭コメントは UTF-8 の符号化規則は説明しているが、
「この enum の網羅性が `Repr` の安全性要件になっている」ことは書かれていない。
`Repr` の定義のところに 1 段落足したい。上の表がそのまま根拠になる。

## 進め方

A → B → D → H の順が効率的。

1. **A** は退行時の被害がもっとも大きい経路をカバーする。
   [plans/12](./12-mutability-conversion.md) の A の前提でもある。
2. **B** は小さく、`write_fmt` のガードという退行しやすい箇所を守る。
   `try_from_fmt` の到達不能な分岐をどうするかの判断もここで済ませる。
3. **D** は [plans/04](./04-append-writer.md) の前提。
4. **H** はコードを変えないので、他のタスクと並行して進められる。

C / E / F / G は気づいたときに足していく。E と F はそれぞれ
[plans/09](./09-doc-fixes.md) と [plans/06](./06-retain-shared-fast-path.md) の中で
自然に片付く。
