# rustdoc の文言の不備 (文法・名前の誤り・説明不足) と、CI が doc をビルドしない点

- 種別: doc, infra
- 対象: `src/lib.rs` の各メソッドの doc コメント、`src/errors/reserve_error.rs`、`src/repr/heap_buffer.rs` (private の doc)、`.github/workflows/ci.yaml`
- 影響: 個々は軽微だが、rustdoc に出る文面の誤り・誤解を招く名前が複数ある。`cargo doc` は CI に無く、private 項目の doc の壊れたリンクが見過ごされている。
- 優先度: 低

## 問題

`RUSTDOCFLAGS="-D warnings" cargo doc --all-features --no-deps` (stable / nightly + `--cfg docsrs`)、`--no-default-features` の `cargo doc`、`cargo test --doc --all-features` (85 件) は、`5dd94ea` でいずれも警告・失敗なし。以下は目視と `cargo clippy -W clippy::doc_markdown` / `cargo doc --document-private-items` で見つけたもの。

### 文法・誤記

- 「The length of after extending / inserting / replacing is greater than ...」が 4 箇所 (`extend_from_within`, `insert`, `insert_str`, `replace_range` の `# Panics`)。「The length after ...」が自然。
- `truncate` の「For 2, If you want to handle ...」の `If` が大文字。
- `LeanString::from_utf16` / `from_utf16_lossy` と `LeanStr` の同名メソッドが「Decodes a slice of UTF-16 encoded **bytes**」と書くが、引数は `&[u16]` (bytes を取るのは `from_utf16le` / `from_utf16be`)。
- `LeanStr::as_bytes` は「Returns a byte slice of the `LeanStr`'s contents.」、`LeanString::as_bytes` は「Returns a byte slice containing the entire `LeanString`.」と、同じ関数なのに文面が違う。
- `LeanString::capacity` が `# Note` 見出しを使う (rustdoc の慣例の節名ではない。std は本文に書く)。`remove` の例の見出し `## Past total length:` / `## Not on char boundary:` だけ末尾にコロンがあり、他の例の見出し (`## inline capacity` など) と不統一。
- 非公開 doc: `src/repr/heap_buffer.rs` の `TextLen` の doc に `unsinged` が 2 箇所。

### 名前・説明の誤解を招くもの

- `reserve` / `reserve_exact` の `# Panics` は「the `capacity` is greater than `2^56 - 1`」と書くが、引数の名前は `additional` で、条件は `len + additional`。`try_reserve` / `try_reserve_exact` / `try_push` / `try_push_str` / `try_insert*` / `try_replace_range` / `try_extend_from_within` の「or the `capacity` is too large」も、`capacity` という名前の引数は無い。「the resulting length (`len + additional`)」と書くとよい。
- (`shrink_to` の例のコメントは [#44](./44-doc-capacity-shared-and-shrink-to-example.md) を参照)
- `LeanString::is_heap_allocated` / `LeanStr::is_heap_allocated` は、static 由来の文字列が `false` になること (`from_static_str` の例では assert されている) を書いていない。名前から「inline 以外は true」と読めてしまう。
- `LeanString::from_static_str` / `LeanStr::from_static_str` の doc は、16 バイト以下は inline にコピーされ static にならないこと、それ以外は確保なしであること、変更すると複製されることを書かない (前者は `as_static_str` の doc にだけある)。
- `LeanString::clear` の例は unique / not unique の 2 つだけで、static 由来の場合 (本文には書いてある) の例が無い。実際の挙動は `from_static_str(<17 バイト以上>)` を `clear()` (または `truncate(0)`) すると `capacity() == 0`、`is_heap_allocated() == false`、`as_static_str() == Some("")` のままになり、次の `push` で inline (容量 16) になる (確認済み)。`new()` の「容量は `2 * size_of::<usize>()`」とは異なるので、例で固定しておくとよい。
- `impl Clone` の doc は「The cloned string shares the same underlying data with the original string」だが、inline はビット単位のコピーで、static はポインタのコピー。参照カウントされるのは heap のときだけ。
- `ReserveError` の doc「An error if allocating or resizing a `LeanString` failed.」は文として不完全 (「An error that occurs when ...」)。また実際には長さ・容量の上限超過や `usize` オーバーフロー (`repeat(usize::MAX)`) でも返り、`LeanStr` の操作でも返る。`Display` の文面「Cannot allocate memory to hold LeanString」は、確認したとおり、確保を試みていない `repeat(usize::MAX)` の panic でも表示される。Rust API Guidelines (C-GOOD-ERR) は小文字始まり・末尾の句読点なしの文面を推奨し、同クレートの `FromUtf16Error` ("invalid utf-16: lone surrogate found") もそうなっている。

### private 項目の doc (`cargo doc --document-private-items` で 2 件警告)

- `src/repr/heap_buffer.rs` `TextLen` の doc の「And the tag is [`LastByte::Heap`].」: `LastByte` に `Heap` という項目は無い (`HeapMarker`)。`unresolved link`。
- 同ファイルの表中の `size_of::<usize>()` が HTML タグとして解釈される (`unclosed HTML tag usize`)。バッククォートで囲む。

### CI

`.github/workflows/ci.yaml` に `cargo doc` のジョブが無い。上のような壊れたリンクや、公開 doc のリンク切れは `cargo test` では検出されない (doctest は走る)。

## 原因

メソッドを追加するたびに近い箇所の文面をコピーしている (`reserve` -> `try_*` の「`capacity` is too large」、`remove` -> `truncate` の「For 2, If」など)。

## 修正例

上の各項目の文言の修正。CI は 1 行足すだけ:

```yaml
- name: Doc
  run: RUSTDOCFLAGS="-D warnings" cargo doc --all-features --no-deps --document-private-items
```

(`--document-private-items` を付けると上の 2 件が警告として検出される。)

## 確認方法

`RUSTDOCFLAGS="-D warnings" cargo doc --all-features --no-deps --document-private-items` が通ること、`cargo test --doc --all-features` が通ること。
