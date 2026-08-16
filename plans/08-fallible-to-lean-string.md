# 08: `try_to_lean_string` / `try_to_lean_str` が確保失敗で panic する

種別: バグ修正 / 実装量: 小 / 設計判断: 小

## 現象

`ToLeanString::try_to_lean_string` は「panic しない版」として文書化されている。

```rust
/// Attempts to convert the value to a [`LeanString`].
///
/// # Errors
///
/// Returns a [`ToLeanStringError`] if the conversion fails.
fn try_to_lean_string(&self) -> Result<LeanString, ToLeanStringError>;
```

しかし `Display` へのフォールバック arm (src/traits.rs:77-81) はこうなっている。

```rust
s => {
    let mut buf = LeanString::new();
    write!(buf, "{}", s)?;
    return Ok(buf)
}
```

`impl fmt::Write for LeanString` (src/lib.rs:2087-2092) は

```rust
fn write_str(&mut self, s: &str) -> fmt::Result {
    self.push_str(s);   // 確保に失敗したら panic する
    Ok(())
}
```

なので、`castaway` のどの arm にも当てはまらない `Display` 型を変換するとき、
確保に失敗すると `Err(ToLeanStringError::Reserve(..))` ではなく panic する。
`ToLeanStringError::Reserve` はこの arm からは到達不能で、
この経路で返りうるのは `Fmt` だけになっている。

`ToLeanStr::try_to_lean_str` (src/traits.rs:162-166) も同じ。

## 方針

`fmt::Error` は情報を運べないので、確保エラーを writer 側に退避する。

```rust
s => {
    struct FallibleWriter {
        string: LeanString,
        reserve_error: Option<ReserveError>,
    }

    impl fmt::Write for FallibleWriter {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            // 一度失敗したら以降は確保を試みない。
            if self.reserve_error.is_some() {
                return Err(fmt::Error);
            }
            self.string.try_push_str(s).map_err(|error| {
                self.reserve_error = Some(error);
                fmt::Error
            })
        }
    }

    let mut writer = FallibleWriter { string: LeanString::new(), reserve_error: None };
    let result = write!(writer, "{}", s);
    return match (result, writer.reserve_error) {
        // 退避したエラーを優先する。
        (_, Some(error)) => Err(ToLeanStringError::Reserve(error)),
        (Ok(()), None) => Ok(writer.string),
        (Err(error), None) => Err(ToLeanStringError::Fmt(error)),
    };
}
```

非自明な点は 2 つある。どちらも「行儀の悪い `Display` 実装」への防御。

1. **一度失敗したら以降の `write_str` で確保を試みない**。`Display` の実装が
   `f.write_str(..)` の戻り値を無視して書き続ける場合、毎回確保を試みることになる。
2. **退避したエラーはフォーマッタの戻り値より優先する**。`Display` の実装が
   書き込みエラーを握り潰して `Ok(())` を返す場合、途中で切れた文字列を
   成功として返してしまう。

`ToLeanStr` 側も同じ writer を使い、最後に `try_into_lean_str()` を通す。
2 か所に同じ構造を書くことになるので、`FallibleWriter` は `traits.rs` の
モジュールレベルに 1 つ置いて両方から使う。

## テスト

`tests/out_of_memory.rs` に確保を失敗させるアロケータが既にあるので、そこへ追加する。
`Display` 実装を 4 つ用意して、それぞれの経路を通す。

| `Display` の実装 | 期待 |
| --- | --- |
| 素直に長い文字列を書く | `Err(Reserve(..))` |
| `let _ = f.write_str(..); Ok(())` (エラーを握り潰す) | `Err(Reserve(..))` |
| 失敗後にもう一度 `write_str` する | `Err(Reserve(..))`、2 回目も `Err` になること |
| 途中まで書いて `Err(fmt::Error)` を返す | `Err(Fmt(..))` |

3 番目は 1 の防御の、2 番目は 2 の防御のテストになる。
確保を 1 回だけ失敗させる仕組みなので、防御が無ければ 2 回目の書き込みが成功して
`Ok` が返ってしまう。

あわせて `to_lean_string()` (panic する側) が従来どおり panic することも確認する。

## 補足

`fmt::Write for LeanString` の `write_str` が panic する設計自体は変えない。
`fmt::Write` は `Result<(), fmt::Error>` しか返せないため、
`String` の同 impl と同じく「確保失敗は panic」で揃えるのが自然。
変えるのは「fallible と名乗っている API がその writer を使っている」ところだけ。

`fmt::Write::write_fmt` (src/lib.rs:2094-2109) の「引数が無いリテラルなら
static バッファに差し替える」fast path は今回の変更の影響を受けない
(`FallibleWriter` は独自の型なので既定の `write_fmt` を使う)。
ただしこの fast path はテストが無い ([plans/11](./11-test-hardening.md) 参照)。

## 依存

なし。単独で実施できる。
