# 整数の変換がスタックのバッファに桁を書いてから読み直し、16 バイトの結果を細切れのストアで書き出す

- 種別: perf
- 対象: `src/repr/num_to_repr.rs` の `impl_NumToRepr_for_integers!` (`NumToRepr::into_repr`)、`src/repr.rs` の `Repr::new_with`
- 影響: `to_lean_string()` / `to_lean_str()` で整数を変換するすべての呼び出し。結果が inline に収まる場合
  (32-bit 以下の整数はすべて、64-bit 整数は 16 文字以下) に、スタック経由の読み直しで store-to-load
  forwarding が成立しない。`i16` / `i32` / `i64` / `u64` では `into_repr` が呼び出し側に展開されない。
- 優先度: 中

## 問題

`into_repr` は `Repr::new_with(digits_count, |buf| ...)` で、`InlineBuffer::empty()` (スタック上の 16 バイト)
に LUT から 2 バイトずつ桁を書き、`set_len` で最後のバイトを書いてから、それを `Repr` として読み直す。

`5dd94ea`、x86-64、rustc 1.97.0、LTO なし。下流のクレートに
`pub fn one_u32(n: u32) -> LeanString { n.to_lean_string() }` のようなラッパを型ごとに 2 つずつ置いて
asm を出した (呼び出し元が 1 つだけだとインライン化の判定が大きく甘くなるので、2 つにしている)。
`u32` の最後の部分:

```asm
	movw	%dx, -18(%rsp,%rsi)   # 桁を 2 バイトずつスタックへ
	...
	orb	$-64, %cl
	movb	%cl, -1(%rsp)         # set_len
	movq	-16(%rsp), %rcx       # 直前の 2 バイト / 1 バイトのストアにまたがるロード
	movq	%rcx, (%rax)
	movl	-8(%rsp), %ecx
	movl	%ecx, 8(%rax)
	movzwl	-4(%rsp), %ecx
	movw	%cx, 12(%rax)
	movzbl	-2(%rsp), %ecx
	movb	%cl, 14(%rax)
	movzbl	-1(%rsp), %ecx
	movb	%cl, 15(%rax)
```

- 16 バイトの結果が 8 / 4 / 2 / 1 / 1 バイトの 5 回のストアに分かれる。
- 読み直しの `movq -16(%rsp)` などは、直前に書いた複数の 2 バイト / 1 バイトのストアにまたがるので、
  x86 では store-to-load forwarding が成立しない。aarch64 でも同様に `strh` / `strb` の直後に
  `ldr q0, [sp]` で読み直している。

`into_repr` 本体の大きさと、呼び出し側に展開されるか (`num2` クレート、型ごとに呼び出し元 2 つ):

| 型 | x86-64 | aarch64 |
| --- | --- | --- |
| `u32` | 展開される (`to_lean_string::<u32>` 95 命令の中) | 展開される (`to_lean_string::<u32>` 123 命令の中) |
| `i32` | `callq into_repr` (本体 138 命令) | `bl into_repr` (本体 170 命令) |
| `i16` | `callq into_repr` (本体 101 命令) | `bl into_repr` (本体 120 命令) |
| `u64` | `callq into_repr` (本体 233 命令) | `bl into_repr` (本体 299 命令) |
| `i64` | `callq into_repr` (本体 314 命令) | `bl into_repr` (本体 417 命令) |

呼び出し元が 1 つだけのときは `u64` (233) は展開されず `usize` (238) は展開される、といった具合に
判定が揺れる。`into_repr` は `M: Mutability` で generic なので MIR は下流に出ており、`#[inline]` の有無ではなく
大きさで展開が決まっている。展開されない場合、`into_repr` 自身の 5 分割ストアに加え、呼び出し側でも
結果をもう一度コピーする ([#17](./17-to-lean-string-codegen.md))。

なお、桁数から inline に収まるかを LLVM はすでに導けており、32-bit 以下の型では heap の分岐は消えている。

## 原因

`new_with` の inline 分岐は「スタック上のバッファに任意のバイト列を書いてもらい、`Repr` として読み直す」
汎用の作りなので、値が必ずメモリを経由する。LLVM はバイト単位のストアの後のワード単位の読み直しを
レジスタ上の値にまとめられず、`Repr` のフィールドごとにコピーしている。8 / 4 / 2 / 1 / 1 バイトという
分割は、`Repr` の `(*const (), [u8; 7], LastByte)` のうち `*const ()` (8)、`[u8; 7]` を 2 の冪に分けた
4 + 2 + 1、`LastByte` (1) に一致する (asm からの推測。x86-64 ではフィールドごとのコピーになり、aarch64 では
16 バイト 1 組のコピーになるが、どちらもバイト単位のストアの直後の読み直しである点は同じ)。

また、64-bit 整数では 16 桁以下 (inline) と 17 桁以上 (heap) の両方の経路を `new_with` のクロージャ 1 つで
抱えるので、`into_repr` が大きくなり展開されにくい。

## 修正例

結果が inline に収まる場合は、`InlineBuffer::from_char` と同じく 2 ワードをレジスタで組み立てる。
桁は下の位から作るので、「それまでの桁を上位へずらして、新しい桁を下位に入れる」とすれば
長さを知らなくても書く位置が決まる (可変量のシフトが要らない)。長さは最上位の非ゼロバイトの位置から
求められる (数字も `-` もゼロではない)。

```rust
macro_rules! impl_to_inline {
    ($name:ident, $u:ty) => {
        /// Formats `n`, preceded by `-` if `negative`, into an `InlineBuffer` whose two words are
        /// assembled in registers.
        ///
        /// # Safety
        /// The text must fit in `MAX_INLINE_SIZE` bytes.
        #[inline]
        unsafe fn $name(mut n: $u, negative: bool) -> InlineBuffer {
            #[inline(always)]
            fn pair(d: $u) -> u128 {
                let i = d as usize * 2;
                u16::from_le_bytes([DEC_DIGITS_LUT[i], DEC_DIGITS_LUT[i + 1]]) as u128
            }

            // Byte `i` of the text is bits `8 * i..8 * (i + 1)` of `acc`. Digits are produced from
            // the least significant one, and each step shifts the earlier (less significant) ones
            // toward the end of the text.
            let mut acc: u128 = 0;
            while n >= 10000 {
                let rem = n % 10000;
                n /= 10000;
                acc = (acc << 32) | pair(rem / 100) | (pair(rem % 100) << 16);
            }
            if n >= 100 {
                acc = (acc << 16) | pair(n % 100);
                n /= 100;
            }
            acc = if n < 10 { (acc << 8) | (b'0' + n as u8) as u128 } else { (acc << 16) | pair(n) };
            if negative {
                acc = (acc << 8) | b'-' as u128;
            }

            // Every byte of the text is nonzero, so the text ends at the highest nonzero byte.
            let len = (u128::BITS - acc.leading_zeros()).div_ceil(8) as usize;
            // A full buffer has no length byte: its last byte is part of the text.
            let tag = if len < MAX_INLINE_SIZE {
                (len | LastByte::MASK_1100_0000 as usize) << (usize::BITS - 8)
            } else {
                0
            };
            let w0 = acc as usize;
            let w1 = (acc >> usize::BITS) as usize | tag;
            // SAFETY: The first `len` bytes are ASCII digits (and the sign), the rest are zero, and
            // the last byte records `len` as an inline length unless the buffer is full.
            unsafe { mem::transmute([w0.to_le(), w1.to_le()]) }
        }
    };
}
impl_to_inline!(u32_to_inline, u32);
impl_to_inline!(u64_to_inline, u64);
```

`into_repr` の先頭で使い分ける (`$u` はいまのマクロの引数)。

```rust
#[allow(unused_comparisons)]
const MAX_TEXT_LEN: usize = <$t>::MAX.ilog10() as usize + 1 + (<$t>::MIN < 0) as usize;
#[allow(unused_comparisons)]
let negative = self < 0;
let abs = if negative { (!(self as $u)).wrapping_add(1) } else { self as $u };
if MAX_TEXT_LEN <= MAX_INLINE_SIZE {
    // 常に inline: 桁数の計算も要らない
    // SAFETY: The text is at most `MAX_TEXT_LEN <= MAX_INLINE_SIZE` bytes.
    return Ok(Repr::from_inline(unsafe { u32_to_inline(abs as u32, negative) }));
}
let digits_count = DigitCount::digit_count(self);
if digits_count <= MAX_INLINE_SIZE {
    // SAFETY: The text is `digits_count <= MAX_INLINE_SIZE` bytes.
    return Ok(Repr::from_inline(unsafe {
        if size_of::<$u>() <= 4 { u32_to_inline(abs as u32, negative) }
        else { u64_to_inline(abs as u64, negative) }
    }));
}
// 17 文字以上: いまの `new_with` の経路 (heap しか通らなくなる)
```

`DigitCount` の `digits_count` は、外側の分岐 (inline か heap か) と heap の長さにだけ使う。
32-bit ターゲットでは `MAX_INLINE_SIZE` が 8 なので、`u32` / `i32` は 2 段目に回り、`$u = u32` の版を使う。

試作の結果 (x86-64 / aarch64、LTO なし、呼び出し元 2 つ):

| 型 | x86-64 | aarch64 |
| --- | --- | --- |
| `u32` | 呼び出し元に展開、計 100 命令、スタック不使用、`movq` 2 回で書き出し | `into_repr` 67 命令 (いま 123 の関数の中)、`stp` 1 回で書き出し |
| `i32` | `to_lean_string::<i32>` (97 命令) に `into_repr` が展開され、sret へ直接 `movq` 2 回 | `into_repr` 75 命令 (いま 170) |
| `i16` | 呼び出し元に展開 (70 命令) | 呼び出し元に展開 (66 命令) |
| `u64` | `into_repr` 172 命令 (いま 233)、inline の場合はレジスタで 2 ストア | 202 命令 (いま 299) |
| `i64` | `into_repr` 213 命令 (いま 314) | 247 命令 (いま 417) |

呼び出し元が 1 つの場合 (`probe` クレート、[#17](./17-to-lean-string-codegen.md) の修正も入れた状態) は、x86-64 で
`u8`〜`i32` と `NonZero` がすべて展開され (`i32` 98 命令、`u32` 85 命令)、`into_repr` の呼び出しが残るのは
`u64` / `i64` だけになった。aarch64 では `u32` / `i32` の `into_repr` は展開されないままだが、本体が半分程度になり、
スタックを使わなくなる。

`u32` だけを残して `DigitCount` で長さを先に求める版 (タグを先に作る版) も試したが、桁数の比較の連鎖
(`i32` は 20 個) が残り、`u32` の `into_repr` が下流で展開されなくなった (x86-64 で本体 107 命令の `callq`)。
長さを最上位の非ゼロバイトから求める形にしたことで、比較の連鎖が消えて展開されるようになった。

ベンチ (`bench/benches/apis.rs` の `to_lean_string` に値を足し、`current` = 試作、`prev` = 0.8.0 (= `main`)
を同じバイナリで比較。LTO あり、4 コアの負荷のある共有マシンなので 1 割程度の揺れは誤差):

| 値 | 経路 (試作) | current 1 回目 | prev 1 回目 | current 2 回目 | prev 2 回目 |
| --- | --- | --: | --: | --: | --: |
| `1_234_567_u32` | 32-bit 以下 | 11.7 ns | 14.2 ns | 7.9 ns | 10.8 ns |
| `42_u32` | 32-bit 以下 | 6.8 ns | 13.5 ns | 3.6 ns | 9.0 ns |
| `u32::MAX` | 32-bit 以下 | 12.6 ns | 18.6 ns | 9.3 ns | 11.2 ns |
| `-1_234_567_i32` | 32-bit 以下 | 12.4 ns | 23.2 ns | 9.9 ns | 17.5 ns |
| `i32::MIN` | 32-bit 以下 | 15.7 ns | 27.1 ns | 10.2 ns | 16.2 ns |
| `12_345_u16` | 32-bit 以下 | 7.2 ns | 13.5 ns | 5.2 ns | 9.4 ns |
| `1_234_567_u64` | 64-bit、inline | 18.6 ns | 20.2 ns | 14.5 ns | 17.5 ns |
| `-9_876_543_210_i64` | 64-bit、inline | 16.6 ns | 25.4 ns | 11.5 ns | 14.3 ns |
| `u64::MAX` | 64-bit、heap (変更なし) | 39.5 ns | 42.1 ns | 35.7 ns | 38.6 ns |
| `12_345.678_9_f64` | 変更なし | 29.0 ns | 26.5 ns | 24.9 ns | 24.7 ns |

(criterion の中央値。2 回の測定で負荷が違うので、列をまたいだ比較はできない。コードが変わらない
`u64::MAX` / `f64` の差が誤差の目安。この測定の `current` には [#17](./17-to-lean-string-codegen.md) の試作も
入っている (効果は分離していない)。)
32-bit 以下の型ではどちらの回でも 2 割弱〜6 割短くなっている。

テスト: `tests/property.rs` の整数のテスト、[#28](./28-int-boundary-tests.md) の境目のテスト、`tests/alloc_string.rs` が
通った。Miri (`-Zmiri-strict-provenance`) で境目のテストと `tests/const.rs` / `tests/lean_string.rs` /
`tests/lean_str.rs` が x86_64 / i686 / powerpc64 / powerpc で通った。

## 確認方法

- asm: `n.to_lean_string()` (`u32` / `i32` / `u64` の小さい値) の inline 経路にスタックへのストアと
  読み直しが無く、結果が 2 ワードのストア (x86-64 は `movq` 2 回、aarch64 は `stp` 1 回) で書かれること。
  x86-64 で `u32` と `i32` の `into_repr` が呼び出し元 (呼び出し元が 2 つ以上ある場合を含む) に展開されていること。
- テスト: [#28](./28-int-boundary-tests.md) の境目のテスト。とくに 16 文字ちょうど (`i64` の `-999999999999999`、
  `u64` の `9999999999999999`) はタグを書かない分岐を通る唯一のケースで、既存のテストでは踏まれない。
- Miri: big-endian (powerpc64 / powerpc) で `to_le` の扱いを確かめる。
- ベンチ: `apis.rs` の `to_lean_string` に `i32`、短い値 (`42_u32`)、`u32::MAX`、`i32::MIN`、16 桁以下の
  `u64` を足す。`comparison.rs` の `Numbers` も。
