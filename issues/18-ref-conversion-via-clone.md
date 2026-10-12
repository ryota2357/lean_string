# 参照からの `LeanString` ⇄ `LeanStr` 変換が `clone()` してから所有権ごと変換している

- 種別: perf
- 対象: `src/lib.rs` の `From<&LeanString> for LeanStr` / `From<&LeanStr> for LeanString`、`src/traits.rs` の
  `impl<T: fmt::Display> ToLeanStr for T` の `&LeanString` の腕 / `impl<T: fmt::Display> ToLeanString for T` の `&LeanStr` の腕
- 影響: heap の文字列では、結果は必ず新しいバッファへのコピーになるのに、その前後で参照カウントの atomic RMW が 2 回
  (`lock incq` と `lock decq`) 入り、共有判定の分岐を通る。別スレッドと共有中ならキャッシュラインの奪い合いも起きる。
  inline / static の文字列でも、200 命令ほどの変換関数を呼ぶ
- 優先度: 低

## 問題

4 か所とも同じ形をしている。

```rust
// src/lib.rs
impl From<&LeanString> for LeanStr {
    fn from(value: &LeanString) -> Self {
        value.clone().into_lean_str()
    }
}
// From<&LeanStr> for LeanString は value.clone().into_lean_string()

// src/traits.rs
&LeanString as s => return Ok(s.clone().try_into_lean_str()?),     // ToLeanStr
&LeanStr as s => return Ok(s.clone().try_into_lean_string()?),     // ToLeanString
```

`clone()` で参照カウントが 2 以上になるので、`Repr::into_immutable` / `Repr::into_mutable` は必ず「共有されている」分岐に入り、
`Repr::from_str` で新しいバッファにコピーしてから `release` で参照カウントを戻す。確保は 1 回で済んでいるが、余計な処理が付く。

`5dd94ea`、rustc 1.97.0、x86-64、LTO なしの下流クレートで `fn(&LeanString) -> LeanStr { LeanStr::from(s) }` を見ると次のようになる。

```asm
probe_from_ref_string:
	pushq	%rbx
	subq	$32, %rsp
	movq	%rdi, %rbx
	cmpb	$-48, 15(%rsi)
	jne	.LBB27_2
	movq	(%rsi), %rax
	lock		incq	-16(%rax)          # clone
	jle	.LBB27_4
.LBB27_2:
	movups	(%rsi), %xmm0
	movaps	%xmm0, 16(%rsp)
	movq	%rsp, %rdi
	leaq	16(%rsp), %rsi
	callq	..LeanString17try_into_lean_str  # 202 命令の関数。中で alloc + コピー + lock decq
	...
```

`pub fn probe_ls_tls(s: &LeanStr) -> LeanString { s.to_lean_string() }` も、heap の場合は `lock incq` (clone) の後に
`LeanStr::try_into_lean_string` を呼び、その中で参照カウント 2 を見て `Repr::<Mutable>::from_str` でコピーしてから `lock decq` する。

確保を数える `#[global_allocator]` で測ると、48 バイトの heap 文字列で 4 か所とも alloc 1 回 (realloc 0 回) で、
確保の回数そのものは最小になっている。

## 原因

参照から反対側の型を作るのに、所有権を取る変換 (`into_lean_str` / `into_lean_string`) を流用しているため。所有権を取る変換は
「unique ならバッファを再利用する」ための分岐を持っているが、参照からの変換では clone した時点で unique になりえないので、その分岐は
使われない。inline / static の場合も、`try_into_lean_str` / `try_into_lean_string` が展開されない ([#09](./09-conversion-restore-on-failure.md))
ので、ビット列を写すだけの変換に関数呼び出しが付く。

## 修正例

heap なら内容をコピーし、inline / static ならビット列をそのまま写す関数を `Repr` に足して使う。

```rust
// src/repr.rs (impl<M: Mutability> Repr<M>)
/// Copies `self` into a `Repr` of the other mutability. Inline and static buffers are copied
/// as they are; a heap buffer's content is copied into a new buffer.
#[inline]
pub(crate) fn copy_as<N: Mutability>(&self) -> Result<Repr<N>, ReserveError> {
    if self.is_heap_buffer() {
        Repr::from_str(self.as_str())
    } else {
        // SAFETY: Inline and static buffers have the same representation in both mutabilities,
        // and own nothing.
        Ok(unsafe { ptr::read((self as *const Self).cast::<Repr<N>>()) })
    }
}

// src/lib.rs
impl From<&LeanString> for LeanStr {
    fn from(value: &LeanString) -> Self {
        LeanStr(value.0.copy_as().unwrap_with_msg())
    }
}
// From<&LeanStr> for LeanString も同様

// src/traits.rs
&LeanStr as s => s.0.copy_as()?,       // ToLeanString
&LeanString as s => s.0.copy_as()?,    // ToLeanStr
```

`ptr::read` でのビット列のコピーは、`Repr::into_immutable` / `into_mutable` の `transmute` と同じ根拠 (inline と static の表現は
両方の型で同じ) に基づく。

分岐を省いて単に `Repr::from_str(s.as_str())` にしてはいけない。static バッファの文字列まで heap にコピーしてしまい、既存の
`tests/conversion.rs` の `static_ref_conversion_keeps_static` (`LeanStr::from(&lean_string).as_static_str().is_some()` を確かめる) が落ちる。
`copy_as` は static をコピーしないので、`&'static str` を指したままになる。

試した結果: atomic RMW と `try_into_lean_str` の呼び出しが無くなり、inline / static の経路は判別子の比較と 16 バイトのコピーだけになる。
代わりに `Repr::from_str` (heap 確保とコピー) が呼び出し側に展開されるので、probe の命令数は 25 → 110 に増える。コードサイズを
気にするなら heap の分岐だけを `#[inline(never)]` の関数に出せばよい (未確認)。確保回数は変わらない (heap で alloc 1 回)。全テストが通る。

## 確認方法

- asm: `fn(&LeanString) -> LeanStr { LeanStr::from(s) }`、`fn(&LeanStr) -> LeanString { s.to_lean_string() }` などに `lock` 付き命令が無いこと。
- 確保回数: heap の文字列で alloc 1 回のまま。static の文字列で alloc 0 回 (`as_static_str()` が `Some`)、inline で alloc 0 回。
- テスト: `LeanStr` / `LeanString` の inline・heap・static それぞれからの `From<&..>`、`to_lean_string()` / `to_lean_str()`
  (いまの `static_ref_conversion_keeps_static` は `From<&..>` だけを見ているので、`to_lean_string()` / `to_lean_str()` でも static のまま残ることを足す)。

## 関連

- [#09](./09-conversion-restore-on-failure.md): `try_into_lean_str` / `try_into_lean_string` が展開されない
- [#17](./17-to-lean-string-codegen.md): `ToLeanString` / `ToLeanStr` の他の腕のコード生成
