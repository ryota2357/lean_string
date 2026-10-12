# `Extend` / `FromIterator` / `AsRef<Path>` / `AsRef<OsStr>` の多くの impl にテストが無い

- 種別: test
- 対象: `src/lib.rs` の `impl Extend<_> for LeanString` / `for String`, `impl FromIterator<_> for LeanString` / `for LeanStr` / `for String`, `impl AsRef<Path>` / `AsRef<OsStr>` (両型)
- 影響: これらの impl を書き換えたとき (たとえば長さの見積もりや確保の回数を変える最適化) に、内容が正しいかを確かめるテストが無い。`FromIterator<LeanStr> for LeanStr` は `into_lean_string` → `extend` → `into_lean_str` と heap のレイアウト変換を 2 回通るのに、テストが無い
- 優先度: 低

## 問題

`5dd94ea` の `tests/` と doctest を grep した結果は次のとおり。

| impl | テスト |
| --- | --- |
| `Extend<char>` / `Extend<&char>` / `Extend<&str>` for `LeanString` | あり (`tests/lean_string.rs` の `extend_char`、`tests/alloc_string.rs` の `test_from_iterator` / `test_extend_ref`) |
| `Extend<LeanString> for LeanString` | `FromIterator<LeanString> for LeanString` のテスト経由のみ |
| `Extend<Box<str>>` / `Extend<Cow<str>>` / `Extend<String>` / `Extend<LeanStr>` for `LeanString` | 無し |
| `Extend<LeanString>` / `Extend<LeanStr>` for `String` | 無し |
| `FromIterator<char>` / `<&str>` / `<LeanString>` for `LeanString` | あり |
| `FromIterator<String> for LeanString` | `tests/property.rs` のみ (Miri では ignore) |
| `FromIterator<&char>` / `<Box<str>>` / `<Cow<str>>` / `<LeanStr>` for `LeanString` | 無し |
| `FromIterator<_> for LeanStr` (8 つ) | 無し |
| `FromIterator<LeanString>` / `<LeanStr>` for `String` | 無し |
| `AsRef<Path>` / `AsRef<OsStr>` (両型) | 無し (doc コメントも無い) |

## 修正例

以下のテストを `tests/lean_string.rs` (または新しいファイル) に足す。x86_64 で `cargo test`、Miri (`-Zmiri-strict-provenance`) の x86_64 / i686 / powerpc64 / powerpc で通ることを確認した。

```rust
use lean_string::{LeanStr, LeanString};
use std::borrow::Cow;

#[test]
fn extend_owned_and_borrowed_strings() {
    let mut s = LeanString::from("0");
    s.extend([Box::<str>::from("1"), Box::from("2")]);
    s.extend([Cow::Borrowed("3"), Cow::Owned(String::from("4"))]);
    s.extend([String::from("5"), String::from("6")]);
    s.extend([LeanStr::from("7"), LeanStr::from_static_str("8")]);
    s.extend([LeanString::from("9")]);
    s.extend(&['a', 'b']);
    assert_eq!(s, "0123456789ab");
}

#[test]
fn extend_string_with_lean_strings() {
    let mut s = String::from("0");
    s.extend([LeanString::from("1"), LeanString::from("a heap-allocated LeanString")]);
    s.extend([LeanStr::from("2"), LeanStr::from("a heap-allocated LeanStr!!")]);
    assert_eq!(s, "01a heap-allocated LeanString2a heap-allocated LeanStr!!");
}

#[test]
fn collect_into_lean_string_from_each_item_type() {
    let expected = "0123456789abcdefghij";
    let parts = ["0123", "4567", "89ab", "cdef", "ghij"];

    let s: LeanString = parts.iter().map(|&p| Box::<str>::from(p)).collect();
    assert_eq!(s, expected);
    let s: LeanString = parts.iter().map(|&p| Cow::Borrowed(p)).collect();
    assert_eq!(s, expected);
    let s: LeanString = parts.iter().map(|&p| String::from(p)).collect();
    assert_eq!(s, expected);
    let s: LeanString = parts.iter().map(|&p| LeanStr::from(p)).collect();
    assert_eq!(s, expected);
    let chars: Vec<char> = expected.chars().collect();
    let s: LeanString = chars.iter().collect();
    assert_eq!(s, expected);
}

#[test]
fn collect_into_lean_str_from_each_item_type() {
    let expected = "0123456789abcdefghij";
    let parts = ["0123", "4567", "89ab", "cdef", "ghij"];
    let chars: Vec<char> = expected.chars().collect();

    let collected: [LeanStr; 8] = [
        expected.chars().collect(),
        chars.iter().collect(),
        parts.iter().copied().collect(),
        parts.iter().map(|&p| Box::<str>::from(p)).collect(),
        parts.iter().map(|&p| Cow::Borrowed(p)).collect(),
        parts.iter().map(|&p| String::from(p)).collect(),
        parts.iter().map(|&p| LeanString::from(p)).collect(),
        parts.iter().map(|&p| LeanStr::from(p)).collect(),
    ];
    for s in collected {
        assert_eq!(s, expected);
        assert!(s.is_heap_allocated());
    }

    // Empty iterators give an empty inline string.
    let s: LeanStr = core::iter::empty::<LeanStr>().collect();
    assert!(s.is_empty() && !s.is_heap_allocated());
    let s: LeanStr = core::iter::empty::<char>().collect();
    assert!(s.is_empty() && !s.is_heap_allocated());
}

#[test]
fn collect_lean_str_with_shared_first_item() {
    let first = LeanStr::from("a heap-allocated LeanStr, longer than inline");
    let shared = first.clone();
    let s: LeanStr = [first, LeanStr::from("!")].into_iter().collect();
    assert_eq!(s, "a heap-allocated LeanStr, longer than inline!");
    assert_eq!(shared, "a heap-allocated LeanStr, longer than inline");
}

#[test]
fn collect_into_string_from_lean_strings() {
    let s: String = [LeanString::from("ab"), LeanString::from("a heap-allocated LeanString")]
        .into_iter()
        .collect();
    assert_eq!(s, "aba heap-allocated LeanString");
    let s: String = [LeanStr::from("ab"), LeanStr::from_static_str("a static LeanStr, long")]
        .into_iter()
        .collect();
    assert_eq!(s, "aba static LeanStr, long");
}

#[test]
#[cfg(feature = "std")]
fn as_ref_path_and_os_str() {
    use std::ffi::OsStr;
    use std::path::Path;

    fn path_of(p: &impl AsRef<Path>) -> &Path {
        p.as_ref()
    }
    fn os_str_of(p: &impl AsRef<OsStr>) -> &OsStr {
        p.as_ref()
    }

    let string = LeanString::from("dir/a heap-allocated file.txt");
    let str_ = LeanStr::from("dir/a heap-allocated file.txt");
    for (path, os_str) in
        [(path_of(&string), os_str_of(&string)), (path_of(&str_), os_str_of(&str_))]
    {
        assert_eq!(path, Path::new("dir/a heap-allocated file.txt"));
        assert_eq!(path.extension(), Some(OsStr::new("txt")));
        assert_eq!(os_str, OsStr::new("dir/a heap-allocated file.txt"));
    }
}
```

`AsRef<Path>` / `AsRef<OsStr>` は `std` feature のときだけある impl なので、`as_ref_path_and_os_str` には `#[cfg(feature = "std")]` が要る (付けないと CI の `cargo test --no-default-features` がコンパイルエラーになることを確認した)。

## 確認方法

上のテストが通ること。
