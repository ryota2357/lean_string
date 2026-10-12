# `capacity()` の doc が共有バッファの場合に触れておらず、`shrink_to()` の例のコメントが逆になっている

- 種別: doc
- 対象: `src/lib.rs` の `LeanString::capacity`, `LeanString::shrink_to` の doc
- 影響: `capacity() - len()` を「再確保なしで書ける量」と読む利用者が、clone と共有している文字列で誤解する。`shrink_to` の例のコメントは挙動と逆のことを言っている。
- 優先度: 低

## 問題

### `capacity()`

doc の Note は「`&'static str` のときは長さと同じ」「それ以外は少なくとも `2 * size_of::<usize>()`」の 2 点だけを書いている。
heap バッファを clone と共有しているときは、共有している heap の容量 (例: 100) を返すが、その `LeanString` に 1 バイトでも
書けばコピーが起きるので、その容量は使えない。`truncate` で共有のまま短くした場合も同じで、例えば 42 バイトの文字列の clone を
`truncate(5)` すると `capacity()` は 42 のままで、`push` すると inline にコピーされて 16 になる (`5dd94ea` でテストして確認)。

`reserve()` の doc には「共有なら容量が足りていても clone する」と書いてあるので、`capacity()` 側にもひとこと要る。

### `shrink_to()` の例

```rust
/// let mut s = LeanString::with_capacity(100);
/// assert_eq!(s.capacity(), 100);
///
/// // if the capacity was already bigger than the argument and unique, the call is no-op.
/// s.shrink_to(100);
/// assert_eq!(s.capacity(), 100);
```

`Repr::shrink_to` は「`max(len, min_capacity)` が今の容量より小さいときだけ縮める」ので、何もしないのは
容量が引数 (と長さ) 以下のときである。コメントの「容量が引数より大きければ何もしない」は逆で、
容量 100 で `shrink_to(50)` すれば縮む (doc の例でもこの直後に `s.shrink_to(50)` で容量が 50 になることを確かめている)。

## 修正例

`capacity()` の Note に足す:

```rust
    /// - If the heap buffer is shared with clones, this returns the capacity of the shared buffer,
    ///   but writing to this [`LeanString`] copies it first, so the capacity is not available to it
    ///   until then. (See [`LeanString::reserve()`].)
```

`shrink_to()` の例のコメント:

```rust
    /// // If the capacity is not greater than the argument, the call is a no-op.
    /// s.shrink_to(100);
    /// assert_eq!(s.capacity(), 100);
```

## 確認方法

- `cargo test --doc` で例が通ること (コメントの変更だけなので挙動は変わらない)。
- 共有バッファの `capacity()` を doc の例に入れるなら、`let t = s.clone(); assert_eq!(t.capacity(), s.capacity());` のあと
  `t.push('x')` で `t.as_ptr() != s.as_ptr()` になることを示す。
