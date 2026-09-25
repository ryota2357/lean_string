// The width of a 128-bit vector register, which most targets have.
const CHUNK_SIZE: usize = 16;

#[derive(Clone, Copy)]
pub(crate) enum Case {
    Lower,
    Upper,
}

#[inline]
pub(crate) fn split_at_first_change(text: &str, case: Case) -> Option<(&str, &str)> {
    let skipped = prefix_len_while(text.as_bytes(), |b| b.is_ascii() && !byte_changes(b, case));
    let (offset, _) = {
        // SAFETY: `text[..skipped]` is all ASCII, so `skipped` is a char boundary.
        let rest = unsafe { text.get_unchecked(skipped..) };
        rest.char_indices().find(|&(_, c)| char_changes(c, case))?
    };
    // SAFETY: `skipped + offset` is where the found char starts.
    Some(unsafe { split_at_unchecked(text, skipped + offset) })
}

#[inline]
pub(crate) fn split_at_first_ascii_change(text: &str, case: Case) -> Option<(&str, &str)> {
    let mid = prefix_len_while(text.as_bytes(), |b| !byte_changes(b, case));
    if mid == text.len() {
        return None;
    }
    // SAFETY: `mid` is right before a byte that changes, which is ASCII and starts a char.
    Some(unsafe { split_at_unchecked(text, mid) })
}

#[inline]
pub(crate) fn split_ascii_prefix(text: &str) -> (&str, &str) {
    let mid = prefix_len_while(text.as_bytes(), |b| b.is_ascii());
    // SAFETY: `text[..mid]` is all ASCII, so `mid` is a char boundary.
    unsafe { split_at_unchecked(text, mid) }
}

#[inline(always)]
fn byte_changes(byte: u8, case: Case) -> bool {
    match case {
        Case::Lower => byte.is_ascii_uppercase(),
        Case::Upper => byte.is_ascii_lowercase(),
    }
}

#[inline]
fn char_changes(ch: char, case: Case) -> bool {
    match case {
        Case::Lower => !ch.to_lowercase().eq([ch]),
        Case::Upper => !ch.to_uppercase().eq([ch]),
    }
}

// NOTE: `str::split_at_unchecked()` is private in the standard library.
/// # Safety
///
/// `mid` must be a char boundary of `text`.
#[inline(always)]
unsafe fn split_at_unchecked(text: &str, mid: usize) -> (&str, &str) {
    unsafe { (text.get_unchecked(..mid), text.get_unchecked(mid..)) }
}

#[inline(always)]
fn prefix_len_while(bytes: &[u8], pred: impl Fn(u8) -> bool) -> usize {
    let count_while = |bytes: &[u8]| bytes.iter().take_while(|&&b| pred(b)).count();

    let (chunks, remainder) = bytes.as_chunks::<CHUNK_SIZE>();
    let mut len = 0;
    for chunk in chunks {
        if !all_bytes(chunk, &pred) {
            return len + count_while(chunk);
        }
        len += CHUNK_SIZE;
    }
    len + count_while(remainder)
}

// Unlike `Iterator::all`, this examines every byte without returning early, so that the bytes of a
// chunk can be processed together. Summing and comparing against the chunk size is the form best
// optimized by LLVM, which doesn't recognize other similar idioms currently.
// See https://github.com/llvm/llvm-project/issues/96395
#[inline(always)]
fn all_bytes(chunk: &[u8; CHUNK_SIZE], pred: impl Fn(u8) -> bool) -> bool {
    chunk.iter().map(|&b| pred(b) as u8).sum::<u8>() as usize == CHUNK_SIZE
}
