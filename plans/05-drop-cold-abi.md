# 05: cold 関数へ参照ではなく値を渡す

種別: perf / 実装量: 小 / 設計判断: 小

## 背景

`b94f15a` で Drop の書き戻しは消え、`Vec<LeanString>` の drop glue からも dead store が
なくなった。しかし単発の drop には 16 バイトのスタックコピーが残っている
([research/codegen-baseline.md](../research/codegen-baseline.md) の §5)。

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

原因は `HeapBuffer::release` (src/repr/heap_buffer.rs:210-222) の形。

```rust
pub(super) unsafe fn release(&mut self) {
    if self.reference_count().fetch_sub(1, Release) == 1 {
        #[cold]
        fn on_last_reference<H: Header>(this: &mut HeapBuffer<H>) {
            fence(Acquire);
            unsafe { this.dealloc() };
        }
        on_last_reference(self);
    }
}
```

`on_last_reference` が `&mut HeapBuffer<H>` を取るため、`*self` のアドレスが
インライン化されない関数へ渡る。アドレスが escape する以上、値をレジスタに置いたままには
できず、by-value で受け取った `LeanString` は**呼び出しが起きない inline 経路でも**
スタックに実体化される。

## 方針

cold 側にはフィールドを値で渡し、アドレスを escape させない。

```rust
#[inline]
pub(super) unsafe fn release(&mut self) {
    if self.reference_count().fetch_sub(1, Release) == 1 {
        // cold 側へは値だけを渡す。`&mut self` を渡すと `*self` のアドレスが escape し、
        // 呼び出しが起きない経路でも呼び出し側で値をスタックに実体化させられる。
        // SAFETY: 直前の値が 1 だったので他の参照は存在しない。`self` はこの後
        //         アクセスされない (`#Safety` の契約)。
        unsafe { Self::on_last_reference(ptr::read(self)) };
    }
}

#[cold]
#[inline(never)]
unsafe fn on_last_reference(mut this: Self) {
    fence(Acquire);
    unsafe { this.dealloc() };
}
```

`HeapBuffer<H>` は `{ ptr: NonNull<u8>, len: TextLen }` の 2 ワードなので、
SysV x86-64 でも AAPCS64 でもレジスタ 2 本で渡る (16 バイトの整数 2 ワードは
`rdi:rsi` / `x0:x1`)。`ptr::read` で値にしてから渡すことで `&mut self` は escape しない。

`dealloc` が `&mut self` を取るので `this` を `mut` で受けているが、これはローカル変数の
アドレスであり呼び出し側には見えない。

## プロトタイプで確認したこと

上のコードを実際に当てて測った。

`probe_drop_one` (= `fn drop_one(s: LeanString)`) はスタックフレームごと消え、
cold への呼び出しは末尾ジャンプになった。

```asm
; after
probe_drop_one:
	movq	8(%rdi), %rsi
	movq	%rsi, %rax
	shrq	$56, %rax
	cmpl	$208, %eax
	jne	.LBB11_2
	movq	(%rdi), %rdi
	lock		decq	-16(%rdi)
	je	.LBB11_3
.LBB11_2:
	retq
.LBB11_3:
	jmpq	*..HeapBuffer$LT$H$GT$7release17on_last_reference...E
```

16 バイトのコピー (`movups`/`movaps` の対) と `subq $24, %rsp` が無くなっている。

`Vec<LeanString>` の drop glue でも、関数ポインタのロードがループの外へ出た。

```asm
	leaq	16(%r14), %r15
	movq	..on_last_reference...@GOTPCREL(%rip), %r12   ; ループ外へ
	jmp	.LBB12_2
	.p2align	4
.LBB12_5:
	addq	$16, %r15
	decq	%r13
	je	.LBB12_6
.LBB12_2:
	cmpb	$-48, -1(%r15)
	jne	.LBB12_5
	movq	-8(%r15), %rsi
	movq	-16(%r15), %rdi
	lock		decq	-16(%rdi)
	jne	.LBB12_5
	callq	*%r12
```

正しさの確認:

- `cargo test --release --all-features` が全て通る。
- `RUSTFLAGS="--cfg loom" cargo test --test loom --release --features loom -- --test-threads=1` が通る。
- `MIRIFLAGS="-Zmiri-strict-provenance" cargo +nightly miri test --all-features` が x86_64 で通る。

### 値渡しがレジスタ渡しになる根拠

`HeapBuffer` は `(NonNull<u8>, TextLen)` の 2 ワードで、この形は
`rax:rdx` / `x0:x1` で受け渡しされる。実測で確認した分類:

| 型の形 | 受け渡し |
| --- | --- |
| `(NonNull<u8>, usize)` (= `HeapBuffer`) | レジスタ |
| `(*const (), [u8; 7], LastByte)` (= `Repr`) | sret / 間接 |

`Repr` は 16 バイトだが scalar pair ではないので値渡しにしても得はない。
`release` が扱うのは `HeapBuffer` なので、ここでは問題にならない。

## 適用範囲

`release` の呼び出し元はすべて恩恵を受ける。

| 呼び出し元 | 場所 |
| --- | --- |
| `Repr::drop_in` | repr.rs:292-300 |
| `Repr::replace_inner` | repr.rs:271-280 |
| `reserve` の unshare 経路 | repr.rs:490 |
| `shrink_to` の共有経路 | repr.rs:559 |
| `truncate_unchecked` の共有経路 | repr.rs:797 |
| `into_immutable` / `into_mutable` | repr.rs:913/929/965 |
| `ensure_modifiable` | repr.rs:831 |

同じ理屈は `release` 以外の cold ヘルパにも当てはまる。今回あわせて確認する箇所:

- `Repr::make_shallow_clone` の `ref_count_overflow` (repr.rs:256-261) は
  `&Repr<M>` を取る。ただしこちらは `panic!` で終わる `-> !` の関数で、
  現行の asm を見るかぎり `probe_clone` は既に理想的な形
  (hot 側が fall-through、heap 側は `lock incq` 2 命令) になっているので、
  変える必要はなさそう。測って確認する。
- `reserve` の `outline!` マクロが作る関数は `&mut Repr<Mutable>` を取る
  (repr.rs:480, 499, 506, 516)。ここは呼び出し後も `self` を使い続けるので
  参照渡しが必要であり、値渡しにはできない。ただし `reserve` を呼ぶ関数
  (`push_str` など) では同じ理由で `self` がスタックに置かれうるので、
  [plans/03](./03-push-str-fast-path.md) の asm を見るときに合わせて確認する。

## 検証方針

- **asm** (主検証):
  - `fn drop_one(s: LeanString)`: 16 バイトのスタックコピー (`movups`/`movaps` の対) が
    消え、inline 変種の drop が「判別子の比較 + 早期 return」だけになること。
  - `fn drop_vec(v: Vec<LeanString>)`: ループ本体が現状より短くなること (現状は
    判別子比較 + `lock decq` + cold 呼び出しで、すでに悪くない)。
  - x86-64 と aarch64 の両方。
- **loom**: 全シナリオ。atomic の順序 (`Release` の decrement + 最終参照時の
  `Acquire` fence) は変えていないので、ここで落ちたら実装ミス。
- **Miri**: 全ターゲット。特に `tests/race_condition.rs` を
  `-Zmiri-preemption-rate=1` 付きで。
- **criterion**: drop 単体のベンチは無い。`apis.rs` の `from` / `clone` は
  イテレーションごとに構築と破棄を行うので、drop のコストはそこに現れる。
  必要なら `Vec<LeanString>` の drop ベンチを足す。

## 依存

なし。単独で実施できる。
