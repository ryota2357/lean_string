# 現行コードの codegen 実測

対象: `8bf3fee` / rustc 1.100.0-nightly (2026-09-24), `x86_64-unknown-linux-gnu`

各プランはここで測った asm を前提にしている。実装後に before/after を比べられるよう、
asm は手を加えずに載せている (ラベルの番号はビルドごとに変わる)。

## 測定方法

lean_string を path 依存で使うだけの小さなクレートを作り、確認したい呼び出しを
`#[unsafe(no_mangle)]` の関数として並べた。

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
// push_str / push / from_char / drop / eq / clone / extend / retain / to_lean_string /
// into_lean_str / repeat も同様
```

```sh
cargo +nightly rustc --release --lib --target x86_64-unknown-linux-gnu \
    -- --emit asm -C llvm-args=--x86-asm-syntax=att
```

ホストは aarch64 なので、nightly に入れてある `x86_64-unknown-linux-gnu` の std を使って
x86-64 の asm を出した (リンクはしないので、x86-64 の実行環境は要らない)。

LTO を切っているのは、下流のクレートから呼んだときに `#[inline]` がどこまで効くかを見るため。
LTO で crate 内の呼び出しがすべてインライン化されると、関数が大きすぎて展開されない、という問題が見えなくなる。
以下の asm は AT&T 構文で、`.cfi_*` と `.p2align` は省いた。

命令数は各プローブの本体の命令を数えたもの。

| プローブ | 命令数 | メモ |
| --- | --- | --- |
| `probe_from_str` | 82 | inline の経路はレジスタだけで完結。heap の経路に memcpy の呼び出し |
| `probe_from_char` | 56 | どの分岐もレジスタだけで完結 |
| `probe_push_str` | 8 | 本体は展開されず `callq` のみ |
| `probe_push` | 48 | encode_utf8 の展開 + `callq push_str` |
| `probe_drop_one` | 14 | 16 バイトのスタックコピーが残る |
| `probe_drop_vec` | 80 | ループ本体は判別子比較 + `lock decq` + cold 呼び出し |
| `probe_eq` | 33 | 削るところなし |
| `probe_clone` | 12 | 削るところなし |
| `probe_from_num` (`u32`) | 96 | 末尾で 16 バイトを 5 回のストアに分割 |
| `probe_from_i32` / `probe_from_i64` | 22 / 22 | `into_repr` が展開されず `callq` (本体 138 / 314 命令) |
| `probe_retain_true` | 29 | 走査だけで終わる。確保も `ensure_modifiable` の呼び出しも無い |
| `probe_insert_str` | 133 | `push_str` と同じ判別子の読み直し |
| `probe_into_lean_str` | 201 | `into_exact` の戻り値が sret |
| `Repr::push_str` (本体) | 108 | 大きく、呼び出し側に展開されない |

## 1. `LeanString::from(&str)` — inline 経路

inline の経路はスタックを使わず、先頭と末尾からの重なったロードで 2 ワードを組み立てて返す。

```asm
probe_from_str:
	...
	cmpq	$17, %rdx
	jae	.LBB19_1              # heap 経路へ
	cmpq	$16, %rdx
	jne	.LBB19_6
	movq	(%rsi), %r14          # len == 16
	movq	8(%rsi), %r12
	jmp	.LBB19_17
	...
.LBB19_6:
	movq	%rdx, %r12
	orq	$-64, %r12
	shlq	$56, %r12             # 長さタグ
	cmpq	$7, %rdx
	jbe	.LBB19_7
	movq	(%rsi), %r14          # len >= 8
	cmpq	$8, %rdx
	je	.LBB19_17
	movq	-8(%rsi,%rdx), %rax   # w0 と重なる in-bounds ロード
	shll	$3, %edx
	negb	%dl
	movl	%edx, %ecx
	shrq	%cl, %rax
	orq	%rax, %r12
	jmp	.LBB19_17
	...
.LBB19_17:
	movq	%r14, (%rbx)          # 2 ストアで終わる
	movq	%r12, 8(%rbx)
```

スタックへのストアもゼロ初期化も無く、バッファの領域すら確保していない。
x86 の store-to-load forwarding の問題が起きる余地は無い。

heap の経路 (`.LBB19_1`) には可変長の `callq *memcpy@GOTPCREL(%rip)` が残っている。

```asm
.LBB19_1:
	movq	%rdx, %rax
	shrq	$56, %rax
	jne	.LBB19_18             # 長さ上限の超過
	...
	callq	*_RNvCs..___rust_alloc@GOTPCREL(%rip)
	testq	%rax, %rax
	je	.LBB19_18
	movq	%r12, %rdx
	movq	%rax, %r14
	movq	$1, (%rax)            # 参照カウント
	movq	%r12, 8(%rax)         # 容量
	addq	$16, %r14
	...
	callq	*memcpy@GOTPCREL(%rip)
```

コピーが短いほど関数呼び出しのコストが相対的に大きくなるので、inline に収まらなくなった直後の
長さ (17〜64 バイト) でいちばん効いてくる。実際、`Construct` ベンチの len 25 では `String` より
37% 遅い ([plans/01](../plans/01-medium-copy.md) の表)。

なお、`Result<Repr, ReserveError>` が Err かどうかを呼び出し側で調べ直す分岐は `assert_unchecked` で
消えている。`.LBB19_18` に飛ぶのは長さの上限を超えたときと確保に失敗したときだけ。

## 2. `LeanString::from(char)`

`InlineBuffer::from_char` が UTF-8 のエンコード結果をレジスタ上で組み立てるので、
どの分岐もスタックを使わず、2 回のストアで終わる。

```asm
probe_from_char:
	movq	%rdi, %rax
	cmpl	$128, %esi
	jae	.LBB13_2
	movabsq	$-4539628424389459968, %rcx   # 長さ 1 のタグ
	movl	%esi, %edx
	movq	%rdx, (%rax)
	movq	%rcx, 8(%rax)
	retq
	...
.LBB13_4:                                 # 4 バイト文字
	movl	%esi, %ecx
	shrl	$18, %ecx
	...
	addl	$-2139062032, %esi
	movabsq	$-4323455642275676160, %rcx
	movl	%esi, %edx
	movq	%rdx, (%rax)
	movq	%rcx, 8(%rax)
	retq
```

ここに直すところは無い。`push(char)` と `Extend<char>` はこの経路を通らず
`push_str` を呼ぶので、そちらは §4 で見る。

## 3. `LeanString::push_str`

`Repr::push_str` には `#[inline]` が付いているが、下流のクレートから呼ぶと展開されない。

```asm
probe_push_str:
	pushq	%rax
	callq	_RNvMs_NtC..11lean_string4reprINtB4_4ReprNtNtB4_10mutability7MutableE8push_str
	testb	%al, %al
	jne	.LBB24_2
	popq	%rax
	retq
```

本体は 108 命令ある。`reserve` の判定がすべて展開されているため、インライン化の閾値を
超えている。本体は次のとおり。

```asm
	testq	%rdx, %rdx            # string.is_empty()
	je	.LBB9_1
	...
	movzbl	15(%rdi), %edi        # 判別子 (1 回目)
	movabsq	$72057594037927935, %rcx
	andq	8(%rbx), %rcx
	leaq	-192(%rdi), %rax      # len() の branchless 復元
	cmpq	$16, %rax
	movl	$16, %r12d
	cmovbq	%rax, %r12
	cmpq	$208, %rdi
	cmovaeq	%rcx, %r12
	movq	%r12, %r15
	movb	$1, %al
	addq	%rdx, %r15
	jb	.LBB9_27              # checked_add のオーバーフロー判定
	cmpl	$208, %edi            # HeapMarker か
	jne	.LBB9_5
	movq	(%rbx), %rax
	movq	-16(%rax), %rax       # 参照カウントのロード (is_unique)
	cmpq	$1, %rax
	jne	.LBB9_19
	movq	(%rbx), %rax
	cmpq	%r15, -8(%rax)        # 容量比較
	jae	.LBB9_12
	...                           # 足りなければ outline された reserve へ
.LBB9_12:
	cmpb	$-48, 15(%rbx)        # 判別子 (2 回目、as_mut_ptr)
	movq	%rbx, %rdi
	jne	.LBB9_14
	movq	(%rbx), %rdi
.LBB9_14:
	addq	%r12, %rdi
	callq	*memcpy@GOTPCREL(%rip)  # 可変長 memcpy の呼び出し
	movzbl	15(%rbx), %eax        # 判別子 (3 回目、set_len)
	cmpl	$208, %eax
	je	.LBB9_28
	cmpl	$209, %eax            # 常に偽になる StaticMarker との比較
	jne	.LBB9_24
```

判定の部分は、`reserve` をそのまま呼んでいる割にはよくまとまっている。ただ次の 4 点が残っている。

1. 判別子を 3 回読んでいる (fast path の判定、`as_mut_ptr`、`set_len`)。
   最初の判定でバッファの種類は分かっているのに、それを後ろに伝える手段がない。
2. `set_len` の `cmpl $209` は StaticMarker との比較だが、`reserve` が `Ok` を返した後に
   static であることはないので、この比較は常に偽になる。
3. 追記のコピーが常に可変長の memcpy の呼び出しになる。inline バッファに数バイト追記するだけでも関数を呼ぶ。
4. 関数全体が大きいので、呼び出し側に展開されない。

1・2・4 は [plans/02](../plans/02-push-str-fast-path.md)、3 は
[plans/01](../plans/01-medium-copy.md) で扱う。

`LeanString::insert_str` (133 命令) も同じ作りで、`memmove` と `memcpy` を 1 回ずつ呼び、判別子を 3 回読んでいる。

## 4. `LeanString::push(char)` と `Extend<char>`

`push` は `char` をスタックの 4 バイトバッファへエンコードしてから `Repr::push_str` を呼ぶ。

```asm
probe_push:
	...                          # encode_utf8 相当の分岐 (4 通り)
	callq	_RNvMs_NtC..11lean_string4reprINtB4_4ReprNtNtB4_10mutability7MutableE8push_str
	testb	%al, %al
	jne	.LBB23_9
	popq	%rax
	retq
```

`Extend<char>` はこれを 1 文字ごとに繰り返す。ループ本体は次の形。

```asm
.LBB14_24:
	movb	%al, (%rsp)           # ASCII のエンコード
	movl	$1, %edx
.LBB14_21:
	movq	%r14, %rdi
	movq	%r15, %rsi
	callq	_RNvMs_NtC..11lean_string4reprINtB4_4ReprNtNtB4_10mutability7MutableE8push_str
	testb	%al, %al
	jne	.LBB14_22
.LBB14_14:
	movq	%rbx, %rdi
	callq	*%r12                 # iterator の next()
	cmpl	$-1, %eax
	je	.LBB14_23
```

つまり 1 文字ごとに、関数呼び出し、長さの復元、オーバーフローの判定、判別子による分岐、
参照カウントのロード、容量の比較、1〜4 バイトのための memcpy の呼び出し、`set_len` での判別子による分岐を通る。
書き込み先は最初に 1 回決めればよく、ループの中ではポインタと長さを進めるだけにできる。
[plans/03](../plans/03-append-writer.md) で扱う。

`FromIterator<char>` も同じで、最初に `size_hint().0` の容量を確保した後は 1 文字ずつ `push_str` を呼ぶ。
`to_lowercase` / `to_uppercase` の非 ASCII の部分も `c.to_lowercase().for_each(|l| mapped.push(l))` なので同じ経路を通る。

## 5. Drop

`Vec<LeanString>` の drop はよいコードになっている。

```asm
                                      # probe_drop_vec のループ本体
	leaq	(%r14,%r15), %rdi
	movq	(%rdi), %rax
	lock		decq	-16(%rax)
	jne	.LBB12_5
	callq	_RINvNvMs2_NtNtC..11lean_string4repr11heap_bufferINtB8_10HeapBufferpE7release17on_last_reference...
	jmp	.LBB12_5
```

一方、1 つだけ drop するときは 16 バイトをスタックにコピーしている。

```asm
probe_drop_one:
	subq	$24, %rsp
	movups	(%rdi), %xmm0         # 引数をまるごとスタックへコピー
	movaps	%xmm0, (%rsp)
	cmpb	$-48, 15(%rsp)
	jne	.LBB11_3
	movq	(%rsp), %rax
	lock		decq	-16(%rax)
	je	.LBB11_2
.LBB11_3:
	addq	$24, %rsp
	retq
.LBB11_2:
	movq	%rsp, %rdi            # cold 側にはアドレスを渡している
	callq	_RINvNvMs2_NtNtC..11lean_string4repr11heap_bufferINtB8_10HeapBufferpE7release17on_last_reference...
	addq	$24, %rsp
	retq
```

`HeapBuffer::release` (src/repr/heap_buffer.rs) は最後の参照が消えるときの処理を
`#[cold] fn on_last_reference(this: &mut HeapBuffer<H>)` に切り出しているが、参照を渡すには
値のアドレスが必要なので、値で受け取った引数をスタックに置くことになる。`HeapBuffer` は 2 ワードなので
値で渡せば、このコピーは要らなくなる。[plans/04](../plans/04-drop-cold-abi.md) で扱う。

`on_last_reference` と `make_shallow_clone::ref_count_overflow` はどちらも `.text.unlikely` に置かれており、
cold 指定自体は効いている。

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
	jne	.LBB13_1
	cmpb	$-48, %cl
	jb	.LBB13_4
	movq	(%rsi), %rsi          # データポインタの選択
.LBB13_4:
	cmpb	$-48, %al
	jb	.LBB13_6
	movq	(%rdi), %rdi
.LBB13_6:
	pushq	%rax
	callq	*bcmp@GOTPCREL(%rip)
```

両辺の長さを分岐なしで復元し、長さを比べ、データポインタを選んで `bcmp` を呼ぶ。
余計な分岐や読み直しは無い。ポインタが同じなら内容を見ずに済ませる近道は入っていない。
入れるかどうかは [research/equality.md](./equality.md) と [plans/05](../plans/05-equality-policy.md) で検討する。

## 7. Clone

```asm
probe_clone:
	cmpb	$-48, 15(%rsi)
	jne	.LBB10_2
	movq	(%rsi), %rax
	lock		incq	-16(%rax)
	jle	.LBB10_3
.LBB10_2:
	movups	(%rsi), %xmm0
	movups	%xmm0, (%rdi)
	movq	%rdi, %rax
	retq
```

12 命令。heap の分岐は判別子の比較と `lock incq` の 2 命令で、どちらの分岐も 16 バイトのコピーに合流する。
オーバーフローの処理は `.text.unlikely` にある。削るところは無い。

## 8. 整数の変換

`Repr::from_num` は LUT で桁を作り、`Repr::new_with` の `init` クロージャからスタック上の
16 バイトのバッファに直接書く。

`u32` (`probe_from_num`) では `into_repr` がすべて展開され、heap の経路も消えている
(10 桁は必ず inline に収まることを、LLVM が桁数の計算から導いている)。
桁を作る部分はよいが、最後に 16 バイトの戻り値を書き出すところが 5 回のストアに分かれている。

```asm
.LBB14_17:
	orb	$-64, %cl
	movb	%cl, -1(%rsp)         # set_len (バイト 15)
	movq	-16(%rsp), %rcx
	movq	%rcx, (%rax)          # ここから 16 バイトを 5 回に分けて書く
	movl	-8(%rsp), %ecx
	movl	%ecx, 8(%rax)
	movzwl	-4(%rsp), %ecx
	movw	%cx, 12(%rax)
	movzbl	-2(%rsp), %ecx
	movb	%cl, 14(%rax)
	movzbl	-1(%rsp), %ecx
	movb	%cl, 15(%rax)
```

§1 の `from_str` は同じ `Repr` を `movq` 2 本で書けている。違いは値の出どころで、
`from_str` は 2 つの u64 をレジスタで計算しているのに対し、`new_with` の値はバイト単位で書いたバッファから来る。
そのため LLVM が `Repr` のフィールドの境界 (バイト 8・12・14・15) で分けたまま、まとめ直せていない。
また桁は 2 バイト単位のストア (`movw %dx, -18(%rsp,%rsi)`) で書かれ、直後の
`movq -16(%rsp)` がそれらを跨いで読むので、x86 では store-to-load forwarding が成立しない。

符号付きの `i32` / `i64` では、`into_repr` 自体が下流で展開されない。

```asm
probe_from_i32:
	...
	callq	_RINvXsg_NtNtC..11lean_string4repr11num_to_reprlNtB6_9NumToRepr9into_repr...
```

`into_repr` の本体は `i32` で 138 命令、`i64` で 314 命令。`impl_NumToRepr_for_integers` が
作る `into_repr` には `#[inline]` が付いていないが、付けても結果は変わらなかった
(generic なので MIR はもともと下流に出ている。大きさが閾値を超えているのが原因)。
`u64` は展開される (`probe_from_u64` は 217 命令)。

詳しくは [plans/08](../plans/08-integer-codegen.md) に書いた。

## 9. `retain`

`Repr::retain` は、最初に削る文字が見つかるまでバッファを共有したまま走査する。
`retain(|_| true)` は走査だけで終わり、確保も `ensure_modifiable` の呼び出しも無い。

```asm
probe_retain_true:
	movzbl	15(%rdi), %ecx
	...                           # len とデータポインタの復元
.LBB26_4:
	movzbl	(%rdi), %ecx          # UTF-8 の先頭バイトから文字幅だけ進める
	testb	%cl, %cl
	jns	.LBB26_5
	...
.LBB26_10:
	retq
```

## 10. `LeanString::into_lean_str`

unique な heap バッファの変換では `HeapBuffer::into_exact` を呼ぶが、その戻り値が sret (隠しポインタ) で返っている。

```asm
	movq	16(%rsp), %rsi
	movq	24(%rsp), %rdx
	leaq	32(%rsp), %rdi        # sret のポインタ
	callq	*..HeapBuffer..GrowableHeader..into_exact@GOTPCREL(%rip)
	movq	8(%rsp), %r12
	movq	40(%rsp), %r15        # 結果をスタックから読む
	movq	48(%rsp), %rbp
	cmpl	$1, 32(%rsp)          # 判別子もスタックから
```

戻り値の型 `Result<HeapBuffer<ExactHeader>, (HeapBuffer<GrowableHeader>, ReserveError)>` が 24 バイトになるため。[plans/09](../plans/09-mutability-conversion.md) で扱う。
