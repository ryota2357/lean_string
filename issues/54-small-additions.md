# 小さな API の欠け: `INLINE_CAPACITY` 定数、`try_to_uppercase`、`const fn` の不足、`Extend<&LeanString>`

- 種別: api
- 対象: `src/lib.rs` の `LeanString` / `LeanStr` (`capacity`, `is_heap_allocated`, `to_uppercase`, `Extend` / `FromIterator` impl)、`src/repr.rs` の `Repr::to_case`
- 影響: いずれも独立した小さな欠落で、実装は数行。どれも std の `String` にはそのままの対応物がない (`try_*` と定数は `String` の対象外)、またはワークアラウンドが容易なので優先度は低い。
- 優先度: 低

## 問題

### 1. inline 容量の定数が無い

docs と利用者コードの両方で `2 * size_of::<usize>()` を直接書くことになる (lib.rs の docs だけで 12 箇所)。`Repr` 内部には `MAX_INLINE_SIZE` があるが private。32-bit でも `2 * size_of::<usize>()` で正しいが、「これが inline 容量である」という契約は docs の散文にしかない。`LeanString::INLINE_CAPACITY` があると、`capacity() == INLINE_CAPACITY` のような条件や `with_capacity` の判断が書きやすい (他クレートの前例は未調査)。

### 2. `try_to_uppercase` が無い (`try_to_lowercase` は作れない理由がある)

`to_lowercase` / `to_uppercase` には `try_` 版が無い。コードの NOTE は `to_lowercase` については「`Σ` の変換が `str::to_lowercase` に依存し、その確保失敗は `ReserveError` にできない」と理由を書くが、`to_uppercase` は「`to_lowercase` と対称にするため」とだけ書く。実際、`Repr::to_case` の `Σ` 分岐は `Case::Lower` だけで、`Case::Upper` の経路は全部 `Repr::push_char` (`Result` を返す) なので、`Repr::to_case` が既に `Result<Option<Repr>, ReserveError>` を返している。

```rust
pub fn try_to_uppercase(&self) -> Result<Self, ReserveError> {
    Repr::to_case(self, Case::Upper).map(|r| r.map_or_else(|| self.clone(), LeanString))
}
```

`LeanStr` 版は `match Repr::to_case(self, Case::Upper)? { None => Ok(self.clone()), Some(u) => LeanString(u).try_into_lean_str() }`。どちらも試作し、`to_uppercase` と同じ結果になることを確認した (inline / heap、`ß` を含む入力)。

`to_lowercase` については、`Σ` を含む場合に `str::to_lowercase` を呼ぶ経路が abort しうること自体が別の問題 ([#39](./39-panics-doc-gaps.md) 参照)。対称性を取るなら、`try_to_lowercase` を作れない理由を docs の `# Panics` に書く。

### 3. `is_heap_allocated` / `capacity` が `const fn` でない

`len` / `is_empty` / `as_str` / `as_bytes` / `as_static_str` / `new` / `from_static_str` は `const fn` (`tests/const.rs`)。`is_heap_allocated` の中身は `Repr::is_heap_buffer` で、すでに `const fn` なので `const fn` にするだけでよい。`capacity` は `HeapBuffer::capacity` / `Capacity::as_usize` を `const fn` にすれば同様。ただし `LeanString` は `Drop` を持つため、const 文脈で値を作った一時値に対して呼ぶには `mem::forget` が必要 ([#41](./41-std-differences-docs.md) 参照)。実用上の必要性は低い。

### 4. `Extend<&LeanString>` / `Extend<&LeanStr>` / `FromIterator<&LeanString>` が無い

`5dd94ea` で `vec_of_lean.iter()` を `extend` に渡すとコンパイルエラー (`LeanString: Extend<&LeanString>` が満たされない)。`String` も `Extend<&String>` / `FromIterator<&String>` を持たない (確認済み) ので std との互換性の問題ではなく、利便性の問題。回避は `.map(LeanString::as_str)`。

```rust
impl<'a> Extend<&'a LeanString> for LeanString {
    fn extend<T: IntoIterator<Item = &'a LeanString>>(&mut self, iter: T) {
        for s in iter { self.push_str(s); }
    }
}
// &'a LeanStr も同じ形。FromIterator<&'a LeanString> / <&'a LeanStr> for LeanString は Extend に委譲。
```

注意: `Extend` の impl を増やすと、`extend` の引数の型推論が曖昧になるコードが出る可能性がある (未検証。`.extend(iter.map(|x| x.as_ref()))` のように戻り値の型が推論頼みのもの)。

### 5. 検討して見送ってよいもの

- `leak` / `LeanStr::leak(self) -> &'static str`: heap の buffer なら `mem::forget(self)` で参照カウントを 1 つ残せば `&'static str` を返せる (sound) が、inline は値自体がバッファなので別に `Box::leak` でコピーが要り、static は元の `&'static str` を返せる。3 つの場合分けが要り、用途が限られる。需要が確認できるまで保留でよい。
- `pop_if` (`String` にも安定版で存在しない)、`remove_matches` / `extract_if` (unstable) は対象外。

## 確認方法

追加する場合、それぞれ `tests/lean_string.rs` / `tests/lean_str.rs` にテストを足す。`try_to_uppercase` は失敗アロケータで `Err` になり `self` が不変であること (`tests/out_of_memory.rs` の仕組み)。`INLINE_CAPACITY` は `assert_eq!(LeanString::new().capacity(), LeanString::INLINE_CAPACITY)` と 32-bit (`i686` の Miri) での値。
