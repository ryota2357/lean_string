# `repeat` の結果が inline に収まるとき、スタック上のバッファへの短い `memcpy` を log n 回呼ぶ

- 種別: perf
- 対象: `src/repr.rs` の `Repr::repeat` (と `Repr::new_with` の inline 分岐)
- 影響: `" ".repeat(indent)` のように、結果が 16 バイト (32-bit では 8 バイト) 以下になる `repeat`。
  `LeanString::from` で同じ長さの文字列を作る場合の 5〜8 倍の命令数になる。優先度は低い。
- 優先度: 低

## 問題

`Repr::repeat` は `Repr::new_with(len, |dst| ...)` の中で、先頭に `text` をコピーしてから、埋まった部分を
倍々にコピーする。inline の場合、`dst` はスタック上の `InlineBuffer::empty()` なので、長さが実行時にしか
決まらないコピーがすべて `memcpy` の呼び出しになる。

`5dd94ea`、x86-64、rustc 1.97.0、LTO なし。`pub fn probe_repeat(s: &LeanString, n: usize) -> LeanString { s.repeat(n) }`
の inline 分岐:

```asm
	movq	$0, 15(%rsp)
	movq	$0, 8(%rsp)
	movb	$-64, 23(%rsp)
	...
	leaq	8(%rsp), %rdi
	callq	*memcpy@GOTPCREL(%rip)          # text をコピー
.LBB40_24:                                  # 倍々のループ
	...
	callq	*%rbp                           # memcpy
	addq	%r13, %r14
	cmpq	%r15, %r14
	jb	.LBB40_24
	...
	movq	8(%rsp), %r12                    # 読み直し
	movq	16(%rsp), %rax
```

公開 API だけを使う小さなバイナリで、`#[inline(never)]` のラッパの中の命令数を callgrind で数えた
(1 回あたり):

| 結果の長さ | `LeanString::from(" ").repeat(len)` | `LeanString::from("ab").repeat(len / 2)` | `LeanString::from(&"   ..."[..len])` |
| --: | --: | --: | --: |
| 4 | 132 | 105 | 36 |
| 8 | 160 | 133 | 28 |
| 16 | 184 | 157 | 23 |

1 バイトを 16 回にするには `memcpy` を 5 回 (1 + 1 + 2 + 4 + 8 バイト) 呼ぶ。最後は 1 バイトや 2 バイトの
ストアの直後にワードで読み直すので、store-to-load forwarding も成立しない。

## 原因

`new_with` の inline 分岐は、任意の書き込みをスタック上のバッファで受けてから読み直す汎用の作りで、
`repeat` の倍々コピーは長さが実行時に決まるので `memcpy` になる。heap の場合は倍々コピーでよいが、
16 バイト以下ではレジスタで済む処理である。

## 修正例

結果が inline に収まる場合は、`n >= 2` なので `text` は `MAX_INLINE_SIZE / 2` 以下 (1 ワード) に収まる。
`text` を 1 ワードとして読み、`acc |= acc << (filled * 8)` で倍々に広げてから 2 ワードに分ける。

```rust
pub(crate) fn repeat(&self, n: usize) -> Result<Self, ReserveError> {
    if n == 1 {
        return Ok(self.make_shallow_clone());
    }
    let text = self.as_bytes();
    let len = text.len().checked_mul(n).ok_or(ReserveError)?;

    if len <= MAX_INLINE_SIZE {
        if len == 0 {
            return Ok(Repr::new());
        }
        // `n >= 2`, so `text.len() <= MAX_INLINE_SIZE / 2`, which fits in one word.
        // SAFETY: `text.len() <= MAX_INLINE_SIZE`.
        let [t, _]: [usize; 2] = unsafe { mem::transmute(InlineBuffer::new(self.as_str())) };
        let t = usize::from_le(t);
        let t = if text.len() == size_of::<usize>() { t } else { t & ((1 << (text.len() * 8)) - 1) };
        let mut acc = t as u128;
        let mut filled = text.len();
        while filled < len {
            acc |= acc << (filled * 8);
            filled *= 2;
        }
        if len < MAX_INLINE_SIZE {
            acc &= (1u128 << (len * 8)) - 1;
        }
        let tag = if len < MAX_INLINE_SIZE {
            (len | LastByte::MASK_1100_0000 as usize) << (usize::BITS - 8)
        } else {
            0
        };
        let w0 = acc as usize;
        let w1 = (acc >> usize::BITS) as usize | tag;
        // SAFETY: The first `len` bytes are `n` copies of `text`, the rest are zero, and the last
        // byte records the length unless the buffer is full.
        return Ok(Repr::from_inline(unsafe { mem::transmute([w0.to_le(), w1.to_le()]) }));
    }
    // heap: いまのまま
    ...
}
```

試作の結果、上の表の値は次のようになった (callgrind、同じ条件)。

| 結果の長さ | `" "` | `"ab"` |
| --: | --: | --: |
| 4 | 132 → 107 | 105 → 95 |
| 8 | 160 → 122 | 133 → 110 |
| 16 | 184 → 123 | 157 → 111 |

`memcpy` の呼び出しは無くなるが、`InlineBuffer::new` の長さの分岐や `as_bytes` の判別などが残るので、
`from` と同等にはならない。`len == 0` を先に返しているのは、`n == 0` で `text` が inline に収まらない長さの
場合に `InlineBuffer::new` の前提が崩れるため (試作の最初の版はこれで `tests/lean_string.rs` の
`repeat_zero_or_empty` の debug チェックに引っかかった)。`tests/lean_string.rs` / `tests/lean_str.rs` の
`repeat` と `tests/property.rs` の `repeat` は通った。

効果が小さいわりにコードが増えるので、`repeat` の短い結果が実際に頻出する用途が無ければ見送ってもよい。

## 確認方法

- asm: `probe_repeat` の inline 分岐に `memcpy` の呼び出しとスタックへのストアが無いこと。
- テスト: `repeat` の結果がちょうど 16 バイト (タグなし)、15 バイト、`n == 0` で `text` が heap / static、
  `text` が空、を含む列挙テスト。Miri を big-endian でも (`to_le` / `from_le` の扱い)。
