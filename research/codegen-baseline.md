# 現行コードの codegen 実測

調査日: 2026-09-09 / 対象: `448a538` / rustc 1.94.1, x86-64

各プランはこの文書で測った asm を出発点にしている。実装後の before/after 比較にも使えるよう、
引用は加工せずそのまま貼ってある。

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
// push_str / push / from_char / drop / eq / clone / extend / retain / to_lean_string も同様
```

```sh
cargo rustc --release --lib -- --emit asm -C llvm-args=--x86-asm-syntax=att
```

LTO を切ってあるのは、下流クレートから見たときに `#[inline]` がどこまで効いているかを
観察するため。crate 内部の呼び出しがすべて潰れた状態では、関数の大きさが原因の
インライン化失敗が見えなくなる。以下のダンプは AT&T 構文で、`.cfi_*` と
`.p2align` は省いてある。

命令数は各プローブの本体を数えたもの。

| プローブ | 命令数 | 一言 |
| --- | --- | --- |
| `probe_from_str` | 82 | inline 経路はレジスタ内で完結。heap 経路に memcpy 呼び出し |
| `probe_from_char` | 57 | ASCII はレジスタ内。2〜4 バイトはスタック往復が残る |
| `probe_push_str` | 8 | 本体は展開されず `callq` 1 本 |
| `probe_push` | 48 | encode_utf8 の展開 + `callq push_str` |
| `probe_drop_one` | 14 | 16 バイトのスタックコピーが残る |
| `probe_drop_vec` | 80 | ループ本体は判別子比較 + `lock decq` + cold 呼び出し |
| `probe_eq` | 33 | 削れる無駄なし |
| `probe_clone` | 12 | 理想形 |
| `probe_from_num` | 106 | 末尾で 16 バイトを 8 回のストアに分割 |
| `probe_retain_true` | 112 | 冒頭で無条件に `ensure_modifiable` |
| `probe_insert_str` | 131 | `push_str` と同じ判別子の読み直し |
| `Repr::push_str` (本体) | 106 | 大きすぎて呼び出し側に展開されない |

## 1. `LeanString::from(&str)` — inline 経路

inline 経路はスタックを経由せず、両端からの重ね合わせロードで 2 ワードを組み立てて返す。

```asm
probe_from_str:
	cmpq	$17, %rdx
	jae	.LBB14_1              # heap 経路へ
	cmpq	$16, %rdx
	jne	.LBB14_5
	movq	(%rsi), %r14          # len == 16
	movq	8(%rsi), %r12
	jmp	.LBB14_16
.LBB14_5:
	movq	%rdx, %r12
	orq	$-64, %r12
	shlq	$56, %r12             # 長さタグ
	cmpq	$7, %rdx
	jbe	.LBB14_6
	movq	(%rsi), %r14          # len >= 8
	cmpq	$8, %rdx
	je	.LBB14_16
	movq	-8(%rsi,%rdx), %rax   # w0 と重なる in-bounds ロード
	shll	$3, %edx
	negb	%dl
	movl	%edx, %ecx
	shrq	%cl, %rax
	orq	%rax, %r12
	jmp	.LBB14_16
	...
.LBB14_16:
	movq	%r14, (%rbx)          # 2 ストアで終わる
	movq	%r12, 8(%rbx)
```

スタックへのストアもゼロ初期化も無く、バッファ用の領域そのものが割り当てられていない。
x86 の store-to-load forwarding を踏む余地はここには残っていない。

heap 経路 (`.LBB14_1`) には可変長の `callq *memcpy@GOTPCREL(%rip)` が残っている。

```asm
.LBB14_1:
	movq	%rdx, %rax
	shrq	$56, %rax
	jne	.LBB14_17             # 長さ上限の超過
	...
	callq	*_RNvCs..___rust_alloc@GOTPCREL(%rip)
	testq	%rax, %rax
	je	.LBB14_17
	movq	$1, (%rax)            # 参照カウント
	movq	%r12, 8(%rax)         # 容量
	addq	$16, %r14
	callq	*memcpy@GOTPCREL(%rip)
```

コピーする長さが短いほど call 境界の割合が大きくなるので、inline に収まらない直後の
長さ帯 (17〜64 バイト) でこの呼び出しがいちばん重く効く。実際 `Construct` ベンチの
len 25 で `String` に 37% 負けている ([plans/02](../plans/02-medium-copy.md) の表)。

なお `Result<Repr, ReserveError>` の Err arm を呼び出し側で再検査する分岐は
`assert_unchecked` により消えており、`.LBB14_17` へ飛ぶのは長さ上限超過と確保失敗のときだけ。

## 2. `LeanString::from(char)`

ASCII の経路は定数畳み込みが効いて完全にレジスタ内で終わる。

```asm
probe_from_char:
	movq	%rdi, %rax
	movl	$0, -4(%rsp)
	cmpl	$128, %esi
	jae	.LBB12_1
	movl	%esi, %esi
	movabsq	$-4539628424389459968, %rdx   # 長さ 1 のタグ
	movq	%rsi, (%rax)
	movq	%rdx, 8(%rax)
	retq
```

一方 2〜4 バイトの `char` にはスタック往復が残る。原因は `Repr::from_char` (src/repr.rs:91-98)
が `char::encode_utf8` でスタック上の 4 バイトバッファに書き、それを `&str` として
`InlineBuffer::new` に渡していること。

```asm
.LBB12_5:
	movb	%dl, (%rsi)           # encode_utf8 の最終バイト
	leaq	-64(%rcx), %rdx
	shlq	$56, %rdx
	movzwl	-4(%rsp), %edi        # ★4 バイトバッファから読み直し
	movzwl	-6(%rsp,%rcx), %esi
	addq	$-2, %rcx
	shll	$3, %ecx
	shlq	%cl, %rsi
	orq	%rdi, %rsi
	movq	%rsi, (%rax)
	movq	%rdx, 8(%rax)
	retq
.LBB12_8:
	...
	movb	%sil, -4(%rsp)        # 4 バイトを 1 バイトずつストア
	movb	%dil, -3(%rsp)
	movb	%cl, -2(%rsp)
	movb	%dl, -1(%rsp)
	movl	-4(%rsp), %esi        # ★4 バイトを 1 回でロード
	movabsq	$-4323455642275676160, %rdx
	movq	%rsi, (%rax)
	movq	%rdx, 8(%rax)
```

4 バイト文字の arm では 1 バイトずつのストア 4 本に対して 4 バイトのロードが跨がるため、
x86 の store-to-load forwarding が成立しない (ロードが単一のストアに完全に含まれる場合にしか
転送できない)。`char` は最大 4 バイトなので、UTF-8 エンコード結果を u32 として
レジスタ内で組み立てれば第 1 ワードに直接置ける。
[plans/01](../plans/01-char-inline-construction.md) の題材。

## 3. `LeanString::push_str`

`Repr::push_str` には `#[inline]` が付いているが、**下流クレートからの呼び出しでは
インライン化されない**。

```asm
probe_push_str:
	callq	_ZN11lean_string4repr50Repr$LT$..Mutable$GT$8push_str...E
	testb	%al, %al
	jne	.LBB18_2
	retq
```

本体は 106 命令ある。`reserve` の判定がすべて展開された結果、インライン化の閾値を
超えている。本体を追うと以下が見える。

```asm
	testq	%rdx, %rdx            # string.is_empty()
	je	.LBB3_16
	movzbl	15(%rdi), %ecx        # ★判別子の 1 回目
	movabsq	$72057594037927935, %rax
	andq	8(%rdi), %rax
	leaq	-192(%rcx), %rdi      # len() の branchless 復元
	cmpq	$16, %rdi
	movl	$16, %r12d
	cmovbq	%rdi, %r12
	cmpq	$208, %rcx
	cmovaeq	%rax, %r12
	movq	%r12, %r15
	addq	%rdx, %r15
	jb	.LBB3_17              # checked_add のオーバーフロー判定
	cmpl	$208, %ecx            # HeapMarker か
	jne	.LBB3_7
	movq	(%rbx), %rcx
	movq	-16(%rcx), %rcx       # 参照カウントのロード (is_unique)
	cmpq	$1, %rcx
	jne	.LBB3_23
	movq	(%rbx), %rcx
	cmpq	%r15, -8(%rcx)        # 容量比較
	jae	.LBB3_9
	...                           # 足りなければ outline された reserve へ
.LBB3_9:
	cmpb	$-48, 15(%rbx)        # ★判別子の 2 回目 (as_mut_ptr)
	movq	%rbx, %rdi
	jne	.LBB3_11
	movq	(%rbx), %rdi
.LBB3_11:
	addq	%r12, %rdi
	callq	*memcpy@GOTPCREL(%rip)  # ★可変長 memcpy の呼び出し
	movzbl	15(%rbx), %eax        # ★判別子の 3 回目 (set_len)
	cmpl	$208, %eax
	je	.LBB3_14
	cmpl	$209, %eax            # ★到達しない StaticMarker の比較
	jne	.LBB3_18
```

判定そのものは `reserve` を丸ごと呼ぶ形にしては十分に畳まれているが、次の 4 点が残る。

1. 判別子の読み直しが 3 回ある (fast path の判定 → `as_mut_ptr` → `set_len`)。
   `push_str` は判定の時点でバッファ種別を知っているのに、それを後段へ伝える手段がない。
2. `set_len` の `cmpl $209` は StaticMarker との比較だが、`reserve` が `Ok` を返した後は
   static ではありえないので死んだ比較になっている。
3. 追記のコピーが常に可変長 memcpy の呼び出しになる。inline バッファへの数バイトの
   追記でも call 境界を跨ぐ。
4. 関数全体が大きいため呼び出し側でインライン化されない。

1・2・4 は [plans/03](../plans/03-push-str-fast-path.md)、3 は
[plans/02](../plans/02-medium-copy.md) で扱う。

`LeanString::insert_str` (131 命令) も同じ形で、`memmove` と `memcpy` の 2 回の呼び出しと
判別子の 3 回読みを持つ。

## 4. `LeanString::push(char)` と `Extend<char>`

`push` は `char` をスタックの 4 バイトバッファへエンコードしてから `Repr::push_str` を呼ぶ。

```asm
probe_push:
	...                          # encode_utf8 相当の分岐 (4 通り)
.LBB17_6:
	leaq	4(%rsp), %rsi
	callq	_ZN11lean_string4repr50Repr$LT$..Mutable$GT$8push_str...E
```

`Extend<char>` はこれを 1 文字ごとに繰り返す。ループ本体は次の形。

```asm
.LBB10_24:
	movb	%al, (%rsp)           # ASCII のエンコード
	movl	$1, %edx
.LBB10_21:
	movq	%r14, %rdi
	movq	%r15, %rsi
	callq	_ZN11lean_string4repr50Repr$LT$..Mutable$GT$8push_str...E
	testb	%al, %al
	jne	.LBB10_22
.LBB10_14:
	movq	%rbx, %rdi
	callq	*%r12                 # iterator の next()
	cmpl	$1114112, %eax
```

つまり 1 文字あたり「関数呼び出し + 長さ復元 + オーバーフロー判定 + 判別子分岐 +
参照カウントのロード + 容量比較 + 1〜4 バイトのための memcpy 呼び出し + set_len の
判別子分岐」を通る。バッファを一度だけ解決してループ内ではポインタと長さだけを進める
形にできる。[plans/04](../plans/04-append-writer.md) の題材。

`FromIterator<char>` (`probe_collect_chars`) も同じ形で、冒頭で `size_hint().0` 分の
容量を確保したあとは 1 文字ずつ `push_str` を呼ぶ。

## 5. Drop

`Vec<LeanString>` の drop glue は良い形になっている。

```asm
.LBB8_2:                              # probe_drop_vec のループ本体
	cmpb	$-48, 15(%r14,%r15)
	jne	.LBB8_5
	leaq	(%r14,%r15), %rdi
	movq	(%rdi), %rax
	lock		decq	-16(%rax)
	jne	.LBB8_5
	callq	_ZN11lean_string4repr11heap_buffer19HeapBuffer$LT$H$GT$7release17on_last_reference...E
```

一方、単発の drop には 16 バイトのスタックコピーが残っている。

```asm
probe_drop_one:
	subq	$24, %rsp
	movups	(%rdi), %xmm0         # ★引数を丸ごとスタックへ実体化
	movaps	%xmm0, (%rsp)
	cmpb	$-48, 15(%rsp)
	jne	.LBB7_3
	movq	(%rsp), %rax
	lock		decq	-16(%rax)
	je	.LBB7_2
.LBB7_3:
	addq	$24, %rsp
	retq
.LBB7_2:
	movq	%rsp, %rdi            # ★cold 側へ「アドレス」を渡す
	callq	_ZN11lean_string4repr11heap_buffer19HeapBuffer$LT$H$GT$7release17on_last_reference...E
```

`HeapBuffer::release` (src/repr/heap_buffer.rs:211-227) は最終参照時の処理を
`#[cold] fn on_last_reference(this: &mut HeapBuffer<H>)` に outline しているが、
参照を渡すために値のアドレスが必要になり、by-value で受け取った引数がスタックに
実体化されている。`HeapBuffer` は 2 ワードなので値渡しにでき、そうすればこのコピーは
不要になる。[plans/05](../plans/05-drop-cold-abi.md) で扱う。

`on_last_reference` と `make_shallow_clone::ref_count_overflow` は
どちらも `.text.unlikely` に配置されており、cold 化そのものは効いている。

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
	jne	.LBB9_1
	movq	(%rdi), %r8
	cmpb	$-48, %cl
	jb	.LBB9_4
	movq	(%rsi), %rsi
.LBB9_4:
	cmpb	$-48, %al
	cmovaeq	%r8, %rdi
	callq	*bcmp@GOTPCREL(%rip)
```

「両辺の len を branchless に復元 → len 比較 → データポインタを cmov で選択 → `bcmp`」で、
無駄な分岐や再読み込みはない。ポインタ一致の fast path は入っていない。
これを入れるかどうかは [research/equality.md](./equality.md) と
[plans/07](../plans/07-equality-policy.md) で検討する。

## 7. Clone

```asm
probe_clone:
	cmpb	$-48, 15(%rsi)
	jne	.LBB5_2
	movq	(%rsi), %rax
	lock		incq	-16(%rax)
	jle	.LBB5_3
.LBB5_2:
	movups	(%rsi), %xmm0
	movups	%xmm0, (%rdi)
	movq	%rdi, %rax
	retq
```

12 命令。heap の arm は判別子の比較と `lock incq` の 2 命令で、どちらの arm も
16 バイトのビット単位コピーへ合流する。オーバーフロー処理は `.text.unlikely` にある。
削るところはない。

## 8. 整数の変換

`Repr::from_num` は LUT で桁を作り、`Repr::new_with` (src/repr.rs:136-151) の
`init` クロージャからスタック上の 16 バイトバッファへ直接書く。桁を作る部分は良いが、
**戻り値を作る最後の 16 バイト転送が 8 回のストアに分割される**。

```asm
.LBB13_17:
	orb	$-64, %cl
	movb	%cl, -1(%rsp)         # set_len (バイト 15)
	movq	-16(%rsp), %rcx       # ワード単位で 2 回ロード
	movq	-8(%rsp), %rdx
	movq	%rcx, %rsi
	shrq	$8, %rsi
	movq	%rdx, %rdi
	shrq	$56, %rdi
	movb	%cl, (%rax)           # ★ここから 16 バイトを 8 回に分けてストア
	movq	%rcx, %r8
	shrq	$56, %r8
	movb	%r8b, 7(%rax)
	shrq	$40, %rcx
	movw	%cx, 5(%rax)
	movl	%esi, 1(%rax)
	movl	%edx, 8(%rax)
	movq	%rdx, %rcx
	shrq	$48, %rcx
	movb	%cl, 14(%rax)
	shrq	$32, %rdx
	movw	%dx, 12(%rax)
	movb	%dil, 15(%rax)
```

ロード自体は 2 本にまとまっているので、分割されているのはストア側だけである。
§1 の `from_str` が同じ `Repr` へ `movq` 2 本で書けていることと対照的で、
違いは値の出どころにある。`from_str` は 2 つの u64 を計算して持っているのに対し、
`new_with` の値はバイト単位のストアで作られたバッファから来るため、LLVM が
`Repr` のフィールド境界 (バイト 8 と 15) ごとに分解したまま再合成できていない。

素直な対処 (`new_with` の inline arm を `InlineBuffer::new` 経由にする) を試したところ、
`into_repr` が大きくなって下流からインライン化されなくなり、**全体としては悪化した**
(106 命令のインライン展開 → 22 命令 + `callq into_repr`)。
詳細と別案は [plans/11](../plans/11-integer-codegen.md) に書いた。

## 9. `retain`

`Repr::retain` (src/repr.rs:664-721) は本体に入る前に無条件で `ensure_modifiable` を呼ぶ。

```asm
probe_retain_true:
	movq	%rdi, %rbx
	callq	*_ZN11lean_string4repr50Repr$LT$..Mutable$GT$17ensure_modifiable...E
	testb	%al, %al
	jne	.LBB19_25
	movzbl	15(%rbx), %eax
	...
```

述語が 1 文字も落とさない場合でも共有バッファのコピーが発生する。
[plans/06](../plans/06-retain-shared-fast-path.md) の題材。
