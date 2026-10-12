# `write!` / `writeln!` が一度もテストされておらず、`write_fmt` の fast path の条件が守られていない

- 種別: test
- 対象: `src/lib.rs` の `impl fmt::Write for LeanString` (`write_str`, `write_fmt`)
- 影響: `write_fmt` の fast path の条件を壊しても既存のテストは全部通る。条件の片方を落とすと、確保済みの heap 容量を黙って捨てる退行になる
- 優先度: 中

## 問題

`impl fmt::Write for LeanString` の `write_fmt` は、`args.as_str()` が `Some` のとき (引数の無いリテラル) に、空で heap でない文字列を `from_static_str` に差し替える fast path を持つ。

```rust
if self.is_empty() && !self.is_heap_allocated() {
    *self = LeanString::from_static_str(s);
} else {
    self.push_str(s);
}
```

`tests/` にも doctest にも `write!` / `writeln!` / `fmt::Write` を `LeanString` に使うコードが無い (`tests/out_of_memory.rs` の `write_str` は `Formatter` に対するもの)。

確認: 条件を `if self.is_empty() {` に変える (確保済みで空の heap 文字列も差し替えて容量を捨てる) と、`cargo test` の `alloc_string` / `lean_string` / `lean_str` / `conversion` / `const` / `out_of_memory` と doctest 85 件はすべて通る (`5dd94ea`, rustc 1.97.0)。下のテストでは `write_literal_into_empty_heap_keeps_capacity` が落ちる。

## 修正例

以下のテストを `tests/lean_string.rs` に足す。x86_64 で `cargo test`、Miri (`-Zmiri-strict-provenance`) の x86_64 / i686 / powerpc64 / powerpc で通ることを確認した。

```rust
use std::fmt::Write as _;

#[test]
fn write_literal_into_empty_inline_becomes_static() {
    let mut s = LeanString::new();
    write!(s, "a literal longer than the inline limit").unwrap();
    assert_eq!(s, "a literal longer than the inline limit");
    // `fmt::Arguments::as_str()` is `Some`, so the literal is referenced, not copied.
    assert!(s.as_static_str().is_some());
    assert!(!s.is_heap_allocated());
}

#[test]
fn write_literal_into_empty_static_becomes_static() {
    let mut s = LeanString::from_static_str("a static string longer than inline");
    s.clear();
    assert!(s.as_static_str().is_some());
    write!(s, "another literal longer than the inline limit").unwrap();
    assert_eq!(s, "another literal longer than the inline limit");
    assert!(s.as_static_str().is_some());
}

#[test]
fn write_literal_into_non_empty_appends() {
    let mut s = LeanString::from("abc");
    write!(s, "def").unwrap();
    writeln!(s, "ghi").unwrap();
    assert_eq!(s, "abcdefghi\n");

    let mut s = LeanString::from_static_str("a static string longer than inline");
    write!(s, "!").unwrap();
    assert_eq!(s, "a static string longer than inline!");
    assert!(s.is_heap_allocated());
}

#[test]
fn write_literal_into_empty_heap_keeps_capacity() {
    let mut s = LeanString::with_capacity(100);
    let ptr = s.as_ptr();
    write!(s, "a literal longer than the inline limit").unwrap();
    assert_eq!(s, "a literal longer than the inline limit");
    assert!(s.is_heap_allocated());
    assert_eq!(s.as_ptr(), ptr);
    assert_eq!(s.capacity(), 100);
}

#[test]
fn write_with_arguments() {
    let mut s = LeanString::new();
    write!(s, "{}-{:>4}-{:?}", 42, "ab", 'c').unwrap();
    assert_eq!(s, "42-  ab-'c'");
    writeln!(s).unwrap();
    assert_eq!(s, "42-  ab-'c'\n");
}
```

`write!(s, "{}", "literal")` もコンパイラがリテラル引数を書式文字列に畳み込むと `as_str()` が `Some` になるが、畳み込むかどうかはコンパイラ次第なので、static になることはテストで固定しない。

## 確認方法

上のテストが通ること。fast path の条件を変えたとき (`!self.is_heap_allocated()` を消す、`is_empty()` を消す) にどれかが落ちること。
