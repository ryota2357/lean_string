# `ToLeanString` / `ToLeanStr` とエラー型の docs が最小限で、使い方・挙動の約束が分からない

- 種別: doc, api
- 対象: `src/traits.rs` の `ToLeanString`, `ToLeanStr`、`src/errors/*.rs` の `ReserveError`, `FromUtf16Error`, `ToLeanStringError`, `ToLeanStrError`
- 影響: 2 つの trait は例が無く、どの型で確保が省かれるか、`to_string()` との関係、失敗の意味が書かれていない。エラー型は将来の拡張の余地についての方針が無く、`ToLeanStringError` / `ToLeanStrError` は variant を足すと破壊的変更になる。
- 優先度: 低

## 問題

### `ToLeanString` / `ToLeanStr`

trait の doc は「A trait for converting a value to a `LeanString`.」の 1 行で、`# Examples` が無い (メソッドは `# Panics` / `# Errors` を持つ)。コードから分かるが書かれていないこと:

- `T: Display` の blanket impl で、`ToString` の `LeanString` 版であること。`str` には別の impl があり (`"..".to_lean_string()` が `&str` に対して使える)、`&&str` のような参照は `Display` 経由になる。
- `castaway` による型の特殊化で、整数 (`NonZero` を含む)、`f32` / `f64`、`bool`、`char`、`String`、`LeanString`、`LeanStr` は `Display` を経由せず直接作られる。`LeanString` / `LeanStr` に対しては clone する (参照カウントの共有、`LeanStr` からは `try_into_lean_string` を経由するので場合によりコピー)。それ以外の型は `fmt::Write` で書き込む。`Box<str>` / `Cow<str>` / `&&str` などは特殊化されず `Display` 経由になる (推測: `castaway` は `'static` な型にしか一致しない)。
- 失敗の意味: `ToLeanStringError::Reserve` (確保失敗・上限超過) と `Fmt` (`Display` 実装が `Err` を返した) の 2 種類。`to_lean_string()` が panic するのは両方。確認: `Display::fmt` が `Err(fmt::Error)` を返す型に対する `to_lean_string()` は `an error occurred when formatting an argument` で panic し、位置は `src/traits.rs` ([#38](./38-track-caller-missing.md) 参照)。
- `fmt::Arguments` に対する特殊化が無い: `format_args!("<45 バイトのリテラル>").to_lean_string()` は確保 1 回 (static にならない) だが、`write!(string, "<同じリテラル>")` は 0 回 (`fmt::Write::write_fmt` が `Arguments::as_str()` で `from_static_str` を使う)。`std` の `ToString` は `Arguments` を特殊化している。この差は [#49](./49-format-macro.md) で扱う。

### エラー型

- `ReserveError` は公開フィールドを持たない単位構造体 (`pub struct ReserveError;`) で、アロケータ失敗・容量上限超過・`usize` オーバーフローを区別できない。単位構造体は式として直接書ける公開型なので、あとから原因を保持する形に変えると破壊的変更になる。`FromUtf16Error` は 0.8.0 で不透明な構造体にして将来の拡張に備えたが、`ReserveError` には同じ備えが無い。`Display` / docs の文言の問題は [#45](./45-doc-wording.md) を参照。
- `ToLeanStringError` / `ToLeanStrError` は `#[non_exhaustive]` でない `enum` (`Reserve`, `Fmt`)。variant を足すと破壊的変更になる。`TryFromUtf8Error` のような新しいエラー型を足す案 ([#47](./47-fallible-decoders.md)) も同じ形を踏襲するなら、この方針をそろえておくとよい。
- 4 つのエラー型はいずれも `Error` を実装し、`Send + Sync + 'static` (確認済み)。`ReserveError` / `ToLeanString(Str)Error` は `Copy` / `Eq` / `Hash`。docs にエラーの `source()` の扱いの記述は無い (`ToLeanStringError::source` は `None` を返す。variant の中身を返さない)。

## 原因

trait とエラー型は最初のリリースで最小の doc だけで作られ、その後の拡張 (`LeanStr` 版、`ToLeanStrError`) もコピーで増えた。

## 修正例

- trait の doc に、`ToString` との対応、特殊化される型 (確保なし・`Display` を経由しない) の一覧、`# Examples` (整数・`bool`・`char`・`Display` を実装した独自型)、`Fmt` エラーの意味を足す。`ToLeanStr` は `ToLeanString` へのリンクを張る。
- `ReserveError` の docs に、返る条件 (確保失敗・長さ上限超過・`usize` オーバーフロー) と `LeanStr` の操作でも返ることを書く。
- 破壊的変更を許す次のメジャー/マイナーのタイミングで、`ToLeanStringError` / `ToLeanStrError` を `#[non_exhaustive]` にするかを決める (`ReserveError` に原因を持たせるかも同時に判断する)。現状の 2 variant で足りるなら不要であり、設計上の判断である。

## 確認方法

`cargo test --doc` で追加例が通ること。`#[non_exhaustive]` を入れる場合は `cargo semver-checks` (未実行) で破壊的変更として検出されることを確認し、CHANGELOG の `Changed` に **Breaking** として載せる。
