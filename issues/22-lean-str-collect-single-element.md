# 要素 1 つの `LeanStr` を `collect::<LeanStr>()` すると realloc が 2 回走る

- 種別: perf
- 対象: `src/lib.rs` の `impl FromIterator<LeanStr> for LeanStr`
- 影響: `LeanStr` のイテレータを `LeanStr` に集めるとき、要素が 1 つだけ (フィルタの結果など) だと、そのまま返せるはずの値に realloc 2 回と文字列全体の `ptr::copy` 2 回がかかる。共有されている要素なら新しい確保とコピーになり、共有も失われる
- 優先度: 低

## 問題

`5dd94ea` の実装:

```rust
impl FromIterator<LeanStr> for LeanStr {
    fn from_iter<T: IntoIterator<Item = LeanStr>>(iter: T) -> Self {
        let mut iter = iter.into_iter();
        let mut buf = match iter.next() {
            Some(str) => str.into_lean_string(),   // Immutable → Mutable
            None => return LeanStr::new(),
        };
        buf.extend(iter);
        buf.into_lean_str()                        // Mutable → Immutable
    }
}
```

確保を数える global allocator で測った (48 バイトの unique な heap 文字列、x86_64、release)。

| 操作 | alloc | realloc | 最小 |
| --- | --- | --- | --- |
| `[LeanString; 1]` → `LeanString` | 0 | 0 | 0 / 0 |
| `[LeanStr; 1]` → `LeanString` | 0 | 1 | 0 / 1 (レイアウトの変換) |
| `[LeanString; 1]` → `LeanStr` | 0 | 1 | 0 / 1 (同上) |
| **`[LeanStr; 1]` → `LeanStr`** | **0** | **2** | **0 / 0** |

(dealloc は `Vec` 自身の解放の 1 回。)

1 要素のとき、`into_lean_string` (exact → growable、`HeapBuffer::into_growable`) と `into_lean_str` (growable → exact、`HeapBuffer::into_exact`) が打ち消し合うだけなのに、どちらも realloc と文字列全体の `ptr::copy` を伴う。要素が共有されていると (`clone()` したものを集めた場合など)、`into_lean_string` が新しいバッファにコピーし、`into_lean_str` がさらに realloc する。

`FromIterator<LeanString> for LeanString` は最初の要素をそのまま使うので、1 要素なら何もしない。

## 原因

先頭の要素を無条件に `LeanString` に変換しているため。追加する要素が無いことを確かめてから変換すればよい。

## 修正例

```rust
impl FromIterator<LeanStr> for LeanStr {
    fn from_iter<T: IntoIterator<Item = LeanStr>>(iter: T) -> Self {
        let mut iter = iter.into_iter();
        let Some(first) = iter.next() else {
            return LeanStr::new();
        };
        let Some(second) = iter.next() else {
            // Nothing to append: hand the only element back as is.
            return first;
        };
        let mut buf = first.into_lean_string();
        buf.push_str(&second);
        buf.extend(iter);
        buf.into_lean_str()
    }
}
```

試した結果: `[LeanStr; 1]` → `LeanStr` は alloc 0 / realloc 0 になる。2 要素以上は変わらない (realloc 3 回: `into_growable`、追加による拡張、`into_exact`)。全テスト通過。

2 要素以上の場合も、先頭を `into_lean_string` で容量 = 長さの growable にした直後に `push_str` で拡張するので、realloc が 2 回続く。`size_hint` の下限や 2 番目の要素の長さを見て先頭の変換時に容量を確保すれば 1 回減らせるが、`into_growable` に容量を渡す口が要るので、ここでは扱わない。

## 確認方法

- 確保回数のテスト: 1 要素で alloc / realloc が 0 回であること、結果の `as_ptr()` が元の要素と同じであること (共有されている要素でも同じバッファを指すこと)。0 要素 / 2 要素で内容が正しいこと。
