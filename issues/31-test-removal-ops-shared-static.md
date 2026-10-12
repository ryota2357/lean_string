# 削除・縮小系の操作に、共有 / static バッファで `String` と突き合わせる property テストが無い

- 種別: test
- 対象: `tests/property.rs` (`truncate`, `pop`, `remove`, `retain`, `split_off`, `clear`, `shrink_to`, `shrink_to_fit`)、`tests/lean_string.rs` の `clear_cow`
- 影響: これらの操作はバッファの種類 (inline / unique heap / 共有 heap / static) ごとに別の経路を通るが、ランダム入力で検証されているのは
  追加系だけ。削除系の経路を書き換えるとき ([#04](./04-retain-memmove-per-char.md)、[#16](./16-remove-retain-shared-copy-whole.md)、[#24](./24-truncate-32bit-shared-needless-copy.md) の修正など) の安全網が薄い。
- 優先度: 中

## 問題

`tests/property.rs` には `push`, `insert`, `replace_range`, `extend_from_within` について「unique と共有 (clone を持ったまま) で
`String` と一致し、clone 側が変わらない」ことを確かめる property テストがある。一方、削除・縮小系
(`truncate`, `pop`, `remove`, `retain`, `split_off`, `clear`, `shrink_to`, `shrink_to_fit`) の property テストは無く、
static バッファ (`from_static_str`) を対象にした property テストはどの操作にも無い。

固定の例のテストは `tests/lean_string.rs` にある。static バッファの `clear` が固定テストでも見られていない点は [#32](./32-test-static-shared-and-panics.md) で扱う。`pop` / `truncate` / `shrink_to` / `retain` には static のテストがある。

## 修正例

`tests/property.rs` に、同じ内容を 3 種類のバッファで持つヘルパと、操作ごとのテストを足す。下のコードを `5dd94ea` に足して
すべて通ることを確かめた (x86_64、`cargo test`)。

```rust
/// The same content in each buffer kind: unique (inline or heap), shared heap, and static.
fn variants(input: &str) -> Vec<(&'static str, LeanString, Option<LeanString>)> {
    let leaked: &'static str = Box::leak(input.to_owned().into_boxed_str());
    let shared = LeanString::from(input);
    let keep = shared.clone();
    vec![
        ("unique", LeanString::from(input), None),
        ("shared", shared, Some(keep)),
        ("static", LeanString::from_static_str(leaked), None),
    ]
}

fn boundary(input: &str, idx: Index) -> usize {
    let b: Vec<usize> = (0..=input.len()).filter(|&i| input.is_char_boundary(i)).collect();
    b[idx.index(b.len())]
}

#[property_test]
#[cfg_attr(miri, ignore)]
fn truncate(input: String, idx: Index) {
    let at = boundary(&input, idx);
    let mut expected = input.clone();
    expected.truncate(at);
    for (kind, mut s, keep) in variants(&input) {
        s.truncate(at);
        prop_assert_eq!(&s, expected.as_str(), "{}", kind);
        // a write after truncating a shared/static buffer must not touch the original
        s.push('x');
        prop_assert_eq!(s, expected.clone() + "x", "{}", kind);
        if let Some(keep) = keep {
            prop_assert_eq!(keep, input.as_str());
        }
    }
}

#[property_test]
#[cfg_attr(miri, ignore)]
fn remove(#[strategy = ".+"] input: String, idx: Index) {
    let b: Vec<usize> = input.char_indices().map(|(i, _)| i).collect();
    let at = b[idx.index(b.len())];
    let mut expected = input.clone();
    let ch = expected.remove(at);
    for (kind, mut s, keep) in variants(&input) {
        prop_assert_eq!(s.remove(at), ch, "{}", kind);
        prop_assert_eq!(&s, expected.as_str(), "{}", kind);
        if let Some(keep) = keep {
            prop_assert_eq!(keep, input.as_str());
        }
    }
}

#[property_test]
#[cfg_attr(miri, ignore)]
fn retain(input: String, mask: Vec<bool>) {
    let keep_at = |i: usize| mask.get(i).copied().unwrap_or(true);
    let mut expected = input.clone();
    let mut i = 0;
    expected.retain(|_| { i += 1; keep_at(i - 1) });
    for (kind, mut s, keep) in variants(&input) {
        let mut i = 0;
        s.retain(|_| { i += 1; keep_at(i - 1) });
        prop_assert_eq!(&s, expected.as_str(), "{}", kind);
        if let Some(keep) = keep {
            prop_assert_eq!(keep, input.as_str());
        }
    }
}

// `pop`, `split_off` (`boundary` で位置を選ぶ)、`clear` / `shrink_to(min % 64)` / `shrink_to_fit`
// (内容が変わらず、`capacity() >= len()`、clone 側が変わらない) も同じ形で書ける。
```

static バッファの `clear` の固定テストは [#32](./32-test-static-shared-and-panics.md) にある。

## 確認方法

- 追加したテストが `cargo test` で通ること。既存の property テストと同じく `#[cfg_attr(miri, ignore)]` を付ける
  (Miri では固定の例のテストで見る)。
- 32-bit で長さが heap にある経路 (`2^24 - 2` 超) は property テストでは届かないので、
  [#24](./24-truncate-32bit-shared-needless-copy.md) の固定テストで見る。
