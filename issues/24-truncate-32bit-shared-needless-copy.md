# 32-bit で長さが heap にある共有バッファの `truncate` / `pop` は、新しい長さがハンドルに収まってもコピーする

- 種別: perf, doc
- 対象: `src/repr.rs` の `Repr::<Mutable>::truncate_unchecked` (`truncate`, `pop`, `split_off` から使われる)、`src/lib.rs` の `LeanString::pop` / `LeanString::truncate` の doc
- 影響: 32-bit ターゲットで、長さが `2^24 - 2` を超える (約 16 MiB 以上) 文字列を clone して共有しているときだけ。
  `truncate` で `2^24 - 2` 以下に縮めると、縮めた内容 (最大 16 MiB) をまるごとコピーして確保する。必要なのはハンドルの長さの更新だけ。
- 優先度: 低

## 問題

32-bit では `TextLen` に収まらない長さ (`2^24 - 2` 超) を heap の先頭に置く。heap 上の長さは共有している
全ハンドルが読むので、共有中に書き換えてはいけない。`truncate_unchecked` はこの場合を次のように扱う。

```rust
            if !heap.is_len_on_heap() {
                unsafe { heap.set_len(new_len) };
            } else if heap.is_unique() {
                unsafe { heap.set_len(new_len) };
            } else {
                // 共有で長さが heap 上: new_len バイトをコピーして新しいバッファにする
                let new_repr = Repr::from_str(str)?;
                ...
            }
```

3 つ目の分岐は「今の長さが heap 上」で判定しているが、書き込みが共有の heap に及ぶのは「新しい長さが heap 上」のとき
だけである。`HeapBuffer::<GrowableHeader>::set_len` は新しい長さが `TextLen` に収まればハンドルの `len` だけを書き換え、
heap には触らない (その場合は unique である必要がないと `# Safety` にも書いてある)。

i686 の Miri (`-Zmiri-strict-provenance`、`5dd94ea`) で確認したテスト:

```rust
#![cfg(target_pointer_width = "32")]
const BIG: usize = 1 << 24; // 長さは heap 上

let s = LeanString::from("a".repeat(BIG).as_str());
let mut t = s.clone();
t.truncate(BIG - 100);           // 新しい長さはハンドルに収まる
assert_eq!(t.len(), BIG - 100);
assert_eq!(s.len(), BIG);
// 現状: t.as_ptr() != s.as_ptr() (約 16 MiB をコピー)
```

## 原因

分岐の条件が「今の長さが heap 上か」になっていて、「新しい長さが heap 上か」を見ていないため (コードと Miri のテストで確認)。

## 修正例

新しい長さがハンドルに収まるなら、共有でも `set_len` してよい。

```rust
// src/repr/heap_buffer.rs
impl<H: Header> HeapBuffer<H> {
    /// Whether a length of `len` is stored in the handle, not on the heap.
    #[inline]
    pub(super) fn len_fits_handle(len: usize) -> bool {
        matches!(TextLen::new(len), Ok(l) if !l.is_heap())
    }
}

// src/repr.rs の truncate_unchecked
            if !heap.is_len_on_heap()
                || HeapBuffer::<Mutable>::len_fits_handle(new_len)
                || heap.is_unique()
            {
                // SAFETY: `new_len <= len <= capacity`. If the buffer is shared, `new_len` is stored
                // in the handle only, so the shared heap is not written.
                unsafe { heap.set_len(new_len) };
            } else {
                // 今のコピーする処理
            }
```

試した結果: 上のテストで `t` は共有のまま (`t.as_ptr() == s.as_ptr()`) になり、そのあと `t.push('b')` でコピーされて
`s` は変わらない。共有のまま長さが heap 上に残るケース (`BIG - 1` への truncate とその後の `pop`)、`clear`、
unique なバッファの truncate のテストとあわせて、i686 の Miri で問題なし。64-bit では `is_len_on_heap()` が定数 `false`
なので生成コードは変わらない。

あわせて doc を直す。`LeanString::pop` の doc は「共有」かつ「長さが `2^24 - 2` を超える」ときにコピーすると書いているが、
修正後は「pop した後の長さが `2^24 - 2` を超える」ときになる。`LeanString::truncate` の doc の
「2. The system is out-of-memory when cloning the LeanString」は条件を書いていないので、`pop` と同じく
「32-bit で、共有されていて、新しい長さが `2^24 - 2` を超えるときだけ」と書く (64-bit の `truncate` は確保しない)。今の `truncate` の doc が 64-bit では起こらない OOM を panic の条件に挙げている点は、[#39](./39-panics-doc-gaps.md) にも挙げている。

## 確認方法

- 上のテストを `#[cfg(target_pointer_width = "32")]` で追加し、`assert_eq!(t.as_ptr(), s.as_ptr())` を確かめる。
  CI の i686 / powerpc の Miri で回る (16 MiB の文字列でも Miri で 1 秒未満だった)。
- 共有のまま、新しい長さも heap 上になるケース (`BIG - 1`) でコピーされ、元が変わらないことを確かめる。
