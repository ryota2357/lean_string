# `format!` 相当 (`lean_format!` / `LeanString::from_fmt`) が無く、`format_args!(..).to_lean_string()` はリテラルでも確保する

- 種別: api, perf
- 対象: `src/traits.rs` の `try_from_fmt` (private)、`impl<T: Display> ToLeanString for T`、`src/lib.rs` の `impl fmt::Write for LeanString`
- 影響: `LeanString` を `format!` のように組み立てるには、`write!(s, ..)` を `unwrap()` つきで書くか、`format!` した `String` から変換する (確保 2 回) しかない。同種のクレートはマクロを提供している。
- 優先度: 中

## 問題

`format!` -> `String` -> `LeanString::from` は、一時 `String` の確保 + コピーで確保が 2 回になる。`write!(&mut s, ..)` なら 1 回 (または 0 回) で済むが、`fmt::Result` を処理するボイラープレートが要る。`ToLeanString` を `format_args!` の結果に使えば書けるが、リテラルだけのときの最適化が効かない。`5dd94ea`、カウント付き `#[global_allocator]` での確保回数:

| 式 | 確保回数 |
| --- | --- |
| `format_args!("<45 バイトのリテラル>").to_lean_string()` | 1 (結果は static でない) |
| `write!(string, "<同じリテラル>")` (空の `LeanString`) | 0 (結果は static) |
| `format_args!("<リテラル> {x}").to_lean_string()` | 2 (書き込み 2 回で成長) |
| `LeanString::from(format!("<リテラル> {x}"))` | 2 |
| `LeanString::from_static_str("<同じリテラル>")` | 0 |

リテラルだけの `Arguments` を `Arguments::as_str()` で見分けて `from_static_str` にする処理は、`fmt::Write::write_fmt` の impl と、`try_from_fmt` (`traits.rs`) の先頭にすでにある。`format_args!(..).to_lean_string()` で最適化が効かないのは、`impl<T: Display> ToLeanString for T` のフォールバックが `try_from_fmt(format_args!("{s}"))` を呼び、`Arguments` を `{s}` で再ラップする (`as_str()` が `None` になる) ため。`castaway` は `'static` な型にしか一致しないので、`Arguments<'_>` を特殊化できない。

同種のクレートのマクロ (ローカルの registry のソースで確認): `compact_str::format_compact!`、`ecow::eco_format!`。

## 原因

`try_from_fmt` が private の関数のままで、外部から直接呼べる API (関数・マクロ) が無い。

## 修正例

```rust
impl LeanString {
    pub fn try_from_fmt(args: fmt::Arguments<'_>) -> Result<Self, ToLeanStringError> { try_from_fmt(args) }
    #[track_caller]
    pub fn from_fmt(args: fmt::Arguments<'_>) -> Self { Self::try_from_fmt(args).unwrap_with_msg() }
}

#[macro_export]
macro_rules! lean_format {
    ($($arg:tt)*) => { $crate::LeanString::from_fmt(::core::format_args!($($arg)*)) };
}
```

試作して `lean_format!("value = {x}, and a long tail so that it is on the heap")` と、リテラルのみの `lean_format!("literal only, ...")` (結果が `as_static_str().is_some()` で確保なし) を確認した (通常実行と Miri)。`LeanStr` 版が欲しければ `lean_str_format!` か、`LeanString::from_fmt(..).into_lean_str()`。

注意: `try_from_fmt` を任意の `fmt::Arguments` で呼べるようにすると、その先頭の `args.as_str()` の fast path (`LeanString::from_static_str` を呼ぶ) が到達可能になる。この分岐は、32-bit で 2^24 バイト以上のリテラルを渡すと panic しうる (`Err` を返す関数の中での panic)。そのため、`try_from_fmt` を公開する前にこの分岐を直しておく必要がある。詳細と修正は [#01](./01-write-fmt-static-str-panic.md) を参照。

`ToLeanString` の `Arguments` 特殊化は不可能なので、docs に「`format_args!` には `LeanString::from_fmt` を使う」と書く。`write!` の方が速い場面 (既存のバッファへ追記) も docs に書いてよい。

## 確認方法

`tests/lean_string.rs` に、`lean_format!` の結果が `format!` と一致すること、リテラルのみが static で確保 0 であること (カウント付きアロケータ)、`Display` 実装が `Err` を返すと `from_fmt` が panic し `try_from_fmt` が `Err(ToLeanStringError::Fmt)` を返すことを足す。
