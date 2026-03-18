// RUSTFLAGS="--cfg loom" cargo test --test loom --release --features loom -- --test-threads=1
#![cfg(loom)]

use lean_string::LeanString;
use loom::thread;

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

macro_rules! loom_test {
    (fn $name:ident() { $($tt:tt)* }) => {
        #[test]
        fn $name() {
            loom::model(|| {
                let _profiler = dhat::Profiler::builder().testing().build();
                {
                    $($tt)*
                }
                let stats = dhat::HeapStats::get();
                // https://github.com/tokio-rs/loom/issues/369
                dhat::assert_eq!(stats.curr_blocks, 1);
            });
        }
    };
}

loom_test! {
    fn concurrent_push() {
        let mut one = LeanString::from("12345678901234567890");
        let two = one.clone();

        let th = thread::spawn(move || {
            let mut three = two.clone();
            three.push('a');
            assert_eq!(two, "12345678901234567890");
            assert_eq!(three, "12345678901234567890a");
        });

        one.push('a');
        assert_eq!(one, "12345678901234567890a");

        th.join().unwrap();
    }
}

loom_test! {
    fn concurrent_remove() {
        let mut one = LeanString::from("abcdefghijklmnopqrstuvwxyz");
        let two = one.clone();

        let th = thread::spawn(move || {
            let mut three = two.clone();
            assert_eq!(three.remove(3), 'd');
            assert_eq!(two, "abcdefghijklmnopqrstuvwxyz");
            assert_eq!(three, "abcefghijklmnopqrstuvwxyz");
        });

        assert_eq!(one.remove(3), 'd');
        assert_eq!(one, "abcefghijklmnopqrstuvwxyz");

        th.join().unwrap();
    }
}
