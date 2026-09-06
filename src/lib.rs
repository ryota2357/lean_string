#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![no_std]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

use core::{
    borrow::Borrow,
    cmp, fmt,
    hash::{Hash, Hasher},
    mem,
    ops::{Add, AddAssign, Deref},
    str,
    str::FromStr,
};

use alloc::{borrow::Cow, boxed::Box, string::String};

#[cfg(feature = "std")]
use std::{ffi::OsStr, path::Path};

mod repr;
use repr::{Immutable, Mutable, Repr};

mod errors;
pub use errors::*;

mod traits;
pub use traits::{ToLeanStr, ToLeanString};

mod features;

/// Compact, clone-on-write, UTF-8 encoded, growable string type.
#[repr(transparent)]
pub struct LeanString(Repr<Mutable>);

const _: () = {
    assert!(size_of::<LeanString>() == size_of::<[usize; 2]>());
    assert!(size_of::<Option<LeanString>>() == size_of::<[usize; 2]>());
    assert!(align_of::<LeanString>() == align_of::<usize>());
    assert!(align_of::<Option<LeanString>>() == align_of::<usize>());
};

impl LeanString {
    /// Creates a new empty [`LeanString`].
    ///
    /// Same as [`String::new()`], this will not allocate on the heap.
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let s = LeanString::new();
    /// assert!(s.is_empty());
    /// assert!(!s.is_heap_allocated());
    /// ```
    #[inline]
    #[must_use]
    pub const fn new() -> Self {
        LeanString(Repr::new())
    }

    /// Creates a new [`LeanString`] from a `&'static str`.
    ///
    /// # Panics
    ///
    /// Panics if the length of `text` is greater than `2^56 - 1` on 64-bit architecture, or
    /// `2^24 - 1` on 32-bit architecture.
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let s = LeanString::from_static_str("Long text but static lifetime");
    /// assert_eq!(s.as_str(), "Long text but static lifetime");
    /// assert_eq!(s.len(), 29);
    /// assert!(!s.is_heap_allocated());
    /// ```
    #[inline]
    #[must_use]
    pub const fn from_static_str(text: &'static str) -> Self {
        match Repr::from_static_str(text) {
            Ok(repr) => LeanString(repr),
            Err(_) => panic!("text is too long"),
        }
    }

    /// Creates a new empty [`LeanString`] with at least `capacity` bytes.
    ///
    /// The returned [`LeanString`] has a capacity of at least `2 * size_of::<usize>()` bytes,
    /// the size of the inline (on the stack) storage.
    ///
    /// # Panics
    ///
    /// Panics if **any** of the following conditions is met:
    ///
    /// - The system is out-of-memory.
    /// - On 64-bit architecture, the `capacity` is greater than `2^56 - 1`.
    /// - On 32-bit architecture, the `capacity` is greater than `2^31 - 16`.
    ///
    /// If you want to handle such a problem manually, use [`LeanString::try_with_capacity()`].
    ///
    /// # Examples
    ///
    /// ## inline capacity
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let s = LeanString::with_capacity(4);
    /// assert_eq!(s.capacity(), 2 * size_of::<usize>());
    /// assert!(!s.is_heap_allocated());
    /// ```
    ///
    /// ## heap capacity
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let s = LeanString::with_capacity(100);
    /// assert_eq!(s.capacity(), 100);
    /// assert!(s.is_heap_allocated());
    /// ```
    #[inline]
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        LeanString::try_with_capacity(capacity).unwrap_with_msg()
    }

    /// Fallible version of [`LeanString::with_capacity()`].
    ///
    /// This method won't panic if the system is out of memory, or if the `capacity` is too large, but
    /// returns a [`ReserveError`]. Otherwise it behaves the same as [`LeanString::with_capacity()`].
    #[inline]
    pub fn try_with_capacity(capacity: usize) -> Result<Self, ReserveError> {
        Repr::with_capacity(capacity).map(LeanString)
    }

    /// Converts a slice of bytes to a [`LeanString`].
    ///
    /// If the slice is not valid UTF-8, an error is returned.
    ///
    /// # Examples
    ///
    /// ## valid UTF-8
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let bytes = vec![240, 159, 166, 128];
    /// let string = LeanString::from_utf8(&bytes).expect("valid UTF-8");
    ///
    /// assert_eq!(string, "🦀");
    /// ```
    ///
    /// ## invalid UTF-8
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let bytes = &[255, 255, 255];
    /// let result = LeanString::from_utf8(bytes);
    ///
    /// assert!(result.is_err());
    /// ```
    #[inline]
    pub fn from_utf8(buf: &[u8]) -> Result<Self, str::Utf8Error> {
        let str = str::from_utf8(buf)?;
        Ok(LeanString::from(str))
    }

    /// Converts a slice of bytes to a [`LeanString`], including invalid characters.
    ///
    /// During this conversion, all invalid characters are replaced with the
    /// [`char::REPLACEMENT_CHARACTER`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let invalid_bytes = b"Hello \xF0\x90\x80World";
    /// let string = LeanString::from_utf8_lossy(invalid_bytes);
    ///
    /// assert_eq!(string, "Hello �World");
    /// ```
    #[inline]
    #[must_use]
    pub fn from_utf8_lossy(buf: &[u8]) -> Self {
        let mut ret = LeanString::with_capacity(buf.len());
        for chunk in buf.utf8_chunks() {
            ret.push_str(chunk.valid());
            if !chunk.invalid().is_empty() {
                ret.push(char::REPLACEMENT_CHARACTER);
            }
        }
        ret
    }

    /// Converts a slice of bytes to a [`LeanString`] without checking if the bytes are valid
    /// UTF-8.
    ///
    /// # Safety
    ///
    /// This function is unsafe because it does not check that the bytes passed to it are valid
    /// UTF-8. If this constraint is violated, it may cause memory unsafety issues.
    #[inline]
    #[must_use]
    pub unsafe fn from_utf8_unchecked(buf: &[u8]) -> Self {
        let str = unsafe { str::from_utf8_unchecked(buf) };
        LeanString::from(str)
    }

    /// Decodes a slice of UTF-16 encoded bytes to a [`LeanString`], returning an error if `buf`
    /// contains any invalid code points.
    ///
    /// # Examples
    ///
    /// ## valid UTF-16
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let v = &[0xD834, 0xDD1E, 0x006d, 0x0075, 0x0073, 0x0069, 0x0063];
    /// assert_eq!(LeanString::from_utf16(v).unwrap(), "𝄞music");
    /// ```
    ///
    /// ## invalid UTF-16
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// // 𝄞mu<invalid>ic
    /// let v = &[0xD834, 0xDD1E, 0x006d, 0x0075, 0xD800, 0x0069, 0x0063];
    /// assert!(LeanString::from_utf16(v).is_err());
    /// ```
    #[inline]
    pub fn from_utf16(buf: &[u16]) -> Result<Self, FromUtf16Error> {
        let mut ret = LeanString::with_capacity(buf.len());
        for c in char::decode_utf16(buf.iter().copied()) {
            match c {
                Ok(c) => ret.push(c),
                Err(_) => return Err(FromUtf16Error),
            }
        }
        Ok(ret)
    }

    /// Decodes a slice of UTF-16 encoded bytes to a [`LeanString`], replacing invalid code points
    /// with the [`char::REPLACEMENT_CHARACTER`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// // 𝄞mus<invalid>ic<invalid>
    /// let v = &[0xD834, 0xDD1E, 0x006d, 0x0075, 0x0073, 0xDD1E, 0x0069, 0x0063, 0xD834];
    /// assert_eq!(LeanString::from_utf16_lossy(v), "𝄞mus\u{FFFD}ic\u{FFFD}");
    /// ```
    #[inline]
    #[must_use]
    pub fn from_utf16_lossy(buf: &[u16]) -> Self {
        char::decode_utf16(buf.iter().copied())
            .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect()
    }

    /// Returns the length of the string in bytes, not [`char`] or graphemes.
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let a = LeanString::from("foo");
    /// assert_eq!(a.len(), 3);
    ///
    /// let fancy_f = LeanString::from("ƒoo");
    /// assert_eq!(fancy_f.len(), 4);
    /// assert_eq!(fancy_f.chars().count(), 3);
    /// ```
    #[inline]
    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns `true` if the [`LeanString`] has a length of 0, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let mut s = LeanString::new();
    /// assert!(s.is_empty());
    ///
    /// s.push('a');
    /// assert!(!s.is_empty());
    /// ```
    #[inline]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the capacity of the [`LeanString`], in bytes.
    ///
    /// # Note
    ///
    /// - [`LeanString`] backed by a `&'static str` does not write the underlying byte data, so its
    ///   capacity matches its current length.
    /// - Otherwise, it at least `2 * size_of::<usize>()` bytes, which is the size of the inline (on
    ///   the stack) storage.
    ///
    /// # Examples
    ///
    /// ## inline capacity
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let s = LeanString::new();
    /// assert_eq!(s.capacity(), 2 * size_of::<usize>());
    /// ```
    ///
    /// ## heap capacity
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let s = LeanString::with_capacity(100);
    /// assert_eq!(s.capacity(), 100);
    /// ```
    ///
    /// ## `&'static str` capacity
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let mut s = LeanString::from_static_str("Long text but static lifetime");
    /// assert_eq!(s.capacity(), s.len());
    ///
    /// s.truncate(4);
    /// assert_eq!(s.capacity(), 4);
    /// ```
    #[inline]
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.0.capacity()
    }

    /// Returns a string slice containing the entire [`LeanString`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let s = LeanString::from("foo");
    /// assert_eq!(s.as_str(), "foo");
    /// ```
    #[inline]
    #[must_use]
    pub const fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Returns a byte slice containing the entire [`LeanString`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let s = LeanString::from("hello");
    /// assert_eq!(&[104, 101, 108, 108, 111], s.as_bytes());
    /// ```
    #[inline]
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// Reserves capacity for at least `additional` bytes more than the current length.
    ///
    /// If this [`LeanString`] is not unique, it is cloned first, even when its capacity is already
    /// sufficient, because the reserved capacity must not be shared with others.
    ///
    /// Does nothing if `additional` is zero.
    ///
    /// # Panics
    ///
    /// Panics if **any** of the following conditions is met:
    ///
    /// - The system is out-of-memory.
    /// - On 64-bit architecture, the `capacity` is greater than `2^56 - 1`.
    /// - On 32-bit architecture, the `capacity` is greater than `2^31 - 16`.
    ///
    /// If you want to handle such a problem manually, use [`LeanString::try_reserve()`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let mut s = LeanString::new();
    ///
    /// // We have an inline storage on the stack.
    /// assert_eq!(s.capacity(), 2 * size_of::<usize>());
    /// assert!(!s.is_heap_allocated());
    ///
    /// s.reserve(100);
    ///
    /// // Now we have a heap storage.
    /// assert!(s.capacity() >= s.len() + 100);
    /// assert!(s.is_heap_allocated());
    /// ```
    ///
    /// Reserving zero bytes does not clone.
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let mut s = LeanString::from("This is a text the length is more than 16 bytes");
    /// let shared = s.clone();
    ///
    /// s.reserve(0);
    ///
    /// // Not cloned: `s` still shares its contents with `shared`.
    /// assert_eq!(s.as_ptr(), shared.as_ptr());
    /// ```
    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.try_reserve(additional).unwrap_with_msg()
    }

    /// Fallible version of [`LeanString::reserve()`].
    ///
    /// This method won't panic if the system is out-of-memory, or the `capacity` is too large, but
    /// return an [`ReserveError`]. Otherwise it behaves the same as [`LeanString::reserve()`].
    #[inline]
    pub fn try_reserve(&mut self, additional: usize) -> Result<(), ReserveError> {
        self.0.reserve(additional)
    }

    /// Shrinks the capacity of the [`LeanString`] to match its length.
    ///
    /// Only a heap buffer can shrink. This is a no-op for a [`LeanString`] stored inline or
    /// backed by a `&'static str`.
    ///
    /// If this [`LeanString`] is not unique and its capacity is greater than its length, it is
    /// cloned first, because the capacity it shares with others must be left as it is.
    ///
    /// # Panics
    ///
    /// Panics if cloning the [`LeanString`] fails due to the system being out-of-memory. If you
    /// want to handle such a problem manually, use [`LeanString::try_shrink_to_fit()`].
    ///
    /// # Examples
    ///
    /// ## short string
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let mut s = LeanString::from("foo");
    ///
    /// s.reserve(100);
    /// assert_eq!(s.capacity(), 3 + 100);
    ///
    /// s.shrink_to_fit();
    /// assert_eq!(s.capacity(), 2 * size_of::<usize>());
    /// ```
    ///
    /// ## long string
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let mut s = LeanString::from("This is a text the length is more than 16 bytes");
    ///
    /// s.reserve(100);
    /// assert!(s.capacity() > 16 + 100);
    ///
    /// s.shrink_to_fit();
    /// assert_eq!(s.capacity(), s.len());
    /// ```
    #[inline]
    pub fn shrink_to_fit(&mut self) {
        self.try_shrink_to_fit().unwrap_with_msg()
    }

    /// Fallible version of [`LeanString::shrink_to_fit()`].
    ///
    /// This method won't panic if the system is out-of-memory, or the `capacity` is too large, but
    /// return an [`ReserveError`]. Otherwise it behaves the same as [`LeanString::shrink_to_fit()`].
    #[inline]
    pub fn try_shrink_to_fit(&mut self) -> Result<(), ReserveError> {
        self.0.shrink_to(0)
    }

    /// Shrinks the capacity of the [`LeanString`] with a lower bound.
    ///
    /// Only a heap buffer can shrink. This is a no-op for a [`LeanString`] stored inline or
    /// backed by a `&'static str`.
    ///
    /// If this [`LeanString`] is not unique and its capacity will be changed, it is cloned first,
    /// because the capacity it shares with others must be left as it is.
    ///
    /// # Panics
    ///
    /// Panics if cloning the [`LeanString`] fails due to the system being out-of-memory. If you
    /// want to handle such a problem manually, use [`LeanString::try_shrink_to()`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let mut s = LeanString::with_capacity(100);
    /// assert_eq!(s.capacity(), 100);
    ///
    /// // if the capacity was already bigger than the argument and unique, the call is no-op.
    /// s.shrink_to(100);
    /// assert_eq!(s.capacity(), 100);
    ///
    /// s.shrink_to(50);
    /// assert_eq!(s.capacity(), 50);
    ///
    /// // if the string can be inlined, it is
    /// s.shrink_to(5);
    /// assert_eq!(s.capacity(), 2 * size_of::<usize>());
    /// ```
    #[inline]
    pub fn shrink_to(&mut self, min_capacity: usize) {
        self.try_shrink_to(min_capacity).unwrap_with_msg()
    }

    /// Fallible version of [`LeanString::shrink_to()`].
    ///
    /// This method won't panic if the system is out-of-memory, or the `capacity` is too large, but
    /// return an [`ReserveError`]. Otherwise it behaves the same as [`LeanString::shrink_to()`].
    #[inline]
    pub fn try_shrink_to(&mut self, min_capacity: usize) -> Result<(), ReserveError> {
        self.0.shrink_to(min_capacity)
    }

    /// Appends the given [`char`] to the end of the [`LeanString`].
    ///
    /// # Panics
    ///
    /// Panics if the system is out-of-memory. If you want to handle such a problem manually, use
    /// [`LeanString::try_push()`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let mut s = LeanString::new();
    /// s.push('f');
    /// s.push('o');
    /// s.push('o');
    /// assert_eq!("foo", s);
    /// ```
    #[inline]
    pub fn push(&mut self, ch: char) {
        self.try_push(ch).unwrap_with_msg()
    }

    /// Fallible version of [`LeanString::push()`].
    ///
    /// This method won't panic if the system is out-of-memory, or the `capacity` is too large, but
    /// return an [`ReserveError`]. Otherwise it behaves the same as [`LeanString::push()`].
    #[inline]
    pub fn try_push(&mut self, ch: char) -> Result<(), ReserveError> {
        self.0.push_str(ch.encode_utf8(&mut [0; 4]))
    }

    /// Removes the last character from the [`LeanString`] and returns it.
    /// If the [`LeanString`] is empty, `None` is returned.
    ///
    /// # Panics
    ///
    /// On 32-bit architectures, this method needs to clone the [`LeanString`] under both of the
    /// following conditions, and may panic if that allocation fails:
    ///
    /// - The [`LeanString`] is not unique.
    /// - Its length is greater than `2^24 - 2`.
    ///
    /// If you want to handle such a problem manually, use [`LeanString::try_pop()`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let mut s = LeanString::from("abč");
    ///
    /// assert_eq!(s.pop(), Some('č'));
    /// assert_eq!(s.pop(), Some('b'));
    /// assert_eq!(s.pop(), Some('a'));
    ///
    /// assert_eq!(s.pop(), None);
    /// ```
    #[inline]
    pub fn pop(&mut self) -> Option<char> {
        self.try_pop().unwrap_with_msg()
    }

    /// Fallible version of [`LeanString::pop()`].
    ///
    /// This method won't panic if the system is out-of-memory, or the `capacity` is too large, but
    /// return an [`ReserveError`]. Otherwise it behaves the same as [`LeanString::pop()`].
    #[inline]
    pub fn try_pop(&mut self) -> Result<Option<char>, ReserveError> {
        self.0.pop()
    }

    /// Appends a given string slice onto the end of this [`LeanString`].
    ///
    /// # Panics
    ///
    /// Panics if cloning the [`LeanString`] fails due to the system being out-of-memory. If you
    /// want to handle such a problem manually, use [`LeanString::try_push_str()`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let mut s = LeanString::from("foo");
    ///
    /// s.push_str("bar");
    ///
    /// assert_eq!("foobar", s);
    /// ```
    #[inline]
    pub fn push_str(&mut self, string: &str) {
        self.try_push_str(string).unwrap_with_msg()
    }

    /// Fallible version of [`LeanString::push_str()`].
    ///
    /// This method won't panic if the system is out-of-memory, or the `capacity` is too large, but
    /// return an [`ReserveError`]. Otherwise it behaves the same as [`LeanString::push_str()`].
    ///
    /// On failure, `self` is left unchanged because capacity is reserved before `string` is
    /// written.
    #[inline]
    pub fn try_push_str(&mut self, string: &str) -> Result<(), ReserveError> {
        self.0.push_str(string)
    }

    /// Removes a [`char`] from the [`LeanString`] at a byte position and returns it.
    ///
    /// # Panics
    ///
    /// Panics if **any** of the following conditions is met:
    ///
    /// 1. `idx` is larger than or equal to the [`LeanString`]'s length, or it does not lie on a
    ///    [`char`] boundary.
    /// 2. The system is out-of-memory when cloning the [`LeanString`].
    ///
    /// For 2, if you want to handle such a problem manually, use [`LeanString::try_remove()`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let mut s = LeanString::from("Hello 世界");
    ///
    /// assert_eq!(s.remove(6), '世');
    /// assert_eq!(s.remove(1), 'e');
    ///
    /// assert_eq!(s, "Hllo 界");
    /// ```
    /// ## Past total length:
    ///
    /// ```should_panic
    /// # use lean_string::LeanString;
    /// let mut c = LeanString::from("hello there!");
    /// c.remove(12);
    /// ```
    ///
    /// ## Not on char boundary:
    ///
    /// ```should_panic
    /// # use lean_string::LeanString;
    /// let mut c = LeanString::from("🦄");
    /// c.remove(1);
    /// ```
    #[inline]
    pub fn remove(&mut self, idx: usize) -> char {
        self.try_remove(idx).unwrap_with_msg()
    }

    /// Fallible version of [`LeanString::remove()`].
    ///
    /// This method won't panic if the system is out-of-memory, but return an [`ReserveError`].
    /// Otherwise it behaves the same as [`LeanString::remove()`].
    ///
    /// # Panics
    ///
    /// This method still panics if the `idx` is larger than or equal to the [`LeanString`]'s
    /// length, or if it does not lie on a [`char`] boundary.
    #[inline]
    pub fn try_remove(&mut self, idx: usize) -> Result<char, ReserveError> {
        self.0.remove(idx)
    }

    /// Retains only the characters specified by the `predicate`.
    ///
    /// If the `predicate` returns `true`, the character is kept, otherwise it is removed.
    ///
    /// # Panics
    ///
    /// Panics if the system is out-of-memory when cloning the [`LeanString`]. If you want to
    /// handle such a problem manually, use [`LeanString::try_retain()`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let mut s = LeanString::from("äb𝄞d€");
    ///
    /// let keep = [false, true, true, false, true];
    /// let mut iter = keep.iter();
    /// s.retain(|_| *iter.next().unwrap());
    ///
    /// assert_eq!(s, "b𝄞€");
    /// ```
    #[inline]
    pub fn retain(&mut self, predicate: impl FnMut(char) -> bool) {
        self.try_retain(predicate).unwrap_with_msg()
    }

    /// Fallible version of [`LeanString::retain()`].
    ///
    /// This method won't panic if the system is out-of-memory, but return an [`ReserveError`].
    #[inline]
    pub fn try_retain(&mut self, predicate: impl FnMut(char) -> bool) -> Result<(), ReserveError> {
        self.0.retain(predicate)
    }

    /// Inserts a character into the [`LeanString`] at a byte position.
    ///
    /// # Panics
    ///
    /// Panics if **any** of the following conditions is met:
    ///
    /// 1. `idx` is larger than the [`LeanString`]'s length, or if it does not lie on a [`char`]
    ///    boundary.
    /// 2. The system is out-of-memory when cloning the [`LeanString`].
    /// 3. The length of after inserting is greater than `2^56 - 1` on 64-bit architecture, or
    ///    `2^31 - 16` on 32-bit architecture.
    ///
    /// For 2 and 3, if you want to handle such a problem manually, use [`LeanString::try_insert()`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let mut s = LeanString::from("Hello world");
    ///
    /// s.insert(11, '!');
    /// assert_eq!("Hello world!", s);
    ///
    /// s.insert(5, ',');
    /// assert_eq!("Hello, world!", s);
    /// ```
    #[inline]
    pub fn insert(&mut self, idx: usize, ch: char) {
        self.try_insert(idx, ch).unwrap_with_msg()
    }

    /// Fallible version of [`LeanString::insert()`].
    ///
    /// This method won't panic if the system is out-of-memory, or the `capacity` becomes too large
    /// by inserting a character, but return an [`ReserveError`]. Otherwise it behaves the same as
    /// [`LeanString::insert()`].
    ///
    /// # Panics
    ///
    /// This method still panics if the `idx` is larger than the [`LeanString`]'s length, or if it
    /// does not lie on a [`char`] boundary.
    #[inline]
    pub fn try_insert(&mut self, idx: usize, ch: char) -> Result<(), ReserveError> {
        self.0.insert_str(idx, ch.encode_utf8(&mut [0; 4]))
    }

    /// Inserts a string slice into the [`LeanString`] at a byte position.
    ///
    /// # Panics
    ///
    /// Panics if **any** of the following conditions is met:
    ///
    /// 1. `idx` is larger than the [`LeanString`]'s length, or if it does not lie on a [`char`] boundary.
    /// 2. The system is out-of-memory when cloning the [`LeanString`].
    /// 3. The length of after inserting is greater than `2^56 - 1` on 64-bit architecture, or
    ///    `2^31 - 16` on 32-bit architecture.
    ///
    /// For 2 and 3, if you want to handle such a problem manually, use [`LeanString::try_insert_str()`].
    ///
    /// # Examples
    /// ```
    /// # use lean_string::LeanString;
    /// let mut s = LeanString::from("bar");
    /// s.insert_str(0, "foo");
    /// assert_eq!("foobar", s);
    /// ```
    #[inline]
    pub fn insert_str(&mut self, idx: usize, string: &str) {
        self.try_insert_str(idx, string).unwrap_with_msg()
    }

    /// Fallible version of [`LeanString::insert_str()`].
    ///
    /// This method won't panic if the system is out-of-memory, or the `capacity` becomes too large
    /// by inserting a string slice, but return an [`ReserveError`]. Otherwise it behaves the same
    /// as [`LeanString::insert_str()`].
    ///
    /// # Panics
    ///
    /// This method still panics if the `idx` is larger than the [`LeanString`]'s length, or if it
    /// does not lie on a [`char`] boundary.
    #[inline]
    pub fn try_insert_str(&mut self, idx: usize, string: &str) -> Result<(), ReserveError> {
        self.0.insert_str(idx, string)
    }

    /// Creates a new [`LeanString`] by repeating `self` `n` times.
    ///
    /// # Panics
    ///
    /// Panics if **any** of the following conditions is met:
    ///
    /// 1. The resulting capacity would overflow (`self.len() * n` exceeds `usize::MAX`).
    /// 2. The system is out-of-memory.
    /// 3. On 64-bit architecture, the resulting length is greater than `2^56 - 1`.
    ///    On 32-bit architecture, it is `2^31 - 16`.
    ///
    /// If you want to handle such a problem manually, use [`LeanString::try_repeat()`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let s = LeanString::from("abc");
    /// assert_eq!(s.repeat(4), "abcabcabcabc");
    /// assert_eq!(s.repeat(0), "");
    /// ```
    #[inline]
    #[must_use]
    pub fn repeat(&self, n: usize) -> Self {
        self.try_repeat(n).unwrap_with_msg()
    }

    /// Fallible version of [`LeanString::repeat()`].
    ///
    /// This method won't panic, but returns a [`ReserveError`] if the capacity would overflow,
    /// the system is out-of-memory, or the resulting length exceeds the maximum. Otherwise it
    /// behaves the same as [`LeanString::repeat()`].
    #[inline]
    pub fn try_repeat(&self, n: usize) -> Result<Self, ReserveError> {
        if n == 0 || self.is_empty() {
            Ok(LeanString::new())
        } else if n == 1 {
            Ok(self.clone())
        } else {
            let capacity = self.len().checked_mul(n).ok_or(ReserveError)?;
            let mut res = LeanString::try_with_capacity(capacity)?;
            for _ in 0..n {
                res.try_push_str(self)?;
            }
            Ok(res)
        }
    }

    /// Shortens a [`LeanString`] to the specified length.
    ///
    /// If `new_len` is greater than or equal to the string's current length, this has no effect.
    ///
    /// # Panics
    ///
    /// Panics if **any** of the following conditions is met:
    ///
    /// 1. `new_len` does not lie on a [`char`] boundary.
    /// 2. The system is out-of-memory when cloning the [`LeanString`].
    ///
    /// For 2, If you want to handle such a problem manually, use [`LeanString::try_truncate()`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let mut s = LeanString::from("hello");
    /// s.truncate(2);
    /// assert_eq!(s, "he");
    ///
    /// // Truncating to a larger length does nothing:
    /// s.truncate(10);
    /// assert_eq!(s, "he");
    /// ```
    #[inline]
    pub fn truncate(&mut self, new_len: usize) {
        self.try_truncate(new_len).unwrap_with_msg()
    }

    /// Fallible version of [`LeanString::truncate()`].
    ///
    /// This method won't panic if the system is out-of-memory, but return an [`ReserveError`].
    /// Otherwise it behaves the same as [`LeanString::truncate()`].
    ///
    /// # Panics
    ///
    /// This method still panics if `new_len` does not lie on a [`char`] boundary.
    #[inline]
    pub fn try_truncate(&mut self, new_len: usize) -> Result<(), ReserveError> {
        self.0.truncate(new_len)
    }

    /// Splits the string into two at the given byte index.
    ///
    /// Returns a newly allocated [`LeanString`]. `self` contains bytes `[0, at)`, and
    /// the returned [`LeanString`] contains bytes `[at, len)`. `at` must be on the
    /// boundary of a UTF-8 code point.
    ///
    /// # Panics
    ///
    /// Panics if **any** of the following conditions is met:
    ///
    /// 1. `at` does not lie on a [`char`] boundary, or is beyond the end of the string.
    /// 2. The system is out-of-memory when creating the new [`LeanString`].
    ///
    /// For 2, if you want to handle such a problem manually, use [`LeanString::try_split_off()`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let mut hello = LeanString::from("Hello, World!");
    /// let world = hello.split_off(7);
    /// assert_eq!(hello, "Hello, ");
    /// assert_eq!(world, "World!");
    /// ```
    #[inline]
    #[must_use = "use `.truncate()` if you don't need the other half"]
    pub fn split_off(&mut self, at: usize) -> Self {
        self.try_split_off(at).unwrap_with_msg()
    }

    /// Fallible version of [`LeanString::split_off()`].
    ///
    /// This method won't panic if the system is out-of-memory, but returns a [`ReserveError`].
    /// Otherwise it behaves the same as [`LeanString::split_off()`].
    ///
    /// # Panics
    ///
    /// This method still panics if `at` does not lie on a [`char`] boundary, or is beyond the
    /// end of the string.
    #[inline]
    #[must_use = "use `.try_truncate()` if you don't need the other half"]
    pub fn try_split_off(&mut self, at: usize) -> Result<Self, ReserveError> {
        let other = LeanString(Repr::from_str(&self.as_str()[at..])?);
        self.try_truncate(at)?;
        Ok(other)
    }

    /// Reduces the length of the [`LeanString`] to zero.
    ///
    /// If the [`LeanString`] is unique, this method will not change the capacity.
    /// Otherwise, creates a new unique [`LeanString`] without heap allocation.
    ///
    /// # Examples
    ///
    /// ## unique
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let mut s = LeanString::from("This is a example of unique LeanString");
    /// assert_eq!(s.capacity(), 38);
    ///
    /// s.clear();
    ///
    /// assert_eq!(s, "");
    /// assert_eq!(s.capacity(), 38);
    /// ```
    ///
    /// ## not unique
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let mut s = LeanString::from("This is a example of not unique LeanString");
    /// assert_eq!(s.capacity(), 42);
    ///
    /// let s2 = s.clone();
    /// s.clear();
    ///
    /// assert_eq!(s, "");
    /// assert_eq!(s.capacity(), 2 * size_of::<usize>());
    /// ```
    #[inline]
    pub fn clear(&mut self) {
        if self.0.is_unique() {
            // SAFETY:
            // - `self` is unique.
            // - 0 bytes is always valid UTF-8, and initialized.
            unsafe { self.0.set_len(0) }
        } else {
            self.0.replace_inner(Repr::new());
        }
    }

    /// Returns whether the [`LeanString`] is heap-allocated.
    ///
    /// # Examples
    ///
    /// ## inline
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let s = LeanString::from("hello");
    /// assert!(!s.is_heap_allocated());
    /// ```
    ///
    /// ## heap
    ///
    /// ```
    /// # use lean_string::LeanString;
    /// let s = LeanString::from("More than 2 * size_of::<usize>() bytes is heap-allocated");
    /// assert!(s.is_heap_allocated());
    /// ```
    #[inline]
    #[must_use]
    pub fn is_heap_allocated(&self) -> bool {
        self.0.is_heap_buffer()
    }

    /// Converts the [`LeanString`] into a [`LeanStr`].
    ///
    /// Inline strings and strings created from a `&'static str` are converted at zero cost.
    /// A heap-allocated string is moved into [`LeanStr`]'s heap layout, dropping any spare
    /// capacity (much like [`String::into_boxed_str()`]): the buffer is reallocated in place
    /// if it is not shared, and copied otherwise. If the content is short enough to be stored
    /// inline, the result is inline instead.
    ///
    /// # Panics
    ///
    /// Panics if converting a heap-allocated string fails to allocate. To handle the failure,
    /// use [`LeanString::try_into_lean_str()`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::{LeanStr, LeanString};
    /// let s = LeanString::from("This is a heap-allocated string!!");
    /// let s: LeanStr = s.into_lean_str();
    /// assert_eq!(s, "This is a heap-allocated string!!");
    /// ```
    #[inline]
    #[must_use]
    pub fn into_lean_str(self) -> LeanStr {
        self.try_into_lean_str().unwrap_with_msg()
    }

    /// Fallible version of [`LeanString::into_lean_str()`].
    ///
    /// # Errors
    ///
    /// Returns a [`ReserveError`] if converting a heap-allocated string fails to allocate.
    /// Because this method consumes `self`, the original string is dropped on failure.
    #[inline]
    pub fn try_into_lean_str(mut self) -> Result<LeanStr, ReserveError> {
        let repr = mem::replace(&mut self.0, Repr::new());
        match repr.into_immutable() {
            Ok(repr) => Ok(LeanStr(repr)),
            Err((repr, err)) => {
                // Hand the original back to `self` so its `Drop` releases the buffer.
                self.0 = repr;
                Err(err)
            }
        }
    }
}

/// A [`Clone`] implementation for [`LeanString`].
///
/// The clone operation is performed using a reference counting mechanism, which means:
/// - The cloned string shares the same underlying data with the original string
/// - The cloning process is very efficient (O(1) time complexity)
/// - No memory allocation occurs during cloning
///
/// # Examples
///
/// ```
/// # use lean_string::LeanString;
/// let s1 = LeanString::from("Hello, World!");
/// let s2 = s1.clone();
///
/// assert_eq!(s1, s2);
/// ```
impl Clone for LeanString {
    #[inline]
    fn clone(&self) -> Self {
        LeanString(self.0.make_shallow_clone())
    }

    #[inline]
    fn clone_from(&mut self, source: &Self) {
        self.0.replace_inner(source.0.make_shallow_clone());
    }
}

/// A [`Drop`] implementation for [`LeanString`].
///
/// When the last reference to a [`LeanString`] is dropped, its heap allocation (if any) is freed.
impl Drop for LeanString {
    #[inline]
    fn drop(&mut self) {
        // SAFETY: This is the `Drop` implementation of a newtype wrapping `Repr`, and `self.0` is
        // never accessed again.
        unsafe { self.0.drop_in() };
    }
}

// SAFETY: `LeanString` is `repr(transparent)` over `Repr`, and `Repr` works like `Arc`.
unsafe impl Send for LeanString {}
unsafe impl Sync for LeanString {}

/// Compact, reference-counted, UTF-8 encoded, immutable string type.
///
/// [`LeanStr`] is the immutable counterpart of [`LeanString`]: the same 2-word size, the same
/// inline storage for short strings, the same O(1) construction from a `&'static str`, and O(1)
/// reference-counted clones. Since it never grows, a heap-allocated [`LeanStr`] occupies exactly
/// as much memory as its content needs, with no spare capacity.
///
/// ## `LeanStr` vs a newtype of `LeanString`
///
/// [`LeanStr`] is not a newtype wrapper around [`LeanString`]. The two types use incompatible
/// heap layouts ([`LeanStr`]'s carries no capacity), so converting a heap-allocated string in
/// either direction may reallocate or copy the buffer. See [`LeanString::into_lean_str()`] and
/// [`LeanStr::into_lean_string()`].
///
/// Because of this, [`LeanStr`] fits best when a string stays immutable for its whole life.
/// If you mainly want to forbid mutation at the type level and convert back and forth freely,
/// a newtype around [`LeanString`] serves better: both sides then share the same representation
/// and convert at zero cost.
#[repr(transparent)]
pub struct LeanStr(Repr<Immutable>);

const _: () = {
    assert!(size_of::<LeanStr>() == size_of::<[usize; 2]>());
    assert!(size_of::<Option<LeanStr>>() == size_of::<[usize; 2]>());
    assert!(align_of::<LeanStr>() == align_of::<usize>());
    assert!(align_of::<Option<LeanStr>>() == align_of::<usize>());
};

impl LeanStr {
    /// Creates a new empty [`LeanStr`].
    ///
    /// Same as [`LeanString::new()`], this will not allocate on the heap.
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::LeanStr;
    /// let s = LeanStr::new();
    /// assert!(s.is_empty());
    /// assert!(!s.is_heap_allocated());
    /// ```
    #[inline]
    #[must_use]
    pub const fn new() -> Self {
        LeanStr(Repr::new())
    }

    /// Creates a new [`LeanStr`] from a `&'static str`.
    ///
    /// # Panics
    ///
    /// Panics if the length of `text` is greater than `2^56 - 1` on 64-bit architecture, or
    /// `2^24 - 1` on 32-bit architecture.
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::LeanStr;
    /// let s = LeanStr::from_static_str("Long text but static lifetime");
    /// assert_eq!(s.as_str(), "Long text but static lifetime");
    /// assert!(!s.is_heap_allocated());
    /// ```
    #[inline]
    #[must_use]
    pub const fn from_static_str(text: &'static str) -> Self {
        match Repr::from_static_str(text) {
            Ok(repr) => LeanStr(repr),
            Err(_) => panic!("text is too long"),
        }
    }

    /// Converts a slice of bytes to a [`LeanStr`].
    ///
    /// If the slice is not valid UTF-8, an error is returned.
    ///
    /// # Examples
    ///
    /// ## valid UTF-8
    ///
    /// ```
    /// # use lean_string::LeanStr;
    /// let bytes = vec![240, 159, 166, 128];
    /// let string = LeanStr::from_utf8(&bytes).expect("valid UTF-8");
    ///
    /// assert_eq!(string, "🦀");
    /// ```
    ///
    /// ## invalid UTF-8
    ///
    /// ```
    /// # use lean_string::LeanStr;
    /// let bytes = &[255, 255, 255];
    /// let result = LeanStr::from_utf8(bytes);
    ///
    /// assert!(result.is_err());
    /// ```
    #[inline]
    pub fn from_utf8(buf: &[u8]) -> Result<Self, str::Utf8Error> {
        let str = str::from_utf8(buf)?;
        Ok(LeanStr::from(str))
    }

    /// Converts a slice of bytes to a [`LeanStr`], including invalid characters.
    ///
    /// During this conversion, all invalid characters are replaced with the
    /// [`char::REPLACEMENT_CHARACTER`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::LeanStr;
    /// let invalid_bytes = b"Hello \xF0\x90\x80World";
    /// let string = LeanStr::from_utf8_lossy(invalid_bytes);
    ///
    /// assert_eq!(string, "Hello �World");
    /// ```
    #[inline]
    #[must_use]
    pub fn from_utf8_lossy(buf: &[u8]) -> Self {
        LeanString::from_utf8_lossy(buf).into_lean_str()
    }

    /// Converts a slice of bytes to a [`LeanStr`] without checking if the bytes are valid UTF-8.
    ///
    /// # Safety
    ///
    /// This function is unsafe because it does not check that the bytes passed to it are valid
    /// UTF-8. If this constraint is violated, it may cause memory unsafety issues.
    #[inline]
    #[must_use]
    pub unsafe fn from_utf8_unchecked(buf: &[u8]) -> Self {
        let str = unsafe { str::from_utf8_unchecked(buf) };
        LeanStr::from(str)
    }

    /// Decodes a slice of UTF-16 encoded bytes to a [`LeanStr`], returning an error if `buf`
    /// contains any invalid code points.
    ///
    /// # Examples
    ///
    /// ## valid UTF-16
    ///
    /// ```
    /// # use lean_string::LeanStr;
    /// let v = &[0xD834, 0xDD1E, 0x006d, 0x0075, 0x0073, 0x0069, 0x0063];
    /// assert_eq!(LeanStr::from_utf16(v).unwrap(), "𝄞music");
    /// ```
    ///
    /// ## invalid UTF-16
    ///
    /// ```
    /// # use lean_string::LeanStr;
    /// // 𝄞mu<invalid>ic
    /// let v = &[0xD834, 0xDD1E, 0x006d, 0x0075, 0xD800, 0x0069, 0x0063];
    /// assert!(LeanStr::from_utf16(v).is_err());
    /// ```
    #[inline]
    pub fn from_utf16(buf: &[u16]) -> Result<Self, FromUtf16Error> {
        LeanString::from_utf16(buf).map(|x| x.into_lean_str())
    }

    /// Decodes a slice of UTF-16 encoded bytes to a [`LeanStr`], replacing invalid code points
    /// with the [`char::REPLACEMENT_CHARACTER`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::LeanStr;
    /// // 𝄞mus<invalid>ic<invalid>
    /// let v = &[0xD834, 0xDD1E, 0x006d, 0x0075, 0x0073, 0xDD1E, 0x0069, 0x0063, 0xD834];
    /// assert_eq!(LeanStr::from_utf16_lossy(v), "𝄞mus\u{FFFD}ic\u{FFFD}");
    /// ```
    #[inline]
    #[must_use]
    pub fn from_utf16_lossy(buf: &[u16]) -> Self {
        LeanString::from_utf16_lossy(buf).into_lean_str()
    }

    /// Returns a string slice containing the entire [`LeanStr`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::LeanStr;
    /// let s = LeanStr::from("foo");
    /// assert_eq!(s.as_str(), "foo");
    /// ```
    #[inline]
    #[must_use]
    pub const fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Returns a byte slice of the [`LeanStr`]'s contents.
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::LeanStr;
    /// let s = LeanStr::from("hello");
    /// assert_eq!(&[104, 101, 108, 108, 111], s.as_bytes());
    /// ```
    #[inline]
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// Returns the length of the [`LeanStr`] in bytes, not [`char`]s or graphemes.
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::LeanStr;
    /// let s = LeanStr::from("foo");
    /// assert_eq!(s.len(), 3);
    ///
    /// let fancy_f = LeanStr::from("ƒoo");
    /// assert_eq!(fancy_f.len(), 4);
    /// assert_eq!(fancy_f.chars().count(), 3);
    /// ```
    #[inline]
    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns `true` if the [`LeanStr`] has a length of zero, and `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::LeanStr;
    /// assert!(LeanStr::new().is_empty());
    /// assert!(!LeanStr::from("foo").is_empty());
    /// ```
    #[inline]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns whether the [`LeanStr`] is heap-allocated.
    ///
    /// # Examples
    ///
    /// ## inline
    ///
    /// ```
    /// # use lean_string::LeanStr;
    /// let s = LeanStr::from("hello");
    /// assert!(!s.is_heap_allocated());
    /// ```
    ///
    /// ## heap
    ///
    /// ```
    /// # use lean_string::LeanStr;
    /// let s = LeanStr::from("More than 2 * size_of::<usize>() bytes is heap-allocated");
    /// assert!(s.is_heap_allocated());
    /// ```
    #[inline]
    #[must_use]
    pub fn is_heap_allocated(&self) -> bool {
        self.0.is_heap_buffer()
    }

    /// Converts the [`LeanStr`] into a [`LeanString`].
    ///
    /// Inline strings and strings created from a `&'static str` are converted at zero cost.
    /// A heap-allocated string is moved into [`LeanString`]'s heap layout: the buffer is
    /// reallocated in place if it is not shared, and copied otherwise.
    ///
    /// # Panics
    ///
    /// Panics if converting a heap-allocated string fails to allocate. To handle the failure,
    /// use [`LeanStr::try_into_lean_string()`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::{LeanStr, LeanString};
    /// let s = LeanStr::from("This is a heap-allocated string!!");
    /// let s: LeanString = s.into_lean_string();
    /// assert_eq!(s, "This is a heap-allocated string!!");
    /// ```
    #[inline]
    #[must_use]
    pub fn into_lean_string(self) -> LeanString {
        self.try_into_lean_string().unwrap_with_msg()
    }

    /// Fallible version of [`LeanStr::into_lean_string()`].
    ///
    /// # Errors
    ///
    /// Returns a [`ReserveError`] if converting a heap-allocated string fails to allocate.
    /// Because this method consumes `self`, the original string is dropped on failure.
    #[inline]
    pub fn try_into_lean_string(mut self) -> Result<LeanString, ReserveError> {
        let repr = mem::replace(&mut self.0, Repr::new());
        match repr.into_mutable() {
            Ok(repr) => Ok(LeanString(repr)),
            Err((repr, err)) => {
                // Hand the original back to `self` so its `Drop` releases the buffer.
                self.0 = repr;
                Err(err)
            }
        }
    }
}

/// A [`Clone`] implementation for [`LeanStr`].
///
/// Same as [`LeanString`], cloning is performed by reference counting, so it is O(1) and does
/// not allocate.
///
/// # Examples
///
/// ```
/// # use lean_string::LeanStr;
/// let s1 = LeanStr::from("Hello, World!");
/// let s2 = s1.clone();
///
/// assert_eq!(s1, s2);
/// ```
impl Clone for LeanStr {
    #[inline]
    fn clone(&self) -> Self {
        LeanStr(self.0.make_shallow_clone())
    }

    #[inline]
    fn clone_from(&mut self, source: &Self) {
        self.0.replace_inner(source.0.make_shallow_clone());
    }
}

/// A [`Drop`] implementation for [`LeanStr`].
///
/// When the last reference to a [`LeanStr`] is dropped, its heap allocation (if any) is freed.
impl Drop for LeanStr {
    #[inline]
    fn drop(&mut self) {
        // SAFETY: This is the `Drop` implementation of a newtype wrapping `Repr`, and `self.0` is
        // never accessed again.
        unsafe { self.0.drop_in() };
    }
}

// SAFETY: `LeanStr` is `repr(transparent)` over `Repr`, and `Repr` works like `Arc`.
unsafe impl Send for LeanStr {}
unsafe impl Sync for LeanStr {}

impl Default for LeanString {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Default for LeanStr {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for LeanString {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl Deref for LeanStr {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Debug for LeanString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl fmt::Debug for LeanStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl fmt::Display for LeanString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_str(), f)
    }
}

impl fmt::Display for LeanStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_str(), f)
    }
}

impl AsRef<str> for LeanString {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for LeanStr {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[cfg(feature = "std")]
impl AsRef<OsStr> for LeanString {
    #[inline]
    fn as_ref(&self) -> &OsStr {
        OsStr::new(self.as_str())
    }
}

#[cfg(feature = "std")]
impl AsRef<OsStr> for LeanStr {
    #[inline]
    fn as_ref(&self) -> &OsStr {
        OsStr::new(self.as_str())
    }
}

#[cfg(feature = "std")]
impl AsRef<Path> for LeanString {
    #[inline]
    fn as_ref(&self) -> &Path {
        Path::new(self.as_str())
    }
}

#[cfg(feature = "std")]
impl AsRef<Path> for LeanStr {
    #[inline]
    fn as_ref(&self) -> &Path {
        Path::new(self.as_str())
    }
}

impl AsRef<[u8]> for LeanString {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl AsRef<[u8]> for LeanStr {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl Borrow<str> for LeanString {
    #[inline]
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for LeanStr {
    #[inline]
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl Eq for LeanString {}

impl Eq for LeanStr {}

impl PartialEq for LeanString {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.as_str().eq(other.as_str())
    }
}

impl PartialEq for LeanStr {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.as_str().eq(other.as_str())
    }
}

impl PartialEq<LeanStr> for LeanString {
    #[inline]
    fn eq(&self, other: &LeanStr) -> bool {
        self.as_str().eq(other.as_str())
    }
}

impl PartialEq<LeanString> for LeanStr {
    #[inline]
    fn eq(&self, other: &LeanString) -> bool {
        self.as_str().eq(other.as_str())
    }
}

impl PartialEq<str> for LeanString {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.as_str().eq(other)
    }
}

impl PartialEq<str> for LeanStr {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.as_str().eq(other)
    }
}

impl PartialEq<LeanString> for str {
    #[inline]
    fn eq(&self, other: &LeanString) -> bool {
        self.eq(other.as_str())
    }
}

impl PartialEq<LeanStr> for str {
    #[inline]
    fn eq(&self, other: &LeanStr) -> bool {
        self.eq(other.as_str())
    }
}

impl PartialEq<&str> for LeanString {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        self.as_str().eq(*other)
    }
}

impl PartialEq<&str> for LeanStr {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        self.as_str().eq(*other)
    }
}

impl PartialEq<LeanString> for &str {
    #[inline]
    fn eq(&self, other: &LeanString) -> bool {
        (*self).eq(other.as_str())
    }
}

impl PartialEq<LeanStr> for &str {
    #[inline]
    fn eq(&self, other: &LeanStr) -> bool {
        (*self).eq(other.as_str())
    }
}

impl PartialEq<String> for LeanString {
    #[inline]
    fn eq(&self, other: &String) -> bool {
        self.as_str().eq(other.as_str())
    }
}

impl PartialEq<String> for LeanStr {
    #[inline]
    fn eq(&self, other: &String) -> bool {
        self.as_str().eq(other.as_str())
    }
}

impl PartialEq<LeanString> for String {
    #[inline]
    fn eq(&self, other: &LeanString) -> bool {
        self.as_str().eq(other.as_str())
    }
}

impl PartialEq<LeanStr> for String {
    #[inline]
    fn eq(&self, other: &LeanStr) -> bool {
        self.as_str().eq(other.as_str())
    }
}

impl PartialEq<Cow<'_, str>> for LeanString {
    #[inline]
    fn eq(&self, other: &Cow<'_, str>) -> bool {
        self.as_str().eq(other.as_ref())
    }
}

impl PartialEq<Cow<'_, str>> for LeanStr {
    #[inline]
    fn eq(&self, other: &Cow<'_, str>) -> bool {
        self.as_str().eq(other.as_ref())
    }
}

impl PartialEq<LeanString> for Cow<'_, str> {
    #[inline]
    fn eq(&self, other: &LeanString) -> bool {
        self.as_ref().eq(other.as_str())
    }
}

impl PartialEq<LeanStr> for Cow<'_, str> {
    #[inline]
    fn eq(&self, other: &LeanStr) -> bool {
        self.as_ref().eq(other.as_str())
    }
}

impl Ord for LeanString {
    #[inline]
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl Ord for LeanStr {
    #[inline]
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl PartialOrd for LeanString {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialOrd for LeanStr {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Hash for LeanString {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state)
    }
}

impl Hash for LeanStr {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state)
    }
}

impl From<char> for LeanString {
    #[inline]
    #[track_caller]
    fn from(value: char) -> Self {
        LeanString(Repr::from_char(value))
    }
}

impl From<char> for LeanStr {
    #[inline]
    #[track_caller]
    fn from(value: char) -> Self {
        LeanStr(Repr::from_char(value))
    }
}

impl From<&str> for LeanString {
    #[inline]
    #[track_caller]
    fn from(value: &str) -> Self {
        LeanString(Repr::from_str(value).unwrap_with_msg())
    }
}

impl From<&str> for LeanStr {
    #[inline]
    #[track_caller]
    fn from(value: &str) -> Self {
        LeanStr(Repr::from_str(value).unwrap_with_msg())
    }
}

impl From<String> for LeanString {
    #[inline]
    #[track_caller]
    fn from(value: String) -> Self {
        LeanString(Repr::from_str(&value).unwrap_with_msg())
    }
}

impl From<String> for LeanStr {
    #[inline]
    #[track_caller]
    fn from(value: String) -> Self {
        LeanStr(Repr::from_str(&value).unwrap_with_msg())
    }
}

impl From<&String> for LeanString {
    #[inline]
    #[track_caller]
    fn from(value: &String) -> Self {
        LeanString(Repr::from_str(value).unwrap_with_msg())
    }
}

impl From<&String> for LeanStr {
    #[inline]
    #[track_caller]
    fn from(value: &String) -> Self {
        LeanStr(Repr::from_str(value).unwrap_with_msg())
    }
}

impl From<Cow<'_, str>> for LeanString {
    fn from(cow: Cow<str>) -> Self {
        match cow {
            Cow::Borrowed(s) => s.into(),
            Cow::Owned(s) => s.into(),
        }
    }
}

impl From<Cow<'_, str>> for LeanStr {
    fn from(cow: Cow<str>) -> Self {
        match cow {
            Cow::Borrowed(s) => s.into(),
            Cow::Owned(s) => s.into(),
        }
    }
}

impl From<Box<str>> for LeanString {
    #[inline]
    #[track_caller]
    fn from(value: Box<str>) -> Self {
        LeanString(Repr::from_str(&value).unwrap_with_msg())
    }
}

impl From<Box<str>> for LeanStr {
    #[inline]
    #[track_caller]
    fn from(value: Box<str>) -> Self {
        LeanStr(Repr::from_str(&value).unwrap_with_msg())
    }
}

impl From<&LeanString> for LeanString {
    #[inline]
    fn from(value: &LeanString) -> Self {
        value.clone()
    }
}

impl From<&LeanStr> for LeanStr {
    #[inline]
    fn from(value: &LeanStr) -> Self {
        value.clone()
    }
}

impl From<LeanString> for String {
    #[inline]
    fn from(value: LeanString) -> Self {
        value.as_str().into()
    }
}

impl From<LeanStr> for String {
    #[inline]
    fn from(value: LeanStr) -> Self {
        value.as_str().into()
    }
}

impl From<&LeanString> for String {
    #[inline]
    fn from(value: &LeanString) -> Self {
        value.as_str().into()
    }
}

impl From<&LeanStr> for String {
    #[inline]
    fn from(value: &LeanStr) -> Self {
        value.as_str().into()
    }
}

impl From<LeanString> for LeanStr {
    #[inline]
    #[track_caller]
    fn from(value: LeanString) -> Self {
        value.into_lean_str()
    }
}

impl From<LeanStr> for LeanString {
    #[inline]
    #[track_caller]
    fn from(value: LeanStr) -> Self {
        value.into_lean_string()
    }
}

impl FromStr for LeanString {
    type Err = ReserveError;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Repr::from_str(s).map(Self)
    }
}

impl FromStr for LeanStr {
    type Err = ReserveError;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Repr::from_str(s).map(Self)
    }
}

impl FromIterator<char> for LeanString {
    fn from_iter<T: IntoIterator<Item = char>>(iter: T) -> Self {
        let iter = iter.into_iter();

        let (lower_bound, _) = iter.size_hint();
        // If reserving `lower_bound` fails, fall back to empty and hope it was inaccurate
        let mut buf = LeanString::try_with_capacity(lower_bound).unwrap_or_default();

        for ch in iter {
            buf.push_str(ch.encode_utf8(&mut [0; 4]));
        }
        buf
    }
}

impl FromIterator<char> for LeanStr {
    fn from_iter<T: IntoIterator<Item = char>>(iter: T) -> Self {
        LeanString::from_iter(iter).into_lean_str()
    }
}

impl<'a> FromIterator<&'a char> for LeanString {
    fn from_iter<T: IntoIterator<Item = &'a char>>(iter: T) -> Self {
        iter.into_iter().copied().collect()
    }
}

impl<'a> FromIterator<&'a char> for LeanStr {
    fn from_iter<T: IntoIterator<Item = &'a char>>(iter: T) -> Self {
        LeanString::from_iter(iter).into_lean_str()
    }
}

impl<'a> FromIterator<&'a str> for LeanString {
    fn from_iter<I: IntoIterator<Item = &'a str>>(iter: I) -> Self {
        let mut buf = LeanString::new();
        buf.extend(iter);
        buf
    }
}

impl<'a> FromIterator<&'a str> for LeanStr {
    fn from_iter<I: IntoIterator<Item = &'a str>>(iter: I) -> Self {
        LeanString::from_iter(iter).into_lean_str()
    }
}

impl FromIterator<Box<str>> for LeanString {
    fn from_iter<I: IntoIterator<Item = Box<str>>>(iter: I) -> Self {
        let mut buf = LeanString::new();
        buf.extend(iter);
        buf
    }
}

impl FromIterator<Box<str>> for LeanStr {
    fn from_iter<I: IntoIterator<Item = Box<str>>>(iter: I) -> Self {
        LeanString::from_iter(iter).into_lean_str()
    }
}

impl<'a> FromIterator<Cow<'a, str>> for LeanString {
    fn from_iter<I: IntoIterator<Item = Cow<'a, str>>>(iter: I) -> Self {
        let mut buf = LeanString::new();
        buf.extend(iter);
        buf
    }
}

impl<'a> FromIterator<Cow<'a, str>> for LeanStr {
    fn from_iter<I: IntoIterator<Item = Cow<'a, str>>>(iter: I) -> Self {
        LeanString::from_iter(iter).into_lean_str()
    }
}

impl FromIterator<String> for LeanString {
    fn from_iter<I: IntoIterator<Item = String>>(iter: I) -> Self {
        let mut buf = LeanString::new();
        buf.extend(iter);
        buf
    }
}

impl FromIterator<String> for LeanStr {
    fn from_iter<I: IntoIterator<Item = String>>(iter: I) -> Self {
        LeanString::from_iter(iter).into_lean_str()
    }
}

impl FromIterator<LeanString> for LeanString {
    fn from_iter<T: IntoIterator<Item = LeanString>>(iter: T) -> Self {
        let mut iter = iter.into_iter();
        let Some(mut buf) = iter.next() else {
            return LeanString::new();
        };
        buf.extend(iter);
        buf
    }
}

impl FromIterator<LeanStr> for LeanString {
    fn from_iter<T: IntoIterator<Item = LeanStr>>(iter: T) -> Self {
        let mut iter = iter.into_iter();
        let Some(first) = iter.next() else {
            return LeanString::new();
        };
        let mut buf = first.into_lean_string();
        buf.extend(iter);
        buf
    }
}

impl FromIterator<LeanString> for LeanStr {
    fn from_iter<T: IntoIterator<Item = LeanString>>(iter: T) -> Self {
        LeanString::from_iter(iter).into_lean_str()
    }
}

impl FromIterator<LeanStr> for LeanStr {
    fn from_iter<T: IntoIterator<Item = LeanStr>>(iter: T) -> Self {
        let mut iter = iter.into_iter();
        let mut buf = match iter.next() {
            Some(str) => str.into_lean_string(),
            None => return LeanStr::new(),
        };
        buf.extend(iter);
        buf.into_lean_str()
    }
}

impl FromIterator<LeanString> for String {
    fn from_iter<T: IntoIterator<Item = LeanString>>(iter: T) -> Self {
        let mut buf = String::new();
        buf.extend(iter);
        buf
    }
}

impl FromIterator<LeanStr> for String {
    fn from_iter<T: IntoIterator<Item = LeanStr>>(iter: T) -> Self {
        let mut buf = String::new();
        buf.extend(iter);
        buf
    }
}

impl Extend<char> for LeanString {
    fn extend<T: IntoIterator<Item = char>>(&mut self, iter: T) {
        let iter = iter.into_iter();

        let (lower_bound, _) = iter.size_hint();
        // Ignore the error and hope that the lower_bound is incorrect.
        let _ = self.try_reserve(lower_bound);

        for ch in iter {
            self.push(ch);
        }
    }
}

impl<'a> Extend<&'a char> for LeanString {
    fn extend<T: IntoIterator<Item = &'a char>>(&mut self, iter: T) {
        self.extend(iter.into_iter().copied());
    }
}

impl<'a> Extend<&'a str> for LeanString {
    fn extend<T: IntoIterator<Item = &'a str>>(&mut self, iter: T) {
        iter.into_iter().for_each(|s| self.push_str(s));
    }
}

impl Extend<Box<str>> for LeanString {
    fn extend<T: IntoIterator<Item = Box<str>>>(&mut self, iter: T) {
        iter.into_iter().for_each(move |s| self.push_str(&s));
    }
}

impl<'a> Extend<Cow<'a, str>> for LeanString {
    fn extend<T: IntoIterator<Item = Cow<'a, str>>>(&mut self, iter: T) {
        iter.into_iter().for_each(move |s| self.push_str(&s));
    }
}

impl Extend<String> for LeanString {
    fn extend<T: IntoIterator<Item = String>>(&mut self, iter: T) {
        iter.into_iter().for_each(move |s| self.push_str(&s));
    }
}

impl Extend<LeanString> for LeanString {
    fn extend<T: IntoIterator<Item = LeanString>>(&mut self, iter: T) {
        for s in iter {
            self.push_str(&s);
        }
    }
}

impl Extend<LeanStr> for LeanString {
    fn extend<T: IntoIterator<Item = LeanStr>>(&mut self, iter: T) {
        for s in iter {
            self.push_str(&s);
        }
    }
}

impl Extend<LeanString> for String {
    fn extend<T: IntoIterator<Item = LeanString>>(&mut self, iter: T) {
        for s in iter {
            self.push_str(&s);
        }
    }
}

impl Extend<LeanStr> for String {
    fn extend<T: IntoIterator<Item = LeanStr>>(&mut self, iter: T) {
        for s in iter {
            self.push_str(&s);
        }
    }
}

impl fmt::Write for LeanString {
    #[inline]
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.push_str(s);
        Ok(())
    }

    #[inline]
    fn write_fmt(&mut self, args: fmt::Arguments<'_>) -> fmt::Result {
        match args.as_str() {
            Some(s) => {
                if self.is_empty() && !self.is_heap_allocated() {
                    // Since self is empty inline buffer or empty static buffer, constructing a new
                    // one with `from_static_str` is more efficient since it is O(1).
                    *self = LeanString::from_static_str(s);
                } else {
                    self.push_str(s);
                }
                Ok(())
            }
            None => fmt::write(self, args),
        }
    }
}

impl Add<&str> for LeanString {
    type Output = Self;

    #[inline]
    fn add(mut self, rhs: &str) -> Self::Output {
        self.push_str(rhs);
        self
    }
}

impl AddAssign<&str> for LeanString {
    #[inline]
    fn add_assign(&mut self, rhs: &str) {
        self.push_str(rhs);
    }
}

trait UnwrapWithMsg {
    type T;
    fn unwrap_with_msg(self) -> Self::T;
}

impl<T, E: fmt::Display> UnwrapWithMsg for Result<T, E> {
    type T = T;
    #[inline(always)]
    #[track_caller]
    fn unwrap_with_msg(self) -> T {
        #[inline(never)]
        #[cold]
        #[track_caller]
        fn do_panic_with_msg<E: fmt::Display>(error: E) -> ! {
            panic!("{error}")
        }

        match self {
            Ok(value) => value,
            Err(err) => do_panic_with_msg(err),
        }
    }
}
