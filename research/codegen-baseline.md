# 現行コードの codegen 実測

調査日: 2026-08-16 / 対象: `8f7fa75` (v0.7.0 + bench deps 更新)

以降の各プランは、この文書で測った asm を出発点にしている。実装後の before/after 比較にもここの
ダンプを使えるようにするため、引用は加工せずそのまま貼ってある。

## 測定方法

lean_string を path 依存で参照するだけの小さなクレートを作り、確認したい呼び出しを
`#[unsafe(no_mangle)]` の関数として置いた。

```toml
# Cargo.toml
[dependencies]
lean_string = { path = "../lean_string" }

[profile.release]
opt-level = 3
codegen-units = 1
lto = false
```

```rust
#[unsafe(no_mangle)]
pub fn probe_from_str(s: &str) -> LeanString { LeanString::from(s) }
// push_str / push / from_char / drop / eq / extend も同様
```

```sh
cargo asm --lib --simplify probe_from_str
```

- rustc 1.94.1, x86-64, opt-level=3, codegen-units=1, LTO なし
- LTO を切っているのは、下流クレートから見たときに `#[inline]` がどこまで効いているかを
  観察したいため。crate 内部の呼び出しがすべて潰れた状態では、関数の大きさが原因の
  インライン失敗が見えなくなる。
- 別のクレートを作らずに `src/lib.rs` へ直接 `#[inline(never)]` の関数を置いて
  `cargo asm` する方法でも同じことができる。この場合はクレート内部の呼び出しになるので、
  上の「下流から見た `#[inline]` の効き方」は測れない点だけ違う。
- 以下に貼ったダンプは AT&T 構文 (`cargo rustc --release -- --emit asm` の出力)。
  `cargo asm` の既定は Intel 構文だが、命令列は一致することを
  `probe_from_str` / `probe_drop_one` / `probe_eq` で確認した。

## 1. `LeanString::from(&str)` — inline 経路

`553c73c` (perf: copy inline bytes with constant-size copies) により、可変長 memcpy の
関数呼び出しは消えている。長さで分岐する固定長コピーがそのまま展開される。

```asm
probe_from_str:
	cmpq	$17, %rdx
	jae	.LBB9_1              # heap 経路へ
	movq	$0, 7(%rsp)          # スタック上のバッファをゼロ初期化
	movq	$0, (%rsp)
	movl	%edx, %eax
	orb	$-64, %al
	movb	%al, 15(%rsp)        # 長さタグ
	cmpq	$16, %rdx
	jne	.LBB9_5
	movups	(%rsi), %xmm0        # len == 16
	movaps	%xmm0, (%rsp)
	jmp	.LBB9_10
.LBB9_5:
	cmpq	$7, %rdx
	jbe	.LBB9_6
	movq	(%rsi), %rax         # len >= 8: 8 バイト × 2 の重ね合わせコピー
	movq	%rax, (%rsp)
	movq	-8(%rsi,%rdx), %rax
	movq	%rax, -8(%rsp,%rdx)
	jmp	.LBB9_10
	...
.LBB9_10:
	movq	(%rsp), %r15         # ★スタックからワード単位で読み直す
	movq	8(%rsp), %r12
.LBB9_11:
	movq	%r15, (%rbx)
	movq	%r12, 8(%rbx)
```

残っている問題は最後の 2 命令 (`.LBB9_10`) で、複数回に分けて書いたスタック上のバッファを
ワード単位で読み直している。x86 の store-to-load forwarding は、ロードが単一のストアに
完全に含まれていないと成立しない。

この条件を満たすのは `len == 16` の arm だけである。そこは `movups`/`movaps` の
16 バイトストア 1 本がバイト 0..15 をまとめて上書きするので、続く 2 本の 8 バイトロードは
どちらも単一のストアに収まる。それ以外の長さでは、第 2 ワード (バイト 8..15) のロードが
「データをコピーしたストア」と「バイト 15 に長さタグを書いたストア」の少なくとも 2 本に
跨がるため forwarding に失敗し、ストアがキャッシュに書かれるのを待つことになる。
`len` が 8 未満の arm では、第 1 ワードのロードもゼロ初期化のストアと部分ワードの
コピーに跨がる。

この段差はベンチにも現れている。`from` の測定では len 16 だけが 1.8 ns で、
0 / 1 / 15 は一律 7.5 ns 前後になる
([plans/01](../plans/01-inline-register-construction.md) の「プロトタイプで確認したこと」参照)。

`Repr` は 2 ワードしかないので、スタックを経由せずレジスタ内でこの 2 ワードを組み立てられる。
これが [plans/01](../plans/01-inline-register-construction.md) の題材。

heap 経路 (`.LBB9_1`) には可変長の `callq *memcpy@GOTPCREL(%rip)` が残っている。

なお `Result<Repr, ReserveError>` の Err arm を呼び出し側で再検査する分岐は、
`7fde9c5` と `2878dfe` の `assert_unchecked` により消えている (`.LBB9_12` の
`unwrap_with_msg` へ飛ぶのは alloc 失敗時のみ)。

## 2. `LeanString::from(char)`

構築系でもっとも codegen が悪い。`Repr::from_char` (src/repr.rs:91-98) が
`char::encode_utf8` でスタック上の 4 バイトバッファに書き、それを `&str` として
`InlineBuffer::new` に渡すため、スタックへの往復が 2 回発生する。

```asm
.LBB8_5:
	movb	%cl, (%rsi)           # encode_utf8 の書き込み
	movq	$0, -17(%rsp)
	movq	$0, -22(%rsp)
	leal	-64(%rdx), %ecx
	movb	%cl, -9(%rsp)
	movzwl	-28(%rsp), %ecx       # 4 バイトバッファから読み直し
	movw	%cx, -24(%rsp)        # InlineBuffer へ書き込み
	movzwl	-30(%rsp,%rdx), %ecx
	movw	%cx, -26(%rsp,%rdx)
.LBB8_7:
	movzbl	-24(%rsp), %ecx       # ★InlineBuffer から戻り値へ、7 回に分割して転送
	movb	%cl, (%rax)
	movq	-23(%rsp), %rcx
	movq	%rcx, 1(%rax)
	movzbl	-15(%rsp), %ecx
	movb	%cl, 9(%rax)
	movzwl	-14(%rsp), %ecx
	movw	%cx, 10(%rax)
	movzwl	-12(%rsp), %ecx
	movw	%cx, 12(%rax)
	movzbl	-10(%rsp), %ecx
	movb	%cl, 14(%rax)
	movzbl	-9(%rsp), %ecx
	movb	%cl, 15(%rax)
	retq
```

`.LBB8_7` の 14 命令は 16 バイトを丸ごと転送しているだけだが、バイト単位のストアが
混ざっているせいで LLVM がワードにまとめられていない。`char` は最大 4 バイトなので、
UTF-8 エンコード結果を u32 として組み立てれば第 1 ワードに直接置ける。
[plans/01](../plans/01-inline-register-construction.md) の Step 3 で扱う。

## 3. `LeanString::push_str`

`Repr::push_str` には `#[inline]` が付いているが、**下流クレートからの呼び出しでは
インライン化されない**。

```asm
probe_push_str:
	callq	_ZN11lean_string4repr50Repr$LT$..Mutable$GT$8push_str...E
	testb	%al, %al
	jne	.LBB11_2
	retq
```

`Repr::push_str` 本体は 106 命令あり、`reserve` の判定がすべて展開された結果、
インライン化の閾値を超えている。本体を追うと以下が見える。

```asm
	testq	%rdx, %rdx            # string.is_empty()
	je	.LBB1_16
	movzbl	15(%rdi), %ecx        # len() の branchless 復元
	movabsq	$72057594037927935, %rax
	andq	8(%rdi), %rax
	leaq	-192(%rcx), %rdi
	cmpq	$16, %rdi
	movl	$16, %r12d
	cmovbq	%rdi, %r12
	cmpq	$208, %rcx
	cmovaeq	%rax, %r12
	movq	%r12, %r15
	addq	%rdx, %r15
	jb	.LBB1_17              # checked_add のオーバーフロー判定
	cmpl	$208, %ecx            # HeapMarker か
	jne	.LBB1_7
	movq	(%rbx), %rcx
	movq	-16(%rcx), %rcx       # 参照カウントのロード (is_unique)
	cmpq	$1, %rcx
	jne	.LBB1_23
	movq	(%rbx), %rcx
	cmpq	%r15, -8(%rcx)        # 容量比較
	jae	.LBB1_9
	...                           # 足りなければ outline された reserve へ
.LBB1_9:
	cmpb	$-48, 15(%rbx)        # ★as_mut_ptr のために判別子を読み直し
	movq	%rbx, %rdi
	jne	.LBB1_11
	movq	(%rbx), %rdi
.LBB1_11:
	addq	%r12, %rdi
	callq	*memcpy@GOTPCREL(%rip)  # ★可変長 memcpy の呼び出し
	movzbl	15(%rbx), %eax        # ★set_len のために判別子をもう一度読み直し
	cmpl	$208, %eax
	je	.LBB1_14
	cmpl	$209, %eax
	jne	.LBB1_18
```

判定そのものは `reserve` を丸ごと呼ぶ形にしては十分に畳まれているが、次の 3 点が残る。

1. 判別子の読み直しが 3 回ある (fast path の判定 → `as_mut_ptr` → `set_len`)。
   `push_str` は判定の時点でバッファ種別を知っているのに、それを後段へ伝える手段がない。
2. 追記のコピーが常に可変長 memcpy の呼び出しになる。inline バッファへの数バイトの
   追記でも call 境界を跨ぐ。
3. 関数全体が大きいため呼び出し側でインライン化されない。

1 と 3 は [plans/03](../plans/03-push-str-fast-path.md)、2 は
[plans/02](../plans/02-medium-copy.md) で扱う。

## 4. `LeanString::push(char)` と `Extend<char>`

`push` は `char` をスタックの 4 バイトバッファへエンコードしてから `Repr::push_str` を呼ぶ。

```asm
probe_push:
	...                          # encode_utf8 相当の分岐 (4 通り)
.LBB10_6:
	leaq	4(%rsp), %rsi
	callq	_ZN11lean_string4repr50Repr$LT$..Mutable$GT$8push_str...E
```

`Extend<char>` はこれを 1 文字ごとに繰り返す。ループ本体は次の形になっている。

```asm
.LBB6_21:
	movq	%r14, %rdi
	movq	%r15, %rsi
	callq	_ZN11lean_string4repr50Repr$LT$..Mutable$GT$8push_str...E
	testb	%al, %al
	jne	.LBB6_22
.LBB6_14:
	movq	%rbx, %rdi
	callq	*%r12                 # iterator の next()
	cmpl	$1114112, %eax
```

つまり 1 文字あたり「関数呼び出し + 長さ復元 + オーバーフロー判定 + 判別子分岐 +
参照カウントのロード + 容量比較 + 1〜4 バイトのための memcpy 呼び出し + set_len の
判別子分岐」を通る。バッファを一度だけ解決してループ内ではポインタと長さだけを進める
形にできる。[plans/04](../plans/04-append-writer.md) の題材。

なお `extend` の冒頭で `size_hint().0` 分の `try_reserve` を呼んでいる
(src/lib.rs:2011-2023) が、こちらは `8c1a40d` で `reserve(0)` が no-op になったため、
size_hint が 0 のイテレータで共有バッファを無駄に detach することはなくなっている。

## 5. Drop

`b94f15a` (perf: avoid the dead write-back when dropping) により、`Vec<LeanString>` の
drop glue から空 repr の書き戻しは消えている。

```asm
.LBB4_2:                              # probe_drop_vec のループ本体
	cmpb	$-48, 15(%r14,%r15)
	jne	.LBB4_5
	leaq	(%r14,%r15), %rdi
	movq	(%rdi), %rax
	lock		decq	-16(%rax)
	jne	.LBB4_5
	callq	_ZN11lean_string4repr11heap_buffer19HeapBuffer$LT$H$GT$7release17on_last_reference...E
```

一方、単発の drop には 16 バイトのスタックコピーが残っている。

```asm
probe_drop_one:
	subq	$24, %rsp
	movups	(%rdi), %xmm0         # ★引数を丸ごとスタックへ実体化
	movaps	%xmm0, (%rsp)
	cmpb	$-48, 15(%rsp)
	jne	.LBB3_3
	movq	(%rsp), %rax
	lock		decq	-16(%rax)
	je	.LBB3_2
.LBB3_3:
	retq
.LBB3_2:
	movq	%rsp, %rdi            # cold 側へ「アドレス」を渡す
	callq	..HeapBuffer$LT$H$GT$7release17on_last_reference...E
```

`HeapBuffer::release` (src/repr/heap_buffer.rs:210-222) は最終参照時の処理を
`#[cold] fn on_last_reference(this: &mut HeapBuffer<H>)` に outline しているが、
参照を渡すために値のアドレスが必要になり、by-value で受け取った引数がスタックに
実体化されている。`HeapBuffer` は 2 ワードなので値渡しにでき、そうすればこのコピーは
不要になる。[plans/05](../plans/05-drop-cold-abi.md) で扱う。

## 6. 比較

```asm
probe_eq:
	movzbl	15(%rdi), %eax
	movabsq	$72057594037927935, %r8
	movq	8(%rdi), %rcx
	andq	%r8, %rcx
	leaq	-192(%rax), %rdx      # 左辺の len を branchless に復元
	cmpq	$16, %rdx
	movl	$16, %r9d
	cmovaeq	%r9, %rdx
	cmpq	$208, %rax
	cmovaeq	%rcx, %rdx
	...                           # 右辺も同様
	cmpq	%r9, %rdx             # len 比較
	jne	.LBB5_1
	movq	(%rdi), %r8
	cmpb	$-48, %cl
	jb	.LBB5_4
	movq	(%rsi), %rsi
.LBB5_4:
	cmpb	$-48, %al
	cmovaeq	%r8, %rdi
	callq	*bcmp@GOTPCREL(%rip)
```

「両辺の len を branchless に復元 → len 比較 → データポインタを cmov で選択 → `bcmp`」で、
無駄な分岐や再読み込みはない。ポインタ一致の fast path は入っていない。
これを入れるかどうかは [research/equality.md](./equality.md) と
[plans/07](../plans/07-equality-policy.md) で検討する。
