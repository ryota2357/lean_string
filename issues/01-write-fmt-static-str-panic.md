# `write!` の fast path と `try_from_fmt` が `from_static_str` の panic をそのまま使っている

- 種別: bug, api
- 対象: `src/lib.rs` の `impl fmt::Write for LeanString` の `write_fmt`, `src/traits.rs` の `try_from_fmt`
- 影響: 32-bit で、2^24 バイト以上のリテラルを空の `LeanString` に `write!` すると panic する (同じ文字列の `push_str` は成功する)。実用上はまず起きない。`try_from_fmt` 側は到達しない分岐で、実害は無いが失敗を `Err` で返す関数の中に panic する経路が残っている
- 優先度: 低

## 問題

`fmt::Write::write_fmt` は、`args.as_str()` が `Some` (フォーマット引数の無いリテラル) で、`self` が空かつ heap でなければ、`LeanString::from_static_str(s)` に差し替える。

```rust
Some(s) => {
    if self.is_empty() && !self.is_heap_allocated() {
        *self = LeanString::from_static_str(s);
    } else {
        self.push_str(s);
    }
    Ok(())
}
```

`from_static_str` は長さが `StaticBuffer::MAX_LENGTH` (64-bit で 2^56 - 1、32-bit で 2^24 - 1) を超えると `"text is too long"` で panic する。64-bit では到達不能だが、32-bit では 16 MiB のリテラルで到達する。`format_args!` は書式文字列の位置のマクロを先に展開するので、`include_str!` で読んだファイルをそのまま `write!` に渡せる。

確認した結果 (`5dd94ea`, nightly 1.101.0 2026-10-11 の Miri, `--target i686-unknown-linux-gnu`):

```rust
// big.txt は 'a' が 2^24 バイト
let mut s = LeanString::new();
write!(s, include_str!("big.txt")).unwrap();      // panic: "text is too long"
let mut t = LeanString::new();
t.push_str(include_str!("big.txt"));              // 成功 (len 16777216)
```

x86_64 では同じコードがどちらも成功する。`String` の `write_fmt` は確保失敗以外で panic しないので、32-bit でだけ動作が変わる。

`src/traits.rs` の `try_from_fmt` にも同じ形の分岐がある。

```rust
if let Some(str) = args.as_str() {
    return Ok(LeanString::from_static_str(str));
}
```

こちらは `ToLeanString::try_to_lean_string` / `ToLeanStr::try_to_lean_str` のように失敗を `Err` で返す API の中なので、通れば契約違反になる。ただし呼び出し元は 2 か所とも `match_type!` の最後の arm の `format_args!("{s}")` で、`s` は実行時の値 (`&T`) なので、`as_str()` は常に `None` になる。`fmt::Arguments::as_str()` が `Some` を返すのは、書式文字列にリテラル以外の引数が無いとき (コンパイラがリテラル引数を書式文字列に畳み込んだ場合を含む) だけ。この分岐は到達しない。

## 原因

O(1) で作れる `from_static_str` を、失敗しない関数として流用している。`Repr::from_static_str` は `Result` を返すので、失敗したときに通常の経路へ戻せる。

## 修正例

`write_fmt` は `Repr::from_static_str` の `Err` で `push_str` に落とす。

```rust
fn write_fmt(&mut self, args: fmt::Arguments<'_>) -> fmt::Result {
    match args.as_str() {
        Some(s) => {
            if self.is_empty() && !self.is_heap_allocated() {
                // Since self is empty inline buffer or empty static buffer, constructing a new
                // one with `from_static_str` is more efficient since it is O(1).
                // A text too long for a static buffer (only possible on 32-bit) is copied.
                if let Ok(repr) = Repr::from_static_str(s) {
                    *self = LeanString(repr);
                    return Ok(());
                }
            }
            self.push_str(s);
            Ok(())
        }
        None => fmt::write(self, args),
    }
}
```

この修正を入れて、Miri i686 で上の再現が panic しなくなること、[#30](./30-test-write-fmt-fast-path.md) のテストが通ることを確認した。古い `self` は空の inline か static なので、上書きしても解放漏れは無い (`*self = ...` の drop も何もしない)。

`try_from_fmt` の分岐は削除を推奨する。到達しないので動作は変わらない。残す場合は `Repr::from_static_str(str)?` で `ReserveError` として返す (`E: From<ReserveError>` は既にある)。ただし [#49](./49-format-macro.md) を採用して `try_from_fmt` を任意の `fmt::Arguments` で呼べるようにすると、この分岐は到達可能になる。その場合は削除せず、`Repr::from_static_str(str)?` を使う。

## 確認方法

- 32-bit の再現は 16 MiB のファイルが要るのでリポジトリのテストには向かない。修正後に一度だけ Miri i686 で上のコードを走らせる。
- 64-bit の fast path の動作は [#30](./30-test-write-fmt-fast-path.md) のテストで固定する。
