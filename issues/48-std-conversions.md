# `LeanString` / `LeanStr` から `Box<str>` / `Rc<str>` / `Arc<str>` / `Vec<u8>` などへの `From` が無い

- 種別: api
- 対象: `src/lib.rs` (`impl From<...> for LeanString` / `LeanStr` の並び)
- 影響: `String` がもつ標準の変換が使えないため、`T: Into<Arc<str>>` / `Into<Box<str>>` / `Into<Box<dyn Error>>` などのジェネリックな境界に `LeanString` を渡せない。回避は `Arc::<str>::from(s.as_str())` と書けば足りるので機能の欠落ではなく、互換性とジェネリックコードでの使いやすさの問題。
- 優先度: 中

## 問題

`5dd94ea` で、次の変換を `rustc 1.97.0` でコンパイルして確認した (`String` はすべて実装している)。

| 変換 | `String` | `LeanString` | `LeanStr` |
| --- | --- | --- | --- |
| `From<_> for Box<str>` | あり | **無い** (`E0277`) | **無い** |
| `From<_> for Rc<str>` | あり | **無い** | - |
| `From<_> for Arc<str>` | あり | **無い** | **無い** |
| `From<_> for Vec<u8>` | あり | **無い** | - |
| `From<_> for Box<dyn Error>` / `Box<dyn Error + Send + Sync>` | あり | **無い** | - |
| `From<_> for OsString` / `PathBuf` | あり | **無い** | - |
| `From<_> for String` / `Cow<'_, str>` | - | あり | あり |

`&String` から `Box<str>` / `Rc<str>` / `Arc<str>` への `From` は std にも無いので、参照版は不要 (`&str` 経由で足りる)。

逆方向 (`Arc<str>` / `Rc<str>` -> `LeanString`) は `String` も持たない (`From<Rc<str>> for String` は無い。`From<Box<str>> for String` はある) ので、`LeanString` が `From<Box<str>>` を持つ現状 (0.7.0 の時点からある) が std に揃っている。

## 原因

0.8.0 の `Cow` / `LeanStr` 間の `From` 追加は、クレート内の型同士と `String` / `Cow` に限定された。ほかの std の型への変換は見送られた (理由はコードにも CHANGELOG にも書かれていない)。

## 修正例

実装はすべて `as_str()` / `as_bytes()` を通すだけでよい。heap の buffer はヘッダ (`count`, `capacity`) 付きのレイアウトなので、`Box<str>` / `Arc<str>` などの確保に流用できず、必ず O(n) のコピーになる (`From<LeanString> for String` が現状すでにそうなっている: 確保 1 回)。docs にその旨を書く。試作 (コンパイルと実行、Miri (`-Zmiri-strict-provenance`) で確認):

```rust
impl From<LeanString> for Box<str> { fn from(s: LeanString) -> Self { s.as_str().into() } }
impl From<LeanStr>    for Box<str> { fn from(s: LeanStr)    -> Self { s.as_str().into() } }
impl From<LeanString> for Rc<str>  { fn from(s: LeanString) -> Self { s.as_str().into() } }
impl From<LeanString> for Arc<str> { fn from(s: LeanString) -> Self { s.as_str().into() } }
impl From<LeanStr>    for Arc<str> { fn from(s: LeanStr)    -> Self { s.as_str().into() } }
impl From<LeanString> for Vec<u8>  { fn from(s: LeanString) -> Self { s.as_bytes().into() } }
```

(`Rc<str>` と `Vec<u8>` の `LeanStr` 版、`&LeanString` 版は同じ形。)

`Box<dyn Error>` は `String` と同様に、`impl From<LeanString> for Box<dyn Error + Send + Sync + 'static>` と `Box<dyn Error + 'static>` を足す。内部は `Box::<str>::from(..)` 経由か、`String` を作って既存の `From<String>` に渡す。`OsString` / `PathBuf` は `AsRef<OsStr>` / `AsRef<Path>` が既にあるので `OsString::from(s.as_str())` で足りる。必要性は低い。

`into_boxed_str()` / `into_bytes()` のようなメソッドは、ゼロコピーにならないことが名前から読み取れず、`std` のものと性能の期待が違うので、足さず `From` / `Into` に任せる方針を推奨する ([#41](./41-std-differences-docs.md) に理由を書く)。

## 確認方法

- 試作と同じ形のテスト (`Box<str>` / `Rc<str>` / `Arc<str>` / `Vec<u8>` に変換して内容が一致すること、inline / heap / static の 3 種類を入力にすること) を `tests/conversion.rs` に足す。
- `cargo +nightly miri test --test conversion` で確認する。
