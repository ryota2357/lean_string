// from: https://github.com/ParkMyCar/compact_str/blob/193d13eaa5a92b3c39c2f7289dc44c95f37c80d1/compact_str/src/features/arbitrary.rs
#![cfg(feature = "arbitrary")]

use arbitrary::{Arbitrary, Unstructured};
use lean_string::{LeanStr, LeanString};

#[test]
fn arbitrary_sanity() {
    let mut data = Unstructured::new(&[42; 50]);
    let lean_string = LeanString::arbitrary(&mut data).expect("generate a LeanString");
    let mut data = Unstructured::new(&[42; 50]);
    let lean_str = LeanStr::arbitrary(&mut data).expect("generate a LeanStr");

    // we don't really care what the content of the string is, just that one's generated
    assert!(!lean_string.is_empty());
    assert!(!lean_str.is_empty());
}

#[test]
fn arbitrary_inlines_strings() {
    let mut data = Unstructured::new(&[42; 8]);
    let lean_string = LeanString::arbitrary(&mut data).expect("generate a LeanString");
    let mut data = Unstructured::new(&[42; 8]);
    let lean_str = LeanStr::arbitrary(&mut data).expect("generate a LeanStr");

    // running this manually, we generate the string "**"
    assert!(!lean_string.is_heap_allocated());
    assert!(!lean_str.is_heap_allocated());
}
