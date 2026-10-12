# `into_lean_str` / `into_lean_string` の doc が「in place で realloc」と書くが、実際は常に文字列全体を動かす O(n) の変換である

- 種別: doc
- 対象: `src/lib.rs` の `LeanString::into_lean_str` / `try_into_lean_str`、`LeanStr::into_lean_string` / `try_into_lean_string`
- 影響: `into_lean_str` は「unique なら O(1) に近い」と読めるが、heap の文字列では毎回 `ptr::copy` と `realloc` が入る。
  `into_lean_string` の結果に余りの容量が無いことも書かれていないので、変換直後の追加で必ずもう一度 realloc する
- 優先度: 低

## 問題

`LeanString::into_lean_str` の doc (`5dd94ea`):

> A heap-allocated string is moved into [`LeanStr`]'s heap layout, dropping any spare capacity (much like
> [`String::into_boxed_str()`]): the buffer is reallocated in place if it is not shared, and copied otherwise.

`LeanStr::into_lean_string` の doc:

> A heap-allocated string is moved into [`LeanString`]'s heap layout: the buffer is reallocated in place if it is not shared,
> and copied otherwise.

実際の unique な heap の変換は次のとおり (コードで確認)。

- `into_lean_str` (`Repr::into_immutable` → `HeapBuffer::into_exact`): ヘッダの大きさが違う (64-bit で `GrowableHeader` 16 バイト、
  `ExactHeader` 8 バイト) ので、文字列全体を `ptr::copy` で前に詰め、その後 `realloc` を呼ぶ (アロケータ次第で別の場所に移り、
  そこでもう一度コピーされうる)。容量に余りが無くても省けない。
- `into_lean_string` (`Repr::into_mutable` → `HeapBuffer::into_growable`): ヘッダが大きくなるので、`realloc` で領域を広げてから文字列を
  `ptr::copy` で後ろへずらす。

つまりどちらも常に O(n) で、「in place」(移動しない、あるいは O(1)) とは限らない。`String::into_boxed_str()` は容量と長さが同じなら
何もしないので、"much like" の比較もこの点では当たらない。

さらに `into_lean_string` の変換後の容量は長さと同じ (`tests/conversion.rs` の `unique_heap_str_to_string` が
`assert_eq!(lean_string.capacity(), text.len())` で確かめている) なので、変換直後の追加は必ずもう一度 realloc する。確保を数える
`#[global_allocator]` で、48 バイトの `LeanStr` を `into_lean_string` して 1 文字 `push` すると realloc 2 回だった。この点も doc に無い。

README の "converting between the two is not always free" と `LeanStr` の型の doc はこの挙動と合っている。

## 原因

doc が `realloc` を使うことだけを書き、2 つのレイアウトの間でデータを動かす必要があること (`into_exact` / `into_growable` の `ptr::copy`) を
書いていない。

## 修正例

`into_lean_str` の doc は、たとえば次のようにする。

```rust
/// A heap-allocated string is moved into [`LeanStr`]'s heap layout, dropping any spare
/// capacity (much like [`String::into_boxed_str()`]). Because the two layouts have headers of
/// different sizes, this moves the whole content and calls `realloc`, so it is O(n) even when the
/// buffer is not shared; a shared buffer is copied instead. If the content is short enough to be
/// stored inline, the result is inline instead.
```

`into_lean_string` には「unique なら `realloc` と内容の移動 (O(n))、共有されていればコピー。結果の容量は長さと同じなので、
続けて追加すると再確保される」と書く。`try_*` 版は本体の doc を参照しているので、そのままでよい。

挙動の側を変える (`LeanStr` の構築で `LeanString` を経由しない、変換の失敗時の扱いを簡単にする) 話は
[#14](./14-lean-str-built-via-lean-string.md) と [#09](./09-conversion-restore-on-failure.md) で扱う。どちらが入っても、ヘッダの大きさが
違う以上、unique な heap の変換が O(n) である点は変わらないので、doc の修正は独立に必要である。

## 確認方法

`cargo doc` で該当箇所を確認する。doc の記述 (容量 == 長さ) は既存の `unique_heap_str_to_string` が守っている。

## 関連

- [#09](./09-conversion-restore-on-failure.md): `into_lean_str` / `into_lean_string` の失敗時の扱いとコード生成
- [#14](./14-lean-str-built-via-lean-string.md): `LeanString` を経由する `LeanStr` の構築関数が払う `into_exact` のコスト
- [#41](./41-std-differences-docs.md): `From<String>` / `From<Box<str>>` / `From<Cow<'_, str>>` がコピーであることの doc
