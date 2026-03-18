use lean_string::LeanString;

static S: LeanString = LeanString::from_static_str("hello world");

const STR: &str = S.as_str();

#[test]
fn use_const() {
    assert_eq!(STR, "hello world");
}

#[test]
fn const_len() {
    const {
        let s = LeanString::from_static_str("hello");
        assert!(s.len() == 5);
        s
    };
}
