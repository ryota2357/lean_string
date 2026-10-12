# 32-bit で長さを heap に置く経路の多くが、中身のある文字列でテストされていない

- 種別: test
- 対象: `src/repr/heap_buffer.rs` の `HeapBuffer::realloc` (レイアウトの切り替え), `HeapBuffer::set_len` / `len` (`write_len_on_heap` / `len_on_heap`), `HeapBuffer::into_exact`, `src/repr.rs` の `Repr::truncate_unchecked`
- 影響: 32-bit (i686, powerpc, armv7, wasm32 など) で 2^24 - 2 バイトを超える文字列を扱う経路。このクレートでいちばん込み入ったレイアウト (長さの prefix | ヘッダ | データ) の切り替えで、壊れると確保と解放の `Layout` がずれる UB になる。CI の Miri (i686 / powerpc) で走るテストが少ない
- 優先度: 中

## 問題

32-bit では `TextLen` に入る長さが 2^24 - 2 まで (`MAX_LEN`) で、それを超えると長さはヘッダの前の prefix に置かれる。容量が `MAX_LEN` を超える growable バッファは prefix の場所を持つ。この切り替えに関係する分岐と、`5dd94ea` の 32-bit 向けテスト (`tests/lean_string.rs` / `tests/conversion.rs` の `MIN_CAPACITY_FOR_HEAP_LENGTH_LAYOUT_ON_32BIT` を使うもの、`heap_buffer.rs` の単体テスト) の対応。

| 分岐 | 既存のテスト |
| --- | --- |
| `HeapBuffer::realloc` の `(false, true)`: prefix の無い確保から有る確保へ、中身をコピー | 無し。`reserve_on_heap` の 32-bit 部分は、inline からの確保 (`from_str_with_capacity`) と、容量 2^24 - 1 から 2^24 への `(true, true)` の `realloc` しか通らない |
| `HeapBuffer::realloc` の `(true, false)`: prefix の有る確保から無い確保へ | 無し |
| `HeapBuffer::set_len` で長さが `MAX_LEN` をまたいで prefix に書かれる (`write_len_on_heap`) | 無し (`LeanString::from` の 2^24 - 1 バイトは `new_uninit` が prefix を書く) |
| `Repr::truncate_unchecked` の、長さが heap にあって共有されている場合 (コピーする) / unique な場合 (その場で書き換える) | 無し |
| `HeapBuffer::into_exact` で古い確保にも新しい確保にも prefix がある場合 | `length_stored_heap_string_to_str` (容量 = 長さなので `realloc` は同じサイズ) のみ。余分な容量を持つ場合は無い |

i686 / powerpc では 16 MiB の文字列でも Miri が実用的な時間で走る (下のテスト 3 つで `cargo miri test` の実行部分は数秒)。

## 修正例

足すテスト。Miri (`-Zmiri-strict-provenance`) の i686 と powerpc で通ることを確認した (64-bit では `cfg` で外れる)。i686 のネイティブ実行は試していない。

```rust
#![cfg(target_pointer_width = "32")]

use lean_string::{LeanStr, LeanString};

// The largest length that `TextLen` stores in the handle on 32-bit. One more byte and the length
// is stored on the heap, in front of the header.
const MAX_INLINE_TEXT_LEN: usize = (1 << 24) - 2;

#[test]
fn push_across_heap_length_boundary() {
    // Capacity exactly MAX_INLINE_TEXT_LEN: no length prefix in the allocation.
    let mut s = LeanString::with_capacity(MAX_INLINE_TEXT_LEN);
    s.push_str(&"a".repeat(MAX_INLINE_TEXT_LEN - 1));
    s.push('b');
    assert_eq!(s.len(), MAX_INLINE_TEXT_LEN);
    assert_eq!(s.capacity(), MAX_INLINE_TEXT_LEN);

    // Growing past it switches the layout to the one with a length prefix, copying the content.
    s.push('c');
    assert_eq!(s.len(), MAX_INLINE_TEXT_LEN + 1);
    assert!(s.capacity() > MAX_INLINE_TEXT_LEN);
    assert!(s.ends_with("abc"));
    assert_eq!(s.as_bytes()[0], b'a');

    // And back: shrinking below the boundary drops the prefix again.
    s.truncate(100);
    assert_eq!(s.len(), 100);
    s.shrink_to_fit();
    assert_eq!(s.capacity(), 100);
    assert_eq!(s, "a".repeat(100));
}

#[test]
fn truncate_and_pop_with_length_on_heap() {
    let len = MAX_INLINE_TEXT_LEN + 2;
    let mut s = LeanString::from("x".repeat(len).as_str());
    let shared = s.clone();
    let ptr = shared.as_ptr();

    // Shared, length on the heap: must copy, since the length is shared with `shared`.
    assert_eq!(s.pop(), Some('x'));
    assert_eq!(s.len(), len - 1);
    assert_ne!(s.as_ptr(), ptr);
    assert_eq!(shared.len(), len);

    // Unique, length on the heap: done in place.
    let mut unique = shared;
    unique.truncate(len - 1);
    assert_eq!(unique.len(), len - 1);
    assert_eq!(unique.as_ptr(), ptr);
    unique.truncate(MAX_INLINE_TEXT_LEN);
    assert_eq!(unique.len(), MAX_INLINE_TEXT_LEN);
    assert_eq!(unique.as_ptr(), ptr);

    // Shared, length no longer on the heap: the handle's own length changes, the buffer is kept.
    let shared = unique.clone();
    unique.truncate(10);
    assert_eq!(unique, "xxxxxxxxxx");
    assert_eq!(unique.as_ptr(), ptr);
    assert_eq!(shared.len(), MAX_INLINE_TEXT_LEN);
}

#[test]
fn into_lean_str_with_both_length_prefixes() {
    let len = MAX_INLINE_TEXT_LEN + 1;
    let mut s = LeanString::with_capacity(len + 100);
    s.push_str(&"y".repeat(len));

    let frozen = s.into_lean_str();
    assert_eq!(frozen.len(), len);
    assert!(frozen.starts_with("yyyy") && frozen.ends_with("yyyy"));

    let mut thawed = frozen.into_lean_string();
    assert_eq!(thawed.capacity(), len);
    thawed.push('z');
    assert_eq!(thawed.len(), len + 1);
    assert!(thawed.ends_with("yz"));

    let s: LeanStr = thawed.into_lean_str();
    assert_eq!(s.len(), len + 1);
}
```

注意: Miri では 16 MiB を 1 文字ずつ調べる処理 (`bytes().all(..)`、`chars().count()` など) は非常に遅い (1 つで 10 分を超えた)。上のテストは `starts_with` / `ends_with` / `len` / ポインタ比較だけで中身を確かめている。比較に `assert_eq!(s, "a".repeat(..))` を使うのは 100 バイト程度にとどめる。

## 確認方法

`MIRIFLAGS=-Zmiri-strict-provenance cargo +nightly miri test --target i686-unknown-linux-gnu --test <file>` と `--target powerpc-unknown-linux-gnu` で通ること。CI の Miri ジョブは全テストを走らせるので、追加すれば 32-bit の 2 ジョブで自動的に走る。
