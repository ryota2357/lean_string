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
