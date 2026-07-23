use super::*;
use alloc::alloc::{alloc, dealloc, realloc};
use core::{
    alloc::Layout,
    hint,
    marker::PhantomData,
    ptr::{self, NonNull},
};

#[cfg(not(loom))]
use core::sync::atomic::AtomicUsize;
#[cfg(loom)]
use loom::sync::atomic::AtomicUsize;

use internal::*;

/// [`HeapBuffer`] grows at an amortized rates of 1.5x
#[inline(always)]
pub(crate) fn amortized_growth(cur_len: usize, additional: usize) -> usize {
    let required = cur_len.saturating_add(additional);
    let amortized = cur_len.saturating_mul(3) / 2;
    amortized.max(required)
}

#[repr(C)]
pub struct HeapBuffer<H: Header> {
    // 64-bit architecture or 32-bit architecture if `is_len_heap_layout` is false:
    // | Header | Data (array of `u8`) |
    //          ^ ptr
    // 32-bit architecture if `is_len_heap_layout` is true:
    // | Length | Header | Data (array of `u8`) |
    //                   ^ ptr
    ptr: NonNull<u8>,
    len: TextLen,
    _header: PhantomData<H>,
}

trait Sealed {}

#[expect(private_bounds)]
pub(crate) trait Header: Sealed + Sized {
    const SIZE: usize;
    const ALIGN: usize;

    #[expect(private_interfaces)]
    fn new(capacity: Capacity) -> Self;

    fn count(&self) -> &AtomicUsize;

    fn layout(buffer: &HeapBuffer<Self>) -> Result<Layout, ReserveError>;

    fn has_len_prefix(buffer: &HeapBuffer<Self>) -> bool;
}

pub struct GrowableHeader {
    count: AtomicUsize,
    capacity: Capacity,
}

impl Sealed for GrowableHeader {}

impl Header for GrowableHeader {
    const SIZE: usize = size_of::<GrowableHeader>();
    const ALIGN: usize = align_of::<GrowableHeader>();

    #[expect(private_interfaces)]
    fn new(capacity: Capacity) -> Self {
        GrowableHeader { count: AtomicUsize::new(1), capacity }
    }

    #[inline(always)]
    fn count(&self) -> &AtomicUsize {
        &self.count
    }

    fn layout(buffer: &HeapBuffer<Self>) -> Result<Layout, ReserveError> {
        HeapBuffer::<Self>::layout_for(buffer.header().capacity)
    }

    #[inline(always)]
    fn has_len_prefix(buffer: &HeapBuffer<Self>) -> bool {
        is_len_heap_layout(buffer.header().capacity)
    }
}

pub struct ExactHeader {
    count: AtomicUsize,
}

impl Sealed for ExactHeader {}

impl Header for ExactHeader {
    const SIZE: usize = size_of::<ExactHeader>();
    const ALIGN: usize = align_of::<ExactHeader>();

    #[expect(private_interfaces)]
    fn new(_capacity: Capacity) -> Self {
        ExactHeader { count: AtomicUsize::new(1) }
    }

    #[inline(always)]
    fn count(&self) -> &AtomicUsize {
        &self.count
    }

    fn layout(buffer: &HeapBuffer<Self>) -> Result<Layout, ReserveError> {
        // SAFETY: The length of a live exact buffer was validated as a `Capacity` when the
        // buffer was allocated.
        let capacity = unsafe { Capacity::new_unchecked(buffer.len()) };
        HeapBuffer::<Self>::layout_for(capacity)
    }

    #[inline(always)]
    fn has_len_prefix(buffer: &HeapBuffer<Self>) -> bool {
        buffer.len.is_heap()
    }
}

const _: () = {
    assert!(size_of::<HeapBuffer<GrowableHeader>>() == MAX_INLINE_SIZE);
    assert!(align_of::<HeapBuffer<GrowableHeader>>() == align_of::<usize>());
    assert!(size_of::<HeapBuffer<ExactHeader>>() == MAX_INLINE_SIZE);
    assert!(align_of::<HeapBuffer<ExactHeader>>() == align_of::<usize>());
};

impl<H: Header> HeapBuffer<H> {
    pub(super) fn new(text: &str) -> Result<Self, ReserveError> {
        // SAFETY: All `text.len()` bytes are initialized by the copy below.
        let buffer = unsafe { Self::new_uninit(text.len()) }?;

        // SAFETY:
        // - src (`text`) and dst (`buffer.ptr`) is valid for `text.len()` bytes because
        //   `new_uninit` allocated at least `text.len()` bytes.
        // - Both src and dst is aligned for u8.
        // - src and dst don't overlap because we allocated dst just now.
        unsafe { ptr::copy_nonoverlapping(text.as_ptr(), buffer.ptr.as_ptr(), text.len()) };

        Ok(buffer)
    }

    /// Allocates a buffer for exactly `len` bytes, recording the length, and leaving the string
    /// data uninitialized.
    ///
    /// # Safety
    ///
    /// The caller must initialize all `len` bytes (via [`HeapBuffer::ptr`]) as valid UTF-8
    /// before the buffer content is read.
    pub(super) unsafe fn new_uninit(len: usize) -> Result<Self, ReserveError> {
        let text_len = TextLen::new(len)?;
        let ptr = Self::allocate_ptr(len)?;

        if text_len.is_heap() {
            // SAFETY: Since the layout is computed from the same `len`, `ptr` is allocated
            // with enough space to store the length.
            unsafe {
                let len_ptr = ptr.sub(Self::header_offset()).sub(size_of::<usize>());
                ptr::write(len_ptr.as_ptr().cast(), len);
            }
        }

        Ok(HeapBuffer { ptr, len: text_len, _header: PhantomData })
    }

    pub(super) fn ptr(&self) -> NonNull<u8> {
        self.ptr
    }

    pub(super) const fn len(&self) -> usize {
        #[cold]
        const fn len_on_heap<H: Header>(ptr: NonNull<u8>) -> usize {
            // SAFETY: The caller just checked that `len` is stored on the heap.
            unsafe {
                let header_offset = HeapBuffer::<H>::header_offset();
                let len_ptr = ptr.sub(header_offset).sub(size_of::<usize>());
                ptr::read(len_ptr.as_ptr().cast())
            }
        }
        if self.len.is_heap() { len_on_heap::<H>(self.ptr) } else { self.len.as_usize() }
    }

    pub(super) fn as_str(&self) -> &str {
        let len = self.len();
        let ptr = self.ptr.as_ptr();
        // SAFETY: HeapBuffer contains valid `len` bytes of UTF-8 string.
        unsafe { core::str::from_utf8_unchecked(slice::from_raw_parts(ptr, len)) }
    }

    #[inline]
    pub(super) fn is_unique(&self) -> bool {
        self.reference_count().load(Acquire) == 1
    }

    #[inline]
    pub(super) fn is_len_on_heap(&self) -> bool {
        self.len.is_heap()
    }

    #[inline]
    pub(super) fn reference_count(&self) -> &AtomicUsize {
        self.header().count()
    }

    /// Decrements the reference count. If this was the last reference, deallocates the buffer.
    ///
    /// # Safety
    ///
    /// - `self` must represent a live, counted reference to the allocation, so the reference count
    ///   must be nonzero.
    /// - After calling this method, `self` must not be accessed. The caller is responsible for
    ///   overwriting `self` or ensuring no further use occurs.
    pub(super) unsafe fn release(&mut self) {
        // Same as `Arc::drop`: `fetch_sub(1, Release)` ensures all prior accesses from other
        // threads are visible before we might deallocate.
        if self.reference_count().fetch_sub(1, Release) == 1 {
            #[cold]
            fn on_last_reference<H: Header>(this: &mut HeapBuffer<H>) {
                // And the `Acquire` fence ensures we see all writes before freeing the memory.
                fence(Acquire);

                // SAFETY: The old value of `fetch_sub` was `1`, so now it is `0`. no other references exist.
                unsafe { this.dealloc() };
            }
            on_last_reference(self);
        }
    }

    /// # Safety
    ///
    /// - No other references to the allocation may exist.
    /// - After deallocation, neither the fields of `self` nor any pointers or references derived
    ///   from them may be read or otherwise accessed. The `HeapBuffer` value itself may only be
    ///   immediately overwritten or forgotten.
    unsafe fn dealloc(&mut self) {
        unsafe {
            dealloc(self.allocation(), self.alloc_layout());
        }
    }

    /// Layout of the allocation `self` currently owns.
    fn alloc_layout(&self) -> Layout {
        match H::layout(self) {
            Ok(layout) => layout,
            Err(_) => {
                if cfg!(debug_assertions) {
                    panic!("invalid layout, unexpected buffer modification may have occurred");
                }
                // SAFETY:
                // `H::layout` should not return `Err` because this layout should not have been
                // changed since it was used in the previous allocation.
                unsafe { hint::unreachable_unchecked() }
            }
        }
    }

    fn allocate_ptr(size: usize) -> Result<NonNull<u8>, ReserveError> {
        let capacity = Capacity::new(size)?;
        let layout = Self::layout_for(capacity)?;

        // SAFETY: layout is non-zero.
        let mut allocation = unsafe { alloc(layout) };
        if allocation.is_null() {
            return Err(ReserveError);
        }

        if is_len_heap_layout(capacity) {
            // SAFETY:
            // - `allocation` is non-null.
            // - Since `layout` is created from the same `capacity`, we know that we reserved
            //   space for the length on the heap.
            unsafe { allocation = allocation.add(size_of::<usize>()) };
        }

        // SAFETY:
        // - allocation is non-null.
        // - allocation size is larger than or equal to the size of the header.
        unsafe {
            ptr::write(allocation.cast(), H::new(capacity));
            let ptr = allocation.add(Self::header_offset());
            Ok(NonNull::new_unchecked(ptr))
        }
    }

    fn layout_for(capacity: Capacity) -> Result<Layout, ReserveError> {
        let alloc_size = Self::header_offset()
            .checked_add(capacity.as_usize())
            .and_then(|size| {
                if is_len_heap_layout(capacity) {
                    size.checked_add(size_of::<usize>())
                } else {
                    Some(size)
                }
            })
            .ok_or(ReserveError)?;
        let align = Self::align();
        Layout::from_size_align(alloc_size, align).map_err(
            #[cold]
            |_| ReserveError,
        )
    }

    unsafe fn allocation(&self) -> *mut u8 {
        unsafe {
            if H::has_len_prefix(self) {
                cold_path();
                self.ptr.as_ptr().cast::<u8>().sub(Self::header_offset()).sub(size_of::<usize>())
            } else {
                self.ptr.as_ptr().cast::<u8>().sub(Self::header_offset())
            }
        }
    }

    const fn header(&self) -> &H {
        unsafe { &*self.ptr.as_ptr().sub(Self::header_offset()).cast() }
    }

    const fn align() -> usize {
        const {
            assert!(H::ALIGN == align_of::<usize>());
            assert!(align_of::<NonNull<u8>>() == align_of::<usize>());
        }
        align_of::<usize>()
    }

    const fn header_offset() -> usize {
        max(H::SIZE, Self::align())
    }
}

impl HeapBuffer<GrowableHeader> {
    pub(super) fn with_capacity(capacity: usize) -> Result<Self, ReserveError> {
        let len = TextLen::new(0)?;
        let ptr = Self::allocate_ptr(capacity)?;
        Ok(HeapBuffer { ptr, len, _header: PhantomData })
    }

    pub(super) fn with_exact_capacity(text: &str, capacity: usize) -> Result<Self, ReserveError> {
        if text.len() > capacity {
            return Err(ReserveError);
        }

        let mut buffer = HeapBuffer::with_capacity(capacity)?;

        // SAFETY:
        // - `buffer` is uniquely owned and has enough capacity for `text`.
        // - `text` contains valid UTF-8 and does not overlap the new allocation.
        unsafe {
            ptr::copy_nonoverlapping(text.as_ptr(), buffer.ptr.as_ptr(), text.len());
            buffer.set_len(text.len());
        }

        Ok(buffer)
    }

    pub(super) fn with_additional(text: &str, additional: usize) -> Result<Self, ReserveError> {
        let text_len = text.len();

        let len = TextLen::new(text_len)?;
        let ptr = Self::allocate_ptr(amortized_growth(text_len, additional))?;

        if len.is_heap() {
            // SAFETY: Since the `new_capacity` is greater than or equal to `text_len`, `ptr` is
            // allocated with enough space to store the length.
            unsafe {
                let len_ptr = ptr.sub(Self::header_offset()).sub(size_of::<usize>());
                ptr::write(len_ptr.as_ptr().cast(), text_len);
            }
        }

        // SAFETY:
        // - src (`text`) and dst (`ptr`) is valid for `text_len` bytes because `text_len` comes
        //   from `text`, and `ptr` was allocated to be at least `new_capacity` bytes, which is
        //   greater than `text_len`.
        // - Both src and dst is aligned for u8.
        // - src and dst don't overlap because we allocated dst just now.
        unsafe { ptr::copy_nonoverlapping(text.as_ptr(), ptr.as_ptr(), text_len) };

        Ok(HeapBuffer { ptr, len, _header: PhantomData })
    }

    pub(super) fn capacity(&self) -> usize {
        self.header().capacity.as_usize()
    }

    /// # Safety
    /// - The buffer must be unique. (HeapBuffer::is_unique() == true)
    /// - `new_capacity` must be greater than or equal to the current string length.
    pub(super) unsafe fn realloc(&mut self, new_capacity: usize) -> Result<(), ReserveError> {
        debug_assert!(self.is_unique());
        debug_assert!(self.len() <= new_capacity);

        let new_capacity = Capacity::new(new_capacity)?;
        let cur_capacity = self.header().capacity;
        let cur_layout = self.alloc_layout();

        let len_heap = match (is_len_heap_layout(cur_capacity), is_len_heap_layout(new_capacity)) {
            (false, false) => false,
            (true, true) => true,
            (true, false) | (false, true) => {
                let str = self.as_str();
                let mut new_buf = HeapBuffer::with_capacity(new_capacity.as_usize())?;
                unsafe {
                    ptr::copy_nonoverlapping(str.as_ptr(), new_buf.ptr.as_ptr(), str.len());
                    new_buf.set_len(str.len());
                    self.dealloc();
                }
                *self = new_buf;
                return Ok(());
            }
        };

        let new_alloc_size = {
            #[cfg(target_pointer_width = "64")]
            {
                // Since The maximum size of `capacity` is limited to 2^56 - 1, we no longer need
                // to check for overflow when rounding up to the nearest multiple of alignment.
                Self::header_offset().wrapping_add(new_capacity.as_usize())
            }
            #[cfg(target_pointer_width = "32")]
            {
                const ALLOC_LIMIT: usize =
                    (isize::MAX as usize + 1) - HeapBuffer::<GrowableHeader>::align();
                let mut alloc_size = Self::header_offset().saturating_add(new_capacity.as_usize());
                if len_heap {
                    alloc_size = alloc_size.saturating_add(size_of::<usize>());
                }
                if alloc_size > ALLOC_LIMIT {
                    return Err(ReserveError);
                }
                alloc_size
            }
        };

        // SAFETY:
        // - `self.allocation()` is already allocated by global allocator.
        // - current allocation is allocated by `cur_layout`.
        // - `new_alloc_size` is greater than zero.
        // - `new_alloc_size` is ensured not to overflow when rounded up to the nearest multiple of
        //    alignment.
        let mut allocation = unsafe { realloc(self.allocation(), cur_layout, new_alloc_size) };
        if allocation.is_null() {
            return Err(ReserveError);
        }

        if len_heap {
            // SAFETY: `allocation` is non-null.
            unsafe { allocation = allocation.add(size_of::<usize>()) };
        }

        // SAFETY:
        // - `allocation` is non-null.
        // - the allocation size is larger than or equal to the size of Header.
        unsafe {
            ptr::write(
                allocation.cast(),
                GrowableHeader {
                    count: AtomicUsize::new(1), // is_unique() is true.
                    capacity: new_capacity,
                },
            );
            let ptr = allocation.add(Self::header_offset());
            self.ptr = NonNull::new_unchecked(ptr);
        }
        Ok(())
    }

    /// Converts this unique growable allocation into an exact allocation, reusing the allocation
    /// with `realloc`.
    ///
    /// On failure, the buffer is returned unchanged along with the error.
    ///
    /// # Safety
    /// - The buffer must be unique. (HeapBuffer::is_unique() == true)
    pub(super) unsafe fn into_exact(self) -> Result<HeapBuffer<ExactHeader>, (Self, ReserveError)> {
        debug_assert!(self.is_unique());

        let len = self.len();
        let old_capacity = self.header().capacity;

        let old_layout = self.alloc_layout();
        // SAFETY: `len` is not greater than `old_capacity`, which is a valid `Capacity`.
        let new_capacity = unsafe { Capacity::new_unchecked(len) };
        let new_layout = match HeapBuffer::<ExactHeader>::layout_for(new_capacity) {
            Ok(layout) => layout,
            Err(err) => return Err((self, err)),
        };

        // The length prefix of an exact allocation exists iff the length itself does not fit in
        // `TextLen`, which is exactly what `self.len.is_heap()` reports.
        let old_len_prefix = if is_len_heap_layout(old_capacity) { size_of::<usize>() } else { 0 };
        let new_len_prefix = if self.len.is_heap() { size_of::<usize>() } else { 0 };

        // SAFETY: `self` owns a live allocation described by `old_layout`.
        let allocation = unsafe { self.allocation() };

        // Move the string bytes to the exact-layout offset before shrinking the allocation,
        // because the exact data starts before the growable data. `ptr::copy` permits overlap.
        let old_data = self.ptr.as_ptr();
        let new_data =
            unsafe { allocation.add(new_len_prefix + HeapBuffer::<ExactHeader>::header_offset()) };
        unsafe { ptr::copy(old_data, new_data, len) };

        // SAFETY:
        // - `allocation` was allocated with `old_layout` by the global allocator.
        // - `new_layout` has the same alignment and a non-zero size.
        let new_allocation = unsafe { realloc(allocation, old_layout, new_layout.size()) };
        if new_allocation.is_null() {
            // `realloc` failure leaves the old allocation live, but the data move above may have
            // overwritten the length prefix and the header. Restore them so that `self` remains
            // valid.
            unsafe {
                ptr::copy(new_data, old_data, len);
                if old_len_prefix != 0 {
                    ptr::write(allocation.cast(), len);
                }
                let header = allocation.add(old_len_prefix).cast::<GrowableHeader>();
                ptr::write(
                    header,
                    // count is 1 because the buffer is unique.
                    GrowableHeader { count: AtomicUsize::new(1), capacity: old_capacity },
                );
            }
            return Err((self, ReserveError));
        }

        // SAFETY: `new_allocation` is a live allocation described by `new_layout`, and the string
        // bytes were already moved to the exact-layout offset.
        let ptr = unsafe {
            if new_len_prefix != 0 {
                ptr::write(new_allocation.cast(), len);
            }
            let header = new_allocation.add(new_len_prefix).cast::<ExactHeader>();
            // count is 1 because the buffer is unique.
            ptr::write(header, ExactHeader { count: AtomicUsize::new(1) });
            let data =
                new_allocation.add(new_len_prefix + HeapBuffer::<ExactHeader>::header_offset());
            NonNull::new_unchecked(data)
        };

        // `TextLen` has the same representation for growable and exact buffers.
        Ok(HeapBuffer { ptr, len: self.len, _header: PhantomData })
    }

    /// # Safety
    /// - `len` bytes in the buffer must be valid UTF-8.
    /// - `len` must be less than or equal to the capacity.
    /// - If `len` is stored on the heap, the buffer must be unique.
    pub(super) unsafe fn set_len(&mut self, len: usize) {
        debug_assert!(len <= self.capacity());

        let new_len = match TextLen::new(len) {
            Ok(len) => len,
            Err(_) => {
                if cfg!(debug_assertions) {
                    panic!("Invalid `set_len` call");
                }
                // SAFETY: `TextSize::new` should not return `Err` because `len` bytes are allocated
                // as a valid UTF-8 string buffer.
                unsafe { hint::unreachable_unchecked() }
            }
        };
        debug_assert!(if new_len.is_heap() { self.is_unique() } else { true });
        self.len = new_len;

        #[cold]
        fn write_len_on_heap(ptr: NonNull<u8>, len: usize) {
            // SAFETY: The caller just checked that `len` is stored on the heap.
            unsafe {
                let header_offset = HeapBuffer::<GrowableHeader>::header_offset();
                let len_ptr = ptr.sub(header_offset).sub(size_of::<usize>());
                ptr::write(len_ptr.as_ptr().cast(), len);
            }
        }
        if self.len.is_heap() {
            write_len_on_heap(self.ptr, len);
        }
    }
}

impl HeapBuffer<ExactHeader> {
    /// Converts this unique exact allocation into a growable allocation whose capacity equals
    /// the current length, reusing the allocation with `realloc`.
    ///
    /// On failure, the buffer is returned unchanged along with the error.
    ///
    /// # Safety
    /// - The buffer must be unique. (HeapBuffer::is_unique() == true)
    pub(super) unsafe fn into_growable(
        self,
    ) -> Result<HeapBuffer<GrowableHeader>, (Self, ReserveError)> {
        debug_assert!(self.is_unique());

        let len = self.len();
        // SAFETY: The length of a live exact buffer was validated as a `Capacity` when the
        // buffer was allocated.
        let capacity = unsafe { Capacity::new_unchecked(len) };

        let old_layout = self.alloc_layout();
        let new_layout = match HeapBuffer::<GrowableHeader>::layout_for(capacity) {
            Ok(layout) => layout,
            Err(err) => return Err((self, err)),
        };

        // The growable capacity equals the exact length, so the presence of the length prefix
        // does not change across the conversion.
        let len_prefix = if self.len.is_heap() { size_of::<usize>() } else { 0 };

        // SAFETY: `self` owns a live allocation described by `old_layout`.
        let allocation = unsafe { self.allocation() };

        // SAFETY:
        // - `allocation` was allocated with `old_layout` by the global allocator.
        // - `new_layout` has the same alignment and a non-zero size.
        let new_allocation = unsafe { realloc(allocation, old_layout, new_layout.size()) };
        if new_allocation.is_null() {
            // `realloc` failure leaves the old allocation untouched, so `self` is still valid.
            return Err((self, ReserveError));
        }

        // The expanded allocation still holds the string at the exact-layout offset. Move it
        // behind the larger growable header before writing the header. `ptr::copy` permits
        // overlap.
        // SAFETY: `new_allocation` is a live allocation described by `new_layout`.
        let ptr = unsafe {
            let old_data = new_allocation.add(len_prefix + Self::header_offset());
            let new_data =
                new_allocation.add(len_prefix + HeapBuffer::<GrowableHeader>::header_offset());
            ptr::copy(old_data, new_data, len);
            if len_prefix != 0 {
                ptr::write(new_allocation.cast(), len);
            }
            let header = new_allocation.add(len_prefix).cast::<GrowableHeader>();
            // count is 1 because the buffer is unique.
            ptr::write(header, GrowableHeader { count: AtomicUsize::new(1), capacity });
            NonNull::new_unchecked(new_data)
        };

        // `TextLen` has the same representation for growable and exact buffers.
        Ok(HeapBuffer { ptr, len: self.len, _header: PhantomData })
    }
}

/// const version of `std::cmp::max::<usize>(x, y)`.
const fn max(x: usize, y: usize) -> usize {
    if x > y { x } else { y }
}

mod internal {
    use super::*;

    /// The length of a [`HeapBuffer`].
    ///
    /// An unsinged integer that uses `size_of::<usize>() - 1` bytes, and the rest 1 byte is used
    /// as a tag.
    ///
    /// Internally, the integer is stored in little-endian order, so the memory layout is like:
    ///
    /// +--------------------------------+--------+
    /// |        unsinged integer        |   tag  |
    /// | (size_of::<usize>() - 1) bytes | 1 byte |
    /// +--------------------------------+--------+
    ///
    /// And the tag is [`LastByte::Heap`].
    ///
    /// In this representation, the max value is limited to:
    ///
    /// - (on 64-bit architecture) 2^56 - 1 = 72057594037927935 = 64 PiB
    /// - (on 32-bit architecture) 2^24 - 2 = 16777214          ≈ 16 MiB
    ///
    /// Practically speaking, on 64-bit architecture, this max value is enough for the
    /// length/capacity of a HeapBuffer. However, it is not enough for 32-bit architectures, and if
    /// more than 3 bytes are needed, the length/capacity must be switched to be stored using the
    /// heap. Therefore, on 32-bit architecture, we use 2^24 - 2 as the maximum value, and 2^24 - 1
    /// as the tag that indicates the length/capacity is stored in the heap.
    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub(super) struct TextLen(usize);

    const USIZE_SIZE: usize = size_of::<usize>();

    const MAX_LEN: usize = {
        let mut bytes = [255; USIZE_SIZE];
        bytes[USIZE_SIZE - 1] = 0;
        usize::from_le_bytes(bytes) - if cfg!(target_pointer_width = "32") { 1 } else { 0 }
    };

    impl TextLen {
        const TAG: usize = {
            let mut bytes = [0; USIZE_SIZE];
            bytes[USIZE_SIZE - 1] = LastByte::HeapMarker as u8;
            usize::from_ne_bytes(bytes)
        };

        #[cfg(target_pointer_width = "32")]
        const ON_THE_HEAP: usize = {
            let mut bytes = [255; USIZE_SIZE];
            bytes[USIZE_SIZE - 1] = LastByte::HeapMarker as u8;
            usize::from_ne_bytes(bytes)
        };

        pub(super) const fn new(size: usize) -> Result<Self, ReserveError> {
            if size > MAX_LEN {
                #[cfg(target_pointer_width = "64")]
                return Err(ReserveError);
                #[cfg(target_pointer_width = "32")]
                return Ok(TextLen(Self::ON_THE_HEAP));
            }
            Ok(TextLen(size.to_le() | Self::TAG))
        }

        #[inline(always)]
        pub(super) const fn is_heap(&self) -> bool {
            #[cfg(target_pointer_width = "64")]
            return false;
            #[cfg(target_pointer_width = "32")]
            return self.0 == Self::ON_THE_HEAP;
        }

        pub(super) const fn as_usize(self) -> usize {
            let size = self.0 ^ Self::TAG;
            let bytes = size.to_ne_bytes();
            usize::from_le_bytes(bytes)
        }
    }

    #[cfg_attr(target_pointer_width = "64", allow(unused_variables))]
    #[inline(always)]
    pub(super) fn is_len_heap_layout(capacity: Capacity) -> bool {
        #[cfg(target_pointer_width = "64")]
        return false;
        #[cfg(target_pointer_width = "32")]
        return capacity.as_usize() > MAX_LEN;
    }

    /// The capacity of a [`HeapBuffer`].
    ///
    /// Maximum capacity is limited to [`Capacity::MAX`].
    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub(super) struct Capacity(usize);

    impl Capacity {
        /// The largest value a [`Capacity`] may hold:
        ///
        /// - (on 64-bit architecture) 2^56 - 1
        /// - (on 32-bit architecture) 2^31 - 1
        pub(super) const MAX: usize =
            if cfg!(target_pointer_width = "64") { MAX_LEN } else { isize::MAX as usize };

        #[inline]
        pub(crate) fn new(capacity: usize) -> Result<Self, ReserveError> {
            if capacity > Self::MAX {
                cold_path();
                return Err(ReserveError);
            }
            // SAFETY: We just checked that `capacity` is not greater than `Capacity::MAX`.
            Ok(unsafe { Capacity::new_unchecked(capacity) })
        }

        /// Creates a `Capacity` without checking the maximum-value invariant.
        ///
        /// # Safety
        ///
        /// `capacity` must not be greater than [`Capacity::MAX`].
        #[inline(always)]
        pub(super) unsafe fn new_unchecked(capacity: usize) -> Self {
            debug_assert!(capacity <= Self::MAX);
            // NOTE: Callers that build a `Capacity` out of a length the allocation already
            //       validated have no range check left in sight, so the optimizer cannot rule out
            //       an oversized `capacity` and keeps the overflow and `Layout` size checks of
            //       `layout_for` alive. Restating the bound here is what folds them away.
            // SAFETY: From `#Safety`, `capacity` is not greater than `Capacity::MAX`.
            unsafe { hint::assert_unchecked(capacity <= Self::MAX) };
            Capacity(capacity)
        }

        #[inline(always)]
        pub(crate) fn as_usize(&self) -> usize {
            self.0
        }
    }

    // TODO: Replace with hint::cold_path when it becomes stable.
    // Related issues:
    // - https://github.com/rust-lang/rust/issues/26179
    // - https://github.com/rust-lang/rust/pull/120370
    // - https://github.com/rust-lang/libs-team/issues/510
    #[cold]
    pub(super) fn cold_path() {}

    #[cfg(all(test, target_pointer_width = "32"))]
    mod tests {
        use super::*;

        #[test]
        fn heap_stored_length_preserves_repr_tag() {
            let len = TextLen::new(MAX_LEN + 1).unwrap();

            assert!(len.is_heap());
            assert_eq!(len.0.to_ne_bytes()[USIZE_SIZE - 1], LastByte::HeapMarker as u8);
        }
    }
}
