use lean_string::{LeanStr, LeanString};

const INLINE_LIMIT: usize = size_of::<LeanString>();

#[cfg(target_pointer_width = "32")]
const MIN_CAPACITY_FOR_HEAP_LENGTH_LAYOUT_ON_32BIT: usize = (1 << 24) - 1;

#[test]
fn inline_string_to_str() {
    let lean_string = LeanString::from("short");
    assert!(!lean_string.is_heap_allocated());

    let lean_str = lean_string.into_lean_str();
    assert_eq!(lean_str, "short");
    assert!(!lean_str.is_heap_allocated());
}

#[test]
fn inline_str_to_string() {
    let lean_str = LeanString::from("short").into_lean_str();
    assert_eq!(lean_str, "short");
    assert!(!lean_str.is_heap_allocated());

    let lean_string = lean_str.into_lean_string();
    assert_eq!(lean_string, "short");
    assert!(!lean_string.is_heap_allocated());
    assert_eq!(lean_string.capacity(), INLINE_LIMIT);
}

#[test]
fn static_string_to_str() {
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
fn static_str_to_string() {
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
fn unique_heap_string_to_str() {
    let text = "a heap-allocated string, longer than the inline limit";

    let lean_string = LeanString::from(text);
    assert!(lean_string.is_heap_allocated());

    let lean_str = lean_string.into_lean_str();
    assert_eq!(lean_str, text);
    assert!(lean_str.is_heap_allocated());
}

#[test]
fn unique_heap_str_to_string() {
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
fn unique_heap_drops_extra_capacity() {
    let text = "content that is longer than the inline limit";
    let mut lean_string = LeanString::with_capacity(100);
    lean_string.push_str(text);
    assert_eq!(lean_string.capacity(), 100);

    let round_tripped = lean_string.into_lean_str().into_lean_string();
    assert_eq!(round_tripped, text);
    assert_eq!(round_tripped.capacity(), text.len());
}

#[cfg(target_pointer_width = "32")]
#[test]
fn unique_heap_drops_capacity_length_prefix_layout() {
    let text = "content that is longer than the inline limit";
    let mut lean_string = LeanString::with_capacity(MIN_CAPACITY_FOR_HEAP_LENGTH_LAYOUT_ON_32BIT);
    lean_string.push_str(text);

    let lean_str = lean_string.into_lean_str();
    assert_eq!(lean_str, text);
    assert!(lean_str.is_heap_allocated());

    let lean_string = lean_str.into_lean_string();
    assert_eq!(lean_string, text);
    assert_eq!(lean_string.capacity(), text.len());
}

#[test]
fn unique_small_heap_into_inline() {
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
fn shared_heap_string_to_str() {
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
fn shared_heap_str_to_string() {
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
fn length_stored_heap_string_to_str() {
    let len = MIN_CAPACITY_FOR_HEAP_LENGTH_LAYOUT_ON_32BIT;
    let text = "a".repeat(len);

    let lean_string = LeanString::from(text.as_str());
    assert_eq!(lean_string.len(), len);
    assert!(lean_string.is_heap_allocated());
    assert!(lean_string.ends_with('a'));

    let lean_str = lean_string.into_lean_str();
    assert_eq!(lean_str.len(), len);
    assert_eq!(lean_str, text);
}

#[cfg(target_pointer_width = "32")]
#[test]
fn length_stored_heap_str_to_string() {
    let len = MIN_CAPACITY_FOR_HEAP_LENGTH_LAYOUT_ON_32BIT;
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
