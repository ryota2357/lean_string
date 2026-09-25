use core::mem::MaybeUninit;

// The width of a 128-bit vector register, which most targets have.
const CHUNK_SIZE: usize = 16;

#[derive(Clone, Copy)]
pub(crate) enum CaseMapping {
    ToLower,
    ToUpper,
}

impl CaseMapping {
    #[inline(always)]
    fn changes_ascii(self, byte: u8) -> bool {
        match self {
            CaseMapping::ToLower => byte.is_ascii_uppercase(),
            CaseMapping::ToUpper => byte.is_ascii_lowercase(),
        }
    }

    #[inline(always)]
    fn map_ascii(self, byte: u8) -> u8 {
        match self {
            CaseMapping::ToLower => byte.to_ascii_lowercase(),
            CaseMapping::ToUpper => byte.to_ascii_uppercase(),
        }
    }

    #[inline]
    fn changes(self, ch: char) -> bool {
        // A `char` unchanged by the case mapping maps to exactly that `char`.
        match self {
            CaseMapping::ToLower => !ch.to_lowercase().eq([ch]),
            CaseMapping::ToUpper => !ch.to_uppercase().eq([ch]),
        }
    }
}

#[inline]
pub(crate) fn changes_ascii(bytes: &[u8], mapping: CaseMapping) -> bool {
    prefix_len_while(bytes, |b| !mapping.changes_ascii(b)) < bytes.len()
}

#[inline]
pub(crate) fn map_ascii(src: &[u8], dst: &mut [MaybeUninit<u8>], mapping: CaseMapping) {
    for (d, &b) in dst.iter_mut().zip(src) {
        d.write(mapping.map_ascii(b));
    }
}

#[inline]
pub(crate) fn map_ascii_in_place(bytes: &mut [u8], mapping: CaseMapping) {
    match mapping {
        CaseMapping::ToLower => bytes.make_ascii_lowercase(),
        CaseMapping::ToUpper => bytes.make_ascii_uppercase(),
    }
}

#[inline]
pub(crate) fn split_at_change(text: &str, mapping: CaseMapping) -> Option<(&str, &str)> {
    let unchanged_ascii =
        prefix_len_while(text.as_bytes(), |b| b.is_ascii() && !mapping.changes_ascii(b));
    let (i, _) = text[unchanged_ascii..].char_indices().find(|&(_, ch)| mapping.changes(ch))?;
    Some(text.split_at(unchanged_ascii + i))
}

#[inline]
pub(crate) fn map_while_ascii(
    src: &[u8],
    dst: &mut [MaybeUninit<u8>],
    mapping: CaseMapping,
) -> usize {
    let map_bytes_while_ascii = |src: &[u8], dst: &mut [MaybeUninit<u8>]| {
        let mut len = 0;
        for (d, &b) in dst.iter_mut().zip(src) {
            if !b.is_ascii() {
                break;
            }
            d.write(mapping.map_ascii(b));
            len += 1;
        }
        len
    };

    // Make them the same length, so that their chunks and remainders correspond.
    let len = src.len().min(dst.len());
    let (src, dst) = (&src[..len], &mut dst[..len]);

    let (src_chunks, src_remainder) = src.as_chunks::<CHUNK_SIZE>();
    let (dst_chunks, dst_remainder) = dst.as_chunks_mut::<CHUNK_SIZE>();
    let mut len = 0;
    for (src, dst) in src_chunks.iter().zip(dst_chunks) {
        if !all_bytes(src, |b| b.is_ascii()) {
            return len + map_bytes_while_ascii(src, dst);
        }
        for (d, &b) in dst.iter_mut().zip(src) {
            d.write(mapping.map_ascii(b));
        }
        len += CHUNK_SIZE;
    }
    len + map_bytes_while_ascii(src_remainder, dst_remainder)
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
