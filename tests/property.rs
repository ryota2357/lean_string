use lean_string::{LeanStr, LeanString, ToLeanStr, ToLeanString};
use proptest::{prelude::*, property_test};

#[property_test]
#[cfg_attr(miri, ignore)]
fn create_from_str(input: String) {
    let input = input.as_str();

    let s1 = LeanString::from(input);
    prop_assert_eq!(&s1, input);
    prop_assert_eq!(s1.len(), input.len());

    let s2 = LeanStr::from(input);
    prop_assert_eq!(&s2, input);
    prop_assert_eq!(s2.len(), input.len());

    if input.len() <= 2 * size_of::<usize>() {
        prop_assert!(!s1.is_heap_allocated());
        prop_assert!(!s2.is_heap_allocated());
    } else {
        prop_assert!(s1.is_heap_allocated());
        prop_assert!(s2.is_heap_allocated());
    }
}

#[property_test]
#[cfg_attr(miri, ignore)]
fn create_from_char(input: char) {
    let expected = String::from(input);
    assert_eq!(LeanString::from(input), expected);
    assert_eq!(LeanStr::from(input), expected);
}

#[property_test]
#[cfg_attr(miri, ignore)]
fn create_from_utf8_bytes(input: Vec<u8>) {
    let input = input.as_slice();

    let s1 = LeanString::from_utf8(input);
    let s2 = LeanStr::from_utf8(input);
    let string = String::from_utf8(input.to_vec());
    prop_assert_eq!(s1.is_err(), string.is_err());
    prop_assert_eq!(s2.is_err(), string.is_err());
    if let (Ok(s1), Ok(string)) = (s1, &string) {
        prop_assert_eq!(&s1, string);
    }
    if let (Ok(s2), Ok(string)) = (s2, &string) {
        prop_assert_eq!(&s2, string);
    }

    let s1 = LeanString::from_utf8_lossy(input);
    let s2 = LeanStr::from_utf8_lossy(input);
    let string = String::from_utf8_lossy(input);
    prop_assert_eq!(&s1, &string);
    prop_assert_eq!(&s2, &string);
}

#[property_test]
#[cfg_attr(miri, ignore)]
fn create_from_utf16_bytes(input: Vec<u16>) {
    let input = input.as_slice();

    let s1 = LeanString::from_utf16(input);
    let s2 = LeanStr::from_utf16(input);
    let string = String::from_utf16(input);
    prop_assert_eq!(s1.is_err(), string.is_err());
    prop_assert_eq!(s2.is_err(), string.is_err());
    if let (Ok(lean), Ok(string)) = (s1, &string) {
        prop_assert_eq!(&lean, string);
    }
    if let (Ok(lean), Ok(string)) = (s2, &string) {
        prop_assert_eq!(&lean, string);
    }

    let s1 = LeanString::from_utf16_lossy(input);
    let s2 = LeanStr::from_utf16_lossy(input);
    let string = String::from_utf16_lossy(input);
    prop_assert_eq!(&s1, &string);
    prop_assert_eq!(&s2, &string);
}

#[property_test]
#[cfg_attr(miri, ignore)]
fn create_from_utf16le_bytes(input: Vec<u8>) {
    let input = input.as_slice();

    let s1 = LeanString::from_utf16le(input);
    let s2 = LeanStr::from_utf16le(input);
    let string = String::from_utf16le(input);
    prop_assert_eq!(s1.is_err(), string.is_err());
    prop_assert_eq!(s2.is_err(), string.is_err());
    if let (Ok(lean), Ok(string)) = (s1, &string) {
        prop_assert_eq!(&lean, string);
    }
    if let (Ok(lean), Ok(string)) = (s2, &string) {
        prop_assert_eq!(&lean, string);
    }

    let s1 = LeanString::from_utf16le_lossy(input);
    let s2 = LeanStr::from_utf16le_lossy(input);
    let string = String::from_utf16le_lossy(input);
    prop_assert_eq!(&s1, &string);
    prop_assert_eq!(&s2, &string);
}

#[property_test]
#[cfg_attr(miri, ignore)]
fn create_from_utf16be_bytes(input: Vec<u8>) {
    let input = input.as_slice();

    let s1 = LeanString::from_utf16be(input);
    let s2 = LeanStr::from_utf16be(input);
    let string = String::from_utf16be(input);
    prop_assert_eq!(s1.is_err(), string.is_err());
    prop_assert_eq!(s2.is_err(), string.is_err());
    if let (Ok(lean), Ok(string)) = (s1, &string) {
        prop_assert_eq!(&lean, string);
    }
    if let (Ok(lean), Ok(string)) = (s2, &string) {
        prop_assert_eq!(&lean, string);
    }

    let s1 = LeanString::from_utf16be_lossy(input);
    let s2 = LeanStr::from_utf16be_lossy(input);
    let string = String::from_utf16be_lossy(input);
    prop_assert_eq!(&s1, &string);
    prop_assert_eq!(&s2, &string);
}

#[property_test]
#[cfg_attr(miri, ignore)]
fn collect_from_chars(input: String) {
    let lean = input.chars().collect::<LeanString>();
    prop_assert_eq!(&lean, &input);
}

#[property_test]
#[cfg_attr(miri, ignore)]
fn collect_from_strings(input: Vec<String>) {
    let lean = input.clone().into_iter().collect::<LeanString>();
    let string = input.into_iter().collect::<String>();
    prop_assert_eq!(&lean, &string);
}

macro_rules! test_integer_to_lean_s {
    ($($ty:ty),* $(,)?) => {$(
        paste::paste! {
            #[test]
            fn [<$ty _to_lean_string>]() {
                for num in <$ty>::MIN..=<$ty>::MAX {
                    let lean = num.to_lean_string();
                    let string = num.to_string();
                    assert_eq!(lean, string);
                }
            }
            #[test]
            fn [<$ty _to_lean_str>]() {
                for num in <$ty>::MIN..=<$ty>::MAX {
                    let lean = num.to_lean_str();
                    let string = num.to_string();
                    assert_eq!(lean, string);
                }
            }
            #[test]
            fn [<nonzero_ $ty _to_lean_string>]() {
                for num in <$ty>::MIN..=<$ty>::MAX {
                    if num == 0 { continue };
                    let num = core::num::NonZero::<$ty>::new(num).unwrap();
                    let lean = num.to_lean_string();
                    let string = num.to_string();
                    assert_eq!(lean, string);
                }
            }
            #[test]
            fn [<nonzero_ $ty _to_lean_str>]() {
                for num in <$ty>::MIN..=<$ty>::MAX {
                    if num == 0 { continue };
                    let num = core::num::NonZero::<$ty>::new(num).unwrap();
                    let lean = num.to_lean_str();
                    let string = num.to_string();
                    assert_eq!(lean, string);
                }
            }
        }
    )*};
}
test_integer_to_lean_s!(u8, i8);

macro_rules! prop_test_integer_to_lean_s {
    ($($ty:ty),* $(,)?) => {$(
        paste::paste! {
            #[property_test]
            #[cfg_attr(miri, ignore)]
            fn [<$ty _to_lean_string>](i: $ty) {
                prop_assert_eq!(i.to_lean_string(), i.to_string());
            }
            #[property_test]
            #[cfg_attr(miri, ignore)]
            fn [<$ty _to_lean_str>](i: $ty) {
                prop_assert_eq!(i.to_lean_str(), i.to_string());
            }
            #[property_test]
            #[cfg_attr(miri, ignore)]
            fn [<nonzero_ $ty _to_lean_string>](i: core::num::NonZero<$ty>) {
                prop_assert_eq!(i.to_lean_string(), i.to_string());
                prop_assert_eq!(i.to_lean_str(), i.to_string());
            }
            #[property_test]
            #[cfg_attr(miri, ignore)]
            fn [<nonzero_ $ty _to_lean_str>](i: core::num::NonZero<$ty>) {
                prop_assert_eq!(i.to_lean_str(), i.to_string());
            }
        }
    )*};
}
prop_test_integer_to_lean_s!(u16, i16, u32, i32, u64, i64, u128, i128, usize, isize);

#[property_test]
#[cfg_attr(miri, ignore)]
fn f32_to_lean_string(f: f32) {
    let lean = f.to_lean_string();
    let float = lean.parse::<f32>().unwrap();
    prop_assert_eq!(f, float);
}

#[property_test]
#[cfg_attr(miri, ignore)]
fn f32_to_lean_str(f: f32) {
    let lean = f.to_lean_str();
    let float = lean.parse::<f32>().unwrap();
    prop_assert_eq!(f, float);
}

#[property_test]
#[cfg_attr(miri, ignore)]
fn f64_to_lean_string(f: f64) {
    let lean = f.to_lean_string();
    let float = lean.parse::<f64>().unwrap();
    prop_assert_eq!(f, float);
}

#[property_test]
#[cfg_attr(miri, ignore)]
fn f64_to_lean_str(f: f64) {
    let lean = f.to_lean_str();
    let float = lean.parse::<f64>().unwrap();
    prop_assert_eq!(f, float);
}

#[test]
fn bool_to_lean_string() {
    let t = true;
    let f = false;
    assert_eq!(t.to_lean_string(), t.to_string());
    assert_eq!(f.to_lean_string(), f.to_string());
}

#[test]
fn bool_to_lean_str() {
    let t = true;
    let f = false;
    assert_eq!(t.to_lean_str(), t.to_string());
    assert_eq!(f.to_lean_str(), f.to_string());
}

#[property_test]
#[cfg_attr(miri, ignore)]
fn char_to_lean_string(c: char) {
    prop_assert_eq!(c.to_lean_string(), c.to_string());
}

#[property_test]
#[cfg_attr(miri, ignore)]
fn char_to_lean_str(c: char) {
    prop_assert_eq!(c.to_lean_str(), c.to_string());
}

#[property_test]
#[cfg_attr(miri, ignore)]
fn string_to_lean_string(s: String) {
    prop_assert_eq!(s.to_lean_string(), s);
}

#[property_test]
#[cfg_attr(miri, ignore)]
fn string_to_lean_str(s: String) {
    prop_assert_eq!(s.to_lean_str(), s);
}

#[property_test]
#[cfg_attr(miri, ignore)]
fn repeat(#[strategy = ".{0,1000}"] input: String, #[strategy = 0..1000usize] n: usize) {
    let expected = input.repeat(n);
    prop_assert_eq!(LeanString::from(input.as_str()).repeat(n), expected.as_str());
    prop_assert_eq!(LeanStr::from(input.as_str()).repeat(n), expected.as_str());
}

// Arbitrary `String`s are mostly non-ASCII, so they rarely contain long ASCII runs, which case
// conversion handles on a separate path. Also mix ASCII letters with non-ASCII chars whose case
// mapping is special: Σ depends on its context, and the others change the length.
fn case_conversion_input() -> impl Strategy<Value = String> {
    prop_oneof![any::<String>(), "([a-zA-Z0-9 ']{0,20}[éÉΣσİıßﬁ\u{212A}]?){0,8}"]
}

#[property_test]
#[cfg_attr(miri, ignore)]
fn to_lowercase(#[strategy = case_conversion_input()] input: String) {
    prop_assert_eq!(LeanString::from(input.as_str()).to_lowercase(), input.to_lowercase());
}

#[property_test]
#[cfg_attr(miri, ignore)]
fn to_uppercase(#[strategy = case_conversion_input()] input: String) {
    prop_assert_eq!(LeanString::from(input.as_str()).to_uppercase(), input.to_uppercase());
}

#[property_test]
#[cfg_attr(miri, ignore)]
fn to_ascii_lowercase(#[strategy = case_conversion_input()] input: String) {
    let expected = input.to_ascii_lowercase();
    prop_assert_eq!(LeanString::from(input.as_str()).to_ascii_lowercase(), expected.as_str());
    prop_assert_eq!(LeanStr::from(input.as_str()).to_ascii_lowercase(), expected.as_str());
}

#[property_test]
#[cfg_attr(miri, ignore)]
fn to_ascii_uppercase(#[strategy = case_conversion_input()] input: String) {
    let expected = input.to_ascii_uppercase();
    prop_assert_eq!(LeanString::from(input.as_str()).to_ascii_uppercase(), expected.as_str());
    prop_assert_eq!(LeanStr::from(input.as_str()).to_ascii_uppercase(), expected.as_str());
}

#[property_test]
#[cfg_attr(miri, ignore)]
fn make_ascii_lowercase(#[strategy = case_conversion_input()] input: String) {
    let mut s = LeanString::from(input.as_str());
    let cloned = s.clone();
    s.make_ascii_lowercase();
    prop_assert_eq!(s, input.to_ascii_lowercase());
    prop_assert_eq!(cloned, input);
}

#[property_test]
#[cfg_attr(miri, ignore)]
fn make_ascii_uppercase(#[strategy = case_conversion_input()] input: String) {
    let mut s = LeanString::from(input.as_str());
    let cloned = s.clone();
    s.make_ascii_uppercase();
    prop_assert_eq!(s, input.to_ascii_uppercase());
    prop_assert_eq!(cloned, input);
}
