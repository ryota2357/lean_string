# 04: cold 関数には参照でなく値を渡す

種別: perf / 実装量: 小 / 設計判断: 小

## 背景

`Vec<LeanString>` の drop はよいコードになっているが、1 つだけ drop するときは
16 バイトをスタックにコピーしている
([research/codegen-baseline.md](../research/codegen-baseline.md) の §5)。

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
	callq	..HeapBuffer$LT$H$GT$7release17on_last_reference...
```

原因は `HeapBuffer::release` (src/repr/heap_buffer.rs) の書き方にある。

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

`on_last_reference` が `&mut HeapBuffer<H>` を取るので、インライン展開されない関数に
`*self` のアドレスが渡る。アドレスが外に出る以上、値をレジスタに置いたままにはできない。
その結果、値で受け取った `LeanString` は、cold 関数を呼ばない経路でもスタックに置かれる。

## 方針

cold 側には値を渡し、アドレスを外に出さない。

```rust
#[inline]
pub(super) unsafe fn release(&mut self) {
    if self.reference_count().fetch_sub(1, Release) == 1 {
        // cold 側には値だけを渡す。`&mut self` を渡すと `*self` のアドレスが外に出て、
        // 呼び出しが起きない経路でも呼び出し側で値がスタックに置かれてしまう。
        // SAFETY: 直前の値が 1 だったので他の参照は無い。この後 `self` は使われない
        //         (`# Safety` の契約)。
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
SysV x86-64 でも AAPCS64 でもレジスタ 2 本で渡せる。`ptr::read` で値にしてから渡せば
`&mut self` は外に出ない。

`dealloc` が `&mut self` を取るので `this` は `mut` で受けているが、これはローカル変数の
アドレスで、呼び出し側からは見えない。

## プロトタイプの結果

上のコードを実際に入れて asm を見た。

`probe_drop_one` (= `fn drop_one(s: LeanString)`) はスタックフレームが無くなり、
14 命令から 10 命令になった。cold 関数の呼び出しは末尾ジャンプになる。

```asm
probe_drop_one:
	movq	8(%rdi), %rsi
	movq	%rsi, %rax
	shrq	$56, %rax
	cmpl	$208, %eax
	jne	.LBB10_2
	movq	(%rdi), %rdi
	lock		decq	-16(%rdi)
	je	.LBB10_3
.LBB10_2:
	retq
.LBB10_3:
	jmpq	*..HeapBuffer$LT$H$GT$7release17on_last_reference...@GOTPCREL(%rip)
```

16 バイトのコピー (`movups`/`movaps`) と `subq $24, %rsp` が消えている。

`probe_drop_vec` (`Vec<LeanString>` の drop) も 80 命令から 61 命令になった。

### 値で渡すとレジスタに載る理由

`HeapBuffer` は `(NonNull<u8>, TextLen)` の 2 ワードで、この形は `rax:rdx` / `x0:x1` で受け渡しされる。

| 型 | 受け渡し |
| --- | --- |
| `(NonNull<u8>, usize)` (= `HeapBuffer`) | レジスタ |
| `(*const (), [u8; 7], LastByte)` (= `Repr`) | sret / 間接 |

`Repr` は 16 バイトでも scalar pair ではないので、値で渡しても得はない。
`release` が扱うのは `HeapBuffer` なので、ここでは関係ない。

## 影響する箇所

`release` を呼んでいる箇所はすべて恩恵を受ける。

- `Repr::drop_in`
- `Repr::replace_inner`
- `reserve` の共有解除の経路
- 共有されているときの `shrink_to`
- 共有されているときの `truncate_unchecked`
- `into_immutable` / `into_mutable`
- `ensure_modifiable`

他の cold ヘルパも見たが、同じ変更が必要なものは無かった。

- `Repr::make_shallow_clone` の `ref_count_overflow` は `&Repr<M>` を取るが、`panic!` で終わる
  `-> !` の関数で、`probe_clone` はすでに十分よい (12 命令、heap 側は判別子の比較と `lock incq` の 2 命令、
  cold 側は `.text.unlikely`)。
- `reserve` の `outline!` マクロが作る関数は `&mut Repr<Mutable>` を取る。呼び出しの後も `self` を
  使うので参照で渡す必要があり、値渡しにはできない。

## あわせて検討: unique なときの drop で atomic RMW を避ける

同じ `release` にもう 1 つ、独立した案がある。参照カウントを減らす前に、`Acquire` のロードで
1 かどうかを確認する方法。

```rust
#[inline]
pub(super) unsafe fn release(&mut self) {
    if self.reference_count().load(Acquire) == 1 {
        // 他に所有者はいないので RMW は要らない。
        unsafe { Self::on_last_reference(ptr::read(self)) };
        return;
    }
    if self.reference_count().fetch_sub(1, Release) == 1 {
        fence(Acquire);
        unsafe { Self::on_last_reference(ptr::read(self)) };
    }
}
```

正しさの根拠は `Arc::get_mut` の fast path と同じ。カウントを増やすにはハンドルを 1 つ持っている
必要があるので、1 を観測した時点で他の所有者はおらず、この後に現れることもない。
`Acquire` のロードは最後の所有者が `Release` で減らした値を読むので、その所有者のアクセスは
すべて解放より前に起きる。したがって `fence(Acquire)` は RMW 側にだけあればよい。

lean_string は `is_unique()` で同じ `load(Acquire) == 1` をすでに使っているので、
この前提はコードベースの中で受け入れ済み。

ただし速くなるとは限らない。x86 では単独所有の drop が `lock xadd` (フルバリアで 20 サイクル程度)
から普通の `mov` になるが、共有されている drop にはロードが 1 本増える。さらに実際に競合していると、
ロードでキャッシュラインを Shared 状態で取ってきた後、RMW のために Exclusive への昇格がもう一度必要になる。

「単独所有の drop」と「複数スレッドが共有バッファを drop する」の両方を測ってから決める。
`tests/loom.rs` と `tests/race_condition.rs` は必ず回す。値渡しの変更とは独立しているので、
値渡しを入れた後で別に測る。

## 検証

- asm (主な確認):
  - `fn drop_one(s: LeanString)`: 16 バイトのコピーが消え、inline のときの drop が
    「判別子の比較 + return」だけになっていること。
  - `fn drop_vec(v: Vec<LeanString>)`: ループが今より短くなっていること。
  - x86-64 と aarch64 の両方で見る。
- loom: 全シナリオ。atomic の順序 (`Release` での減算と、最後の参照での `Acquire` fence) は
  変えていないので、ここで落ちたら実装の誤り。
- Miri: 4 ターゲット。`tests/race_condition.rs` は `-Zmiri-preemption-rate=1` を付けて回す。
- criterion: drop 単体のベンチは無い。`apis.rs` の `from` / `clone` は毎回構築と破棄をしているので、
  drop のコストはそこに表れる。必要なら `Vec<LeanString>` の drop のベンチを足す。

## 依存

なし。
