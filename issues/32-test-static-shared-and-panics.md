# static / 共有バッファでの `clear` / `split_off` と、heap 上でのユーザー関数の panic がテストされていない

- 種別: test
- 対象: `src/repr.rs` の `Repr::clear` / `Repr::retain`, `src/lib.rs` の `LeanString::try_split_off` / `impl Extend<char> for LeanString`
- 影響: 下の振る舞いを変えても既存のテストは通る。`retain` の panic 時の後始末 (`SetLenOnDrop`) は unsafe な `set_len` を含むが、inline の文字列でしか試されていない
- 優先度: 低

## 問題

`5dd94ea` の `tests/` を確認した結果。

- `clear`: `tests/lean_string.rs` の `clear_cow` と doctest は inline / unique な heap / 共有の heap だけ。static バッファの分岐 (`as_static_buffer_mut().set_len(0)`、static のまま長さ 0 にする) は通らない。この分岐を `self.replace_inner(Repr::new())` に変えても (inline の空文字列になる) `alloc_string` / `lean_string` / `lean_str` / `conversion` / `const` / `out_of_memory` と doctest はすべて通ることを確認した。
- `split_off`: `tests/alloc_string.rs` の 5 つのテストは unique な文字列だけ。static バッファ (`try_truncate` で static のまま縮む) と、共有された heap (64-bit では切り詰めがハンドルの長さだけを変え、相手は変わらない) の組み合わせが無い。
- `retain` の述語が panic した場合: `tests/alloc_string.rs` の `test_retain` に 1 件あるが、`"0è0"` (inline) だけ。heap の場合と、共有の heap を `ensure_modifiable` でコピーした直後の場合が無い。
- `Extend<char>` の途中でイテレータが panic した場合: `FromIterator<char>` には `from_iter_drops_reserved_buffer_on_panic` があるが、既存の文字列に `extend` する場合は無い。

## 修正例

足すテスト。x86_64 で `cargo test`、Miri (`-Zmiri-strict-provenance`) の x86_64 / i686 / powerpc64 / powerpc で通ることを確認した。

```rust
const INLINE_LIMIT: usize = size_of::<LeanString>();

#[test]
fn clear_static_keeps_static_buffer() {
    let text = "a static string longer than inline";
    let mut s = LeanString::from_static_str(text);
    let cloned = s.clone();
    s.clear();
    assert_eq!(s, "");
    assert_eq!(s.capacity(), 0);
    assert_eq!(s.as_static_str(), Some(""));
    assert!(!s.is_heap_allocated());
    assert_eq!(cloned, text);

    s.push('a');
    assert_eq!(s, "a");
    assert!(!s.is_heap_allocated());
    assert_eq!(s.capacity(), INLINE_LIMIT);
}

#[test]
fn split_off_static_and_shared() {
    let text = "0123456789abcdefghijklmnopqrstuvwxyz";

    let mut static_ = LeanString::from_static_str(text);
    let tail = static_.split_off(20);
    assert_eq!(static_, &text[..20]);
    assert_eq!(static_.as_static_str(), Some(&text[..20]));
    assert_eq!(tail, &text[20..]);

    let mut heap = LeanString::from(text);
    let shared = heap.clone();
    let tail = heap.split_off(10);
    assert_eq!(heap, &text[..10]);
    assert_eq!(tail, &text[10..]);
    assert_eq!(shared, text);

    let mut heap = LeanString::from(text);
    let shared = heap.clone();
    let tail = heap.split_off(INLINE_LIMIT + 1);
    assert_eq!(heap, &text[..INLINE_LIMIT + 1]);
    assert_eq!(tail, &text[INLINE_LIMIT + 1..]);
    assert_eq!(shared, text);
}

#[test]
#[cfg_attr(not(panic = "unwind"), ignore = "test requires unwinding support")]
fn retain_panic_on_heap_and_shared_heap() {
    let text = "αbcdefghijklmnopqrstuvwxyζ";
    for shared in [false, true] {
        let mut s = LeanString::from(text);
        let cloned = shared.then(|| s.clone());
        let mut count = 0;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            s.retain(|c| {
                count += 1;
                match count {
                    2 => false, // remove 'b'
                    5 => panic!("predicate panic"),
                    _ => c != 'c',
                }
            })
        }));
        assert!(result.is_err());
        // 'α' kept, 'b' and 'c' removed, 'd' kept; 'e' (the panicking call) and the rest dropped.
        assert_eq!(s, "αd");
        assert!(std::str::from_utf8(s.as_bytes()).is_ok());
        if let Some(cloned) = cloned {
            assert_eq!(cloned, text);
        }
        s.push_str("!");
        assert_eq!(s, "αd!");
    }
}

#[test]
#[cfg_attr(not(panic = "unwind"), ignore = "test requires unwinding support")]
fn extend_keeps_appended_part_when_iterator_panics() {
    struct PanicAfter(usize);
    impl Iterator for PanicAfter {
        type Item = char;
        fn next(&mut self) -> Option<char> {
            if self.0 == 0 {
                panic!("iterator panic");
            }
            self.0 -= 1;
            Some('x')
        }
    }

    let mut s = LeanString::from("a heap-allocated string, longer than inline");
    let shared = s.clone();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.extend(PanicAfter(3))));
    assert!(result.is_err());
    assert_eq!(s, "a heap-allocated string, longer than inlinexxx");
    assert_eq!(shared, "a heap-allocated string, longer than inline");
}
```

`retain_panic_on_heap_and_shared_heap` の期待値 `"αd"` は、panic した呼び出しの文字とそれ以降を捨てる今の実装 (`SetLenOnDrop` が `dst_idx` で長さを確定する) と、`String::retain` の振る舞いに合わせたもの。panic 時に残りの文字を保つ方針に変えるなら期待値も変える。

## 確認方法

上のテストが通ること。`Repr::clear` の static の分岐を変えると `clear_static_keeps_static_buffer` が落ちることを確認した。
