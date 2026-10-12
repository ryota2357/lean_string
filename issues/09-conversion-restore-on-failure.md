# `into_lean_str` / `into_lean_string` が使われない「失敗時に元へ戻す」仕組みのせいで重く、変換不要な文字列でも大きな関数を呼ぶ

- 種別: perf, safety-doc
- 対象: `src/lib.rs` の `LeanString::try_into_lean_str` / `LeanStr::try_into_lean_string` (と `into_lean_str` / `into_lean_string` / `From<LeanString> for LeanStr` などそれらを呼ぶもの)、`src/repr.rs` の `Repr::<Mutable>::into_immutable` / `Repr::<Immutable>::into_mutable`、`src/repr/heap_buffer.rs` の `HeapBuffer::into_exact` / `into_growable`
- 影響: inline / static の文字列の変換は「ゼロコスト」と書かれているが、実際には 200 命令ほどの関数を呼び出す。heap の変換では、失敗時に元の値を有効なまま返すための復元処理 (クレートでいちばん込み入った unsafe) を抱えているが、呼び出し側はその値を drop するだけである
- 優先度: 中

## 問題

`5dd94ea`、rustc 1.97.0、`opt-level=3`、`codegen-units=1`、LTO なし、下流クレートの probe で確認。

1. 変換関数がインライン展開されない。`pub fn probe_try_into_lean_str(s: LeanString) -> Result<LeanStr, ReserveError> { s.try_into_lean_str() }` は `try_into_lean_str` の呼び出しになり、その本体は x86-64 で 202 命令 (aarch64 203)、`try_into_lean_string` は 168 (161) 命令ある。inline / static の文字列でも、callee-saved レジスタ 6 本の退避、`mem::replace` による `Repr::new()` の書き込み、`self` の Drop のための判別子の再読み込みを通る。

   ```asm
   _RNv..LeanString17try_into_lean_str:
   	pushq	%rbp
   	pushq	%r15
   	pushq	%r14
   	pushq	%r13
   	pushq	%r12
   	pushq	%rbx
   	subq	$40, %rsp
   	...
   	movq	(%rsi), %r13
   	movq	8(%rsi), %rbp
   	movq	$0, (%rsi)              # mem::replace(&mut self.0, Repr::new())
   	movq	%rax, 8(%rsi)
   	...
   	cmpl	$208, %ecx              # heap でなければ ↓ へ
   	jne	.LBB8_46
   	...
   .LBB8_47:
   	movq	%r15, (%r14)
   	movq	%rbp, 8(%r14)
   	cmpb	$-48, 15(%rbx)          # self の Drop: いま書いた Repr::new() を読み直す
   	jne	.LBB8_50
   	...
   ```

   `self` に Drop があり、途中の呼び出しが unwind しうるので landing pad も付く。

2. heap の unique な変換では `HeapBuffer::into_exact` を呼ぶが、戻り値の `Result<HeapBuffer<ExactHeader>, (HeapBuffer<GrowableHeader>, ReserveError)>` は 24 バイトでレジスタに収まらず、sret で返る。

   ```asm
   	leaq	16(%rsp), %rdi          # sret
   	callq	*..HeapBuffer..GrowableHeader..10into_exact@GOTPCREL(%rip)
   	movq	24(%rsp), %r15
   	movq	32(%rsp), %rbp
   	cmpl	$1, 16(%rsp)
   ```

3. `into_exact` は `realloc` の前に文字列を新しい位置へ `ptr::copy` するので、`realloc` が失敗したときに「バイト列を元の位置へ戻す」「(32-bit で) 長さの prefix を書き戻す」「`GrowableHeader` を書き戻す」という復元をしている。ところが失敗時に元の値を受け取る `try_into_lean_str` は、それを `self` に戻して即座に drop するだけである (doc にも "the original string is dropped on failure" とある)。`into_immutable` / `into_mutable` / `into_exact` / `into_growable` の呼び出し元は他に無い。

   - バイト列の復元は、どこからも観測されない。復元の `ptr::copy(new_data, old_data, len)` を消しても、realloc 失敗のテスト ([#25](./25-realloc-failure-tests.md) のもの) は Miri (x86_64) で通る。
   - ヘッダの復元が要るのは、その後の Drop が `GrowableHeader` の容量から `Layout` を計算して解放するためだけである。

## 原因

`Repr::into_immutable` / `into_mutable` と `HeapBuffer::into_exact` / `into_growable` が「失敗したら元の値をエラーと一緒に返す」(`Err((Self, ReserveError))`) 形をしていることが、上の 3 つすべての元になっている。

- 呼び出し側 (`try_into_lean_str`) は返ってきた元の値を drop させるために `mem::replace` で `self.0` を空にして後で戻す必要があり、`self` の Drop と landing pad が残る。fast path (inline / static) と heap の経路が 1 つの関数に入っているので、関数全体が大きくなりインライン展開されない。
- エラー側に値を持つ `Result` は `LastByte` / `NonNull` の niche を使えないので 24 バイトになる。
- 元の値を有効なまま返すために、`into_exact` は移動したバイト列とヘッダを書き戻す必要がある。

## 修正例

失敗したら変換関数の中で解放する。heap の経路は別関数に切り出し、fast path だけが呼び出し側に展開されるようにする。

```rust
// src/lib.rs
#[inline]
pub fn try_into_lean_str(self) -> Result<LeanStr, ReserveError> {
    let this = mem::ManuallyDrop::new(self);
    // SAFETY: `this` is never dropped, so the repr is moved out of it.
    let repr = unsafe { core::ptr::read(&this.0) };
    repr.into_immutable().map(LeanStr)
}
// try_into_lean_string も同じ形 (into_mutable / LeanString)

// src/repr.rs
impl Repr<Mutable> {
    /// Converts into an immutable representation. On failure, `self` is dropped.
    #[inline]
    pub(crate) fn into_immutable(self) -> Result<Repr<Immutable>, ReserveError> {
        if !self.is_heap_buffer() {
            return Ok(unsafe { mem::transmute::<Repr<Mutable>, Repr<Immutable>>(self) });
        }
        heap_into_immutable(unsafe { ptr::read(self.as_heap_buffer()) })
    }
}

// #[inline] を付けない (クレート内でだけコード生成される)
fn heap_into_immutable(mut heap: HeapBuffer<Mutable>) -> Result<Repr<Immutable>, ReserveError> {
    let heap_str = heap.as_str();
    if heap.is_unique() {
        if heap_str.len() <= MAX_INLINE_SIZE {
            let inline = Repr::from_inline(unsafe { InlineBuffer::new(heap_str) });
            unsafe { heap.release() };
            Ok(inline)
        } else {
            unsafe { heap.into_exact() }.map(Repr::from_heap)
        }
    } else {
        // 共有されているときはコピーし、成否にかかわらず参照を手放す
        let next = Repr::<Immutable>::from_str(heap_str);
        unsafe { heap.release() };
        next
    }
}
// into_mutable / heap_into_mutable も同じ形

// src/repr/heap_buffer.rs
/// On failure, the buffer is freed.
pub(super) unsafe fn into_exact(self) -> Result<HeapBuffer<ExactHeader>, ReserveError> {
    // ... (前半は今と同じ。old_len_prefix と old_capacity は不要になる)
    let new_allocation = unsafe { realloc(allocation, old_layout, new_layout.size()) };
    if new_allocation.is_null() {
        // 移動でヘッダが上書きされていても、移動前に計算した old_layout で解放できる。
        unsafe { dealloc(allocation, old_layout) };
        return Err(ReserveError);
    }
    // ... (後半は今と同じ)
}
// into_growable も realloc 失敗時は dealloc(allocation, old_layout) して Err(ReserveError)。
// layout_for の失敗時は `{ self }.dealloc()` してから返す。
```

試した結果 (x86-64):

| | 現行 | 修正後 |
| --- | --- | --- |
| `probe_try_into_lean_str` | 6 命令 + `try_into_lean_str` (202 命令) の呼び出し | 30 命令。inline / static は呼び出しなし、heap のときだけ `heap_into_immutable` を呼ぶ |
| `probe_try_into_lean_string` | 6 + 168 | 30。heap のときだけ `heap_into_mutable` |
| `probe_into_lean_str` (`unwrap` 付き) | 15 + 202 | 26 |
| heap 側の本体 | `try_into_lean_str` 202 + `into_exact` 48 | `heap_into_immutable` 110 (`into_exact` を含む) |

`into_exact` は `heap_into_immutable` に展開され、24 バイトの戻り値の sret は無くなる。`heap_into_immutable` は `HeapBuffer` を値で受けるのでレジスタ 2 本で渡る。`heap_into_immutable` 自身の戻り値 (`Result<Repr<Immutable>, ReserveError>`、16 バイト) は、`Repr` が scalar pair ではないので隠しポインタで返る (`LeanString` を返す関数一般と同じで、ここでは問題にしない)。

確保回数 (確保を数える global allocator で測定) は変わらない (unique: realloc 1 回、共有: alloc 1 回)。既存の `conversion` / `out_of_memory` のテストと [#25](./25-realloc-failure-tests.md) のテスト (サイズを照合するアロケータ付き) が native と Miri (x86_64 / i686、strict provenance。i686 では 16 MiB の長さの prefix のテストを含む) で通る。loom と `race_condition` (Miri、`-Zmiri-preemption-rate=1`) も通る。aarch64 も同じ傾向 (`probe_try_into_lean_str` は 3 + 203 → 29)。

修正後の fast path にはまだ「結果をいったんスタックに書いてから読み直す」無駄が残る (`Result<LeanStr, _>` が sret で返るため)。必要なら `try_into_lean_str` 側で `is_heap_buffer()` を見て直接返す形にすれば詰められるが、今回は確認していない。

### 他の案

- 「失敗したら元の値を返す」を残したまま `&mut self` を取り `Result<(), ReserveError>` を返す形 (`Repr<Mutable>` のまま中身を書き換え、成功後に呼び出し側で `transmute`) にもできる。sret は消えるが、`into_exact` の復元はそのまま残る。将来 `Result<LeanStr, (LeanString, ReserveError)>` のような「失敗時に元を返す」公開 API を足す予定があるならこちら。無いなら上の案のほうが unsafe が減るので推奨。

## 確認方法

- asm: `fn(LeanString) -> LeanStr { s.into_lean_str() }` で、inline / static の経路に呼び出しが無いこと。heap の経路に 24 バイトの `Result` の受け渡しが無いこと。x86-64 と aarch64 で見る。
- 確保回数が変わらないこと (unique は realloc 1 回、共有は alloc 1 回)。
- [#25](./25-realloc-failure-tests.md) の realloc 失敗テスト (サイズを照合するアロケータ付き) を native と Miri の x86_64 / i686 で。`into_exact` の失敗経路が単純になるので、32-bit の長さの prefix の有無が変わる場合 (`with_capacity((1 << 24) - 1)` に短い文字列) も含める。
