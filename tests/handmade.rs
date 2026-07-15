use lean_string::{LeanStr, LeanString};

const INLINE_LIMIT: usize = size_of::<LeanString>();

#[cfg(target_pointer_width = "32")]
const CAPACITY_WITH_HEAP_LENGTH_LAYOUT: usize = (1 << 24) - 1;

// LeanString basic API

#[test]
fn new_empty() {
    assert_eq!(LeanString::new(), "");

    let s = LeanString::new();
    assert_eq!(s.as_str(), "");
    assert!(s.is_empty());
    assert_eq!(s.len(), 0);
    assert!(!s.is_heap_allocated());
    assert_eq!(s.capacity(), INLINE_LIMIT);
}

#[cfg(target_pointer_width = "32")]
#[test]
fn drop_large_capacity_with_inline_length() {
    let string = LeanString::with_capacity(CAPACITY_WITH_HEAP_LENGTH_LAYOUT);
    assert_eq!(string.len(), 0);
    assert_eq!(string.capacity(), CAPACITY_WITH_HEAP_LENGTH_LAYOUT);
    drop(string);
}

#[cfg(target_pointer_width = "32")]
#[test]
fn realloc_large_capacity_with_inline_length() {
    let mut string = LeanString::new();
    string.reserve(CAPACITY_WITH_HEAP_LENGTH_LAYOUT);
    assert_eq!(string.len(), 0);
    assert_eq!(string.capacity(), CAPACITY_WITH_HEAP_LENGTH_LAYOUT);

    string.reserve(CAPACITY_WITH_HEAP_LENGTH_LAYOUT + 1);
    assert_eq!(string.len(), 0);
    assert_eq!(string.capacity(), CAPACITY_WITH_HEAP_LENGTH_LAYOUT + 1);
}

#[test]
fn new_from_char() {
    assert_eq!(LeanString::from('a'), "a");
    assert_eq!(LeanString::from('👍'), "👍");
    assert_eq!(LeanString::from(''), "");
}

#[test]
fn from_around_inline_limit() {
    let s = &String::from("0123456789abcdefg");

    let inline = LeanString::from(&s[..INLINE_LIMIT - 1]);
    assert_eq!(inline, s[..INLINE_LIMIT - 1]);
    assert!(!inline.is_heap_allocated());
    assert_eq!(inline.capacity(), INLINE_LIMIT);

    let inline = LeanString::from(&s[..INLINE_LIMIT]);
    assert_eq!(inline, s[..INLINE_LIMIT]);
    assert!(!inline.is_heap_allocated());
    assert_eq!(inline.capacity(), INLINE_LIMIT);

    let heap = LeanString::from(&s[..INLINE_LIMIT + 1]);
    assert_eq!(heap, s[..INLINE_LIMIT + 1]);
    assert!(heap.is_heap_allocated());
    assert_eq!(heap.capacity(), INLINE_LIMIT + 1);
}

#[test]
fn from_around_inline_limit_static() {
    let s: &'static str = "0123456789abcdefg";

    let inline = LeanString::from_static_str(&s[..INLINE_LIMIT - 1]);
    assert_eq!(inline, s[..INLINE_LIMIT - 1]);
    assert!(!inline.is_heap_allocated());
    assert_eq!(inline.capacity(), INLINE_LIMIT);

    let inline = LeanString::from_static_str(&s[..INLINE_LIMIT]);
    assert_eq!(inline, s[..INLINE_LIMIT]);
    assert!(!inline.is_heap_allocated());
    assert_eq!(inline.capacity(), INLINE_LIMIT);

    let static_ = LeanString::from_static_str(&s[..INLINE_LIMIT + 1]);
    assert_eq!(static_, s[..INLINE_LIMIT + 1]);
    assert!(!static_.is_heap_allocated());
    assert_eq!(static_.capacity(), INLINE_LIMIT + 1);
}

#[test]
fn shrink_to_inline_buffer() {
    let mut inline = LeanString::from("Hello");
    assert!(!inline.is_heap_allocated());
    assert_eq!(inline.capacity(), INLINE_LIMIT);

    inline.shrink_to(100);
    assert_eq!(inline, "Hello");
    assert_eq!(inline.capacity(), INLINE_LIMIT);
    assert!(!inline.is_heap_allocated());

    inline.shrink_to(3);
    assert_eq!(inline, "Hello");
    assert_eq!(inline.capacity(), INLINE_LIMIT);
    assert!(!inline.is_heap_allocated());
}

#[test]
fn shrink_to_static_buffer() {
    let text: &'static str = "A static str that is longer than inline limit";
    let mut static_ = LeanString::from_static_str(text);
    assert!(!static_.is_heap_allocated());

    static_.shrink_to(30);
    assert_eq!(static_.capacity(), text.len());
    assert!(!static_.is_heap_allocated());

    static_.shrink_to(10);
    assert_eq!(static_.capacity(), text.len());
    assert!(!static_.is_heap_allocated());
}

#[test]
fn shrink_to_heap_buffer() {
    let text = "A heap allocated string that is longer than inline limit";
    let mut heap = LeanString::from(text);
    assert!(heap.is_heap_allocated());
    let original_capacity = heap.capacity();

    heap.shrink_to(100);
    assert_eq!(heap.capacity(), original_capacity);
    assert_eq!(heap, text);
    assert!(heap.is_heap_allocated());

    heap.shrink_to(30);
    assert_eq!(heap.capacity(), original_capacity);
    assert_eq!(heap, text);
    assert!(heap.is_heap_allocated());

    heap.shrink_to(0);
    assert_eq!(heap.capacity(), heap.len());
    assert_eq!(heap, text);
    assert!(heap.is_heap_allocated());
}

#[test]
fn shrink_to_heap_to_inline() {
    let mut heap = LeanString::with_capacity(100);
    heap.push_str("Hello");
    assert!(heap.is_heap_allocated());
    assert_eq!(heap.capacity(), 100);
    assert!(heap.len() < INLINE_LIMIT);

    heap.shrink_to(100);
    assert_eq!(heap.capacity(), 100);
    assert_eq!(heap, "Hello");

    heap.shrink_to(120);
    assert_eq!(heap.capacity(), 100);

    heap.shrink_to(50);
    assert_eq!(heap.capacity(), 50);
    assert_eq!(heap, "Hello");
    assert!(heap.is_heap_allocated());

    heap.shrink_to(INLINE_LIMIT);
    assert_eq!(heap.capacity(), INLINE_LIMIT);
    assert_eq!(heap, "Hello");
    assert!(!heap.is_heap_allocated());

    heap.shrink_to(0);
    assert_eq!(heap.capacity(), INLINE_LIMIT);
    assert_eq!(heap, "Hello");
    assert!(!heap.is_heap_allocated());
}

#[test]
fn shrink_to_cow_shared_buffer() {
    let mut heap1 = LeanString::with_capacity(100);
    heap1.push_str("Shared");

    let mut heap2 = heap1.clone();
    assert_eq!(heap1.as_ptr(), heap2.as_ptr());

    heap1.shrink_to(50);
    assert_eq!(heap1.capacity(), 50);
    assert_eq!(heap1, "Shared");

    assert_eq!(heap2.capacity(), 100);
    assert_eq!(heap2, "Shared");

    assert_ne!(heap1.as_ptr(), heap2.as_ptr());

    let heap3 = heap2.clone();
    assert_eq!(heap2.as_ptr(), heap3.as_ptr());

    heap2.shrink_to(0); // = shrink_to_fit
    assert_eq!(heap2.capacity(), INLINE_LIMIT);
    assert_eq!(heap2, "Shared");
    assert!(!heap2.is_heap_allocated());

    assert_eq!(heap3.capacity(), 100);
    assert_eq!(heap3, "Shared");
    assert!(heap3.is_heap_allocated());

    assert_ne!(heap2.as_ptr(), heap3.as_ptr());
}

#[test]
fn shrink_cow_shared_buffer_to_exact_capacity() {
    let text = "a".repeat(100);
    let mut shrink_to = LeanString::from(text.as_str());
    shrink_to.reserve(50);
    assert_eq!(shrink_to.capacity(), 150);

    let mut shrink_to_fit = shrink_to.clone();
    let original = shrink_to.clone();

    shrink_to.shrink_to(120);
    assert_eq!(shrink_to, text);
    assert_eq!(shrink_to.capacity(), 120);

    shrink_to_fit.shrink_to_fit();
    assert_eq!(shrink_to_fit, text);
    assert_eq!(shrink_to_fit.capacity(), 100);

    assert_eq!(original, text);
    assert_eq!(original.capacity(), 150);
}

#[test]
fn push_cow() {
    let mut s = LeanString::new();
    s.push('a');
    s.push('b');
    s.push_str("cdefgh");
    assert_eq!(s, "abcdefgh");
    assert_eq!(s.len(), 8);

    s.push_str("12345678");
    assert_eq!(s.len(), 16);
    assert_eq!(s, "abcdefgh12345678");

    // clone and push
    let mut s1 = s.clone();
    assert_eq!(s1, "abcdefgh12345678");
    s1.push('0');
    assert_eq!(s1, "abcdefgh123456780");
    assert_eq!(s1.len(), 17);

    // clone and push_str
    let mut s2 = s.clone();
    s2.push_str("90");
    assert_eq!(s2, "abcdefgh1234567890");
    assert_eq!(s2.len(), 18);

    // s is not changed
    assert_eq!(s.len(), 16);

    // s into heap
    s.push_str("90");
    assert!(s.is_heap_allocated());
    assert_eq!(s.len(), 18);

    // clone and push
    let mut s3 = s.clone();
    s3.push('');
    assert_eq!(s3, "abcdefgh1234567890");
    assert_eq!(s3.len(), 21);

    // clone and push_str
    let mut s4 = s.clone();
    s4.push_str("👍👍");
    assert_eq!(s4.len(), 26);
    assert_eq!(s4, "abcdefgh1234567890👍👍");
}

#[test]
fn push_to_static() {
    let mut inline = LeanString::from_static_str("abcdefgh");
    assert_eq!(inline, "abcdefgh");
    assert_eq!(inline.len(), 8);
    assert!(!inline.is_heap_allocated());
    assert_eq!(inline.capacity(), INLINE_LIMIT);

    inline.push_str("12345678");
    assert_eq!(inline, "abcdefgh12345678");
    assert_eq!(inline.len(), 16);
    if cfg!(target_pointer_width = "64") {
        assert!(!inline.is_heap_allocated());
        assert_eq!(inline.capacity(), 16);
    } else {
        assert!(inline.capacity() >= 16);
    }

    inline.push_str("90");
    assert_eq!(inline, "abcdefgh1234567890");
    assert_eq!(inline.len(), 18);
    assert!(inline.is_heap_allocated());

    let mut static_ = LeanString::from_static_str("abcdefghijklmnopqrstuvwxyz");
    assert_eq!(static_, "abcdefghijklmnopqrstuvwxyz");
    assert_eq!(static_.len(), 26);
    assert!(!static_.is_heap_allocated());

    static_.push('0');
    assert_eq!(static_, "abcdefghijklmnopqrstuvwxyz0");
    assert_eq!(static_.len(), 27);
    assert!(static_.is_heap_allocated());
}

#[test]
fn pop_keep_capacity() {
    let mut inline = LeanString::from("Hello!");
    assert_eq!(inline.pop(), Some('!'));
    assert_eq!(inline, "Hello");
    assert_eq!(inline.len(), 5);

    for _ in 0..4 {
        inline.pop();
    }
    assert_eq!(inline, "H");
    assert_eq!(inline.pop(), Some('H'));
    assert_eq!(inline, "");
    assert!(inline.is_empty());
    assert_eq!(inline.capacity(), INLINE_LIMIT);

    let mut heap = LeanString::from("abcdefghijklmnopqrstuvwxyz");
    assert_eq!(heap.pop(), Some('z'));
    assert_eq!(heap, "abcdefghijklmnopqrstuvwxy");
    assert_eq!(heap.len(), 25);

    for _ in 0..24 {
        heap.pop();
    }
    assert_eq!(heap, "a");
    assert_eq!(heap.pop(), Some('a'));
    assert_eq!(heap, "");
    assert!(heap.is_empty());
    assert_eq!(heap.capacity(), 26);
}

#[test]
fn pop_share_buffer() {
    // s is inlined
    let mut s = LeanString::from("abcdefgh");
    assert_eq!(s.pop(), Some('h'));
    assert_eq!(s.len(), 7);

    let mut s1 = s.clone();
    assert_eq!(s1.pop(), Some('g'));
    assert_eq!(s1, "abcdef");
    assert_eq!(s1.len(), 6);

    // s is not changed
    assert_eq!(s, "abcdefg");

    // s into heap
    s.push_str("hijklmnopqrstuvwxyz");

    let mut s2 = s.clone();
    assert_eq!(s.as_ptr(), s2.as_ptr());

    assert_eq!(s2.pop(), Some('z'));
    assert_eq!(s2.len(), 25);

    // s is not changed
    assert_eq!(s, "abcdefghijklmnopqrstuvwxyz");

    // buffer is shared
    assert_eq!(s.as_ptr(), s2.as_ptr());

    // modify makes a new buffer
    s2.push('0');
    assert_ne!(s.as_ptr(), s2.as_ptr());
}

#[test]
fn pop_from_static() {
    let mut static_ = LeanString::from_static_str("abcdefghijklmnopqrstuvwxyz");
    assert_eq!(static_.len(), 26);
    assert_eq!(static_.pop(), Some('z'));
    assert_eq!(static_, "abcdefghijklmnopqrstuvwxy");
    assert_eq!(static_.len(), 25);

    // static_ capacity equals to len
    assert_eq!(static_.capacity(), static_.len());

    // pop in static buffer is only changing its length
    assert!(!static_.is_heap_allocated());
}

#[test]
fn pop_from_static_cow() {
    let mut static1 = LeanString::from_static_str("0123456789abcdef!");
    assert_eq!(static1.pop(), Some('!'));
    let static2 = static1.clone();
    assert_eq!(static1.pop(), Some('f'));

    assert_eq!(static1, "0123456789abcde");
    assert_eq!(static1.capacity(), static1.len());
    assert!(!static1.is_heap_allocated());

    assert_eq!(static2, "0123456789abcdef");
    assert_eq!(static2.capacity(), static2.len());
    assert!(!static2.is_heap_allocated());

    assert_eq!(static1.as_ptr(), static2.as_ptr());
}

#[test]
fn pop_from_empty() {
    let mut inline = LeanString::new();
    assert_eq!(inline, "");
    assert_eq!(inline.pop(), None);
    assert_eq!(inline, "");

    let mut heap = LeanString::from("a".repeat(INLINE_LIMIT + 1));
    for _ in 0..INLINE_LIMIT + 1 {
        heap.pop();
    }
    assert_eq!(inline, "");
    assert_eq!(heap.pop(), None);
    assert_eq!(heap, "");

    let mut static_ = LeanString::from_static_str("");
    assert_eq!(static_.pop(), None);
    assert_eq!(static_, "");
}

#[test]
fn remove_cow() {
    let mut inline = LeanString::from("Hello");
    assert_eq!(inline.remove(4), 'o');
    assert_eq!(inline.remove(0), 'H');
    assert_eq!(inline, "ell");

    let mut heap = LeanString::from("abcdefghijklmnopqrstuvwxyz");
    assert_eq!(heap.remove(0), 'a');
    let cloned = heap.clone();
    assert_eq!(heap.as_ptr(), cloned.as_ptr());
    assert_eq!(heap.remove(24), 'z');
    assert_eq!(heap, "bcdefghijklmnopqrstuvwxy");
    assert_eq!(cloned, "bcdefghijklmnopqrstuvwxyz");
}

#[test]
#[should_panic(expected = "index out of bounds (index: 12, len: 12)")]
fn remove_fail() {
    let mut s = LeanString::from("Hello World!");
    assert_eq!(s.len(), 12);
    s.remove(12);
}

#[test]
fn retain_f_apply_count() {
    let mut inline = LeanString::from("012");
    let mut count = 0;
    inline.retain(|_| {
        count += 1;
        true
    });
    assert_eq!(count, 3);

    let mut heap = LeanString::from("abcdefghijklmnopqrstuvwxyz");
    let mut count = 0;
    heap.retain(|_| {
        count += 1;
        true
    });
    assert_eq!(count, 26);
}

#[test]
fn retain_cow() {
    let mut heap = LeanString::from("qwer tyui opas dfgh jklz xcvb nm");
    let cloned = heap.clone();
    assert_eq!(heap.as_ptr(), cloned.as_ptr());
    heap.retain(|c| c.is_alphabetic());
    assert_eq!(heap, "qwertyuiopasdfghjklzxcvbnm");
    assert_eq!(cloned, "qwer tyui opas dfgh jklz xcvb nm");

    let mut static_ = LeanString::from_static_str("aBcDeFgHiJkLmNoPqRsTuVwXyZ");
    let cloned = static_.clone();
    static_.retain(|c| c.is_lowercase());
    assert!(!cloned.is_heap_allocated());
    assert_eq!(static_, "acegikmoqsuwy");
    assert_eq!(cloned, "aBcDeFgHiJkLmNoPqRsTuVwXyZ");
}

#[test]
fn insert() {
    let mut s = LeanString::from("01234");
    s.insert(3, 'a');
    assert_eq!(s, "012a34");
    assert_eq!(s.len(), 6);
    assert_eq!(s.capacity(), INLINE_LIMIT);

    s.insert(0, 'b');
    assert_eq!(s, "b012a34");
    assert_eq!(s.len(), 7);
    assert_eq!(s.capacity(), INLINE_LIMIT);

    s.insert(7, 'c');
    assert_eq!(s, "b012a34c");
    assert_eq!(s.len(), 8);
    assert_eq!(s.capacity(), INLINE_LIMIT);

    s.insert_str(8, "12345678");
    assert_eq!(s, "b012a34c12345678");
    assert_eq!(s.len(), 16);
    if cfg!(target_pointer_width = "64") {
        assert_eq!(s.capacity(), INLINE_LIMIT);
        assert!(!s.is_heap_allocated());
    }

    s.insert_str(0, "ABCDEFGH");
    assert_eq!(s, "ABCDEFGHb012a34c12345678");

    s.insert(20, '.');
    assert_eq!(s, "ABCDEFGHb012a34c1234.5678");
}

#[test]
fn insert_to_static() {
    let mut static_ = LeanString::from_static_str("01234567890123456789");
    let cloned = static_.clone();
    static_.insert(10, 'a');
    assert_eq!(static_, "0123456789a0123456789");
    assert!(static_.is_heap_allocated());
    assert_eq!(cloned, "01234567890123456789");
    assert!(!cloned.is_heap_allocated());
}

#[test]
#[should_panic(expected = "index is not a char boundary or out of bounds (index: 7)")]
fn insert_fail() {
    let mut s = LeanString::from("012345");
    s.insert(7, 'a');
}

#[test]
fn repeat_inline() {
    let s = LeanString::from("ab");
    assert!(!s.is_heap_allocated());

    assert_eq!(s.repeat(0), "");
    assert!(!s.repeat(0).is_heap_allocated());

    assert_eq!(s.repeat(1), "ab");

    let repeated = s.repeat(INLINE_LIMIT);
    assert_eq!(repeated, "ab".repeat(INLINE_LIMIT));
    assert!(repeated.is_heap_allocated());
}

#[test]
fn repeat_heap() {
    let s = LeanString::from("a".repeat(INLINE_LIMIT + 1).as_str());
    assert!(s.is_heap_allocated());

    assert_eq!(s.repeat(0), "");
    assert!(!s.repeat(0).is_heap_allocated());

    let r1 = s.repeat(1);
    assert_eq!(r1, s);
    assert_eq!(r1.as_ptr(), s.as_ptr()); // n == 1 returns clone

    let repeated = s.repeat(2);
    assert_eq!(repeated.len(), s.len() * 2);
    assert_eq!(repeated.capacity(), s.len() * 2);
    assert!(repeated.is_heap_allocated());
}

#[test]
fn repeat_empty() {
    let empty = LeanString::new();
    assert_eq!(empty.repeat(0), "");
    assert_eq!(empty.repeat(1), "");
    assert_eq!(empty.repeat(100), "");
}

#[test]
#[should_panic(expected = "Cannot allocate memory to hold LeanString")]
fn repeat_overflow() {
    let s = LeanString::from("ab");
    let _ = s.repeat(usize::MAX);
}

#[test]
fn truncate_keep_capacity() {
    let mut inline = LeanString::from("abcdef");

    assert!(!inline.is_heap_allocated());
    inline.truncate(3);
    assert_eq!(inline, "abc");
    assert_eq!(inline.len(), 3);
    assert!(!inline.is_heap_allocated());
    assert_eq!(inline.capacity(), INLINE_LIMIT);

    let mut heap = LeanString::from("a".repeat(INLINE_LIMIT + 10).as_str());
    let original_capacity = heap.capacity();
    assert!(heap.is_heap_allocated());

    heap.truncate(INLINE_LIMIT + 1);
    assert_eq!(heap, "a".repeat(INLINE_LIMIT + 1));
    assert_eq!(heap.len(), INLINE_LIMIT + 1);
    assert!(heap.is_heap_allocated());
    assert_eq!(heap.capacity(), original_capacity);

    heap.truncate(1);
    assert_eq!(heap, "a");
    assert_eq!(heap.len(), 1);
    assert!(heap.is_heap_allocated());
    assert_eq!(heap.capacity(), original_capacity);
}

#[test]
fn truncate_from_static() {
    let mut static_ = LeanString::from_static_str("abcdefghijklmnopqrstuvwxyz");
    assert_eq!(static_.len(), 26);
    assert!(!static_.is_heap_allocated());

    static_.truncate(20);
    assert_eq!(static_, "abcdefghijklmnopqrst");
    assert_eq!(static_.len(), 20);
    assert_eq!(static_.capacity(), 20);
    assert!(!static_.is_heap_allocated());
}

#[test]
fn truncate_share_buffer() {
    // s is inlined
    let mut s = LeanString::from("abcdefgh");
    assert_eq!(s.len(), 8);

    let mut s1 = s.clone();
    s1.truncate(4);
    assert_eq!(s1, "abcd");
    assert_eq!(s1.len(), 4);

    // s is not changed
    assert_eq!(s, "abcdefgh");

    // s into heap
    s.push_str("ijklmnopqrstuvwxyz");
    assert_eq!(s.len(), 26);
    assert!(s.is_heap_allocated());

    // buffer is shared
    let mut s2 = s.clone();
    s2.truncate(20);
    assert_eq!(s.as_ptr(), s2.as_ptr());

    // modify makes a new buffer
    s2.push('0');
    assert_ne!(s.as_ptr(), s2.as_ptr());
}

#[test]
fn convert_static_to_inline_with_reserve() {
    let s: &'static str = "1234567890ABCDEFGHIJ";
    let mut static_ = LeanString::from_static_str(s);

    for _ in 0..14 {
        static_.pop();
    }

    assert_eq!(static_, "123456");
    assert_eq!(static_.capacity(), static_.len()); // still in static buffer

    static_.reserve(1);
    assert_eq!(static_.capacity(), INLINE_LIMIT);
}

#[test]
fn clear_cow() {
    let mut inline = LeanString::from("foo");
    inline.clear();
    assert_eq!(inline, "");

    let mut heap: LeanString = core::iter::repeat_n('a', 100).collect();
    let cloned = heap.clone();
    heap.clear();

    assert_eq!(heap, "");
    assert_eq!(cloned.len(), 100);

    // heap is changed to inline
    assert_eq!(heap.capacity(), INLINE_LIMIT);
    assert!(!heap.is_heap_allocated());
}

#[test]
fn extend_char() {
    let mut s = LeanString::from("Hello, ");
    s.extend("world!".chars());
    assert_eq!(s, "Hello, world!");
}

#[test]
#[cfg_attr(not(panic = "unwind"), ignore = "test requires unwinding support")]
fn from_iter_drops_reserved_buffer_on_panic() {
    struct PanickingIterator;
    impl Iterator for PanickingIterator {
        type Item = char;
        fn next(&mut self) -> Option<Self::Item> {
            panic!("iterator panic");
        }
        fn size_hint(&self) -> (usize, Option<usize>) {
            (17, Some(17))
        }
    }

    let result = std::panic::catch_unwind(|| PanickingIterator.collect::<LeanString>());
    assert!(result.is_err());
}

#[test]
#[cfg(target_pointer_width = "32")]
#[cfg_attr(not(panic = "unwind"), ignore = "test requires unwinding support")]
fn from_iter_drops_prefixed_reserved_buffer_on_panic() {
    struct PanickingIterator;
    impl Iterator for PanickingIterator {
        type Item = char;
        fn next(&mut self) -> Option<Self::Item> {
            panic!("iterator panic");
        }
        fn size_hint(&self) -> (usize, Option<usize>) {
            (CAPACITY_WITH_HEAP_LENGTH_LAYOUT, Some(CAPACITY_WITH_HEAP_LENGTH_LAYOUT))
        }
    }

    let result = std::panic::catch_unwind(|| PanickingIterator.collect::<LeanString>());
    assert!(result.is_err());
}

#[test]
fn from_iter_owned_lean_strings_reuses_first() {
    let text = "abcdefghijklmnopqrstuvwxyz012345";

    let s = LeanString::from(text);
    let s_ptr = s.as_ptr();
    let one: LeanString = [s].into_iter().collect();
    assert_eq!(one, "abcdefghijklmnopqrstuvwxyz012345");
    assert_eq!(one.as_ptr(), s_ptr);

    let mut s = LeanString::with_capacity(128);
    s.push_str("abcdefghijklmnopqrstuvwxyz012345");
    let s_ptr = s.as_ptr();
    let tow: LeanString = [s, LeanString::from("6789")].into_iter().collect();
    assert_eq!(tow, text.to_owned() + "6789");
    assert_eq!(tow.as_ptr(), s_ptr);
    assert_eq!(tow.capacity(), 128);
}

#[test]
fn from_iter_owned_lean_strings_detaches_shared_first() {
    let s = LeanString::from("abcdefghijklmnopqrstuvwxyz012345");
    let cloned = s.clone();
    let clonded_ptr = cloned.as_ptr();
    let collected: LeanString = [s, LeanString::from("6789")].into_iter().collect();
    assert_eq!(cloned, "abcdefghijklmnopqrstuvwxyz012345");
    assert_eq!(cloned.as_ptr(), clonded_ptr);
    assert_eq!(collected, "abcdefghijklmnopqrstuvwxyz0123456789");
    assert_ne!(collected.as_ptr(), clonded_ptr);
}

#[test]
fn from_iter_no_owned_lean_strings_is_empty_inline() {
    let collected: LeanString = core::iter::empty::<LeanString>().collect();
    assert!(collected.is_empty());
    assert!(!collected.is_heap_allocated());
}

// LeanStr basic API

#[test]
fn lean_str_new_empty() {
    assert_eq!(LeanStr::new(), "");

    let s = LeanStr::new();
    assert_eq!(s.as_str(), "");
    assert!(s.is_empty());
    assert_eq!(s.len(), 0);
    assert!(!s.is_heap_allocated());

    assert_eq!(LeanStr::default(), "");
}

#[test]
fn lean_str_from_char() {
    assert_eq!(LeanStr::from('a'), "a");
    assert_eq!(LeanStr::from('👍'), "👍");
    assert_eq!(LeanStr::from(''), "");
}

#[test]
fn lean_str_from_around_inline_limit() {
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
fn lean_str_from_around_inline_limit_static() {
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
fn lean_str_clone_shares_heap_buffer() {
    let s = LeanStr::from("abcdefghijklmnopqrstuvwxyz");
    let cloned = s.clone();
    assert_eq!(s.as_ptr(), cloned.as_ptr());

    drop(s);
    assert_eq!(cloned, "abcdefghijklmnopqrstuvwxyz");
}

// LeanStr <-> LeanString conversion

#[test]
fn convert_inline_string_to_str() {
    let lean_string = LeanString::from("short");
    assert!(!lean_string.is_heap_allocated());

    let lean_str = lean_string.into_lean_str();
    assert_eq!(lean_str, "short");
    assert!(!lean_str.is_heap_allocated());
}

#[test]
fn convert_inline_str_to_string() {
    let lean_str = LeanString::from("short").into_lean_str();
    assert_eq!(lean_str, "short");
    assert!(!lean_str.is_heap_allocated());

    let lean_string = lean_str.into_lean_string();
    assert_eq!(lean_string, "short");
    assert!(!lean_string.is_heap_allocated());
    assert_eq!(lean_string.capacity(), INLINE_LIMIT);
}

#[test]
fn convert_static_string_to_str() {
    let text: &'static str = "A static str that is longer than inline limit";
    let ptr = text.as_ptr();

    let lean_string = LeanString::from_static_str(text);
    assert!(!lean_string.is_heap_allocated());
    assert_eq!(lean_string.as_ptr(), ptr);

    let lean_str = lean_string.into_lean_str();
    assert_eq!(lean_str, text);
    assert!(!lean_str.is_heap_allocated());
    assert_eq!(lean_str.as_ptr(), ptr);
}

#[test]
fn convert_static_str_to_string() {
    let text: &'static str = "A static str that is longer than inline limit";
    let ptr = text.as_ptr();

    let lean_str = LeanString::from_static_str(text).into_lean_str();
    assert_eq!(lean_str, text);
    assert!(!lean_str.is_heap_allocated());
    assert_eq!(lean_str.as_ptr(), ptr);

    let lean_string = lean_str.into_lean_string();
    assert_eq!(lean_string, text);
    assert!(!lean_string.is_heap_allocated());
    assert_eq!(lean_string.as_ptr(), ptr);
}

#[test]
fn convert_unique_heap_string_to_str() {
    let text = "a heap-allocated string, longer than the inline limit";

    let lean_string = LeanString::from(text);
    assert!(lean_string.is_heap_allocated());

    let lean_str = lean_string.into_lean_str();
    assert_eq!(lean_str, text);
    assert!(lean_str.is_heap_allocated());
}

#[test]
fn convert_unique_heap_str_to_string() {
    let text = "a heap-allocated string, longer than the inline limit";

    let lean_str = LeanString::from(text).into_lean_str();
    assert_eq!(lean_str, text);
    assert!(lean_str.is_heap_allocated());

    let mut lean_string = lean_str.into_lean_string();
    assert_eq!(lean_string, text);
    assert!(lean_string.is_heap_allocated());
    assert_eq!(lean_string.capacity(), text.len());

    lean_string.push_str(" ...and it is still growable");
    assert_eq!(lean_string, text.to_owned() + " ...and it is still growable");
}

#[test]
fn convert_unique_heap_drops_extra_capacity() {
    let text = "content that is longer than the inline limit";
    let mut lean_string = LeanString::with_capacity(100);
    lean_string.push_str(text);
    assert_eq!(lean_string.capacity(), 100);

    let round_tripped = lean_string.into_lean_str().into_lean_string();
    assert_eq!(round_tripped, text);
    assert_eq!(round_tripped.capacity(), text.len());
}

#[test]
fn convert_unique_small_heap_into_inline() {
    let text = "a heap-allocated string, longer than the inline limit";
    let mut lean_string = LeanString::from(text);
    for _ in 0..(text.len() - INLINE_LIMIT) {
        lean_string.pop();
    }
    assert_eq!(lean_string.len(), INLINE_LIMIT);
    assert!(lean_string.is_heap_allocated());

    let lean_str = lean_string.into_lean_str();
    assert_eq!(lean_str, &text[..INLINE_LIMIT]);
    assert!(!lean_str.is_heap_allocated());
}

#[test]
fn convert_shared_heap_string_to_str() {
    let text = "a shared heap-allocated string, longer than inline";
    let lean_string = LeanString::from(text);
    let shared = lean_string.clone();

    let lean_str = lean_string.into_lean_str();
    assert_eq!(lean_str, text);
    assert!(lean_str.is_heap_allocated());
    assert_ne!(lean_str.as_ptr(), shared.as_ptr());

    assert_eq!(shared, text);
    assert!(shared.is_heap_allocated());
}

#[test]
fn convert_shared_heap_str_to_string() {
    let text = "a shared heap-allocated string, longer than inline";
    let lean_str = LeanStr::from(text);
    let shared = lean_str.clone();

    let mut lean_string = lean_str.into_lean_string();
    assert_ne!(lean_string.as_ptr(), shared.as_ptr());

    lean_string.push('!');
    assert_eq!(lean_string, text.to_owned() + "!");

    assert_eq!(shared, text);
    assert!(shared.is_heap_allocated());
}

#[cfg(target_pointer_width = "32")]
#[test]
fn convert_drops_capacity_length_prefix_layout() {
    let text = "content that is longer than the inline limit";
    let mut lean_string = LeanString::with_capacity(CAPACITY_WITH_HEAP_LENGTH_LAYOUT);
    lean_string.push_str(text);

    let lean_str = lean_string.into_lean_str();
    assert_eq!(lean_str, text);
    assert!(lean_str.is_heap_allocated());

    let lean_string = lean_str.into_lean_string();
    assert_eq!(lean_string, text);
    assert_eq!(lean_string.capacity(), text.len());
}

#[cfg(target_pointer_width = "32")]
#[test]
fn convert_length_stored_heap_str_to_string() {
    let len = CAPACITY_WITH_HEAP_LENGTH_LAYOUT;
    let text = "a".repeat(len);

    let lean_str = LeanStr::from(text.as_str());
    assert_eq!(lean_str.len(), len);
    assert!(lean_str.is_heap_allocated());
    assert!(lean_str.ends_with('a'));

    let lean_string = lean_str.into_lean_string();
    assert_eq!(lean_string.len(), len);
    assert_eq!(lean_string.capacity(), len);
    assert_eq!(lean_string, text);
}
