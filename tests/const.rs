use lean_string::{LeanStr, LeanString};

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
