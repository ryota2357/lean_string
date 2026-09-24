use lean_string::LeanStr;

const INLINE_LIMIT: usize = size_of::<LeanStr>();

#[test]
fn new_empty() {
    assert_eq!(LeanStr::new(), "");

    let s = LeanStr::new();
    assert_eq!(s.as_str(), "");
    assert!(s.is_empty());
    assert_eq!(s.len(), 0);
    assert!(!s.is_heap_allocated());

    assert_eq!(LeanStr::default(), "");
}

#[test]
fn from_char() {
    assert_eq!(LeanStr::from('a'), "a");
    assert_eq!(LeanStr::from('👍'), "👍");
    assert_eq!(LeanStr::from(''), "");
}

#[test]
fn from_char_utf8_length_boundaries() {
    let chars = [
        '\0',
        'a',
        '\u{7F}',
        '\u{80}',
        'é',
        '\u{7FF}',
        '\u{800}',
        'あ',
        '\u{FFFF}',
        '\u{10000}',
        '🦀',
        '\u{10FFFF}',
    ];
    for ch in chars {
        let s = LeanStr::from(ch);
        assert_eq!(s, String::from(ch));
        assert_eq!(s.len(), ch.len_utf8());
        assert!(!s.is_heap_allocated());
    }
}

#[test]
fn from_around_inline_limit() {
    let s = &String::from("0123456789abcdefg");

    let inline = LeanStr::from(&s[..INLINE_LIMIT - 1]);
    assert_eq!(inline, s[..INLINE_LIMIT - 1]);
    assert_eq!(inline.len(), INLINE_LIMIT - 1);
    assert!(!inline.is_heap_allocated());

    let inline = LeanStr::from(&s[..INLINE_LIMIT]);
    assert_eq!(inline, s[..INLINE_LIMIT]);
    assert_eq!(inline.len(), INLINE_LIMIT);
    assert!(!inline.is_heap_allocated());

    let heap = LeanStr::from(&s[..INLINE_LIMIT + 1]);
    assert_eq!(heap, s[..INLINE_LIMIT + 1]);
    assert_eq!(heap.len(), INLINE_LIMIT + 1);
    assert!(heap.is_heap_allocated());
}

#[test]
fn from_static_str_around_inline_limit() {
    let s: &'static str = "0123456789abcdefg";

    let inline = LeanStr::from_static_str(&s[..INLINE_LIMIT - 1]);
    assert_eq!(inline, s[..INLINE_LIMIT - 1]);
    assert!(!inline.is_heap_allocated());

    let inline = LeanStr::from_static_str(&s[..INLINE_LIMIT]);
    assert_eq!(inline, s[..INLINE_LIMIT]);
    assert!(!inline.is_heap_allocated());

    let static_ = LeanStr::from_static_str(&s[..INLINE_LIMIT + 1]);
    assert_eq!(static_, s[..INLINE_LIMIT + 1]);
    assert!(!static_.is_heap_allocated());
}

#[test]
fn clone_shares_heap_buffer() {
    let s = LeanStr::from("abcdefghijklmnopqrstuvwxyz");
    let cloned = s.clone();
    assert_eq!(s.as_ptr(), cloned.as_ptr());

    drop(s);
    assert_eq!(cloned, "abcdefghijklmnopqrstuvwxyz");
}

#[test]
fn repeat_around_inline() {
    let s = LeanStr::from("a");
    assert!(!s.is_heap_allocated());

    assert_eq!(s.repeat(1), "a");

    let inline = s.repeat(INLINE_LIMIT);
    assert_eq!(inline, "a".repeat(INLINE_LIMIT));
    assert!(!inline.is_heap_allocated());

    let heap = s.repeat(INLINE_LIMIT + 1);
    assert_eq!(heap, "a".repeat(INLINE_LIMIT + 1));
    assert!(heap.is_heap_allocated());
}

#[test]
fn repeat_zero_or_empty() {
    let inline = LeanStr::from("ab");
    assert!(!inline.is_heap_allocated());
    assert_eq!(inline.repeat(0), "");
    assert!(!inline.repeat(0).is_heap_allocated());

    let heap = LeanStr::from("a".repeat(INLINE_LIMIT + 1).as_str());
    assert!(heap.is_heap_allocated());
    assert_eq!(heap.repeat(0), "");
    assert!(!heap.repeat(0).is_heap_allocated());

    let empty = LeanStr::new();
    assert_eq!(empty.repeat(0), "");
    assert_eq!(empty.repeat(1), "");
    assert_eq!(empty.repeat(100), "");
}

#[test]
fn repeat_heap() {
    let s = LeanStr::from("a".repeat(INLINE_LIMIT + 1).as_str());
    assert!(s.is_heap_allocated());

    let r1 = s.repeat(1);
    assert_eq!(r1, s);
    assert_eq!(r1.as_ptr(), s.as_ptr()); // n == 1 returns clone

    let r2 = s.repeat(2);
    assert_eq!(r2.len(), s.len() * 2);
    assert!(r2.is_heap_allocated());

    let r5 = s.repeat(5);
    assert_eq!(r5.len(), s.len() * 5);
    assert!(r5.is_heap_allocated());
}

#[test]
fn repeat_static() {
    let s = LeanStr::from_static_str("0123456789abcdefghijklmnopqrstuvwxyz");
    assert!(!s.is_heap_allocated());
    assert!(s.as_static_str().is_some());

    let r1 = s.repeat(1);
    assert!(!r1.is_heap_allocated());
    assert_eq!(r1.as_static_str(), s.as_static_str());

    let r2 = s.repeat(2);
    assert!(r2.is_heap_allocated());
    assert_eq!(r2, s.as_str().repeat(2))
}

#[test]
fn try_repeat_overflow_is_err() {
    let s = LeanStr::from("ab");
    assert!(s.try_repeat(usize::MAX).is_err());
}
