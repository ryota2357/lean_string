use super::*;

#[cfg(target_pointer_width = "64")]
#[repr(C, align(8))]
pub(super) struct InlineBuffer([u8; MAX_INLINE_SIZE]);

#[cfg(target_pointer_width = "32")]
#[repr(C, align(4))]
pub(super) struct InlineBuffer([u8; MAX_INLINE_SIZE]);

const _: () = {
    assert!(size_of::<InlineBuffer>() == MAX_INLINE_SIZE);
    assert!(align_of::<InlineBuffer>() == align_of::<usize>());
};

impl InlineBuffer {
    /// # Safety
    /// `text` must have a length less than or equal to `MAX_INLINE_SIZE`.
    #[inline]
    #[cfg(all(target_pointer_width = "64", target_endian = "little"))]
    pub(super) const unsafe fn new(text: &str) -> Self {
        debug_assert!(text.len() <= MAX_INLINE_SIZE);
        const _: () = assert!(MAX_INLINE_SIZE == 2 * size_of::<u64>());

        use core::ptr::read_unaligned as load;

        // Assemble `InlineBuffer` entirely in registers.
        // Ref: https://github.com/ParkMyCar/compact_str/blob/v0.10.0/compact_str/src/repr/inline.rs#L22-L128

        let len = text.len();
        let src = text.as_ptr();

        let last_byte = ((len as u64) | LastByte::MASK_1100_0000 as u64) << 56;

        let (w0, w1);
        unsafe {
            if len == MAX_INLINE_SIZE {
                w0 = load(src as *const u64);
                w1 = load(src.add(8) as *const u64);
            } else if len >= 8 {
                // SAFETY: `src` is valid for `len >= 8` bytes.
                w0 = load(src as *const u64);
                w1 = if len == 8 {
                    last_byte
                } else {
                    let tail = load(src.add(len - 8) as *const u64);
                    (tail >> ((MAX_INLINE_SIZE - len) * 8)) | last_byte
                };
            } else if len >= 4 {
                // SAFETY: `src` is valid for `len >= 4` bytes.
                let head = load(src as *const u32) as u64;
                let tail = load(src.add(len - 4) as *const u32) as u64;
                w0 = head | (tail << ((len - 4) * 8));
                w1 = last_byte;
            } else if len >= 2 {
                // SAFETY: `src` is valid for `len >= 2` bytes.
                let head = load(src as *const u16) as u64;
                let tail = load(src.add(len - 2) as *const u16) as u64;
                w0 = head | (tail << ((len - 2) * 8));
                w1 = last_byte;
            } else if len == 1 {
                w0 = *src as u64; // load(src as *const u8) as u64
                w1 = last_byte;
            } else {
                w0 = 0;
                w1 = last_byte;
            }
            mem::transmute([w0, w1])
        }
    }

    /// # Safety
    /// `text` must have a length less than or equal to `MAX_INLINE_SIZE`.
    #[cfg(not(all(target_pointer_width = "64", target_endian = "little")))]
    pub(super) const unsafe fn new(text: &str) -> Self {
        debug_assert!(text.len() <= MAX_INLINE_SIZE);

        let len = text.len();
        let mut buffer = [0u8; MAX_INLINE_SIZE];
        buffer[MAX_INLINE_SIZE - 1] = len as u8 | LastByte::MASK_1100_0000;

        // A `copy_nonoverlapping` with a runtime length emits a `memcpy` call, which is far too
        // expensive for a copy this short. Constant-size copies are inlined instead: for
        // `n <= len <= 2 * n`, a pair of `n`-byte copies taken from either end covers `0..len`
        // exactly, so halving `n` down from `MAX_INLINE_SIZE / 2` reaches every length that fits.
        //
        // `len == MAX_INLINE_SIZE` is peeled off first. It is the one length whose copy overwrites
        // the length byte written above, and peeling it lets the optimizer see that the remaining
        // copies never touch that byte.
        //
        // SAFETY:
        // - Every copy stays within `0..len`, for which src (`text`) is valid, and dst (`buffer`)
        //   is valid because `len <= MAX_INLINE_SIZE`.
        // - Both src and dst is aligned for u8.
        // - src and dst don't overlap because we created dst.
        unsafe {
            let src = text.as_ptr();
            let dst = buffer.as_mut_ptr();
            if len == MAX_INLINE_SIZE {
                ptr::copy_nonoverlapping(src, dst, MAX_INLINE_SIZE);
            } else if len >= MAX_INLINE_SIZE / 2 {
                const N: usize = MAX_INLINE_SIZE / 2;
                ptr::copy_nonoverlapping(src, dst, N);
                ptr::copy_nonoverlapping(src.add(len - N), dst.add(len - N), N);
            } else if len >= 4 {
                // Unreachable where `MAX_INLINE_SIZE / 2 == 4`; folded away at compile time.
                ptr::copy_nonoverlapping(src, dst, 4);
                ptr::copy_nonoverlapping(src.add(len - 4), dst.add(len - 4), 4);
            } else if len >= 2 {
                ptr::copy_nonoverlapping(src, dst, 2);
                ptr::copy_nonoverlapping(src.add(len - 2), dst.add(len - 2), 2);
            } else if len == 1 {
                *dst = *src;
            }
        }

        Self(buffer)
    }

    pub(super) const fn empty() -> Self {
        let mut buffer = [0; MAX_INLINE_SIZE];
        buffer[MAX_INLINE_SIZE - 1] = LastByte::Length00 as u8;
        Self(buffer)
    }

    pub(super) fn as_mut_ptr(&mut self) -> *mut u8 {
        self.0.as_mut_ptr()
    }

    /// # Safety
    /// - `len` bytes in the buffer must be valid UTF-8.
    /// - `len` must be less than or equal to `MAX_INLINE_SIZE`.
    pub(super) unsafe fn set_len(&mut self, len: usize) {
        debug_assert!(len <= MAX_INLINE_SIZE);

        if len < MAX_INLINE_SIZE {
            self.0[MAX_INLINE_SIZE - 1] = len as u8 | LastByte::MASK_1100_0000;
        }
    }
}
