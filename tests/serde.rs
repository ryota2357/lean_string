// from: https://github.com/ParkMyCar/compact_str/blob/193d13eaa5a92b3c39c2f7289dc44c95f37c80d1/compact_str/src/features/serde.rs
#![cfg(feature = "serde")]

use lean_string::{LeanStr, LeanString};
use proptest::property_test;
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
struct PersonString {
    name: String,
    phones: Vec<String>,
    address: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
struct PersonLeanString {
    name: LeanString,
    phones: Vec<LeanString>,
    address: Option<LeanString>,
}

#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
struct PersonLeanStr {
    name: LeanStr,
    phones: Vec<LeanStr>,
    address: Option<LeanStr>,
}

#[test]
fn test_roundtrip() {
    let name = "Ferris the Crab";
    let phones = ["1-800-111-1111", "2-222-222-2222"];
    let address = Some("123 Sesame Street");

    let std = PersonString {
        name: name.to_string(),
        phones: phones.iter().map(|s| s.to_string()).collect(),
        address: address.as_ref().map(|s| s.to_string()),
    };
    let lean_string = PersonLeanString {
        name: name.into(),
        phones: phones.iter().map(|s| LeanString::from(*s)).collect(),
        address: address.as_ref().map(|s| LeanString::from(*s)),
    };
    let lean_str = PersonLeanStr {
        name: LeanStr::from(name),
        phones: phones.iter().map(|s| LeanStr::from(*s)).collect(),
        address: address.as_ref().map(|s| LeanStr::from(*s)),
    };

    let std_json = serde_json::to_string(&std).unwrap();
    let lean_string_json = serde_json::to_string(&lean_string).unwrap();
    let lean_str_json = serde_json::to_string(&lean_str).unwrap();

    // the serialized forms should be the same
    assert_eq!(std_json, lean_string_json);
    assert_eq!(std_json, lean_str_json);

    let std_de_compact: PersonString = serde_json::from_str(&lean_string_json).unwrap();
    let lean_string_de_std: PersonLeanString = serde_json::from_str(&std_json).unwrap();
    let lean_str_de_std: PersonLeanStr = serde_json::from_str(&std_json).unwrap();

    // we should be able to deserailze from the opposite, serialized, source
    assert_eq!(std_de_compact, std);
    assert_eq!(lean_string_de_std, lean_string);
    assert_eq!(lean_str_de_std, lean_str);
}

#[property_test]
#[cfg_attr(miri, ignore)]
fn proptest_roundtrip(name: String, phones: Vec<String>, address: Option<String>) {
    let std =
        PersonString { name: name.clone(), phones: phones.to_vec(), address: address.clone() };
    let lean_string = PersonLeanString {
        name: name.clone().into(),
        phones: phones.iter().map(LeanString::from).collect(),
        address: address.clone().map(LeanString::from),
    };
    let lean_str = PersonLeanStr {
        name: name.into(),
        phones: phones.iter().map(LeanStr::from).collect(),
        address: address.map(LeanStr::from),
    };

    let std_json = serde_json::to_string(&std).unwrap();
    let lean_string_json = serde_json::to_string(&lean_string).unwrap();
    let lean_str_json = serde_json::to_string(&lean_str).unwrap();

    // the serialized forms should be the same
    assert_eq!(std_json, lean_string_json);
    assert_eq!(std_json, lean_str_json);

    let std_de_lean_string: PersonString = serde_json::from_str(&lean_string_json).unwrap();
    let lean_string_de_std: PersonLeanString = serde_json::from_str(&std_json).unwrap();
    let lean_str_de_std: PersonLeanStr = serde_json::from_str(&std_json).unwrap();

    // we should be able to deserialize from the opposite serialized source
    assert_eq!(std_de_lean_string, std);
    assert_eq!(lean_string_de_std, lean_string);
    assert_eq!(lean_str_de_std, lean_str);
}
