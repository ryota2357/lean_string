# UTF-8 / UTF-16 デコード関数に `try_` 版がなく、OOM を `Result` で受け取れない

- 種別: api
- 対象: `src/lib.rs` の `LeanString` / `LeanStr` の `from_utf8`, `from_utf8_lossy`, `from_utf8_unchecked`, `from_utf16`, `from_utf16_lossy`, `from_utf16le`, `from_utf16le_lossy`, `from_utf16be`, `from_utf16be_lossy`、`src/errors/from_utf16_error.rs` の `FromUtf16Error`
- 影響: このクレートは「確保する操作には必ず `try_` 版がある」という方針 (`try_reserve`, `try_push_str`, `try_repeat`, `try_to_ascii_lowercase` ... と `ToLeanString::try_to_lean_string`) だが、デコード系だけが例外で、OOM を回復したい利用者 (組み込み・サーバの大きな入力) は `&str` に直した後で `try_to_lean_string` を通すしかない。
- 優先度: 中

## 問題

`5dd94ea` で、失敗アロケータ (次の 1 回の確保を失敗させる `#[global_allocator]`) を仕掛けて呼ぶと、すべて panic する (`PANIC: Cannot allocate memory to hold LeanString`)。

- `LeanString::from_utf8(&[u8]) -> Result<Self, Utf8Error>`: 入力が 16 バイト超なら `From<&str>` で確保する。
- `LeanString::from_utf8_lossy(&[u8]) -> Self`: `with_capacity(buf.len())` と `push_str` / `push`。
- `LeanString::from_utf16(&[u16]) -> Result<Self, FromUtf16Error>` / `from_utf16_lossy`: 同様に `with_capacity(buf.len())` と `push`。
- `from_utf16le` / `from_utf16be` とその `_lossy`、および `LeanStr` の同名関数 (`LeanString` 版を呼んで `into_lean_str`)。

`FromUtf16Error` は 0.8.0 で `std` に合わせて不透明な構造体になり (`kind` は `pub(crate)`)、`LoneSurrogate` / `OddBytes` の 2 種類しか持たない。`Utf8Error` は `core` の型で、確保失敗を表せない。`Result` を返すのに OOM では panic する、という点自体が文書化されていない ([#39](./39-panics-doc-gaps.md) 参照)。

回避策は存在する (`String::from_utf8_lossy` などで `&str` を作ってから `try_to_lean_string()` / `"..".parse::<LeanString>()`) が、その場合は一時的な `String` / `Cow` の確保が余計に入り、`utf8_chunks` を使った直接デコードの利点が失われる。

## 原因

デコード関数は `std` の `String::from_*` を写して作られ、確保失敗を `Result` に載せる設計 (`ReserveError`) が追加されたときに対象から漏れた。エラー型 (`Utf8Error` は `core` のもの、`FromUtf16Error` は `std` と同じ形) を保つ方針が、OOM の表現を難しくしている。

## 修正例

lossy 系は `Result<Self, ReserveError>` を返すだけでよく、簡単。`try_from_utf8_lossy` を試作してテストを通した (`try_with_capacity` と `try_push_str` / `try_push` に置き換えるだけ):

```rust
impl LeanString {
    pub fn try_from_utf8_lossy(buf: &[u8]) -> Result<Self, ReserveError> {
        let mut ret = LeanString::try_with_capacity(buf.len())?;
        for chunk in buf.utf8_chunks() {
            ret.try_push_str(chunk.valid())?;
            if !chunk.invalid().is_empty() {
                ret.try_push(char::REPLACEMENT_CHARACTER)?;
            }
        }
        Ok(ret)
    }
}
```

`try_from_utf16_lossy`, `try_from_utf16le_lossy`, `try_from_utf16be_lossy` も同形 (`from_utf16_units_lossy` を `try_` にする)。`LeanStr` 側は `LeanString::try_from_*_lossy(buf)?.try_into_lean_str()`。

検証つきデコード (`from_utf8`, `from_utf16`, `from_utf16le`, `from_utf16be`) は、エラー型の設計が必要:

- 案 A: 既存の `ToLeanStringError { Reserve, Fmt }` と同じ形の列挙体を足す。
  ```rust
  pub enum TryFromUtf8Error  { Utf8(core::str::Utf8Error), Reserve(ReserveError) }
  pub enum TryFromUtf16Error { Utf16(FromUtf16Error),      Reserve(ReserveError) }
  ```
  `try_from_utf8`, `try_from_utf16`, `try_from_utf16le`, `try_from_utf16be` が返す。既存関数は変更しない。
- 案 B: `FromUtf16Error` に `Reserve` 種別を足し、`from_utf16` 系が OOM で `Err` を返すように変える。`FromUtf16Error` は不透明なので型の互換性は保てるが、OOM で panic していた関数が `Err` を返すようになり挙動が変わる。`Utf8Error` 側は解決しない。
- 案 C (最小): `try_` は lossy 系だけにして、検証つきの関数には `# Panics` を書くだけにする。

推奨は A (lossy 系の追加と同時に)。型の形がすでに `ToLeanStringError` に前例があり、既存関数の挙動を変えない。関数が 8 本 x 2 型に増えるのが気になるなら C から始める。

## 確認方法

`tests/out_of_memory.rs` の `FAIL_NEXT_ALLOCATION` を使って、16 バイトを超える入力で `try_from_*` が `Err(ReserveError)` を返す (panic しない) テストを足す。有効な入力で既存関数と同じ結果になることは、既存の `tests/alloc_string.rs` の `test_from_utf8*` / `test_from_utf16*` を `try_` 版でも回して確認する。
