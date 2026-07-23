use lean_string::{LeanStr, LeanString};

const INLINE_LIMIT: usize = size_of::<LeanString>();

static LEAN_STRING: LeanString = LeanString::from_static_str("hello world");
static LEAN_STR: LeanStr = LeanStr::from_static_str("hello world");

#[test]
fn use_const() {
    const {
        const _: &str = LEAN_STRING.as_str();
        const _: &str = LEAN_STR.as_str();
    }
    assert_eq!(LEAN_STRING.as_str(), "hello world");
    assert_eq!(LEAN_STR.as_str(), "hello world");
}

#[test]
fn const_len() {
    const {
        let s = LeanString::from_static_str("hello");
        assert!(s.len() == 5);
        s
    };
}

#[test]
fn from_every_inline_length_in_const() {
    const {
        let s = "0123456789abcdefg";

        let mut len = 0;
        while len <= INLINE_LIMIT {
            let expected = s.split_at(len).0;
            let inline = LeanString::from_static_str(expected);
            assert!(inline.len() == len);

            let bytes = inline.as_str().as_bytes();
            let mut i = 0;
            while i < len {
                assert!(bytes[i] == expected.as_bytes()[i]);
                i += 1;
            }

            // `LeanString` has drop glue, which is not allowed to run in a const block.
            core::mem::forget(inline);
            len += 1;
        }
    }
}
