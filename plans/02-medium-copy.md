# 02: 中サイズのコピーから memcpy 呼び出しを外す

種別: perf / 実装量: 小 / 設計判断: 小

## 背景

`ptr::copy_nonoverlapping` は長さがコンパイル時定数でないと `memcpy` の関数呼び出しに
落ちる。`InlineBuffer::new` はレジスタ構築でこれを回避しているが、heap 経路と追記経路には
可変長 memcpy が残っている
([research/codegen-baseline.md](../research/codegen-baseline.md) の §1 と §3)。

- `LeanString::from(&str)` の heap 経路: `callq *memcpy@GOTPCREL(%rip)`
- `Repr::push_str` の追記コピー: `callq *memcpy@GOTPCREL(%rip)`
- `Repr::insert_str`: `memmove` と `memcpy` の 2 回

`bench/benches/comparison.rs` の `Construct` で、inline を外れた直後の長さ帯が
他クレートに負けている。

| len | LeanString | CompactString | String | EcoString |
| --: | --- | --- | --- | --- |
| 16 | **1.4652 ns** | 1.5610 ns | 11.456 ns | 16.854 ns |
| 24 | 16.101 ns | 1.8251 ns (inline) | **11.829 ns** | 19.094 ns |
| 25 | 16.270 ns | 12.215 ns | **11.918 ns** | 18.624 ns |
| 256 | **16.838 ns** | 18.115 ns | 17.794 ns | 18.539 ns |

len 24 で CompactString が速いのは 24 バイト表現に inline で収まるからで、これは比較にならない。
見るべきは len 25 で、LeanString が String より 37% 遅い。256 まで伸ばすと逆転するので、
差は確保そのものではなく短いコピーの call 境界にあると考えてよい。

## 方針

長さの帯ごとに固定長の重ね合わせコピーを使い、64 バイト超だけ `memcpy` に委譲する
ヘルパを `src/repr.rs` に置く。

```rust
/// `src` から `dst` へ `len` バイトをコピーする。ヒープ経路と追記経路向け。
///
/// 可変長の `copy_nonoverlapping` は `memcpy` の呼び出しになる。コピーが短いほど
/// call 境界の割合が大きくなるので、[16, 64] の帯は両端からの固定長コピー 2 回で
/// 済ませて呼び出しを避ける。64 バイト超は CPU ごとにチューニングされた
/// `memcpy` の大サイズ経路に任せたほうが速いので委譲する。
///
/// # Safety
/// - `src` は `len` バイトの読み出しに対して有効。
/// - `dst` は `len` バイトの書き込みに対して有効。
/// - `src` と `dst` は重ならない。
#[inline]
unsafe fn copy_bytes(src: *const u8, dst: *mut u8, len: usize) { ... }
```

境界の 16 / 32 / 64 は SIMD レジスタ幅に合わせたもので、`MAX_INLINE_SIZE` とは独立に決まる。

`< 16` の帯をどうするかは 2 案ある。

- 案A: `InlineBuffer::new` と同じ 8/4/2/1 の固定長コピーを続ける。
  追記経路 (`push_str`) では短い追記が支配的なのでこちらが効く。
- 案B: `memcpy` に委譲する。ヒープ経路だけを対象にするなら短い帯は稀なので委譲で十分。

`push_str` にも使うなら案A。ヘルパを 1 つにまとめられるので、まず案A で書いて、
`< 16` の帯を足したことで heap 経路の命令数が増えていないかを asm で確認する。

## 適用先

| 場所 | 現状 | 備考 |
| --- | --- | --- |
| `HeapBuffer::new` (heap_buffer.rs:136) | 可変長 | `from_str` の heap 経路。最頻 |
| `HeapBuffer::with_additional` (heap_buffer.rs:375) | 可変長 | `reserve` の unshare / inline→heap 昇格 |
| `HeapBuffer::with_exact_capacity` (heap_buffer.rs:347) | 可変長 | `shrink_to` の共有時 |
| `HeapBuffer::realloc` の layout 変換 arm (heap_buffer.rs:402) | 可変長 | 32-bit のみ到達 |
| `Repr::push_str` (repr.rs:598) | 可変長 | 追記。短い帯が支配的 |
| `Repr::insert_str` (repr.rs:753) | 可変長 | 挿入する文字列側 |

`realloc` の中でヘッダ配置を変えるための `ptr::copy` (heap_buffer.rs:500/511/627) は
領域が重なるので対象外。`insert_str` の末尾ずらし (repr.rs:750) と `retain` /
`remove` の詰め直し (repr.rs:656/702) も同様。

CoW である分、clone がバッファを共有するので「確保してコピー」を通る頻度は
非 CoW のクレートより低い。効果はそちらでの報告より小さいはずで、
`from_str` の heap 経路と `push_str` の追記が主に恩恵を受ける。

## 検証方針

- asm: `LeanString::from(&str)` の heap 経路と `push_str` から `callq *memcpy` が
  消え、固定長のロード/ストアに置き換わること。64 バイト超では `memcpy` が残ること。
- criterion: `apis.rs` の `from` (17/256) と `push_str`、`push_str/after_clone`。
  `comparison.rs` の `Construct` / `Grow` / `CoW write`。
  17〜64 バイト帯を測れるよう、`apis.rs` の `from` の長さリストに 32 と 64 を足す
  (現状は 0/1/15/16/17/256 でこの帯に点が 1 つしかない)。
- Miri: 重ね合わせコピーの境界を踏むので全 4 ターゲット。

効果が誤差の範囲なら見送ってよい。その場合も「試して効かなかった」という記録を残す。

## 依存

[plans/03](./03-push-str-fast-path.md) と `push_str` の同じ行を触るので、どちらかを先に済ませる。
