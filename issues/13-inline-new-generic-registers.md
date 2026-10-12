# 32-bit / big-endian の `InlineBuffer::new` がスタック上のバッファを経由し、部分的に重なるストアの直後にワードで読み直す

- 種別: perf
- 対象: `src/repr/inline_buffer.rs` の `InlineBuffer::new` (`#[cfg(not(all(target_pointer_width = "64", target_endian = "little")))]` 版)
- 影響: 32-bit ターゲット (i686、armv7、wasm32 など) と big-endian ターゲットで、inline に収まる文字列からの構築
  (`LeanString::from(&str)`、`from_static_str`、`FromStr`、`into_lean_str` の inline 化など、`Repr::from_str` を通るものすべて)。
- 優先度: 低

## 問題

LE 64-bit 版の `InlineBuffer::new` は 2 ワードをレジスタで組み立てるが、それ以外のターゲット向けの版は
`[0u8; MAX_INLINE_SIZE]` をスタックに置き、最後のバイトにタグを書き、先頭と末尾から固定長のコピーを
重ねて書いてから、`Self(buffer)` として 2 ワードで読み直す。

`5dd94ea`、`--target i686-unknown-linux-gnu`、rustc 1.97.0、LTO なし。
`pub fn probe_from_static(s: &'static str) -> LeanString { LeanString::from_static_str(s) }` の inline 経路
(長さ 4〜7 の場合を抜粋):

```asm
	movl	$0, 4(%esp)
	movl	$0, (%esp)
	orb	$-64, %al
	movb	%al, 7(%esp)          # タグ (1 バイト)
	...
	movl	(%ecx), %eax
	movl	-4(%ecx,%edx), %ecx
	movl	%eax, (%esp)          # 先頭 4 バイト
	movl	%ecx, -4(%esp,%edx)   # 末尾 4 バイト: (%esp) と 4(%esp) の両方に部分的に重なる
	...
	movl	(%esp), %ecx          # 読み直し: 直前の 2 つのストアにまたがる
	movl	4(%esp), %edx         # 読み直し: 3 つのストアにまたがる
	movl	%ecx, (%eax)
	movl	%edx, 4(%eax)
```

長さ 1〜3 も同じ形 (1 / 2 バイトのストアの後に 4 バイトのロード) になる。読み直しのロードが
1 つのストアに収まっていないので、x86 ではストアバッファからの転送 (store-to-load forwarding) が
成立せず、ストアがキャッシュに書かれるまで待つ。`LeanString::from(&str)` の inline 経路
(`probe_from_str`) も同じ。

## 原因

汎用版は `ptr::copy_nonoverlapping` をバイト配列に対して行うので、値がメモリを経由する。LE 64-bit 版が
レジスタで組み立てているのは、ロードしたワードのバイト順がメモリ上の順と一致することを前提にしているからで、
その前提はロードとストアの前後で `from_le` / `to_le` を挟めばどのエンディアンでも成り立つ
(`InlineBuffer::from_char` がすでにこの方法で、エンディアンに依存しない 2 ワードの組み立てをしている)。
ワード幅も `usize` にすれば 32-bit でそのまま使える。

## 修正例

LE 64-bit 版を `usize` とエンディアン変換で一般化し、版を 1 つにまとめる。

```rust
#[inline]
pub(super) const unsafe fn new(text: &str) -> Self {
    debug_assert!(text.len() <= MAX_INLINE_SIZE);
    const W: usize = size_of::<usize>();
    const _: () = assert!(MAX_INLINE_SIZE == 2 * W);

    use core::ptr::read_unaligned as load;

    // Byte `i` in memory becomes bits `8 * i..8 * (i + 1)` of the result on any endianness.
    const unsafe fn load_w(p: *const u8) -> usize {
        usize::from_le(unsafe { load(p as *const usize) })
    }
    const unsafe fn load_32(p: *const u8) -> usize {
        u32::from_le(unsafe { load(p as *const u32) }) as usize
    }
    const unsafe fn load_16(p: *const u8) -> usize {
        u16::from_le(unsafe { load(p as *const u16) }) as usize
    }

    let len = text.len();
    let src = text.as_ptr();
    let last_byte = (len | LastByte::MASK_1100_0000 as usize) << (usize::BITS - 8);

    let (w0, w1);
    unsafe {
        if len == MAX_INLINE_SIZE {
            w0 = load_w(src);
            w1 = load_w(src.add(W));
        } else if len >= W {
            w0 = load_w(src);
            w1 = if len == W {
                last_byte
            } else {
                let tail = load_w(src.add(len - W));
                (tail >> ((MAX_INLINE_SIZE - len) * 8)) | last_byte
            };
        } else if W == 8 && len >= 4 {
            let head = load_32(src);
            let tail = load_32(src.add(len - 4));
            w0 = head | (tail << ((len - 4) * 8));
            w1 = last_byte;
        } else if len >= 2 {
            let head = load_16(src);
            let tail = load_16(src.add(len - 2));
            w0 = head | (tail << ((len - 2) * 8));
            w1 = last_byte;
        } else if len == 1 {
            w0 = *src as usize;
            w1 = last_byte;
        } else {
            w0 = 0;
            w1 = last_byte;
        }
        mem::transmute([w0.to_le(), w1.to_le()])
    }
}
```

試した結果:

- x86-64: `probe_from_str` / `probe_from_static` / `probe_from_utf8` の asm はラベル名以外いまと同一
  (`from_le` / `to_le` は no-op)。
- i686: inline 経路からスタックのバッファが消え、2 ワードをレジスタで作って 2 回ストアするだけになる
  (長さ 4〜7 の部分):

  ```asm
  	movl	%ecx, %ebp
  	orl	$-64, %ebp
  	shll	$24, %ebp              # タグ
  	cmpl	$3, %ecx
  	jbe	...
  	movl	(%edx), %edi
  	cmpl	$4, %ecx
  	je	...
  	movl	-4(%edx,%ecx), %eax
  	shll	$3, %ecx
  	negb	%cl
  	shrl	%cl, %eax
  	orl	%eax, %ebp
  ```

  関数全体の静的な命令数は `probe_from_static` 55 → 61、`probe_from_str` 112 → 114 と少し増える
  (分岐ごとに末尾の 2 ストアが複製され、callee-saved レジスタの退避が 1 つ増える) が、どの長さでも
  実行されるのは「ロード 1〜2 回 + シフト/OR + 2 ストア」で、スタックへのストアと読み直しが無くなる。
- Miri (`-Zmiri-strict-provenance`): `tests/const.rs`、`tests/lean_string.rs`、`tests/lean_str.rs` が
  powerpc64 / powerpc (big-endian) と i686 / x86_64 で通った。`const fn` のまま
  (`INLINE_BUFFER_TRUE` などの定数評価も通る)。

wasm32 / armv7 の asm は見ていない (ターゲット未導入)。wasm には store-to-load forwarding に相当する
制約は無いが、ロード・ストアの数自体は減るはずである (推測)。

## 確認方法

- `cargo rustc --release --lib --target i686-unknown-linux-gnu -- --emit asm` で `from_str` /
  `from_static_str` の inline 経路にスタックへのストアが無いこと。x86-64 の asm が変わらないこと。
- Miri を CI の 4 ターゲット (特に powerpc64 / powerpc) で。長さ 0〜`MAX_INLINE_SIZE` の全長は
  `tests/const.rs` の `from_every_inline_length_in_const` (const 評価) と `tests/lean_string.rs` の
  全長ループ (実行時) がすでに通しているので、これらが通ればよい。
