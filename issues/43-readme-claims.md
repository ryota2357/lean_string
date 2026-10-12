# README (クレートのトップページ) の記述が実際の挙動と食い違う・足りない

- 種別: doc
- 対象: `README.md` (`src/lib.rs` の `#![doc = include_str!("../README.md")]` でクレートドキュメントにもなる)
- 影響: README の "Properties" と例を読んだだけの利用者が、`&'static str` からの構築が確保なしだと思って `LeanString::from("...")` を使ったり、`no_std` の使い方や feature の存在に気づけなかったりする。
- 優先度: 低

## 問題

1. `- O(1), zero allocation construction from &'static str.` (Properties)
   - これが成り立つのは `LeanString::from_static_str` だけである。`From<&str>` は `'static` を区別できず (ライフタイムは型情報に現れない)、17 バイト以上なら確保する。確認 (`5dd94ea`、カウント付きアロケータ): `LeanString::from("<45 バイトの &'static str>")` と `"..".to_lean_string()` は確保 1 回、`from_static_str` は 0 回。README の本文にも例にも関数名が出てこない。
2. `- Stores up to 16 bytes inline ... Strings larger than that are stored on the heap.`
   - `from_static_str` で作った 17 バイト以上の文字列は heap ではなく static (`is_heap_allocated() == false`、`as_static_str() == Some(..)`、`capacity() == len()`) である。変更すると heap に複製される。README の 3 種類の記憶域の説明が 2 種類になっている。
3. `- Supports no_std environment.`
   - `default-features = false` で `std` feature を切る必要があること、`alloc` が必須であることが書かれていない。`std` feature が有効にするのは `AsRef<OsStr>` / `AsRef<Path>` だけである。
   - `serde` / `arbitrary` feature が README のどこにも出てこない (docs.rs の `all-features` でだけ分かる)。
4. 例の最後の `assert_eq!(large  + "!", cloned);`
   - `+` は `large` を消費するので、CoW の要点である「`cloned` を変更しても `large` は変わらない」ことを示していない。また `large  +` と空白が 2 個ある (rustfmt の対象外のため残っている)。
5. `- High API compatibility for String.` に差分への案内が無い ([#41](./41-std-differences-docs.md) 参照)。
6. `Send + Sync` であること、MSRV (badge はあるが本文に無い) の記載が無い。
7. 付随事項: LeanStr の説明の `[its documentation](https://docs.rs/lean_string/latest/lean_string/struct.LeanStr.html)` は絶対 URL なので、特定バージョンの docs.rs や `cargo doc` ローカルでは `latest` に飛ぶ。GitHub / crates.io 上の README でも使うため intra-doc link にはできないので、現状のままでもよい (指摘のみ)。

## 原因

README は `LeanStr` や `from_static_str` が加わる前の記述が中心で、0.7.0 / 0.8.0 の変更 (479ce91 の docs 修正を含む) で関連する行だけを直してきた。`From<&str>` が `'static` を活かせない点は、構築パスの選択肢 (`from_static_str` / `From<&str>`) を並べて説明する場所が無い。

## 修正例

- Properties の該当行を差し替える:
  ```markdown
  - `O(1)`, zero allocation construction from `&'static str` with `LeanString::from_static_str()`.
    (`LeanString::from(&str)` copies the text, even for a `'static` one, if it is longer than the inline size.)
  ```
  inline の説明は「Strings larger than that are stored on the heap, or point to the `&'static str` if created by `from_static_str()`.」とする。
- 例に静的文字列を足す:
  ```rust
  // A `&'static str` longer than the inline size is not copied or allocated.
  const GREETING: LeanString = LeanString::from_static_str("This text is not stored inline, nor allocated");
  assert!(!GREETING.is_heap_allocated());
  ```
  (`const X: LeanString = LeanString::from_static_str(..)` がコンパイルできることは確認した。例として doctest に入れるときは未検証なので実行して確かめる。)
- CoW の例を `large` を残す形にする:
  ```rust
  cloned.push('!');
  assert_eq!(cloned, "This is not long, but can't be stored inline!");
  assert_eq!(large, "This is not long, but can't be stored inline"); // `large` is untouched.
  ```
- `no_std` の行に「(requires `alloc`; disable the default `std` feature)」、feature の一覧 (`std` (default), `serde`, `arbitrary`) と MSRV を足す。

## 確認方法

README は doctest になるため、変更後に `cargo test --doc --all-features` を実行する (現状は 85 件通過)。静的文字列の主張は例中の `assert!(!GREETING.is_heap_allocated())` と `as_static_str().is_some()` で検査される。
