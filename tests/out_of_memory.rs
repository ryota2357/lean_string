use lean_string::{LeanString, ReserveError};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    ptr,
};

// When set, the next allocation (reallocation) attempt made by the current thread fails.
//
// The flags are thread local so that arming them only affects the test that armed them, and
// neither the test harness threads nor other tests running in parallel can consume them.
// `const` initialization keeps the access itself allocation free.
thread_local! {
    static FAIL_NEXT_ALLOCATION: Cell<bool> = const { Cell::new(false) };
    static FAIL_NEXT_REALLOCATION: Cell<bool> = const { Cell::new(false) };
}

struct FailNextAllocation;

// SAFETY: Every request is delegated to `System`, except the single allocation a test explicitly
// rejects by arming the flags above.
unsafe impl GlobalAlloc for FailNextAllocation {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if FAIL_NEXT_ALLOCATION.replace(false) {
            ptr::null_mut()
        } else {
            // SAFETY: The caller provides a valid layout.
            unsafe { System.alloc(layout) }
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` was allocated by `System` with this layout.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if FAIL_NEXT_REALLOCATION.replace(false) {
            ptr::null_mut()
        } else {
            // SAFETY: The caller provides a pointer allocated with `layout`, and `new_size` is the
            // requested replacement allocation size.
            unsafe { System.realloc(ptr, layout, new_size) }
        }
    }
}

#[global_allocator]
static ALLOCATOR: FailNextAllocation = FailNextAllocation;

const TEXT: &str = "a string longer than the inline limit";

#[track_caller]
fn without_allocating<T>(f: impl FnOnce() -> T) -> T {
    FAIL_NEXT_ALLOCATION.set(true);
    FAIL_NEXT_REALLOCATION.set(true);
    let value = f();
    let allocated = !FAIL_NEXT_ALLOCATION.replace(false);
    let reallocated = !FAIL_NEXT_REALLOCATION.replace(false);

    assert!(!allocated, "an allocation was attempted");
    assert!(!reallocated, "a reallocation was attempted");
    value
}

#[test]
fn try_reserve_reports_allocation_failure() {
    let mut string = LeanString::from("inline");

    FAIL_NEXT_ALLOCATION.set(true);
    let result = string.try_reserve(64);
    FAIL_NEXT_ALLOCATION.set(false);

    assert_eq!(result, Err(ReserveError));
    assert_eq!(string, "inline");
    assert!(!string.is_heap_allocated());
}

#[test]
fn try_reserve_zero_keeps_static_buffer() {
    let mut string = LeanString::from_static_str(TEXT);
    let static_ptr = string.as_ptr();

    let result = without_allocating(|| string.try_reserve(0));

    assert_eq!(result, Ok(()));
    assert_eq!(string, TEXT);
    assert_eq!(string.as_ptr(), static_ptr);
    assert!(!string.is_heap_allocated());

    // Reserving a non-zero amount still converts the static buffer into a writable one.
    string.try_reserve(1).unwrap();
    assert!(string.is_heap_allocated());
    string.push('!');
    assert_eq!(string, "a string longer than the inline limit!");
}

#[test]
fn try_reserve_zero_keeps_heap_buffer_shared() {
    let mut string = LeanString::from(TEXT);
    let shared = string.clone();
    let shared_ptr = string.as_ptr();

    let result = without_allocating(|| string.try_reserve(0));

    assert_eq!(result, Ok(()));
    assert_eq!(string, TEXT);
    assert_eq!(shared, TEXT);
    assert_eq!(string.as_ptr(), shared_ptr);
    assert_eq!(shared.as_ptr(), shared_ptr);

    // Reserving a non-zero amount still detaches from the shared buffer.
    string.try_reserve(1).unwrap();
    assert_ne!(string.as_ptr(), shared_ptr);
    assert_eq!(shared.as_ptr(), shared_ptr);
    string.push('!');
    assert_eq!(string, "a string longer than the inline limit!");
    assert_eq!(shared, TEXT);
}

#[test]
fn insert_empty_str_keeps_static_buffer() {
    let mut string = LeanString::from_static_str(TEXT);
    let static_ptr = string.as_ptr();

    let result = without_allocating(|| string.try_insert_str(2, ""));

    assert_eq!(result, Ok(()));
    assert_eq!(string, TEXT);
    assert_eq!(string.as_ptr(), static_ptr);
    assert!(!string.is_heap_allocated());
}

#[test]
fn insert_empty_str_keeps_heap_buffer_shared() {
    let mut string = LeanString::from(TEXT);
    let shared = string.clone();
    let shared_ptr = string.as_ptr();

    let result = without_allocating(|| string.try_insert_str(2, ""));

    assert_eq!(result, Ok(()));
    assert_eq!(string, TEXT);
    assert_eq!(shared, TEXT);
    assert_eq!(string.as_ptr(), shared_ptr);
    assert_eq!(shared.as_ptr(), shared_ptr);
}

#[test]
fn extend_with_empty_iterator_keeps_heap_buffer_shared() {
    let mut string = LeanString::from(TEXT);
    let shared = string.clone();
    let shared_ptr = string.as_ptr();

    // `Extend` reserves `size_hint().0` upfront, which is zero here.
    without_allocating(|| string.extend(core::iter::empty::<char>()));

    assert_eq!(string, TEXT);
    assert_eq!(string.as_ptr(), shared_ptr);
    assert_eq!(shared.as_ptr(), shared_ptr);
}
