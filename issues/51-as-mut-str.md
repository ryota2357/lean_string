# `&mut str` を得る手段 (`as_mut_str` / `AsMut<str>` / `DerefMut` / `IndexMut`) が無い

- 種別: api (設計判断を含む)
- 対象: `src/lib.rs` の `LeanString`、`src/repr.rs` の `Repr::ensure_modifiable` / `Repr::as_mut_ptr`
- 影響: `&mut str` を要求する `str` のメソッド (`split_at_mut`、`unsafe` の `as_bytes_mut` など) と、`AsMut<str>` / `BorrowMut<str>` の境界を要求するジェネリックコードが使えない。0.8.0 で `make_ascii_lowercase` / `make_ascii_uppercase` が `LeanString` 自身に足されたため、最も多い用途は解消済み。
- 優先度: 低

## 問題

`5dd94ea`、rustc 1.97.0 で次はいずれもコンパイルできない (`E0596` / `E0599`)。`String` は全部持っている:

```rust
let mut s = LeanString::from("hello world, long enough to be heap allocated");
let _: &mut str = &mut *s;          // DerefMut
let _: &mut str = s.as_mut();       // AsMut<str>
let _ = &mut s[..2];                // IndexMut
s.as_mut_str();                     // inherent
use std::borrow::BorrowMut; let _: &mut str = s.borrow_mut();
```

同種のクレートには前例がある (ローカルの registry にあるソースで確認): `compact_str` 0.10.0 に `CompactString::as_mut_str`、`ecow` 0.3.0 に `EcoString::make_mut(&mut self) -> &mut str`。

## 原因

`DerefMut` / `AsMut` は `&mut self` を受けて `&mut str` をそのまま返す形なので、fallible (`Result`) にできない。`LeanString` は heap buffer が共有されているか static のとき、`&mut str` を渡す前にコピー (CoW) を取らなければならず、その確保が失敗しうる (panic)。`Deref` の自動参照で暗黙に確保が走るのを避けるために入れていない、と読める (推測。コードにも docs にも理由は書かれていない)。

## 修正例

`DerefMut` は暗黙の確保 (と panic) を隠すので入れない方針を推奨する。代わりに、確保の可能性が名前から分かるメソッドを足す:

```rust
impl LeanString {
    pub fn try_as_mut_str(&mut self) -> Result<&mut str, ReserveError> {
        self.0.ensure_modifiable()?;          // static / 共有 heap なら複製して unique にする (Repr の既存関数)
        let len = self.len();
        // SAFETY: ensure_modifiable で buffer は static でなく unique。先頭 len バイトは初期化済みの UTF-8。
        Ok(unsafe { str::from_utf8_unchecked_mut(slice::from_raw_parts_mut(self.0.as_mut_ptr(), len)) })
    }
    #[track_caller]
    pub fn as_mut_str(&mut self) -> &mut str { self.try_as_mut_str().unwrap_with_msg() }
}
```

試作済み: `Repr::ensure_modifiable` と `Repr::as_mut_ptr` を `pub(crate)` にするだけで上のコードが動き、共有中のヒープ文字列と static 文字列に対して `as_mut_str().make_ascii_uppercase()` を呼ぶと元の clone は不変で、結果は heap に複製される (通常テストと Miri `-Zmiri-strict-provenance` で確認)。`AsMut<str>` / `BorrowMut<str>` を入れるかは別判断で、入れるなら `as_mut_str` と同じ「確保して panic しうる」を docs に書く必要がある。入れなくても、上のメソッドがあれば足りる場面が大半である。

代替案: (1) 何も足さず docs に「`make_ascii_*` / `replace_range` を使う」と書く ([#41](./41-std-differences-docs.md) に含める) — 現状に最も近い。(2) `DerefMut` を足す — `&mut *s` のたびに確保しうる点が最大の欠点。

## 確認方法

追加する場合は、共有 heap / static / inline の 3 種類でそれぞれ呼び、元の clone と static のデータが変化しないこと、結果が unique であることを `tests/lean_string.rs` に足す。`Miri` で確認する (`cargo +nightly miri test --test lean_string as_mut_str`)。確保失敗時は `try_as_mut_str` が `Err` を返し `self` が不変であることを `tests/out_of_memory.rs` で確認する。
