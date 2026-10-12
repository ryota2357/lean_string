# `tests/const.rs` が static バッファを const 評価していない

- 種別: test
- 対象: `tests/const.rs`, `src/repr.rs` の `Repr::from_static_str` / `len` / `as_bytes` / `as_static_str`, `src/repr/static_buffer.rs`
- 影響: `LeanString::from_static_str` / `LeanStr::from_static_str` / `len` / `as_str` / `as_bytes` / `as_static_str` は `const fn` として公開されているが、17 バイト以上 (static バッファ) の const 評価はテストされていない。`as_bytes` や `StaticBuffer` を変えて const で使えなくなっても気づけない
- 優先度: 低

## 問題

`tests/const.rs` の 3 つのテストが const 評価するのは `"hello world"` (11 バイト)、`"hello"`、長さ `0..=16` の部分文字列だけで、`from_static_str` はこれらをすべて inline バッファにする。static バッファの経路 (`StaticBuffer::new`、64-bit の `Repr::len` が `tail_word()` で読む static の長さ、`as_bytes` がフィールド 0 のポインタを読む分岐、`as_static_str`) は const 評価では一度も通らない。

const 評価は Miri と同じエンジンで、ポインタのバイトを整数として読む、範囲外を読むといった操作をエラーにする。たとえば `Repr::tail_word` は 2 語目を `usize` として読むので、2 語目にポインタが入る表現に変えると const 評価だけが失敗する。実行時のテストではこれを検出できない。

## 修正例

以下のテストを `tests/const.rs` に足す。x86_64 で `cargo test`、Miri (`-Zmiri-strict-provenance`) の x86_64 / i686 / powerpc64 / powerpc で通ることを確認した (const 評価はコンパイル時なので、各ターゲットでのコンパイルが通ることが本体)。

```rust
const LONG: &str = "a static string longer than the inline limit";

static LONG_STRING: LeanString = LeanString::from_static_str(LONG);
static LONG_STR: LeanStr = LeanStr::from_static_str(LONG);

#[test]
fn static_buffer_in_const() {
    const {
        assert!(LONG_STRING.len() == LONG.len());
        assert!(LONG_STR.len() == LONG.len());
        assert!(!LONG_STRING.is_empty());

        let a = LONG_STRING.as_str().as_bytes();
        let b = LONG_STR.as_bytes();
        let mut i = 0;
        while i < LONG.len() {
            assert!(a[i] == LONG.as_bytes()[i]);
            assert!(b[i] == LONG.as_bytes()[i]);
            i += 1;
        }

        match LONG_STRING.as_static_str() {
            Some(s) => assert!(s.len() == LONG.len()),
            None => panic!("expected a static buffer"),
        }
        match LONG_STR.as_static_str() {
            Some(s) => assert!(s.len() == LONG.len()),
            None => panic!("expected a static buffer"),
        }
    }
    assert_eq!(LONG_STRING, LONG);
    assert_eq!(LONG_STR, LONG);
    assert!(!LONG_STRING.is_heap_allocated());
    assert_eq!(LONG_STRING.as_str().as_ptr(), LONG.as_ptr());
}

#[test]
fn every_length_around_inline_limit_in_const() {
    const {
        let s = "0123456789abcdefghijklmnopqrstuvwxyz";
        let mut len = 0;
        while len <= s.len() {
            let expected = s.split_at(len).0;
            let lean = LeanString::from_static_str(expected);
            assert!(lean.len() == len);
            // Inline up to `2 * size_of::<usize>()` bytes, static buffer above.
            assert!(lean.as_static_str().is_some() == (len > 2 * size_of::<usize>()));
            let bytes = lean.as_bytes();
            let mut i = 0;
            while i < len {
                assert!(bytes[i] == expected.as_bytes()[i]);
                i += 1;
            }
            core::mem::forget(lean);
            len += 1;
        }
    }
}
```

2 つめは既存の `from_every_inline_length_in_const` を static バッファの長さまで広げたもので、置き換えてもよい。

## 確認方法

上のテストがコンパイルでき、通ること。
