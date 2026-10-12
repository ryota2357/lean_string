# `LeanString` 型自体のドキュメントが 1 行しかなく、記憶域・容量・CoW・panic 方針が型の docs から分からない

- 種別: doc
- 対象: `src/lib.rs` の `LeanString` (struct の doc コメント)、対比として `LeanStr` の doc コメント
- 影響: docs.rs で `LeanString` のページを開いた利用者は、inline / heap / static の 3 つの記憶域、`capacity()` の意味、どの操作が確保・複製するか、panic と `try_` の方針、スレッド安全性を、型のページから読み取れない (README はクレートのトップページにしか表示されない)。
- 優先度: 中

## 問題

`LeanString` の doc は「Compact, clone-on-write, UTF-8 encoded, growable string type.」の 1 文のみ。対して `LeanStr` は、サイズ、inline 記憶域、O(1) の clone、`LeanString` との表現の違いと相互変換のコスト、使い分けまでを段落で説明している。

型のページからは分からないが、コードで確認できる性質 (`5dd94ea`):

1. 記憶域は inline (最大 `2 * size_of::<usize>()` = 16 / 8 バイト)、heap (参照カウント付き)、static (`&'static str` を指すだけ) の 3 種類。`capacity()` は inline で常に 16 (32-bit では 8)、static では `len()` と等しい。docs の例ごとに `2 * size_of::<usize>()` と書かれ (12 箇所)、`INLINE_CAPACITY` のような定数は無い ([#54](./54-small-additions.md) 参照)。
2. 変更系メソッドが確保・複製するのは「heap で共有中」「static」のときで、inline では起きない。例えば `reserve(1)` は共有中のヒープ文字列で必ず 1 回確保する (カウント: 1)。静的文字列への `push` も 1 回確保する。`reserve(0)` / 空文字列の `push_str` / 何も除去しない `retain` などは共有・static のまま (確保 0)。この規則は個々のメソッドの docs に分散している。
3. `try_*` は失敗時に `self` を変更しない。`try_reserve`, `try_reserve_exact`, `try_push`, `try_push_str`, `try_insert`, `try_insert_str`, `try_replace_range`, `try_remove`, `try_make_ascii_uppercase`, `try_extend_from_within`, `try_retain`, `try_split_off` で、共有ヒープと静的文字列の両方で確認済み (失敗アロケータ)。記述があるのは `try_push_str` / `try_extend_from_within` だけ。`try_truncate` / `try_pop` は 64-bit では確保しないので常に `Ok`。
4. 失敗するのは確保だけでなく長さ上限 (64-bit で `2^56 - 1`、32-bit で `2^31 - 16`、`LeanStr` は `2^31 - 12`、`from_static_str` は 64-bit で `2^56 - 1`、32-bit で `2^24 - 1`) でもある。上限は各メソッドにばらばらに書かれている。値の導出: 32-bit の `LeanString` は header 8 + capacity + 長さの heap 保存用 4 バイトが `Layout` の上限 `isize::MAX` を align 4 に切り上げた値以下 (`2^31 - 4`) になる必要があるので `cap <= 2^31 - 16`、`LeanStr` は header が 4 バイトなので `2^31 - 12`。
5. `Send + Sync` (参照カウントは atomic。`unsafe impl Send/Sync`)。docs に記述が無い。
6. panic 方針: 確保失敗・容量超過・index 違反は panic。`From` / `FromIterator` / `Extend` / `Add` / `fmt::Write` / `Clone` (refcount overflow) など、`try_` 版を持てないトレイト impl も同様 ([#39](./39-panics-doc-gaps.md) 参照)。
7. `Deref<Target = str>` なので `str` のメソッドがそのまま使えるが、`&mut str` は得られない ([#41](./41-std-differences-docs.md) 参照)。

## 原因

README (クレートのトップページ) に性質を書き、個々のメソッドの docs に例外を書く構成のまま、型のページ向けの総論が作られていない。`LeanStr` を追加するときに `LeanStr` 側だけ型の docs が書かれた。

## 修正例

`LeanString` の doc コメントを拡張する。構成案 (見出しと要点。英語):

```rust
/// Compact, clone-on-write, UTF-8 encoded, growable string type.
///
/// # Storage
/// A `LeanString` is two words long and stores its text in one of three ways:
/// - **inline**: up to `2 * size_of::<usize>()` bytes (16 on 64-bit, 8 on 32-bit) live in the value
///   itself. `capacity()` is that size.
/// - **heap**: a reference-counted buffer like `Arc`. `clone()` is O(1) and shares it.
/// - **static**: created by [`LeanString::from_static_str`] when longer than the inline size; it
///   points at the `&'static str`. Its capacity equals its length.
///
/// # Clone-on-write
/// Methods that modify the string copy the buffer first if it is shared or static, so they may
/// allocate (and panic / return an error on failure) even if the capacity would be enough. Inline
/// strings never allocate for this reason. ...
///
/// # Panics and `try_` methods
/// ... every method that allocates has a `try_` twin returning [`ReserveError`]; on error the string is unchanged ...
///
/// # Limits
/// ... 2^56 - 1 / 2^31 - 16 ...
///
/// # Thread safety
/// `LeanString` is `Send + Sync` ...
///
/// # Differences from `String`
/// (differences listed in #41)
```

`LeanStr` の記述 (「`LeanStr` vs a newtype of `LeanString`」) は現状のままで足りる。

## 確認方法

- `cargo test --doc` で追加した例が通ること、`RUSTDOCFLAGS="-D warnings" cargo doc --all-features --no-deps` で警告が無いこと (現状は両方クリーン: doctest 85 件通過、警告 0)。
- 記述した性質ごとに、既存のテスト (`tests/out_of_memory.rs`、`tests/lean_string.rs`) で保証されているかを確認し、足りないもの (「`try_*` は失敗時に `self` 不変」「`try_truncate` / `try_pop` は 64-bit で確保しない」) はテストとして足す。
