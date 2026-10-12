# 比較・順序・ハッシュのテストが無く、`!=` と `Ord` は一度も検証されていない

- 種別: test
- 対象: `src/lib.rs` の `PartialEq` / `Ord` / `PartialOrd` / `Hash` / `Borrow<str>` の impl (`LeanString` と `LeanStr`)
- 影響: 比較の実装を変えたとき ([#11](./11-short-eq-bcmp.md)、[#10](./10-leanstr-word-eq.md)、[#19](./19-shared-buffer-ptr-eq.md)) に、誤りを検出できない
- 優先度: 中

## 問題

`5dd94ea` の `tests/` と doctest を調べた結果は次のとおり。

- `HashMap` / `HashSet` / `BTreeMap` / `BTreeSet` を使うテストが 1 つも無い。`Borrow<str>` と `Hash` / `Eq` / `Ord` が整合していて、
  `&str` でキーを引けることを確かめるテストが無い。
- `.cmp(` / `partial_cmp` / `<` / `.sort()` を使うテストが無い。`Ord` / `PartialOrd` は両方の型とも検証されていない。
- `assert_ne!` はすべてポインタ (`as_ptr()`) の比較で、`LeanString` / `LeanStr` の値どうしが「等しくない」ことを確かめるテストが無い。
  `==` が常に `true` を返しても、今のテストは通る。
- `tests/lean_string.rs` の `truncate_share_buffer` は「ポインタが同じで長さが違う」2 つのハンドルを作る
  (共有された heap を `truncate` すると、ハンドル側の長さだけが縮む)。しかし、その 2 つが等しくないことは確かめていない。
  ポインタで近道する比較を入れると、まさにここで間違えやすい。
- 縮めた inline の値 (`InlineBuffer::set_len` が最後のバイトしか書き換えないので、使っていないバイトに古い内容が残る) と、
  新しく作った値との比較・ハッシュも検証されていない。

今の実装はすべて `as_str()` に委ねているので、正しい。下のテストは `5dd94ea` で全部通ることを確認した。
`property_test` 以外は Miri (`-Zmiri-strict-provenance`、x86_64) でも通る。

## 修正例

`tests/eq_ord_hash.rs` として追加する。property test は既存の `tests/property.rs` と同じく、Miri では無視する。

```rust
use lean_string::{LeanStr, LeanString};
use proptest::{prelude::*, property_test};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{BuildHasher, RandomState};
use std::ops::Bound;

const INLINE_LIMIT: usize = size_of::<LeanString>();
const LONG: &str = "abcdefghijklmnopqrstuvwxyz";

/// The same text in different representations.
fn variants(text: &'static str) -> Vec<LeanString> {
    let mut out = vec![LeanString::from(text), LeanString::from_static_str(text)];
    // heap, truncated (the length is shorter than the buffer)
    let mut s = LeanString::from(format!("{text}{}", "x".repeat(INLINE_LIMIT)).as_str());
    s.truncate(text.len());
    out.push(s);
    // inline (if short enough), truncated: stale bytes remain after the new length
    let mut s = LeanString::from(format!("{text}xyz").as_str());
    s.truncate(text.len());
    out.push(s);
    // heap with spare capacity
    let mut s = LeanString::with_capacity(100);
    s.push_str(text);
    out.push(s);
    out
}

#[test]
fn eq_hash_across_representations() {
    let state = RandomState::new();
    for text in ["", "a", "abc", "abcdefgh", "abcdefghijklmno", "abcdefghijklmnop", LONG] {
        let all = variants(text);
        for a in &all {
            for b in &all {
                assert_eq!(a, b);
                assert_eq!(a.cmp(b), Ordering::Equal);
                assert_eq!(state.hash_one(a), state.hash_one(b));
            }
            assert_eq!(state.hash_one(a), state.hash_one(text));
            let s = a.clone().into_lean_str();
            assert_eq!(s, LeanStr::from(text));
            assert_eq!(state.hash_one(&s), state.hash_one(text));
        }
    }
}

#[test]
fn shared_buffer_with_different_lengths() {
    for base in [LeanString::from(LONG), LeanString::from_static_str(LONG)] {
        let mut short = base.clone();
        short.truncate(20);
        // same pointer, different lengths
        assert_eq!(base.as_ptr(), short.as_ptr());
        assert_ne!(base, short);
        assert_ne!(short, base);
        assert_eq!(short.cmp(&base), Ordering::Less);
        assert_eq!(base.cmp(&short), Ordering::Greater);
        assert!(short < base);
    }
}

#[test]
fn hash_map_lookup_by_str() {
    let keys = ["", "short", "abcdefghijklmnop", LONG];
    let mut string_map = HashMap::new();
    let mut str_map = HashMap::new();
    for (i, k) in keys.iter().enumerate() {
        string_map.insert(LeanString::from(*k), i);
        str_map.insert(LeanStr::from(*k), i);
    }
    string_map.insert(LeanString::from_static_str("static key longer than inline"), 9);
    str_map.insert(LeanStr::from_static_str("static key longer than inline"), 9);
    for (i, k) in keys.iter().enumerate() {
        assert_eq!(string_map.get(*k), Some(&i));
        assert_eq!(str_map.get(*k), Some(&i));
    }
    assert_eq!(string_map.get("static key longer than inline"), Some(&9));
    assert_eq!(str_map.get("static key longer than inline"), Some(&9));
    assert_eq!(string_map.get("missing"), None);

    let set: HashSet<LeanStr> = keys.iter().map(|k| LeanStr::from(*k)).collect();
    assert!(set.contains("short"));
    assert!(!set.contains("shor"));
}

#[test]
fn btree_map_order_and_lookup_by_str() {
    let words = ["pear", "", "apple", "apple pie with cream", "a", "zzzzzzzzzzzzzzzzz", "\u{e9}"];
    let map: BTreeMap<LeanString, usize> = words.iter().map(|w| (LeanString::from(*w), w.len())).collect();
    let mut sorted = words.to_vec();
    sorted.sort();
    assert!(map.keys().map(LeanString::as_str).eq(sorted.iter().copied()));
    assert_eq!(map.get("apple"), Some(&5));
    assert_eq!(map.range::<str, _>((Bound::Included("apple"), Bound::Excluded("b"))).count(), 2);

    let set: BTreeSet<LeanStr> = words.iter().map(|w| LeanStr::from(*w)).collect();
    assert!(set.iter().map(LeanStr::as_str).eq(sorted.iter().copied()));
}

#[property_test]
#[cfg_attr(miri, ignore)]
fn ord_and_eq_match_str(a: String, b: String) {
    for (x, y) in [
        (LeanString::from(a.as_str()), LeanString::from(b.as_str())),
        (LeanString::from(a.as_str()), LeanString::from(a.as_str())),
    ] {
        let (xs, ys) = (x.as_str().to_owned(), y.as_str().to_owned());
        prop_assert_eq!(x == y, xs == ys);
        prop_assert_eq!(x.cmp(&y), xs.cmp(&ys));
        prop_assert_eq!(x.partial_cmp(&y), xs.partial_cmp(&ys));
        let (x, y) = (x.into_lean_str(), y.into_lean_str());
        prop_assert_eq!(x == y, xs == ys);
        prop_assert_eq!(x.cmp(&y), xs.cmp(&ys));
    }
}

#[property_test]
#[cfg_attr(miri, ignore)]
fn sort_matches_string(mut input: Vec<String>) {
    let mut lean: Vec<LeanStr> = input.iter().map(|s| LeanStr::from(s.as_str())).collect();
    input.sort();
    lean.sort();
    prop_assert!(lean.iter().map(LeanStr::as_str).eq(input.iter().map(String::as_str)));
}

#[property_test]
#[cfg_attr(miri, ignore)]
fn eq_after_shrink(a: String, extra: String) {
    let state = RandomState::new();
    let fresh = LeanString::from(a.as_str());
    let mut shrunk = LeanString::from(format!("{a}{extra}").as_str());
    shrunk.truncate(a.len());
    prop_assert_eq!(&shrunk, &fresh);
    prop_assert_eq!(shrunk.cmp(&fresh), Ordering::Equal);
    prop_assert_eq!(state.hash_one(&shrunk), state.hash_one(&fresh));
    let (shrunk, fresh) = (shrunk.into_lean_str(), LeanStr::from(a.as_str()));
    prop_assert_eq!(&shrunk, &fresh);
    prop_assert_eq!(shrunk.cmp(&fresh), Ordering::Equal);
    prop_assert_eq!(state.hash_one(&shrunk), state.hash_one(&fresh));
}
```

## 確認方法

- `cargo test --test eq_ord_hash` と、`MIRIFLAGS=-Zmiri-strict-provenance cargo +nightly miri test --test eq_ord_hash` (CI の各ターゲット)。
- 比較の実装を変える変更では、このテストが通ることを条件にする。
