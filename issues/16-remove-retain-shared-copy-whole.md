# 共有 / static バッファの `remove` と `retain` は全体をコピーしてから詰めるので、inline に収まる結果も heap に残る

- 種別: perf
- 対象: `src/repr.rs` の `Repr::<Mutable>::remove`, `Repr::<Mutable>::retain` (どちらも `ensure_modifiable` 経由のコピー)
- 影響: clone と共有している、または `from_static_str` で作った文字列に `remove` / `retain` をすると、
  結果が 16 バイト (64-bit) 以下でも heap 確保が起きる。`retain` で全部消しても元の長さぶん確保する。
  コピーのあとに詰め直す分の `memmove` も余計にかかる。
- 優先度: 低

## 問題

`remove` と `retain` は、書き換える前に `ensure_modifiable` で「今の内容をそのまま」新しいバッファにコピーし、
そのあとで文字を削って詰める。コピーは `Repr::from_str(self.as_str())` なので、元の長さが 16 バイトを超えていれば
heap に確保され、削ったあとも heap に残る。

同じ「共有バッファを書き換える」操作でも、`replace_range` は非 modifiable のとき結果の長さで `Repr::new_with` を呼ぶので、
収まれば inline になる。`truncate` のあとに書き換える場合も inline になる。CHANGELOG 0.8.0 の
「共有バッファを書き換えるとき、内容が収まれば inline にコピーする」という変更と、この 2 つだけ挙動が揃っていない。

確認したテスト (`5dd94ea`、x86_64):

```rust
let src = LeanString::from("abcdefghijklmnopq"); // 17 バイト、heap
let mut a = src.clone();
a.remove(0);                       // -> heap=true,  capacity=17
let mut b = src.clone();
b.replace_range(0..1, "");         // -> heap=false, capacity=16
let mut c = src.clone();
c.retain(|ch| ch != 'a');          // -> heap=true,  capacity=17
let mut st = LeanString::from_static_str("abcdefghijklmnopq");
st.remove(0);                      // -> heap=true,  capacity=17

let big = LeanString::from("a".repeat(100).as_str());
let mut d = big.clone();
d.retain(|ch| ch != 'a');          // 空になるのに heap=true, capacity=100
```

## 原因

`ensure_modifiable` が「変更後の長さ」を知らずに今の内容を丸ごとコピーするため (コードとテストで確認)。
`remove` ではさらに、コピーした全体のうち削った位置より後ろを `ptr::copy` (`memmove`) で詰め直すので、
後ろ側を 2 回コピーしている。

## 修正例

`remove` は `replace_range` の非 modifiable 経路と同じ形にできる。削る文字と結果の長さはコピーの前に分かるので、
結果を直接新しいバッファ (収まれば inline) に作る。

```rust
        if !self.is_modifiable() {
            let text = self.as_str();
            // SAFETY: `idx < len` is a char boundary, so the suffix is nonempty valid UTF-8.
            let ch = unsafe { text.get_unchecked(idx..).chars().next().unwrap_unchecked() };
            let ch_len = ch.len_utf8();
            // SAFETY: The two copies fill all `len - ch_len` bytes with `text` without `ch`, which
            // is valid UTF-8, and the new buffer doesn't overlap `text`.
            let removed = unsafe {
                Repr::new_with(len - ch_len, |dst| {
                    let dst = dst.as_mut_ptr().cast::<u8>();
                    let src = text.as_ptr();
                    ptr::copy_nonoverlapping(src, dst, idx);
                    ptr::copy_nonoverlapping(src.add(idx + ch_len), dst.add(idx), len - idx - ch_len);
                })
            }?;
            self.replace_inner(removed);
            return Ok(ch);
        }
        // 以降は今の in-place の処理 (`ensure_modifiable` は不要になる)
```

試した結果: 上のテストで `a` と `st` が inline (capacity 16) になった。`tests/lean_string.rs`、`tests/alloc_string.rs`、
`remove` を unique / 共有 / static で `String` と突き合わせる property テストが通る。

`retain` は結果の長さが述語を全部呼ぶまで分からないので、案は次のとおり。

- 案 A (推奨): 非 modifiable のときは、新しいバッファに「最初に削る文字より前」をコピーし、残りは元の (共有の) バッファから
  読みながら残す文字だけを書き込む。容量は `len - removed.len_utf8()` を上限にでき、全体コピー + 詰め直しの 2 パスが 1 パスになる。
  結果が 16 バイト以下なら最後に inline へ移す (`Repr::from_str` で作り直して解放)。述語の panic に備えて、
  書きかけのバッファを解放するガードが要る (`Repr` は `Drop` を持たないので `drop_in` を呼ぶガード)。
- 案 B: 今の処理のまま、最後に「heap で長さが 16 以下なら inline に移す」だけを足す。確保は減らないが結果は揃う。

## 確認方法

- 上のテストを `assert!(!x.is_heap_allocated())` の形にして追加する (共有 / static の両方、17 バイトから 1 文字削るケースと、
  `retain` で全部消すケース)。
- `remove` / `retain` を unique / 共有 / static の各バッファで `String` と突き合わせる property テスト
  ([#31](./31-test-removal-ops-shared-static.md) の `remove` / `retain`)。
- 確保回数を数えるテスト (カウントする `#[global_allocator]`) で、共有 17 バイトの `remove` が 0 回になること。
