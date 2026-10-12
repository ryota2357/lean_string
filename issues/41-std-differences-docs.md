# `String` との違い・提供しない API とその理由、所有権を取る変換がコピーであることがどこにも書かれていない

- 種別: doc
- 対象: `src/lib.rs` の `LeanString` / `LeanStr` の型ドキュメント、`From<String>` / `From<Box<str>>` / `From<Cow<'_, str>>`
  (`LeanString` と `LeanStr` の両方)、`README.md` の "High API compatibility for `String`."
- 影響: README は「`String` との高い API 互換性」をうたうが、利用者が最初に踏む差分 (コンパイルエラーになるもの、黙って `String` を
  返す・コピーするもの) を知る場所が無い。特に `String` からの変換がバッファを引き継ぐ O(1) だと思って使うと、長い文字列ごとにコピーと
  確保・解放が起きる。issue や質問の定番になる項目で、設計判断なので文書化するだけでよい
- 優先度: 中

## 問題

以下はすべて `5dd94ea` / rustc 1.97.0 で確認した差分で、いずれもドキュメントに記述が無い。

### コンパイルできない (`String` にはある)

| `String` の機能 | `LeanString` | 備考 |
| --- | --- | --- |
| `DerefMut`, `AsMut<str>`, `BorrowMut<str>`, `IndexMut`, `as_mut_str` | 無い (`E0596: cannot borrow data in dereference of LeanString as mutable`) | 共有・static の buffer に `&mut str` を渡すには CoW の複製が要り、確保 (= panic しうる) が暗黙に入る。`make_ascii_*` や `replace_range` で用が足りる場面が多い。設計判断の余地は [#51](./51-as-mut-str.md) を参照 |
| `into_bytes`, `into_boxed_str`, `leak`, `from_raw_parts`, `as_mut_vec` | 無い | heap 領域は `[count, capacity][data]` の独自レイアウトで `Vec`/`Box` の確保と互換が無い。inline / static は確保自体が無い。ゼロコピーでは作れない。コピーする変換は [#48](./48-std-conversions.md) を参照 |
| `drain` | 無い | 作れる。[#50](./50-drain.md) を参照 |
| `&String` を `Pattern` として使う (`s.contains(&other)`) | `&LeanString` / `&LeanStr` は `Pattern` を実装しない (`E0277: &LeanString: Pattern is not satisfied`) | `Pattern` は unstable で外部クレートから実装できない。`s.contains(other.as_str())` を使う |
| `From<String>` -> `Box<str>` / `Rc<str>` / `Arc<str>` / `Vec<u8>` / `Box<dyn Error>` / `OsString` / `PathBuf` | `LeanString` -> 同じ型は無い | [#48](./48-std-conversions.md) を参照 |

### コンパイルはできるが `String` と異なる

- `LeanString::from_utf8(&[u8]) -> Result<Self, Utf8Error>`。`String::from_utf8(Vec<u8>) -> Result<String, FromUtf8Error>` と引数・エラー型が
  違い、`into_utf8_lossy` / `into_bytes` で元のバイト列を取り戻せない。`from_utf8_lossy` は `Cow` ではなく `Self` を返す。
- `FromStr` の `Err` は `ReserveError` (`String` は `Infallible`)。
- `From<&str>` / `From<String>` / `From<Box<str>>` / `From<Cow<'_, str>>` は常にコピーする。`From<String>` / `From<Box<str>>` /
  `Cow::Owned` (`From<String>` に委譲) はどれも `Repr::from_str(&value)` で中身をコピーしてから元を解放する。`String::from(Box<str>)` が
  O(1) なのとは違う。確保を数える `#[global_allocator]` で確認: 40 バイト・48 バイトの `String` から `LeanString::from`、48 バイトの
  `Box<str>` から `LeanStr::from` は、どれも alloc 1 / dealloc 1 (元の解放) で、バッファは引き継がれない。heap のレイアウトが参照カウントのヘッダをデータの前に置いている以上、`String` のバッファは引き継げないので、挙動は変えられない。
- `From<&'static str>` も 17 バイト以上なら 1 回確保する (`From` 経由では static にならない)。確保なしで作れるのは `LeanString::from_static_str` のみ。
  `Cow<'static, str>` の `Borrowed` も `From<Cow<'_, str>>` が `'static` を区別できないためコピーになる。逆向きの
  `From<LeanString> for Cow<'a, str>` は static なら確保なし (`Cow::Borrowed`) なので、往復すると static でなくなる。
- `From<LeanString> for String` / `From<&LeanString> for String` は heap の buffer を流用せず、常にコピーする (確保 1 回)。こちらは `String` の
  レイアウト上当然で、驚きは少ない。
- `Deref<Target = str>` の `str` メソッドのうち、新しい文字列を作るものは `String` を返す: `replace`, `replacen`, `[LeanString]::concat/join`,
  `to_string()` (0.8.0 で `to_lowercase` 等は `LeanString` を返すようになった)。API 側の議論は [#53](./53-str-methods-returning-string.md) を参照。
- `TryFrom<&str>` は `From` からの blanket 実装 (`Error = Infallible`) で、OOM のとき panic する。確保失敗を受け取れる `&str` からの変換は
  `"..".parse::<LeanString>()` か `ToLeanString::try_to_lean_string()` だが、どこからも案内されていない。
- const 文脈: `LeanString::new` / `from_static_str` / `len` / `is_empty` / `as_str` / `as_bytes` / `as_static_str` は `const fn` だが、`LeanString` は
  `Drop` を持つので、`const { LeanString::from_static_str("x").len() }` のように一時値を作るとコンパイルエラー
  (`E0493: destructor cannot be evaluated at compile-time`)。`static` / `const` 項目か、`mem::forget` が必要 (`tests/const.rs` が実際そう書いている)。
  `capacity()` / `is_heap_allocated()` は `const fn` ではない。

## 原因

README の "High API compatibility for `String`." だけで済ませ、差分を列挙する場所がない。`String::into_*` や `DerefMut` が無い理由
(レイアウトが共有・static を持つ) や、`From<String>` がコピーになる理由 (参照カウントのヘッダ) は実装の都合で、コード上にも書かれていない。
`From<String>` / `From<Box<str>>` / `From<Cow<'_, str>>` の impl には doc が無い。

## 修正例

`LeanString` の型ドキュメント ([#40](./40-type-level-docs.md)) に「Differences from `String`」の節を足す。上の表と箇条書きから、利用者が
つまずくもの (`DerefMut` なし、変換はコピー、`Pattern` にならない、`from_utf8` の署名、`from_static_str` だけが確保なし) を残す。例:

```rust
/// # Differences from `String`
///
/// - There is no `DerefMut`, `as_mut_str`, `into_bytes`, `into_boxed_str` or `leak`: the buffer may
///   be shared with clones or point to a `&'static str`, so handing out a mutable view or ownership of the
///   allocation is not generally possible without copying.
/// - Conversions from `String`, `Box<str>`, `Cow<str>` and `&str` copy the text, even for a `&'static str`
///   longer than the inline size. Use [`LeanString::from_static_str`] for the allocation-free path.
/// - `&LeanString` can't be used as a pattern for `str` methods (`Pattern` is unstable). Pass
///   `lean.as_str()`.
```

`From<String>` (と `Box<str>` / `Cow`) の impl にも doc を付ける (`LeanStr` も同様)。

```rust
/// Creates a [`LeanString`] by copying the content of a [`String`].
///
/// Unlike `String::from(Box<str>)`, this does not take over the buffer: [`LeanString`] keeps a
/// reference count in front of its heap data, so the content is copied into a new allocation
/// (unless it fits inline) and the [`String`] is freed. This is O(n).
impl From<String> for LeanString { ... }
```

README の "High API compatibility for `String`." からも型ドキュメントの節にリンクする ([#43](./43-readme-claims.md))。

## 確認方法

- 上記の各項目を doctest の `compile_fail` (```` ```compile_fail ````) にすると、将来 `DerefMut` などを追加したときにドキュメントの陳腐化を
  検出できる。例: `let mut s = LeanString::new(); let _: &mut str = &mut *s;` (compile_fail)。
- `From<String>` のコピーは、確保を数える `#[global_allocator]` で `LeanString::from(String)` の確保回数が 1 であることを `tests/` に足す
  (現状は確保回数の保証が無い)。doc test にするなら、`String` から作った `LeanString` の `as_ptr()` が元の `String` の `as_ptr()` と違うことを示す例が使える。
- `cargo doc` で該当箇所を確認する。

## 関連

- [#40](./40-type-level-docs.md): 型レベルのドキュメント
- [#42](./42-conversion-cost-docs.md): `into_lean_str` / `into_lean_string` のコストの doc
- [#43](./43-readme-claims.md): README の記述
- [#48](./48-std-conversions.md): `std` の型への変換
- [#50](./50-drain.md): `drain`
- [#51](./51-as-mut-str.md): `as_mut_str`
- [#53](./53-str-methods-returning-string.md): `String` を返す `str` メソッド
