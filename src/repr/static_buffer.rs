use super::*;

#[repr(C)]
pub(super) struct StaticBuffer {
    ptr: ptr::NonNull<u8>,
    len: usize, // stored as little-endian
}

const _: () = {
    assert!(size_of::<StaticBuffer>() == MAX_INLINE_SIZE);
    assert!(align_of::<StaticBuffer>() == align_of::<usize>());
};

const USIZE_SIZE: usize = size_of::<usize>();

impl StaticBuffer {
    const MAX_LENGTH: usize = {
        let mut bytes = [255; USIZE_SIZE];
        bytes[USIZE_SIZE - 1] = 0;
        usize::from_le_bytes(bytes)
    };

    const TAG: usize = {
        let mut bytes = [0; USIZE_SIZE];
        bytes[USIZE_SIZE - 1] = LastByte::StaticMarker as u8;
        usize::from_ne_bytes(bytes)
    };

    pub(super) const fn new(text: &'static str) -> Result<Self, ReserveError> {
        let text_len = text.len();

        if text_len > Self::MAX_LENGTH {
            return Err(ReserveError);
        }
        let len = text_len.to_le() | Self::TAG;

        // SAFETY: `&'static str` must have a non-null, properly aligned address
        let ptr = unsafe { ptr::NonNull::new_unchecked(text.as_ptr() as *mut _) };

        Ok(Self { ptr, len })
    }

    pub(super) const fn len(&self) -> usize {
        let len = self.len ^ Self::TAG;
        let bytes = len.to_ne_bytes();
        usize::from_le_bytes(bytes)
    }

    pub(super) const fn as_str(&self) -> &'static str {
        // SAFETY: `self.ptr` points to &'static str with valid `self.len()` length
        unsafe { str::from_utf8_unchecked(slice::from_raw_parts(self.ptr.as_ptr(), self.len())) }
    }

    /// # Safety
    /// - `len` bytes in the buffer must be valid UTF-8.
    /// - `len` must be less than or equal to the current length.
    pub(super) unsafe fn set_len(&mut self, len: usize) {
        debug_assert!(len <= self.len());
        self.len = len.to_le() | Self::TAG;
    }
}
