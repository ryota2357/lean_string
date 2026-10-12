# `Deref<Target = str>` 由来の `replace` / `replacen` / `join` / `concat` は `LeanString` でなく `String` を返す

- 種別: api (設計判断を含む), doc
- 対象: `src/lib.rs` の `LeanString` / `LeanStr`、`impl Deref<Target = str>`
- 影響: 0.8.0 で `to_lowercase` / `to_uppercase` / `to_ascii_*` / `repeat` を `LeanString` / `LeanStr` を返すように直した (CHANGELOG の Breaking 項目) ときと同じ落とし穴が、同じ分類の他のメソッドに残っている。`lean.replace("a", "b")` は黙って `String` を返し、`LeanString` を期待する型へ戻すにはコピーが 1 回余計に要る。
- 優先度: 低

## 問題

`5dd94ea` で確認 (コンパイル): `let r: LeanString = s.replace("a", "b");` はエラー (`mismatched types`)、`let r: String = s.replace("a", "b");` は通る。`[LeanString]` の `join` / `concat` も `String` を返す (`Borrow<str>` 経由)。`to_owned()` は `Clone` が優先されて `LeanString` を返すので問題ない。`to_string()` は `Display` 経由で `String` を返す (std と同じ)。

同種のクレートの対応: `ecow` 0.3.0 は `EcoString::replace(&self, pat: &str, to: &str) -> Self` と `replacen(&self, pat: &str, to: &str, count: usize) -> Self` を持つ (ローカルのソースで確認)。パターンを `&str` に限っている。`compact_str` 0.10.0 は `CompactStringExt::{concat_compact, join_compact}` を持つ (同上)。

## 原因

`str::replace<P: Pattern>` のシグネチャは `Pattern` (unstable) を使うため、安定版の外部クレートは同じ形で再実装できない (`&LeanString` が `Pattern` にならない理由と同じ)。`to_lowercase` などは `Pattern` を取らないので自前で書けた。

## 修正例

案 A: `replace` / `replacen` を `&str` パターンに限って inherent メソッドとして追加する (`ecow` と同じ)。inherent メソッドは `Deref` 先の `str::replace` より先に解決されるので、既存の `s.replace('a', "b")` (`char` パターン) や `s.replace(|c| ..., "b")` は型エラーになる (`str::replace` には落ちない。`Deref<Target = str>` の型に `&str` 専用の inherent `replace` を足した最小例で `E0308` になることを確認した)。したがって破壊的変更になり、CHANGELOG の Breaking に載せる必要がある。`char` パターンも使いたい利用者には `s.as_str().replace(..)` が残る。

案 B: 拡張トレイトを提供する (`compact_str` 流)。

```rust
pub trait LeanStringExt {
    fn join_lean(self, sep: &str) -> LeanString;
    fn concat_lean(self) -> LeanString;
}
impl<I, S> LeanStringExt for I where I: IntoIterator<Item = S>, S: AsRef<str> { ... }
```

案 C: 何も足さず、docs に「`Deref` 経由の `str` メソッドで新しい文字列を作るものは `String` を返す。`LeanString` にするには `LeanString::from(..)` (コピー 1 回)」と書く ([#41](./41-std-differences-docs.md) に含める)。

どれを採るかは `replace` の利用頻度次第で、設計判断。A は互換性に影響するので、利用者の `replace(char, ..)` 呼び出しが壊れないかを先に調べる必要がある (未調査)。

## 確認方法

追加する場合は、`replace` / `replacen` の結果が `str::replace` と一致すること (`proptest` で `&str` パターン)、一致が無いとき確保しないで clone を返せるか (将来の最適化) を `tests/lean_string.rs` に足す。
