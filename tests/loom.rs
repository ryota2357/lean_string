// RUSTFLAGS="--cfg loom" cargo test --test loom --release --features loom -- --test-threads=1
#![cfg(loom)]

use lean_string::{LeanStr, LeanString};
use loom::thread;

#[test]
fn concurrent_push() {
    loom::model(|| {
        let mut one = LeanString::from("12345678901234567890");
        let two = one.clone();

        thread::spawn(move || {
            let mut three = two.clone();
            three.push('a');
            assert_eq!(two, "12345678901234567890");
            assert_eq!(three, "12345678901234567890a");
        });

        one.push('a');
        assert_eq!(one, "12345678901234567890a");
    });
}

#[test]
fn concurrent_remove() {
    loom::model(|| {
        let mut one = LeanString::from("abcdefghijklmnopqrstuvwxyz");
        let two = one.clone();

        thread::spawn(move || {
            let mut three = two.clone();
            assert_eq!(three.remove(3), 'd');
            assert_eq!(two, "abcdefghijklmnopqrstuvwxyz");
            assert_eq!(three, "abcefghijklmnopqrstuvwxyz");
        });

        assert_eq!(one.remove(3), 'd');
        assert_eq!(one, "abcefghijklmnopqrstuvwxyz");
    });
}

#[test]
fn concurrent_lean_str_clone_and_to_lean_string() {
    loom::model(|| {
        let one = LeanStr::from("abcdefghijklmnopqrstuvwxyz");
        let two = one.clone();

        thread::spawn(move || {
            let three = two.clone();
            assert_eq!(three, "abcdefghijklmnopqrstuvwxyz");
        });

        let mut one_p = one.into_lean_string();
        one_p.push('!');
        assert_eq!(one_p, "abcdefghijklmnopqrstuvwxyz!");
    });
}

#[test]
fn concurrent_lean_string_clone_and_to_lean_str() {
    loom::model(|| {
        let one = LeanString::from("abcdefghijklmnopqrstuvwxyz");
        let two = one.clone();

        thread::spawn(move || {
            let three = two.clone();
            assert_eq!(three, "abcdefghijklmnopqrstuvwxyz");
        });

        let one_p = one.into_lean_str();
        assert_eq!(one_p, "abcdefghijklmnopqrstuvwxyz");
    });
}
