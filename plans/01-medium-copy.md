# 01: 中サイズのコピーで memcpy を呼ばないようにする

種別: perf / 実装量: 小 / 設計判断: 小

## 背景

`ptr::copy_nonoverlapping` は、長さがコンパイル時に決まっていないと `memcpy` の呼び出しになる。
inline 構築 (`InlineBuffer::new`) はレジスタで組み立てているので呼び出しは無いが、
heap への構築と追記では可変長の memcpy が残っている
([research/codegen-baseline.md](../research/codegen-baseline.md) の §1、§3)。

- `LeanString::from(&str)` の heap 経路: `callq *memcpy@GOTPCREL(%rip)`
- `Repr::push_str` の追記: `callq *memcpy@GOTPCREL(%rip)`
- `Repr::insert_str`: `memmove` と `memcpy` を 1 回ずつ

`bench/benches/comparison.rs` の `Construct` を見ると、inline に収まらなくなった直後の長さで
他のクレートに負けている。

| len | LeanString | CompactString | String | EcoString |
| --: | --- | --- | --- | --- |
| 16 | **1.4652 ns** | 1.5610 ns | 11.456 ns | 16.854 ns |
| 24 | 16.101 ns | 1.8251 ns (inline) | **11.829 ns** | 19.094 ns |
| 25 | 16.270 ns | 12.215 ns | **11.918 ns** | 18.624 ns |
| 256 | **16.838 ns** | 18.115 ns | 17.794 ns | 18.539 ns |

len 24 の CompactString は 24 バイト表現の inline に収まっているだけなので比較対象にならない。
問題は len 25 で、`String` より 37% 遅い。256 では逆に勝っているので、
差は確保のコストではなく、短いコピーでの関数呼び出しのコストだと考えられる。

## 方針

長さの帯ごとに固定長のコピーを重ねて使い、64 バイトを超えるときだけ `memcpy` に任せる
ヘルパを `src/repr.rs` に置く。

```rust
/// `src` から `dst` へ `len` バイトをコピーする。
///
/// 可変長の `copy_nonoverlapping` は `memcpy` の呼び出しになり、短いコピーでは呼び出しの
/// コストが目立つ。[16, 64] は先頭と末尾からの固定長コピー 2 回で済ませる。
/// 64 バイトを超える場合は `memcpy` のほうが速いのでそちらに任せる。
///
/// # Safety
/// - `src` は `len` バイト読める。
/// - `dst` は `len` バイト書ける。
/// - `src` と `dst` は重ならない。
#[inline]
unsafe fn copy_bytes(src: *const u8, dst: *mut u8, len: usize) { ... }
```

16 / 32 / 64 という区切りは SIMD レジスタの幅から決めたもので、`MAX_INLINE_SIZE` とは関係ない。

16 バイト未満の扱いは 2 通り考えられる。

- 案A: `InlineBuffer::new` と同じく 8/4/2/1 バイトの固定長コピーで処理する。
  `push_str` は短い追記が多いので、追記に使うならこちらがよい。
- 案B: `memcpy` に任せる。heap への構築だけに使うなら短いコピーはほぼ来ないので、これで足りる。

`push_str` にも使うので案A にする。ヘルパを 1 つで済ませられる。
16 バイト未満の分岐を足したことで heap 経路の命令数が増えていないかは asm で確認する。

## 適用先

| 場所 | 用途 |
| --- | --- |
| `HeapBuffer::new` | `from_str` の heap 経路。いちばん頻度が高い |
| `HeapBuffer::with_additional` | `reserve` での共有解除と、inline から heap への移行 |
| `HeapBuffer::with_exact_capacity` | 共有されているときの `shrink_to` |
| `HeapBuffer::realloc` のレイアウト変換 | 32-bit でしか通らない |
| `Repr::push_str` | 追記。短いコピーが多い |
| `Repr::insert_str` | 挿入する文字列のコピー |
| `Repr::repeat` | 倍々に埋めていく `copy_from_slice`。最初の数回は短い |

`HeapBuffer::realloc` の中でヘッダの位置を変えるための `ptr::copy` は領域が重なるので対象外。
`insert_str` の末尾をずらす処理や、`retain` / `remove` の詰め直しも同じ理由で対象外。

lean_string は clone でバッファを共有するので、「確保してコピー」の回数は CoW でないクレートより少ない。
効果はそちらで報告されている数字より小さいはずで、主に効くのは `from_str` の heap 経路と `push_str` の追記。

## 検証

- asm: `LeanString::from(&str)` の heap 経路と `push_str` から `callq *memcpy` が消え、
  固定長のロード/ストアになっていること。64 バイトを超える場合は `memcpy` が残ること。
- criterion: `apis.rs` の `from` (17/256)、`push_str`、`push_str/after_clone`。
  `comparison.rs` の `Construct` / `Grow` / `CoW write`。
  `apis.rs` の `from` は長さが 0/1/15/16/17/256 で、17〜64 の間に点が 1 つしかない。32 と 64 を足す。
- Miri: コピーの境界をまたぐので 4 ターゲットすべて。

効果が誤差程度なら入れなくてよい。その場合も、試して効かなかったことは記録に残す。

## 依存

[02](./02-push-str-fast-path.md) と `push_str` の同じ箇所を触るので、どちらかを先に終わらせる。
