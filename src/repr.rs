use super::ReserveError;

use core::{hint, marker::PhantomData, mem, ptr, slice, str};

#[cfg(not(loom))]
use core::sync::atomic::{Ordering::*, fence};
#[cfg(loom)]
use loom::sync::atomic::{Ordering::*, fence};

mod inline_buffer;
use inline_buffer::InlineBuffer;

mod static_buffer;
use static_buffer::StaticBuffer;

mod heap_buffer;
mod mutability {
    use super::heap_buffer::{ExactHeader, GrowableHeader, Header};

    trait Sealed {}

    #[expect(private_bounds)]
    pub(crate) trait Mutability: Sized + Sealed {
        type Header: Header;
    }

    pub(crate) enum Mutable {}
    impl Sealed for Mutable {}
    impl Mutability for Mutable {
        type Header = GrowableHeader;
    }

    pub(crate) enum Immutable {}
    impl Sealed for Immutable {}
    impl Mutability for Immutable {
        type Header = ExactHeader;
    }
}
pub(crate) use mutability::{Immutable, Mutability, Mutable};
type HeapBuffer<M> = heap_buffer::HeapBuffer<<M as Mutability>::Header>;

mod last_byte;
use last_byte::LastByte;

mod num_to_repr;
use num_to_repr::NumToRepr;

const MAX_INLINE_SIZE: usize = 2 * size_of::<usize>();

#[repr(C)]
#[cfg(target_pointer_width = "64")]
pub(crate) struct Repr<M: Mutability>(*const (), [u8; 7], LastByte, PhantomData<M>);

#[repr(C)]
#[cfg(target_pointer_width = "32")]
pub(crate) struct Repr<M: Mutability>(*const (), [u8; 3], LastByte, PhantomData<M>);

const _: () = {
    assert!(size_of::<Repr<Mutable>>() == MAX_INLINE_SIZE);
    assert!(size_of::<Option<Repr<Mutable>>>() == MAX_INLINE_SIZE);
    assert!(align_of::<Repr<Mutable>>() == align_of::<usize>());
    assert!(align_of::<Option<Repr<Mutable>>>() == align_of::<usize>());

    assert!(size_of::<Repr<Immutable>>() == MAX_INLINE_SIZE);
    assert!(size_of::<Option<Repr<Immutable>>>() == MAX_INLINE_SIZE);
    assert!(align_of::<Repr<Immutable>>() == align_of::<usize>());
    assert!(align_of::<Option<Repr<Immutable>>>() == align_of::<usize>());
};

// SAFETY: "true" and "false" are short enough (less than 8 bytes) to fit in InlineBuffer.
const INLINE_BUFFER_TRUE: InlineBuffer = unsafe { InlineBuffer::new("true") };
const INLINE_BUFFER_FALSE: InlineBuffer = unsafe { InlineBuffer::new("false") };

impl<M: Mutability> Repr<M> {
    #[inline]
    pub(crate) const fn new() -> Self {
        Repr::from_inline(InlineBuffer::empty())
    }

    #[inline]
    pub(crate) fn from_str(text: &str) -> Result<Self, ReserveError> {
        if text.len() <= MAX_INLINE_SIZE {
            // SAFETY: `text.len()` is less than or equal to `MAX_INLINE_SIZE`
            Ok(Repr::from_inline(unsafe { InlineBuffer::new(text) }))
        } else {
            HeapBuffer::<M>::new(text).map(Repr::from_heap)
        }
    }

    #[inline]
    pub(crate) fn from_char(ch: char) -> Self {
        let inline = unsafe {
            let mut buffer = [0; 4];
            let str = ch.encode_utf8(&mut buffer);
            InlineBuffer::new(str)
        };
        Repr::from_inline(inline)
    }

    #[inline]
    pub(crate) fn from_bool(b: bool) -> Self {
        if b {
            Repr::from_inline(INLINE_BUFFER_TRUE)
        } else {
            Repr::from_inline(INLINE_BUFFER_FALSE)
        }
    }

    #[inline]
    pub(crate) const fn from_static_str(text: &'static str) -> Result<Self, ReserveError> {
        if text.len() <= MAX_INLINE_SIZE {
            // SAFETY: `text.len()` is less than or equal to `MAX_INLINE_SIZE`
            Ok(Repr::from_inline(unsafe { InlineBuffer::new(text) }))
        } else {
            // NOTE: .map(Repr::from_heap) is not possible in a `const fn`
            match StaticBuffer::new(text) {
                Ok(buffer) => Ok(Repr::from_static(buffer)),
                Err(e) => Err(e),
            }
        }
    }

    #[inline]
    #[allow(private_bounds)]
    pub(crate) fn from_num(value: impl NumToRepr) -> Result<Self, ReserveError> {
        value.into_repr()
    }

    /// Creates a `Repr` of exactly `len` bytes whose contents are written by `init`.
    ///
    /// NOTE: If `init` panics, a heap allocation may leak (which is safe).
    ///
    /// # Safety
    ///
    /// `init` must initialize all `len` bytes with valid UTF-8.
    unsafe fn new_with(len: usize, init: impl FnOnce(*mut u8)) -> Result<Self, ReserveError> {
        if len <= MAX_INLINE_SIZE {
            let mut buffer = InlineBuffer::empty();
            init(buffer.as_mut_ptr());
            // SAFETY:
            // - From `#Safety`, `init` initialized `len` bytes with valid UTF-8.
            // - `len` is less than or equal to `MAX_INLINE_SIZE`.
            unsafe { buffer.set_len(len) };
            Ok(Repr::from_inline(buffer))
        } else {
            // SAFETY: From `#Safety`, `init` initializes all `len` bytes below.
            let buffer = unsafe { HeapBuffer::<M>::new_uninit(len) }?;
            init(buffer.ptr().as_ptr());
            Ok(Repr::from_heap(buffer))
        }
    }

    #[cfg(target_pointer_width = "64")]
    #[inline]
    pub(crate) const fn len(&self) -> usize {
        let last_byte = self.last_byte();

        let inline_len = {
            let this = (last_byte as usize).wrapping_sub(LastByte::MASK_1100_0000 as usize);
            // inline Ord::min because the trait impl is not const
            if MAX_INLINE_SIZE < this { MAX_INLINE_SIZE } else { this }
        };

        let mut len = self.tail_word() & (usize::MAX >> 8);

        // This code is compiled to a single branchless instruction, such as `cmov`
        if last_byte < LastByte::HeapMarker as u8 {
            len = inline_len
        }

        len
    }

    #[cfg(target_pointer_width = "32")]
    #[inline]
    pub(crate) const fn len(&self) -> usize {
        if self.is_heap_buffer() {
            // SAFETY: We just checked the discriminant to make sure we're heap allocated
            unsafe { self.as_heap_buffer() }.len()
        } else if self.is_static_buffer() {
            // SAFETY: we just checked that `self` is StaticBuffer
            unsafe { self.as_static_buffer() }.len()
        } else {
            // Remaining is InlineBuffer
            {
                let this =
                    (self.last_byte() as usize).wrapping_sub(LastByte::MASK_1100_0000 as usize);
                // inline Ord::min because the trait impl is not const
                if MAX_INLINE_SIZE < this { MAX_INLINE_SIZE } else { this }
            }
        }
    }

    #[inline]
    pub(crate) const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub(crate) const fn as_str(&self) -> &str {
        // SAFETY: A `Repr` contains valid UTF-8
        unsafe { str::from_utf8_unchecked(self.as_bytes()) }
    }

    #[inline]
    pub(crate) const fn as_bytes(&self) -> &[u8] {
        let len = self.len();

        let ptr = if self.last_byte() >= LastByte::HeapMarker as u8 {
            self.0 as *const u8
        } else {
            self as *const _ as *const u8
        };

        // SAFETY: data (`ptr`) is valid, aligned, and part of the same contiguous allocated `len`
        // chunk
        unsafe { slice::from_raw_parts(ptr, len) }
    }

    #[inline]
    pub(crate) fn is_unique(&self) -> bool {
        if self.is_heap_buffer() {
            // SAFETY: We just checked the discriminant to make sure we're heap allocated
            unsafe { self.as_heap_buffer() }.is_unique()
        } else {
            true
        }
    }

    #[inline]
    pub(crate) fn make_shallow_clone(&self) -> Self {
        if self.is_heap_buffer() {
            // SAFETY: We just checked that `self` is HeapBuffer.
            let heap = unsafe { self.as_heap_buffer() };

            // Same as Arc::clone.
            // No need to use `Acquire` ordering because a new reference is created from the
            // existing reference, we don't need to wait for the previous operations to complete.
            // No need to use `Release` ordering because we don't need after operations to wait for
            // the new reference to be created, which should be handled (synchronized) at the
            // drop/dealloc (decrement reference count) time.
            let prev = heap.reference_count().fetch_add(1, Relaxed);

            // Same as Arc::clone.
            // We use `isize::MAX` instead of `usize::MAX` because a reference count slightly
            // larger than the threshold may be observed if a large number of threads stay between
            // fetch_add ~ if. Using isize::MAX requires an unusual amount of threads to be stuck
            // in this position in order to overflow the reference counter. Therefore, in practice,
            // the reference counter can be guaranteed not to overflow at this position.
            if prev > isize::MAX as usize {
                ref_count_overflow(self)
            }

            // NOTE: A nested function cannot use the generic parameters of the enclosing impl,
            // so it declares its own `M`.
            #[cold]
            fn ref_count_overflow<M: Mutability>(repr: &Repr<M>) -> ! {
                // Decrement the reference count and deallocate the buffer (if needed).
                unsafe { ptr::read(repr) }.replace_inner(Repr::new());
                panic!("reference count overflow");
            }
        }

        // SAFETY:
        // - if `self` is HeapBuffer, we just incremented the reference count.
        // - if `self` is InlineBuffer or StaticBuffer, we just copied the bytes.
        unsafe { ptr::read(self) }
    }

    #[inline]
    pub(crate) fn replace_inner(&mut self, other: Self) {
        if self.is_heap_buffer() {
            // SAFETY: We just checked the discriminant to make sure we're heap allocated
            let heap = unsafe { self.as_heap_buffer_mut() };
            // SAFETY: `self` is overwritten immediately below and `heap` is not accessed again.
            unsafe { heap.release() };
        }

        *self = other;
    }

    /// [`Drop`] implementation for `Repr`.
    ///
    /// NOTE: DO NOT implement [`Drop`] for `Repr`. Keeping `Repr` free of drop glue lets us handle
    /// it without worrying about implicitly inserted drops or `ManuallyDrop` wrappers.
    ///
    /// # Safety
    ///
    /// - Must be called from the [`Drop`] implementation of a newtype wrapping `Repr`.
    /// - After calling this method, `self` must never be accessed again.
    #[inline]
    pub(crate) unsafe fn drop_in(&mut self) {
        if self.is_heap_buffer() {
            // SAFETY: We just checked the discriminant to make sure we're heap allocated
            let heap = unsafe { self.as_heap_buffer_mut() };
            // SAFETY: From `#Safety`, `self` is being dropped, so neither `self` nor `heap` is
            // accessed again.
            unsafe { heap.release() };
        }
    }

    #[inline(always)]
    pub(crate) const fn is_heap_buffer(&self) -> bool {
        self.last_byte() == LastByte::HeapMarker as u8
    }

    #[inline(always)]
    const fn is_static_buffer(&self) -> bool {
        self.last_byte() == LastByte::StaticMarker as u8
    }

    #[inline(always)]
    const fn is_inline_buffer(&self) -> bool {
        self.last_byte() < LastByte::HeapMarker as u8
    }

    #[inline(always)]
    const fn from_inline(buffer: InlineBuffer) -> Self {
        let repr: Self = unsafe { mem::transmute(buffer) };
        // SAFETY: We just transmuted from `InlineBuffer`, which is always tagged `InlineMarker`.
        unsafe { hint::assert_unchecked(repr.is_inline_buffer()) };
        repr
    }

    #[inline(always)]
    const fn from_heap(buffer: HeapBuffer<M>) -> Self {
        let repr: Self = unsafe { mem::transmute(buffer) };
        // SAFETY: We just transmuted from `HeapBuffer`, which is always tagged `HeapMarker`.
        unsafe { hint::assert_unchecked(repr.is_heap_buffer()) };
        repr
    }

    #[inline(always)]
    const fn from_static(buffer: StaticBuffer) -> Self {
        let repr: Self = unsafe { mem::transmute(buffer) };
        // SAFETY: We just transmuted from `StaticBuffer`, which is always tagged `StaticMarker`.
        unsafe { hint::assert_unchecked(repr.is_static_buffer()) };
        repr
    }

    #[inline(always)]
    const fn last_byte(&self) -> u8 {
        let last_byte = self.2 as u8;
        // NOTE: The optimizer does not realize that this byte read overlaps a word read, such as
        //       `tail_word()`. Stating this identity allows it to reason at both byte and word
        //       granularities, which promotes better codegen.
        // SAFETY: `last_byte` is stored as the top byte of `tail_word`.
        unsafe {
            hint::assert_unchecked(last_byte as usize == self.tail_word() >> (usize::BITS - 8))
        };
        last_byte
    }

    #[inline(always)]
    const fn tail_word(&self) -> usize {
        // SAFETY: `Repr` has the same size as `[usize; 2]` and is aligned as `usize`
        unsafe { usize::from_le(*(self as *const _ as *const usize).add(1)) }
    }

    #[inline(always)]
    unsafe fn as_inline_buffer_mut(&mut self) -> &mut InlineBuffer {
        // SAFETY: The caller guarantees `self` is an `InlineBuffer`.
        unsafe {
            hint::assert_unchecked(self.is_inline_buffer());
            &mut *(self as *mut _ as *mut InlineBuffer)
        }
    }

    #[inline(always)]
    const unsafe fn as_heap_buffer(&self) -> &HeapBuffer<M> {
        // SAFETY: The caller guarantees `self` is a `HeapBuffer`.
        unsafe {
            hint::assert_unchecked(self.is_heap_buffer());
            &*(self as *const _ as *const HeapBuffer<M>)
        }
    }

    #[inline(always)]
    unsafe fn as_heap_buffer_mut(&mut self) -> &mut HeapBuffer<M> {
        // SAFETY: The caller guarantees `self` is a `HeapBuffer`.
        unsafe {
            hint::assert_unchecked(self.is_heap_buffer());
            &mut *(self as *mut _ as *mut HeapBuffer<M>)
        }
    }

    #[inline(always)]
    const unsafe fn as_static_buffer(&self) -> &StaticBuffer {
        // SAFETY: The caller guarantees `self` is a `StaticBuffer`.
        unsafe {
            hint::assert_unchecked(self.is_static_buffer());
            &*(self as *const _ as *const StaticBuffer)
        }
    }

    #[inline(always)]
    unsafe fn as_static_buffer_mut(&mut self) -> &mut StaticBuffer {
        // SAFETY: The caller guarantees `self` is a `StaticBuffer`.
        unsafe {
            hint::assert_unchecked(self.is_static_buffer());
            &mut *(self as *mut _ as *mut StaticBuffer)
        }
    }
}

impl Repr<Mutable> {
    #[inline]
    pub(crate) fn with_capacity(capacity: usize) -> Result<Self, ReserveError> {
        if capacity <= MAX_INLINE_SIZE {
            Ok(Repr::new())
        } else {
            HeapBuffer::<Mutable>::with_capacity(capacity).map(Repr::from_heap)
        }
    }

    #[inline]
    pub(crate) fn capacity(&self) -> usize {
        if self.is_heap_buffer() {
            // SAFETY: We just checked the discriminant to make sure we're heap allocated
            unsafe { self.as_heap_buffer() }.capacity()
        } else if self.is_static_buffer() {
            // SAFETY: we just checked that `self` is StaticBuffer
            unsafe { self.as_static_buffer() }.len()
        } else {
            MAX_INLINE_SIZE
        }
    }

    #[inline]
    pub(crate) fn reserve(&mut self, additional: usize) -> Result<(), ReserveError> {
        let len = self.len();
        let needed_capacity = len.checked_add(additional).ok_or(ReserveError)?;

        macro_rules! outline {
            (($this:ident = self : $self_ty:ty $(, $var:ident : $ty:ty)* $(,)?) $(: $ret:ty)? $body:block) => {{
                #[cold]
                #[inline(never)]
                fn outlined_impl($this: $self_ty, $($var: $ty),*) $(-> $ret)? $body
                outlined_impl(self, $($var),*)
            }};
            (($($var:ident : $ty:ty),* $(,)?) $(: $ret:ty)? $body:block) => {{
                #[cold]
                #[inline(never)]
                fn outlined_impl($($var: $ty),*) $(-> $ret)? $body
                outlined_impl($($var),*)
            }};
        }

        if self.is_heap_buffer() {
            // SAFETY: We just checked that `self` is HeapBuffer
            let heap = unsafe { self.as_heap_buffer_mut() };

            if heap.is_unique() {
                if heap.capacity() >= needed_capacity {
                    // No need to reserve more capacity.
                    return Ok(());
                }

                outline!((heap: &mut HeapBuffer<Mutable>, len: usize, additional: usize): Result<(), ReserveError> {
                    let amortized_capacity = heap_buffer::amortized_growth(len, additional);
                    // SAFETY:
                    // - `heap` is unique (verified by `is_unique()`).
                    // - `amortized_capacity` is greater than `len`.
                    unsafe { heap.realloc(amortized_capacity) }
                })
            } else {
                // The heap is shared. We must read the data while our reference is still live
                // (ref count unchanged), then create a new independent buffer.

                outline!((this = self: &mut Repr<Mutable>, additional: usize): Result<(), ReserveError> {
                    // NOTE: We want to make this args for `outline!`, but `this` is a mutable reference so we can't do that.
                    // SAFETY: `this` is comes from `self`, we checked `self` is HeapBuffer.
                    let heap = unsafe { this.as_heap_buffer_mut() };

                    let str = heap.as_str();
                    let new_heap = HeapBuffer::<Mutable>::with_additional(str, additional)?;
                    // Release our reference only after the copy is complete. If the allocation above
                    // fails, ref count remains untouched (no leak).
                    // SAFETY: `this` is overwritten immediately below and `heap` is not accessed again.
                    unsafe { heap.release() };
                    *this = Repr::from_heap(new_heap);
                    Ok(())
                })
            }
        } else if self.is_static_buffer() {
            // We can't modify it, need to convert to other buffer.

            if needed_capacity <= MAX_INLINE_SIZE {
                outline!((this = self: &mut Repr<Mutable>): Result<(), ReserveError> {
                    // SAFETY: `len <= needed_capacity <= MAX_INLINE_SIZE`
                    let inline = unsafe { InlineBuffer::new(this.as_str()) };
                    *this = Repr::from_inline(inline);
                    Ok(())
                })
            } else {
                outline!((this = self: &mut Repr<Mutable>, additional: usize): Result<(), ReserveError> {
                    let heap = HeapBuffer::<Mutable>::with_additional(this.as_str(), additional)?;
                    *this = Repr::from_heap(heap);
                    Ok(())
                })
            }
        } else {
            // self is InlineBuffer

            if needed_capacity > MAX_INLINE_SIZE {
                outline!((this = self: &mut Repr<Mutable>, additional: usize): Result<(), ReserveError> {
                    let heap = HeapBuffer::<Mutable>::with_additional(this.as_str(), additional)?;
                    *this = Repr::from_heap(heap);
                    Ok(())
                })
            } else {
                // We have enough capacity, no need to reserve.
                Ok(())
            }
        }
    }

    #[inline]
    pub(crate) fn shrink_to(&mut self, min_capacity: usize) -> Result<(), ReserveError> {
        // If the buffer is not heap allocated, we can't shrink it.
        if !self.is_heap_buffer() {
            return Ok(());
        }

        // SAFETY: We did early return if the buffer is not HeapBuffer.
        let heap = unsafe { self.as_heap_buffer_mut() };

        let new_capacity = heap.len().max(min_capacity);
        let old_capacity = heap.capacity();

        if new_capacity <= MAX_INLINE_SIZE {
            // We can convert the HeapBuffer to InlineBuffer.
            // SAFETY:
            // `heap.len() <= new_capacity` and `new_capacity <= MAX_INLINE_SIZE`
            // thus, `heap.len() <= MAX_INLINE_SIZE`
            let inline = unsafe { InlineBuffer::new(heap.as_str()) };
            self.replace_inner(Repr::from_inline(inline));
        } else if new_capacity >= old_capacity {
            // No need to shrink the buffer.
        } else if heap.is_unique() {
            // Try to extend the buffer in place.
            // SAFETY: `heap` is unique, and `new_capacity < old_capacity`
            unsafe { heap.realloc(new_capacity)? };
        } else {
            // We need to create a new buffer because the current buffer is shared with others.
            let str = heap.as_str();
            let new_heap = HeapBuffer::<Mutable>::with_exact_capacity(str, new_capacity)?;
            // SAFETY: `self` is overwritten immediately below and `heap` is not accessed again.
            unsafe { heap.release() };
            *self = Repr::from_heap(new_heap);
        };

        Ok(())
    }

    #[inline]
    pub(crate) fn push_str(&mut self, string: &str) -> Result<(), ReserveError> {
        if string.is_empty() {
            return Ok(());
        }
        let len = self.len();
        let str_len = string.len();

        self.reserve(str_len)?;

        // SAFETY:
        // by calling `self.reserve()`:
        // - We have reserved enough capacity.
        // - The buffer is not StaticBuffer.
        // - If the buffer is HeapBuffer, it must be unique.
        // The source and destination don't overlap: any shared heap buffer was copied by
        // `reserve`, and safe Rust can't borrow the same unique buffer as both `&mut self` and
        // `string`.
        // After `copy_nonoverlapping`:
        // - `0..(len + str_len)` is initialized.
        unsafe {
            let data = self.as_mut_ptr();
            ptr::copy_nonoverlapping(string.as_ptr(), data.add(len), str_len);
            self.set_len(len + str_len);
        }

        Ok(())
    }

    #[inline]
    pub(crate) fn pop(&mut self) -> Result<Option<char>, ReserveError> {
        let ch = match self.as_str().chars().next_back() {
            Some(ch) => ch,
            None => return Ok(None),
        };

        // SAFETY: We know this is a valid length which falls on a char boundary
        let new_len = self.len() - ch.len_utf8();

        // SAFETY:
        // - `new_len` is less than `len()` because we calculated it from `len() - ch.len_utf8()`.
        // - `new_len` is a valid char boundary because `ch` is a valid char.
        unsafe { self.truncate_unchecked(new_len) }?;

        Ok(Some(ch))
    }

    #[inline]
    pub(crate) fn remove(&mut self, idx: usize) -> Result<char, ReserveError> {
        assert!(
            self.as_str().is_char_boundary(idx),
            "index is not a char boundary or out of bounds (index: {idx})",
        );

        let len = self.len();
        assert!(idx < len, "index out of bounds (index: {idx}, len: {len})",);

        // We will modify the buffer, we need to make sure it.
        self.ensure_modifiable()?;

        // SAFETY: `ensure_modifiable` guarantees that the buffer is not StaticBuffer and that
        // a heap buffer is unique.
        let ptr = unsafe { self.as_mut_ptr() };

        // Get the char we want to remove
        // SAFETY:
        // - `idx < len`, and `ptr` is valid for `len` initialized bytes.
        // - `idx` is a character boundary, so the nonempty suffix is valid UTF-8.
        let ch = unsafe {
            let suffix = slice::from_raw_parts(ptr.add(idx), len - idx);
            str::from_utf8_unchecked(suffix).chars().next().unwrap_unchecked()
        };
        let ch_len = ch.len_utf8();

        // Remove the char by shifting the rest of the string to the left.
        // SAFETY:
        // - Both ranges are within the initialized `0..len` bytes, and `ptr::copy` permits them to
        //   overlap.
        // - Removing a complete character leaves valid UTF-8 in `0..len - ch_len`.
        unsafe {
            ptr::copy(ptr.add(idx + ch_len), ptr.add(idx), len - idx - ch_len);
            self.set_len(len - ch_len);
        }

        Ok(ch)
    }

    #[inline]
    pub(crate) fn retain(
        &mut self,
        mut predicate: impl FnMut(char) -> bool,
    ) -> Result<(), ReserveError> {
        // We will modify the buffer, we need to make sure it.
        self.ensure_modifiable()?;

        struct SetLenOnDrop<'a> {
            self_: &'a mut Repr<Mutable>,
            src_idx: usize,
            dst_idx: usize,
        }

        let len = self.len();
        let mut g = SetLenOnDrop { self_: self, src_idx: 0, dst_idx: 0 };

        // SAFETY: `ensure_modifiable` guarantees that the buffer is not StaticBuffer and that
        // a heap buffer is unique.
        let ptr = unsafe { g.self_.as_mut_ptr() };

        while g.src_idx < len {
            // SAFETY:
            // - `g.src_idx < len`, and `ptr` is valid for `len` initialized bytes.
            // - Previous writes end at or before `g.src_idx`, so the untouched suffix remains
            //   valid UTF-8 and starts on a character boundary.
            let ch = unsafe {
                let suffix = slice::from_raw_parts(ptr.add(g.src_idx), len - g.src_idx);
                str::from_utf8_unchecked(suffix).chars().next().unwrap_unchecked()
            };
            let ch_len = ch.len_utf8();

            if predicate(ch) {
                if g.dst_idx != g.src_idx {
                    // SAFETY:
                    // - Both ranges are within the initialized `0..len` bytes.
                    // - The source is the UTF-8 encoding of `ch`, and `g.dst_idx` is a character
                    //   boundary. `ptr::copy` permits the ranges to overlap.
                    unsafe {
                        ptr::copy(ptr.add(g.src_idx), ptr.add(g.dst_idx), ch_len);
                    }
                }
                g.dst_idx += ch_len;
            }
            g.src_idx += ch_len;
        }

        impl Drop for SetLenOnDrop<'_> {
            #[inline]
            fn drop(&mut self) {
                // SAFETY:
                // - `dst_idx <= src_idx`, and `src_idx <= len`, so `dst_idx <= len`.
                // - `dst_idx` doesn't split a char because it is a sum of `ch_len`.
                unsafe { self.self_.set_len(self.dst_idx) }
            }
        }
        drop(g);

        Ok(())
    }

    #[inline]
    pub(crate) fn insert_str(&mut self, idx: usize, string: &str) -> Result<(), ReserveError> {
        assert!(
            self.as_str().is_char_boundary(idx),
            "index is not a char boundary or out of bounds (index: {idx})",
        );

        let new_len = self.len().checked_add(string.len()).ok_or(ReserveError)?;

        // reserve makes self unique and modifiable
        self.reserve(string.len())?;
        debug_assert!(self.is_unique());
        debug_assert!(!self.is_static_buffer());

        // SAFETY:
        // - We contracted that we can split self at `idx`.
        // - We just reserved enough capacity and set length after reserving.
        // - The gap is filled by valid UTF-8 bytes.
        unsafe {
            // first move the tail to the new back
            let data = self.as_mut_ptr();
            ptr::copy(data.add(idx), data.add(idx + string.len()), new_len - idx - string.len());

            // then insert the new bytes
            ptr::copy_nonoverlapping(string.as_ptr(), data.add(idx), string.len());

            // and lastly resize the string
            self.set_len(new_len);
        }
        Ok(())
    }

    #[inline]
    pub(crate) fn truncate(&mut self, new_len: usize) -> Result<(), ReserveError> {
        if new_len >= self.len() {
            return Ok(());
        }

        let str = self.as_str();
        assert!(
            str.is_char_boundary(new_len),
            "index is not a char boundary or out of bounds (index: {new_len})",
        );

        // SAFETY: We just checked that `new_len < len()` and `new_len` is a valid char
        unsafe { self.truncate_unchecked(new_len) }
    }

    /// # Safety
    ///
    /// - `new_len` must be less than or equal to `len()`
    /// - `new_len` must be a valid char boundary.
    unsafe fn truncate_unchecked(&mut self, new_len: usize) -> Result<(), ReserveError> {
        debug_assert!(new_len <= self.len());
        debug_assert!(self.as_str().is_char_boundary(new_len));

        if self.is_heap_buffer() {
            // SAFETY: We just checked that `self` is HeapBuffer
            let heap = unsafe { self.as_heap_buffer_mut() };

            if !heap.is_len_on_heap() {
                // Since len is inlined and we don't modify the buffer by popping a char, it is ok
                // to just set the new length.
                // SAFETY: `new_len <= len <= capacity`
                unsafe { heap.set_len(new_len) };
            } else if heap.is_unique() {
                // SAFETY: `heap` is unique, we can set the new length in place.
                unsafe { heap.set_len(new_len) };
            } else {
                // SAFETY: `heap.ptr` is valid for `new_len` bytes, and `HeapBuffer` contains
                // valid UTF-8. Use the pointer directly to avoid a len read from the heap header.
                let str = unsafe {
                    let ptr = heap.ptr().as_ptr();
                    let slice = slice::from_raw_parts(ptr, new_len);
                    str::from_utf8_unchecked(slice)
                };
                let new_repr = Repr::from_str(str)?;
                // SAFETY: `self` is overwritten immediately below and `heap` is not accessed again.
                unsafe { heap.release() };
                *self = new_repr;
            }
        } else if self.is_static_buffer() {
            // SAFETY:
            // - We just checked that `self` is StaticBuffer
            // - `new_len <= len <= capacity`
            unsafe { self.as_static_buffer_mut().set_len(new_len) };
        } else {
            // SAFETY:
            // - The number of types of buffer is 3, and the remaining is InlineBuffer.
            // - From `#Safety`, `new_len <= MAX_INLINE_SIZE` is true.
            unsafe { self.as_inline_buffer_mut().set_len(new_len) };
        }

        Ok(())
    }

    /// Convert the buffer to a modifiable buffer.
    ///
    /// This method ensures:
    ///
    /// - The buffer is not StaticBuffer.
    /// - If the buffer is HeapBuffer, it must be unique.
    fn ensure_modifiable(&mut self) -> Result<(), ReserveError> {
        if self.is_heap_buffer() {
            // SAFETY: we just checked self is HeapBuffer
            let heap = unsafe { self.as_heap_buffer_mut() };

            if !heap.is_unique() {
                // `heap` is shared, we need to create a new buffer.
                let str = heap.as_str();
                let new_heap = HeapBuffer::<Mutable>::new(str)?;
                // SAFETY: `self` is overwritten immediately below and `heap` is not accessed again.
                unsafe { heap.release() };
                *self = Repr::from_heap(new_heap);
            } else {
                // `heap` is unique, we can modify it in place.
            }
        } else if self.is_static_buffer() {
            // StaticBuffer is immutable, need to convert to other buffer.
            let next = Repr::from_str(self.as_str())?;
            self.replace_inner(next);
        }
        Ok(())
    }

    /// Gets a mutable pointer to the data buffer.
    ///
    /// # Safety
    /// - The buffer is not StaticBuffer
    /// - If the buffer is HeapBuffer, it must be unique.
    ///
    /// Only the bytes in `0..self.len()` are initialized. The bytes from `self.len()` to
    /// `self.capacity()` may be uninitialized and must not be used to create references to `u8`.
    unsafe fn as_mut_ptr(&mut self) -> *mut u8 {
        debug_assert!(!self.is_static_buffer());

        if self.is_heap_buffer() {
            let ptr = self.0 as *mut u8;
            // SAFETY: We just checked that `self` is HeapBuffer
            let heap = unsafe { self.as_heap_buffer() };
            debug_assert!(heap.is_unique());
            ptr
        } else {
            self as *mut _ as *mut u8
        }
    }

    /// # Safety
    /// - `new_len` must be less than or equal to `capacity()`
    /// - The elements at `0..new_len` must be initialized and valid UTF-8.
    /// - If the underlying buffer is a `HeapBuffer`, it must be unique.
    /// - If the underlying buffer is a `InlineBuffer`, `new_len <= MAX_INLINE_SIZE` must be true.
    #[inline]
    pub(crate) unsafe fn set_len(&mut self, new_len: usize) {
        debug_assert!(new_len <= self.capacity());

        if self.is_static_buffer() {
            // SAFETY:
            // - We just checked that `self` is StaticBuffer
            // - `new_len` is less than or equal to `capacity()`
            unsafe { self.as_static_buffer_mut().set_len(new_len) };
        } else if self.is_heap_buffer() {
            // SAFETY:
            // - We just checked that `self` is HeapBuffer.
            // - From `#Safety`, the buffer is unique.
            unsafe { self.as_heap_buffer_mut().set_len(new_len) };
        } else {
            // SAFETY:
            // - The number of types of buffer is 3, and the remaining is InlineBuffer.
            // - From `#Safety`, `new_len <= MAX_INLINE_SIZE` is true.
            unsafe { self.as_inline_buffer_mut().set_len(new_len) };
        }
    }

    #[inline]
    pub(crate) fn into_immutable(self) -> Result<Repr<Immutable>, (Self, ReserveError)> {
        if !self.is_heap_buffer() {
            // SAFETY: Only the heap variant differs between the two mutabilities: inline and
            // static buffers have the same representation in `Repr<Mutable>` and
            // `Repr<Immutable>`.
            return Ok(unsafe { mem::transmute::<Repr<Mutable>, Repr<Immutable>>(self) });
        }

        // SAFETY: We just checked that `self` is HeapBuffer. `Repr` has no drop glue, so the
        // counted reference is moved (not duplicated) into `heap`.
        let mut heap = unsafe { ptr::read(self.as_heap_buffer()) };
        let heap_str = heap.as_str();

        if heap.is_unique() {
            if heap_str.len() <= MAX_INLINE_SIZE {
                // SAFETY: We just checked that `heap_str.len() <= MAX_INLINE_SIZE`
                let inline = Repr::from_inline(unsafe { InlineBuffer::new(heap_str) });
                // The content is copied into the inline buffer, so drop our (unique) reference.
                // SAFETY: `heap` is not accessed again.
                unsafe { heap.release() };
                Ok(inline)
            } else {
                // SAFETY: `heap` is unique (verified by `is_unique()`).
                match unsafe { heap.into_exact() } {
                    Ok(exact) => Ok(Repr::from_heap(exact)),
                    Err((heap, err)) => Err((Repr::from_heap(heap), err)),
                }
            }
        } else {
            // The heap is shared, we need to copy it into a new immutable `Repr`.
            match Repr::<Immutable>::from_str(heap_str) {
                Ok(next) => {
                    // Release our reference only after the copy is complete. If the allocation
                    // above fails, the ref count remains untouched (no leak).
                    // SAFETY: `heap` is not accessed again.
                    unsafe { heap.release() };
                    Ok(next)
                }
                Err(err) => Err((Repr::from_heap(heap), err)),
            }
        }
    }
}

impl Repr<Immutable> {
    #[inline]
    pub(crate) fn into_mutable(self) -> Result<Repr<Mutable>, (Self, ReserveError)> {
        if !self.is_heap_buffer() {
            // SAFETY: Only the heap variant differs between the two mutabilities: inline and
            // static buffers have the same representation in `Repr<Mutable>` and
            // `Repr<Immutable>`.
            return Ok(unsafe { mem::transmute::<Repr<Immutable>, Repr<Mutable>>(self) });
        }

        // SAFETY: We just checked that `self` is HeapBuffer. `Repr` has no drop glue, so the
        // counted reference is moved (not duplicated) into `heap`.
        let mut heap = unsafe { ptr::read(self.as_heap_buffer()) };

        if heap.is_unique() {
            // SAFETY: `heap` is unique (verified by `is_unique()`).
            match unsafe { heap.into_growable() } {
                Ok(growable) => Ok(Repr::from_heap(growable)),
                Err((heap, err)) => Err((Repr::from_heap(heap), err)),
            }
        } else {
            // The heap is shared, we need to copy it into a new growable `Repr`.
            match Repr::<Mutable>::from_str(heap.as_str()) {
                Ok(next) => {
                    // Release our reference only after the copy is complete. If the allocation
                    // above fails, the ref count remains untouched (no leak).
                    // SAFETY: `heap` is not accessed again.
                    unsafe { heap.release() };
                    Ok(next)
                }
                Err(err) => Err((Repr::from_heap(heap), err)),
            }
        }
    }
}
