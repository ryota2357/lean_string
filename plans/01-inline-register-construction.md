# 01: InlineBuffer をレジスタ上で組み立てる

種別: perf / 実装量: 中 / 設計判断: 小 (プロトタイプで解消済み)

## 背景

`553c73c` により `InlineBuffer::new` の可変長 memcpy 呼び出しは消えたが、
スタック上のバッファに書いてから 2 ワードで読み直す構造が残っている。

```asm
.LBB9_10:
	movq	(%rsp), %r15
	movq	8(%rsp), %r12
```

`len` が 2〜7 の arm では 2 バイト・4 バイトのストアに対して 8 バイトのロードが跨がるため、
x86 の store-to-load forwarding が成立しない。詳細と全体のダンプは
[research/codegen-baseline.md](../research/codegen-baseline.md) の §1 を参照。

`Repr` は 2 ワードしかないので、スタックを経由せずレジスタ内で 2 ワードを組み立てられる。

## 方針

`len` で場合分けし、両端からの重ね合わせロードで 2 ワードを作って `transmute` する。
`cfg(all(target_pointer_width = "64", target_endian = "little"))` でゲートし、
それ以外のターゲットは現行実装をそのまま残す。

```rust
#[inline]
#[cfg(all(target_pointer_width = "64", target_endian = "little"))]
pub(super) const unsafe fn new(text: &str) -> Self {
    use core::ptr::read_unaligned as load;
    debug_assert!(text.len() <= MAX_INLINE_SIZE);

    let len = text.len();
    let src = text.as_ptr();
    let tag = ((len as u64) | LastByte::MASK_1100_0000 as u64) << 56;

    let (w0, w1);
    unsafe {
        if len == MAX_INLINE_SIZE {
            // 16 バイトちょうどのときは最終バイトがデータそのものになる。
            // 有効な UTF-8 の末尾バイトは ASCII か継続バイトなので必ず 0xC0 未満であり、
            // `len()` 側の `min(last_byte - 0xC0, 16)` が 16 を返す (set_len と同じ規約)。
            w0 = load(src as *const u64);
            w1 = load(src.add(8) as *const u64);
        } else if len >= 8 {
            w0 = load(src as *const u64);
            w1 = if len == 8 {
                tag
            } else {
                // w0 と重なる in-bounds ロード。上位 (16 - len) バイトを捨てると
                // バイト [8, len) が下位に残り、最上位バイトが tag 用に空く。
                let tail = load(src.add(len - 8) as *const u64);
                (tail >> ((16 - len) * 8)) | tag
            };
        } else if len >= 4 {
            let head = load(src as *const u32) as u64;
            let tail = load(src.add(len - 4) as *const u32) as u64;
            w0 = head | (tail << ((len - 4) * 8));
            w1 = tag;
        } else if len >= 2 {
            let head = load(src as *const u16) as u64;
            let tail = load(src.add(len - 2) as *const u16) as u64;
            w0 = head | (tail << ((len - 2) * 8));
            w1 = tag;
        } else if len == 1 {
            w0 = *src as u64;
            w1 = tag;
        } else {
            w0 = 0;
            w1 = tag;
        }
        // `[u64; 2]` と `InlineBuffer` は同サイズで、little-endian ではワードの
        // バイト順とバッファのバイト順が一致する。
        mem::transmute([w0, w1])
    }
}
```

`len == 8` を分けているのは、`(16 - len) * 8` が 64 になりシフト幅の上限を超えるため。
すべてのロードは `0..len` の範囲に収まっており、範囲外は読まない。

## プロトタイプで確認したこと

上のコードを実際に `src/repr/inline_buffer.rs` へ当てて確認した結果。

### 1. `const fn` のまま書ける (分離は不要)

当初の懸念は「`read_unaligned` を使うと `const fn` にできず、`from_static_str` と
`INLINE_BUFFER_TRUE/FALSE` のために const 版を別に持つ必要がある」だったが、
`ptr::read_unaligned` も `mem::transmute` も const で使える。

- `cargo +1.85.1 build --all-features` (MSRV) が通ることを確認した。
- `const TRUE_BUF: InlineBuffer = unsafe { InlineBuffer::new("true") };` および
  16 バイトちょうどの文字列リテラルが const 評価できることを確認した。

したがって `new` / `new_const` を分ける必要はなく、既存の呼び出し元
(`from_static_str`、`INLINE_BUFFER_TRUE/FALSE`、`tests/const.rs`) をそのまま使える。

### 2. `#[inline]` が必須

`#[inline]` を付けずに書くと、関数が現行実装より大きいため LLVM が outline し、
**戻り値が sret 経由になって現行よりむしろ悪化する**。

```asm
; #[inline] なし: 呼び出し + スタック経由の受け取り
	leaq	8(%rsp), %rdi
	callq	*..InlineBuffer3new..E
	movq	8(%rsp), %r15
	movq	16(%rsp), %r12
```

現行実装は `#[inline]` なしでもインライン展開されていたので、ここは変更前後で
条件が変わる点として明記しておく。

### 3. 効果 (`LeanString::from(&str)`)

`#[inline]` を付けた場合、inline 経路のスタック往復は完全に消える。

```asm
; after: 各 arm がレジスタで w0/w1 を作り、共通の出口で戻り値へ 2 ストアするだけ
	cmpq	$16, %rdx
	jne	.LBB9_5
	movq	(%rsi), %r14          ; len == 16
	movq	8(%rsi), %r12
	jmp	.LBB9_16
...
.LBB9_5:
	movq	%rdx, %r12
	orq	$-64, %r12
	shlq	$56, %r12             ; tag
	cmpq	$7, %rdx
	jbe	.LBB9_6
	movq	(%rsi), %r14          ; len >= 8
	cmpq	$8, %rdx
	je	.LBB9_16
	movq	-8(%rsi,%rdx), %rax
	shll	$3, %edx
	negb	%dl
	movl	%edx, %ecx
	shrq	%cl, %rax
	orq	%rax, %r12
	jmp	.LBB9_16
...
.LBB9_16:
	movq	%r14, (%rbx)
	movq	%r12, 8(%rbx)
```

バッファのゼロ初期化ストア 2 本も消えている (`tag` 以外のビットが常に代入で埋まるため)。

### 4. `LeanString::from(char)` への波及

`from_char` は `encode_utf8` の結果を `InlineBuffer::new` に渡しているだけだが、
ASCII 経路は LLVM が定数畳み込みして完全にレジスタ内で終わるようになる。

```asm
probe_from_char:
	movq	%rdi, %rax
	cmpl	$128, %esi
	jae	.LBB8_1
	movl	%esi, %esi
	movabsq	$-4539628424389459968, %rdx   ; tag (len = 1)
	movq	%rsi, (%rax)
	movq	%rdx, 8(%rax)
	retq
```

現行では [research/codegen-baseline.md](../research/codegen-baseline.md) の §2 のとおり
14 命令に分割された転送が末尾に付いていたので、ここは大きく改善する。
非 ASCII 経路は `encode_utf8` がスタックの 4 バイトバッファに書く構造が残るため、
スタック往復は残る (Step 3 参照)。

### 5. ベンチマーク

`bench/benches/apis.rs` の `from` グループを、`current` = プロトタイプ、`prev` = 現行の
v0.7.0 (crates.io 版) として実行した結果 (criterion 中央値、`--warm-up-time 0.5
--measurement-time 1.5`)。

| len | prev (現行) | current (プロトタイプ) |
| --- | --- | --- |
| 0 | 7.4523 ns | **1.4128 ns** |
| 1 | 7.5023 ns | **1.4004 ns** |
| 15 | 7.7678 ns | **1.5827 ns** |
| 16 | 1.7752 ns | **1.1236 ns** |
| 17 | 13.748 ns | 13.734 ns |
| 256 | 15.069 ns | 14.934 ns |

現行では len 16 だけが 1.8 ns で、それ以外の inline 長 (0/1/15) が一律 7.5 ns 前後に
張り付いている。この段差は上で述べた forwarding の失敗と整合する。len 16 は
`movups`/`movaps` の 16 バイトストア 1 本なので、続く 8 バイトロード 2 本が
どちらも単一ストアに含まれ forwarding が成立する。他の長さは重ね合わせストアや
タグバイトの単独ストアを跨ぐため成立しない。

レジスタ構築にすると全長が 1.1〜1.6 ns に揃う。17 以降 (heap 経路) は変化なしで、
劣化していないことも確認できる。

### 6. 正しさ

- 長さ 0..=16 の全長と、マルチバイト文字を含む文字列 (16 バイトちょうどで末尾が
  継続バイトになるものを含む) について、現行実装と結果が一致することを確認した。
- 既存のテストスイート (`cargo test --release --all-features`) が全て通る。
- `MIRIFLAGS="-Zmiri-strict-provenance" cargo +nightly miri test --all-features` が
  x86_64 で通る (重なり合う unaligned read が strict provenance でも問題ないこと)。

## 手順

### Step 1: レジスタ構築版 `InlineBuffer::new`

上のコードを `#[inline]` 付きで入れる。既存実装は
`#[cfg(not(all(target_pointer_width = "64", target_endian = "little")))]` を付けて残す。

SAFETY コメントには arm ごとに「どのロードがなぜ in-bounds か」を書く。
`len == MAX_INLINE_SIZE` の arm については、最終バイトがタグではなくデータになる理由
(有効な UTF-8 の末尾バイトは 0xC0 未満) を `set_len` (inline_buffer.rs:77-83) の
同じ規約と対応付けて書く。

### Step 2: 境界テスト

長さ 0..=16 の全長を明示的に列挙するテストを `src/repr/inline_buffer.rs` の
ユニットテストとして足す。各 arm の境界 (0, 1, 2, 3, 4, 7, 8, 9, 15, 16) と、
16 バイトちょうどで末尾が継続バイトになるマルチバイト文字列を必ず含める。
すでに `tests/property.rs` が `String` との等価性を回しているが、arm の境界を
決め打ちで押さえるテストは別に置いたほうが退行時に原因が分かりやすい。

### Step 3: `from_char` の直接構築

`Repr::from_char` (src/repr.rs:91-98) は `encode_utf8` でスタック上の 4 バイトに
書いてから `&str` として渡している。Step 1 の後、ASCII 経路は畳み込まれるが
2〜4 バイト文字ではスタック往復が残る。

`char` を u32 の UTF-8 表現へレジスタ内で変換し、第 1 ワードへ直接置く形にできる。
分岐は `len_utf8()` の 4 通りで、`encode_utf8` と同じビット演算を自前で書くだけ。

Step 1 の asm を見てから判断する。ASCII 以外の `char` からの構築がどれだけ
現実的な負荷かは呼び出しパターン次第なので、Step 1 だけで打ち切る判断もあり得る。

### Step 4: heap 経路の確認

`LeanString::from(&str)` の heap 経路には `callq *memcpy` が残る。
これは [plans/02](./02-medium-copy.md) の題材なので、このプランでは触らない。
ただし Step 1 の後で heap arm のブロック配置 (現状 `.LBB9_1` として先頭寄りに置かれる)
が変わっていないかだけ確認する。

## 検証方針

- **asm**: `LeanString::from(&str)` と `From<char>` を x86-64 と aarch64 の両方で。
  確認事項は (1) inline 経路にスタックのストア/ロードが無い、(2) `#[inline]` が
  効いていて呼び出しが残っていない、の 2 点。before/after をコミットメッセージに残す。
- **Miri**: 4 ターゲット (64/32-bit × LE/BE) すべて。BE と 32-bit は
  フォールバック経路が壊れていないことの確認を兼ねる。
- **criterion**: `bench/benches/apis.rs` の `from` (0/1/15/16/17/256) と
  `comparison.rs` の構築系。inline 全長で効くはず。

## 依存

なし。単独で実施できる。
