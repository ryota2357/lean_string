# 05: cold 関数へ参照ではなく値を渡す

種別: perf / 実装量: 小 / 設計判断: 小

## 背景

`Vec<LeanString>` の drop glue は良い形になっているが、単発の drop には
16 バイトのスタックコピーが残っている
([research/codegen-baseline.md](../research/codegen-baseline.md) の §5)。

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
	movq	%rsp, %rdi            # cold 側へ「アドレス」を渡す
	callq	..HeapBuffer$LT$H$GT$7release17on_last_reference...E
```

原因は `HeapBuffer::release` (src/repr/heap_buffer.rs:211-227) の形。

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
できず、by-value で受け取った `LeanString` は呼び出しが起きない inline 経路でも
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
        //         アクセスされない (`# Safety` の契約)。
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
SysV x86-64 でも AAPCS64 でもレジスタ 2 本で渡る。`ptr::read` で値にしてから渡すことで
`&mut self` は escape しない。

`dealloc` が `&mut self` を取るので `this` を `mut` で受けているが、これはローカル変数の
アドレスであり呼び出し側には見えない。

## プロトタイプで確認したこと

上のコードを実際に当てて測った。

`probe_drop_one` (= `fn drop_one(s: LeanString)`) はスタックフレームごと消え、
14 命令から 10 命令になった。cold への呼び出しは末尾ジャンプになる。

```asm
; after
probe_drop_one:
	movq	8(%rdi), %rsi
	movq	%rsi, %rax
	shrq	$56, %rax
	cmpl	$208, %eax
	jne	.LBB8_2
	movq	(%rdi), %rdi
	lock		decq	-16(%rdi)
	je	.LBB8_3
.LBB8_2:
	retq
.LBB8_3:
	jmpq	*..HeapBuffer$LT$H$GT$17on_last_reference...E@GOTPCREL(%rip)
```

16 バイトのコピー (`movups`/`movaps` の対) と `subq $24, %rsp` が無くなっている。

`Vec<LeanString>` の drop glue (80 命令 → 66 命令) では、関数ポインタのロードが
ループの外へ出た。

```asm
	leaq	16(%r14), %r15
	movq	..on_last_reference...@GOTPCREL(%rip), %r12   ; ループ外へ
	jmp	.LBB9_2
.LBB9_5:
	addq	$16, %r15
	decq	%r13
	je	.LBB9_6
.LBB9_2:
	cmpb	$-48, -1(%r15)
	jne	.LBB9_5
	movq	-8(%r15), %rsi
	movq	-16(%r15), %rdi
	lock		decq	-16(%rdi)
	jne	.LBB9_5
	callq	*%r12
```

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
| `Repr::drop_in` | repr.rs:302-310 |
| `Repr::replace_inner` | repr.rs:281-291 |
| `reserve` の unshare 経路 | repr.rs:499 |
| `shrink_to` の共有経路 | repr.rs:569 |
| `truncate_unchecked` の共有経路 | repr.rs:807 |
| `into_immutable` / `into_mutable` | repr.rs:923/939/975 |
| `ensure_modifiable` | repr.rs:841 |

同じ理屈は `release` 以外の cold ヘルパにも当てはまるが、今回の調査では他に対象は無い。

- `Repr::make_shallow_clone` の `ref_count_overflow` (repr.rs:267-271) は `&Repr<M>` を
  取るが、`panic!` で終わる `-> !` の関数で、`probe_clone` は既に理想形
  (12 命令、heap 側は判別子比較と `lock incq` の 2 命令、cold は `.text.unlikely`)。
  変える必要はない。
- `reserve` の `outline!` マクロが作る関数は `&mut Repr<Mutable>` を取る
  (repr.rs:479, 490, 509, 516, 526)。ここは呼び出し後も `self` を使い続けるので
  参照渡しが必要であり、値渡しにはできない。

## 併せて検討する: unique な drop で atomic RMW を避ける

同じ `release` に、もう 1 つ独立した案がある。参照カウントを減らす前に
`Acquire` のロードで 1 かどうかを見る形。

```rust
#[inline]
pub(super) unsafe fn release(&mut self) {
    if self.reference_count().load(Acquire) == 1 {
        // 他の所有者はいない。RMW は不要。
        unsafe { Self::on_last_reference(ptr::read(self)) };
        return;
    }
    if self.reference_count().fetch_sub(1, Release) == 1 {
        fence(Acquire);
        unsafe { Self::on_last_reference(ptr::read(self)) };
    }
}
```

健全性の根拠は `Arc::get_mut` の fast path と同じ。カウントを増やすにはすでに
ハンドルを 1 つ持っている必要があるので、1 を観測できた時点で他の所有者は存在せず、
以後現れることもない。`Acquire` のロードは最後の所有者の `Release` デクリメントが
書いた値を読むので、それと synchronizes-with が成立し、解放はその所有者の
すべてのアクセスの後に起きる。したがって `fence(Acquire)` は RMW 側の arm にだけ残る。

lean_string は同じ `load(Acquire) == 1` の判定を `is_unique()` (heap_buffer.rs:189-191) で
既に使っているので、メモリモデルの前提はコードベースに受け入れ済み。

**ただし利得は自明ではない**。x86 では `lock xadd` (フルバリア、20 サイクル程度) が
素の `mov` に変わるので単独所有の drop は速くなるが、共有されている drop には
ロードが 1 本増える。さらに悪いことに、実際に競合しているとロードがキャッシュラインを
Shared 状態で持ち込み、続く RMW が Shared→Exclusive のコヒーレンス昇格を余計に踏む。

したがって「単独所有の drop」と「複数スレッドが共有バッファを drop する」の
両方を測ってから判断する。`tests/loom.rs` と `tests/race_condition.rs` は必須。
この案は本プランの値渡し化とは独立なので、値渡しを入れてから別に測るとよい。

## 検証方針

- asm (主検証):
  - `fn drop_one(s: LeanString)`: 16 バイトのスタックコピー (`movups`/`movaps` の対) が
    消え、inline 変種の drop が「判別子の比較 + 早期 return」だけになること。
  - `fn drop_vec(v: Vec<LeanString>)`: ループ本体が現状より短くなること。
  - x86-64 と aarch64 の両方。
- loom: 全シナリオ。atomic の順序 (`Release` の decrement + 最終参照時の
  `Acquire` fence) は変えていないので、ここで落ちたら実装ミス。
- Miri: 全ターゲット。特に `tests/race_condition.rs` を
  `-Zmiri-preemption-rate=1` 付きで。
- criterion: drop 単体のベンチは無い。`apis.rs` の `from` / `clone` は
  イテレーションごとに構築と破棄を行うので、drop のコストはそこに現れる。
  必要なら `Vec<LeanString>` の drop ベンチを足す。

## 依存

なし。単独で実施できる。
