//! Shared fixtures for benchmarks.

const ECOSTRING_INLINE_MAX: usize = 15;
const LEAN_STRING_INLINE_MAX: usize = 16;
const COMPACT_STRING_INLINE_MAX: usize = 24;

const LENS: &[usize] = &[
    0,
    1,
    ECOSTRING_INLINE_MAX,
    LEAN_STRING_INLINE_MAX,
    LEAN_STRING_INLINE_MAX + 1,
    COMPACT_STRING_INLINE_MAX,
    COMPACT_STRING_INLINE_MAX + 1,
    256,
];

pub const STATIC_STR_16: &str = "0123456789abcdef";
pub const STATIC_STR_40: &str = "0123456789abcdef0123456789abcdef01234567";

pub fn ascii(n: usize) -> String {
    let mut state = 0x2545_F491_4F6C_DD1D ^ (n as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    if state == 0 {
        state = 1;
    }
    let mut out = String::with_capacity(n);
    while out.len() < n {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        for byte in state.to_le_bytes() {
            if out.len() == n {
                break;
            }
            out.push((b'!' + byte % (b'~' - b'!' + 1)) as char);
        }
    }
    out
}

pub fn differ_at_last(s: &str) -> String {
    assert!(s.is_ascii());
    let mut out = s.to_string();
    if let Some(last) = out.pop() {
        out.push(if last == '~' { '!' } else { (last as u8 + 1) as char });
    }
    out
}

pub fn samples() -> Vec<String> {
    LENS.iter().map(|&n| ascii(n)).collect()
}
