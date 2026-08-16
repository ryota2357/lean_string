# 02: 中サイズのコピーから memcpy 呼び出しを外す

種別: perf / 実装量: 小 / 設計判断: 小

## 背景

`ptr::copy_nonoverlapping` は長さがコンパイル時定数でないと `memcpy` の関数呼び出しに
落ちる。`553c73c` は `InlineBuffer::new` に対してこれを長さで分岐する固定長コピーに置き換えて回避したが、
heap 経路と追記経路には可変長 memcpy が残っている。

現行の asm では次の 2 か所で確認できる
([research/codegen-baseline.md](../research/codegen-baseline.md) の §1 と §3)。

- `LeanString::from(&str)` の heap 経路: `callq *memcpy@GOTPCREL(%rip)`
- `Repr::push_str` の追記コピー: `callq *memcpy@GOTPCREL(%rip)`

前者は「inline に収まらない直後の長さ帯 (17〜64 バイト)」が新規ヒープ文字列として
もっとも多い長さなので、call 境界のコストが相対的に大きい。後者は inline バッファへの
数バイトの追記でも call を跨ぐ。

## 方針

長さの帯ごとに固定長の重ね合わせコピーを使い、64 バイト超だけ `memcpy` に委譲する
ヘルパを `src/repr.rs` に置く。

```rust
/// `src` から `dst` へ `len` バイトをコピーする。ヒープ経路と追記経路向け。
///
/// 可変長の `copy_nonoverlapping` は `memcpy` の呼び出しになる。新規ヒープ文字列で
/// もっとも多いのは inline 上限直後の長さなので、[16, 64] の帯は両端からの固定長コピー
/// 2 回で済ませて call 境界を避ける。64 バイト超は CPU ごとにチューニングされた
/// `memcpy` の大サイズ経路に任せたほうが速いので委譲する。16 バイト未満は
/// `MAX_INLINE_SIZE` 以下のコピー用の分岐 (`InlineBuffer::new` と同じ形) を使う。
///
/// # Safety
/// - `src` は `len` バイトの読み出しに対して有効。
/// - `dst` は `len` バイトの書き込みに対して有効。
/// - `src` と `dst` は重ならない。
#[inline]
unsafe fn copy_bytes(src: *const u8, dst: *mut u8, len: usize) { ... }
```

境界の 16 / 32 / 64 は SIMD レジスタ幅に合わせたもので、`MAX_INLINE_SIZE` とは独立に
決まる。`compact_str` が `copy_medium` として使っている値をそのまま採る。

`< 16` の帯をどうするかは 2 案ある。

- 案A: `InlineBuffer::new` と同じ 8/4/2/1 の固定長コピーを続ける。
  追記経路 (`push_str`) では短い追記が支配的なのでこちらが効く。
- 案B: `memcpy` に委譲する (`compact_str` の `copy_medium` はこちら)。
  ヒープ経路だけを対象にするなら短い帯は稀なので委譲で十分。

`push_str` にも使うなら案A。ヘルパを 1 つにまとめられるので、まず案A で書いて、
`< 16` の帯を足したことで heap 経路の命令数が増えていないかを asm で確認する。

## 適用先

| 場所 | 現状 | 備考 |
| --- | --- | --- |
| `HeapBuffer::new` (heap_buffer.rs:136) | 可変長 | `from_str` の heap 経路。最頻 |
| `HeapBuffer::with_additional` (heap_buffer.rs:375) | 可変長 | `reserve` の unshare / inline→heap 昇格 |
| `HeapBuffer::with_exact_capacity` (heap_buffer.rs:347) | 可変長 | `shrink_to` の共有時 |
| `HeapBuffer::realloc` の layout 変換 arm (heap_buffer.rs:402) | 可変長 | 32-bit のみ到達 |
| `Repr::push_str` (repr.rs:588) | 可変長 | 追記。短い帯が支配的 |
| `Repr::insert_str` (repr.rs:743) | 可変長 | 挿入する文字列側 |

`realloc` の中でヘッダ配置を変えるための `ptr::copy` (heap_buffer.rs:500/511/627) は
領域が重なるので対象外。`insert_str` の末尾ずらし (repr.rs:740) も同様。

CoW である分、`compact_str` より「確保してコピー」の経路を通る頻度は低い
(clone は参照カウントの増加で済む)。したがって効果は `compact_str` での報告より小さいはずで、
`from_str` の heap 経路と `push_str` の追記が主に恩恵を受ける。

## 検証方針

- **asm**: `LeanString::from(&str)` の heap 経路と `push_str` から `callq *memcpy` が
  消え、固定長のロード/ストアに置き換わること。64 バイト超では `memcpy` が残ること。
- **criterion**: `apis.rs` の `from` (17/256) と `push_str`、`push_str/after_clone`。
  `comparison.rs` の `Construct` / `Grow` / `CoW write`。
  17〜64 バイト帯を測れるよう、`from` の長さリストに 32 と 64 を足すことを検討する
  (現状は 0/1/15/16/17/256 で、この帯にちょうど点がない)。
- **Miri**: 重ね合わせコピーの境界を踏むので全 4 ターゲット。

効果が誤差の範囲なら見送ってよい。その場合も「試して効かなかった」という記録を
残す (このリポジトリの方針として、意味もなく速くなるだけの変更は採らない)。

## 依存

[plans/01](./01-inline-register-construction.md) の後だと、inline 経路の asm が安定して
heap 経路の差分が読みやすい (soft)。
[plans/03](./03-push-str-fast-path.md) と `push_str` の同じ行を触るので、
どちらかを先に済ませる。
