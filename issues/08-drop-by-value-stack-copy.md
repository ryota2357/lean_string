# 値で持っている `LeanString` / `LeanStr` の drop が 16 バイトをスタックにコピーする

- 種別: perf
- 対象: `src/repr/heap_buffer.rs` の `HeapBuffer::release` (内側の `on_last_reference`)。影響は `Repr::drop_in` / `Repr::replace_inner` 経由ですべての drop に及ぶ
- 影響: 関数の引数や一時値として値で持っている文字列を drop するたびに、inline / static であっても 16 バイトのスタックコピーが入る。呼び出し側が直前に 8 バイト 2 回で書いた値を 16 バイト 1 回で読み直す形になり、x86-64 では store-to-load forwarding が効かないぶん遅くなっているとみられる (推測)。`Vec<LeanString>` のように要素のアドレスがすでにある drop には影響しない
- 優先度: 中

## 問題

`5dd94ea`、rustc 1.97.0、`opt-level=3`、`codegen-units=1`、LTO なし。下流クレートに `#[unsafe(no_mangle)]` のラッパを置いて `--emit asm` で見た。

```rust
pub fn probe_drop_one(s: LeanString) { drop(s) }
```

x86-64:

```asm
probe_drop_one:
	subq	$24, %rsp
	movups	(%rdi), %xmm0          # 引数を丸ごとスタックへコピー
	movaps	%xmm0, (%rsp)
	cmpb	$-48, 15(%rsp)         # 判別子もコピーから読む
	jne	.LBB20_3
	movq	(%rsp), %rax
	lock		decq	-16(%rax)
	je	.LBB20_2
.LBB20_3:
	addq	$24, %rsp
	retq
.LBB20_2:
	movq	%rsp, %rdi             # cold 関数にはコピーのアドレスを渡す
	callq	..HeapBuffer..release17on_last_reference..GrowableHeader..
	addq	$24, %rsp
	retq
```

aarch64 でも同じく `ldr q0, [x0]` / `str q0, [sp]` でスタックにコピーし、`mov x0, sp` で渡している。

同じ形は値で持つ drop のすべてに出る。

| probe (x86-64) | 命令数 | スタックコピー |
| --- | --- | --- |
| `fn(LeanString)` の drop | 14 | あり |
| `fn(LeanStr)` の drop | 14 | あり |
| `fn(Option<LeanString>)` の drop | 17 | あり |
| `let _ = s.clone();` (clone した一時値の drop) | 20 | あり |
| `fn(a: LeanString, b: LeanString)` で 2 つ drop | 43 | 2 回 |
| `&mut ManuallyDrop<LeanString>` を drop | 6 | なし |
| `Vec<LeanString>` の drop (ループ本体) | — | なし (要素のアドレスを渡している) |

`LeanString::try_into_lean_str` の中で heap の参照を手放す箇所 (`heap.release()`) も、同じ理由でローカルの `HeapBuffer` をスタックに置いている (`movq %r13, (%rsp)` / `movq %rbp, 8(%rsp)` の後、`movq %rsp, %rdi` で cold 関数を呼ぶ)。

## 原因

`HeapBuffer::release` は最後の参照のときの処理を入れ子の `#[cold] fn on_last_reference<H>(this: &mut HeapBuffer<H>)` に切り出している。`release` 自体はインライン展開されるが、`on_last_reference` はされないので、`&mut` で渡す `*self` のアドレスが関数の外に出る。アドレスを渡す以上、値はメモリ上に置く必要がある。

値で受け取った `LeanString` は Rust ABI では呼び出し側のメモリへのポインタで渡されるが、LLVM はその領域をそのまま cold 関数に渡さず、自分のフレームにコピーしてから使っている (なぜ引数の領域を直接使わないのかは未確認)。いずれにしても、参照を渡さなければコピーは要らない。`&mut ManuallyDrop<LeanString>` を drop する probe ではコピーが出ないこと、下の修正で消えることから、原因が「cold 関数への参照渡し」であることは確認できている。

`HeapBuffer<H>` は `{ ptr: NonNull<u8>, len: TextLen }` の 2 ワードなので、値で渡せば x86-64 でも aarch64 でもレジスタ 2 本で済む。

## 修正例

cold 側に値を渡し、アドレスを外に出さない。

```rust
pub(super) unsafe fn release(&mut self) {
    if self.reference_count().fetch_sub(1, Release) == 1 {
        // SAFETY: 直前の値が 1 なので他の参照は無い。`# Safety` により `self` はこの後使われない。
        unsafe { Self::on_last_reference(ptr::read(self)) };
    }
}

#[cold]
#[inline(never)]
unsafe fn on_last_reference(mut this: Self) {
    // And the `Acquire` fence ensures we see all writes before freeing the memory.
    fence(Acquire);
    // SAFETY: 呼び出し側の条件により他の参照は無い。
    unsafe { this.dealloc() };
}
```

試した結果 (同じ probe):

```asm
probe_drop_one:
	movq	8(%rdi), %rsi
	movq	%rsi, %rax
	shrq	$56, %rax
	cmpl	$208, %eax
	jne	.LBB21_2
	movq	(%rdi), %rdi
	lock		decq	-16(%rdi)
	je	.LBB21_3
.LBB21_2:
	retq
.LBB21_3:
	jmpq	*..HeapBuffer..GrowableHeader..on_last_reference..@GOTPCREL(%rip)   # 末尾呼び出し
```

| probe (x86-64) | 前 | 後 |
| --- | --- | --- |
| `fn(LeanString)` の drop | 14 | 10 (フレームなし) |
| `fn(LeanStr)` の drop | 14 | 10 |
| `fn(Option<LeanString>)` の drop | 17 | `probe_drop_one` と同一コードに畳まれた |
| clone した一時値の drop | 20 | 13 |
| `fn(a, b)` で 2 つ drop | 43 | 37 |
| `Box<[LeanString]>` の drop | 75 | 62 |
| `Vec<LeanString>` の drop | 80 | 66 (ループ本体は同等。差は主に後始末の部分) |

aarch64 では `fn(LeanString)` の drop が 22 → 21 命令で、スタックコピーの代わりに callee-saved の `x19`/`x20` に値を保持する形になる (`__aarch64_ldadd8_rel` の呼び出しをまたぐため)。コピーは消えるが、x86-64 ほどの差は無い。i686 (`--emit asm` のみ) では、現行は値の 2 ワードを hot path でスタックに書いてから cold 関数にそのアドレスを渡しているが、修正後はスタックへの書き込みが cold path の `pushl` 2 回だけになる。

`&mut self` で受ける呼び出し元 (`clone_from`、`clear` など) は、もともと `self` のアドレスを渡していただけなのでほぼ変わらない (cold 側の引数のために 2 ワードを読み込む分、`clone_from` は 27 → 30 命令になる。cold 側なので実質的な差は無い)。

時間 (同じバイナリ内で 3 つの版 (現行、修正後、下の「unique な drop で atomic RMW を避ける」を足した版) を交互に測定。4 コアの負荷のある共有マシンなのでノイズは大きい。3 回とも同じ傾向):

| 操作 (`T48` は 48 バイト) | 現行 | 修正後 |
| --- | --- | --- |
| `LeanString::from(T48)` → `#[inline(never)] fn(LeanString)` で drop | 27〜32 ns | 20 ns |
| 共有バッファの clone → 同じ関数で drop (1 スレッド) | 15.5 ns | 13.5〜15.8 ns |

1 行目の差は 16 バイトのコピーだけでは説明しきれない大きさで、呼び出し側が 8 バイト 2 回で書いた値を呼び出し先が `movups` の 16 バイト 1 回で読んでいる (逆アセンブルで確認) ことによる store-to-load forwarding の失敗がおそらく効いている。性能カウンタでは確認していない。

### 併せて試したもの: unique な drop で atomic RMW を避ける

`release` の先頭で `load(Acquire) == 1` なら RMW なしで解放する案も試した (`Arc::get_mut` / このクレートの `is_unique` と同じ根拠で健全。loom 4 シナリオ、Miri x86_64 の `race_condition` と [#26](./26-race-tests-unique-path.md) の race テストを `-Zmiri-preemption-rate=1 -Zmiri-many-seeds=0..32` で通過。同じ fast path を `load(Relaxed)` にすると既存の `race_condition` が検出するので、テストに検出力はある)。結果は得失が分かれる。

| 操作 | 値渡しのみ | 値渡し + unique fast path |
| --- | --- | --- |
| unique な heap 文字列の構築 + drop | 20 ns | 14.5〜16 ns |
| 共有バッファの clone + drop (1 スレッド) | 13.5〜15.8 ns | 18.8 ns |
| 共有バッファの clone + drop (2 スレッドで競合) | 58〜61 ns | 77〜78 ns |

unique な drop は `lock` 付き命令が消える分 5 ns ほど速くなるが、共有されている drop にはロードと分岐が 1 つずつ増え、競合時はキャッシュラインを Shared で読んでから RMW で Exclusive に取り直すことになって 3 割ほど遅くなる。clone が安いことがこのクレートの特徴なので、共有されている drop を遅くする変更は既定にしないほうがよい (std の `Arc` も採っていない)。採るなら実際の利用に近いベンチで判断する。

## 確認方法

- asm: `fn(LeanString)` / `fn(LeanStr)` / `fn(Option<LeanString>)` の drop にスタックフレームと 16 バイトのコピーが無いこと。cold 関数が `.text.unlikely` に置かれたままであること。x86-64 と aarch64 で見る。
- 既存のテスト、loom、Miri (`race_condition` は `-Zmiri-preemption-rate=1`)。atomic の順序は変えていないので、ここで落ちるなら実装の誤り。
