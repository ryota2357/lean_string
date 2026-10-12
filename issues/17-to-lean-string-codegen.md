# `to_lean_string()` / `to_lean_str()` が `Result<_, ToLeanStringError>` を経由し、結果のコピーが細切れになる。`str` の impl は展開されない

- 種別: perf
- 対象: `src/traits.rs` の `ToLeanString::to_lean_string` (既定実装)、`impl<T: fmt::Display> ToLeanString for T`、
  `impl ToLeanString for str`、同じく `ToLeanStr` の 3 か所
- 影響:
  - `n.to_lean_string()` / `n.to_lean_str()` で、変換の本体 (`NumToRepr::into_repr`) が呼び出し側に展開されない場合
    (いまの x86-64 では `i16` / `i32` / `i64` / `u64`、aarch64 ではそれに加えて場合により `u32`)。1 回の変換ごとに、
    16 バイトの結果をスタックから細切れに読み直す。
  - `"literal".to_lean_string()` は Rust でよく使われる書き方だが、`LeanString::from("literal")` と違って毎回関数呼び出しと
    結果のメモリ経由のコピーになる。1 回あたり数 ns 程度の小さな差 (推測、未計測)。
- 優先度: 低

## 問題

`5dd94ea`、rustc 1.97.0、LTO なしの下流クレートの asm。

### 整数: 結果のコピーが重なり合う 4 つのロード/ストアに分かれる

x86-64、`pub fn probe_i32(n: i32) -> LeanString { n.to_lean_string() }`:

```asm
	movq	%rsp, %rdi
	callq	..._9NumToRepr9into_repr...      # Result<Repr, ReserveError> を (%rsp) に書く
	movzbl	15(%rsp), %eax
	cmpb	$-1, %al                          # Err か
	je	...
	movzbl	(%rsp), %ecx                      # 0 バイト目
	movq	7(%rsp), %rdx                     # 7..15 バイト目
	movq	%rdx, 7(%rbx)
	movq	1(%rsp), %rdx                     # 1..9 バイト目
	movq	%rdx, 1(%rbx)
	movb	%cl, (%rbx)
	movb	%al, 15(%rbx)
```

16 バイトの値を 1 + 8 + 8 + 1 バイトの重なり合うロード/ストアでコピーしている。`movq 7(%rsp)` と `movq 1(%rsp)` は
`into_repr` が書いた複数のストアにまたがるので、x86 では store-to-load forwarding が成立しない。

aarch64 ではさらに悪く、一度スタックの別の場所に移してから書き出している (`u32` の `into_repr` が展開されなかった状態。
[#06](./06-int-to-repr-registers.md) の修正例を当てたときにそうなった):

```asm
	bl	..._9NumToRepr9into_repr...
	ldrb	w8, [sp, #31]
	cmp	w8, #255
	b.eq	...
	ldur	x9, [sp, #17]
	ldur	x10, [sp, #23]
	str	x9, [sp]
	ldrb	w9, [sp, #16]
	stur	x10, [sp, #6]
	ldr	x10, [sp]
	strb	w9, [x19]
	ldur	x9, [sp, #6]
	stur	x10, [x19, #1]
	stur	x9, [x19, #7]
	strb	w8, [x19, #15]
```

比較として、同じ `into_repr` の結果を `Result<Repr, ReserveError>` のまま `unwrap` した場合は、x86-64 で `movups` 1 組、
aarch64 で `ldr q0` / `str q0` 1 組のコピーになる (下の修正例)。

### `str`: 呼び出しが展開されない

x86-64、`pub fn probe_str_tls(s: &str) -> LeanString { s.to_lean_string() }`:

```asm
	movq	%rsp, %rdi
	callq	*_RNvXNtCs..._11lean_string6traitseNtB2_12ToLeanString18try_to_lean_string@GOTPCREL(%rip)
	cmpb	$-1, 15(%rsp)
	je	...
	movups	(%rsp), %xmm0
	movups	%xmm0, (%rbx)
```

`LeanString::from(s)` は inline の経路がレジスタだけで完結し、2 つのストアで返すのに対し、こちらは GOT 経由の呼び出しと
`Result<LeanString, ToLeanStringError>` のスタック経由の受け渡しになる。`to_lean_str()` も同じ形。

## 原因

1. `to_lean_string` の既定実装は `self.try_to_lean_string().unwrap_with_msg()` で、`try_to_lean_string` は
   `Repr::from_num(*s)?` で `ReserveError` を `ToLeanStringError` に変換して `Result<LeanString, ToLeanStringError>` を返す。
   `ToLeanStringError` は 2 つのバリアントを持つ 1 バイトの enum なので、`Result<LeanString, ToLeanStringError>` は Ok/Err の判別を
   `LastByte` の niche (15 バイト目の 0xFF) に置き、Err の中身 (1 バイト) を 0 バイト目に置くレイアウトになる。そのため LLVM は
   この値を 0 バイト目 / 1〜14 バイト目 / 15 バイト目に分割して扱い、`Repr` のフィールド境界 (8 バイト目) とも合わない 14 バイトの
   部分を重なり合う 2 つの 8 バイトのロード/ストアでコピーする (asm からの推測。`Result<Repr, ReserveError>` では Err が ZST なので
   分割されないことは確認した)。`into_repr` が展開される型ではこのコピーは消えるが、どの型が展開されるかはインライン化の判定次第で揺れる。
2. `impl ToLeanString for str` / `impl ToLeanStr for str` の `try_to_lean_string` / `try_to_lean_str` に `#[inline]` が無い。
   generic でない関数なので、LTO なしでは下流のクレートで展開できない。既定実装の `to_lean_string` は asm のとおり呼び出し側で
   展開されているが、中から呼ぶ `try_to_lean_string` が呼び出しのまま残る。

## 修正例

失敗しうる理由が確保だけの型 (整数・浮動小数点・`bool`・`char`・`String`・`str`) では、`ToLeanStringError` を経由せずに
`Result<Repr, ReserveError>` から直接 `unwrap` する `to_lean_string` を書く。

```rust
impl ToLeanString for str {
    #[inline]
    fn to_lean_string(&self) -> LeanString {
        LeanString(Repr::from_str(self).unwrap_with_msg())
    }

    #[inline]
    fn try_to_lean_string(&self) -> Result<LeanString, ToLeanStringError> {
        let repr = Repr::from_str(self)?;
        Ok(LeanString(repr))
    }
}

impl<T: fmt::Display> ToLeanString for T {
    fn to_lean_string(&self) -> LeanString {
        let repr: Result<Repr<Mutable>, ReserveError> = match_type!(self, {
            &i8 as s => Repr::from_num(*s),
            &u8 as s => Repr::from_num(*s),
            // ... try_to_lean_string と同じ型の列 (`?` なし)
            &usize as s => Repr::from_num(*s),
            s => return s.try_to_lean_string().unwrap_with_msg(),
        });
        LeanString(repr.unwrap_with_msg())
    }

    fn try_to_lean_string(&self) -> Result<LeanString, ToLeanStringError> { /* いまのまま */ }
}
```

`ToLeanStr` も同様。型の列が 2 か所に重複するので、`match_type!` の腕を返すマクロにまとめるか、
`fn to_repr<M>(&self) -> Option<Result<Repr<M>, ReserveError>>` のような内部関数に括り出して `try_to_lean_string` /
`to_lean_string` の両方から使う形にするとよい。

試作 (blanket impl の整数の腕だけ) で、x86-64 の `u64` の呼び出し側は

```asm
	callq	..._9NumToRepr9into_repr...
	cmpb	$-1, 15(%rsp)
	je	...
	movups	(%rsp), %xmm0
	movups	%xmm0, (%rbx)
```

aarch64 の `u32` は `ldrb` / `cmp` の後 `ldr q0, [sp]` / `str q0, [x19]` だけになり、関数全体の命令数は x86-64 で 22 → 15、
aarch64 で 30 → 20 になった。16 バイトのロードが 2 つの 8 バイトのストアにまたがる点は残る (`into_repr` が sret で返す以上
避けられない) が、転送失敗の回数とコピーの命令は減る。`str` の impl への `#[inline]` と `to_lean_string` の上書きは試していない。

整数については、本筋の改善は `into_repr` 自体が展開されること ([#06](./06-int-to-repr-registers.md)) で、そちらが入れば x86-64 の
32-bit 以下の型ではこのコピー自体が消える。この修正は `u64` / `i64` と aarch64 のように展開されない場合の後始末にあたる。

## 確認方法

- asm: `pub fn f(n: i64) -> LeanString { n.to_lean_string() }` (x86-64 / aarch64、LTO なし、呼び出し元を 2 つ以上置く) で、
  `into_repr` の呼び出しの後のコピーが 16 バイト 1 組になっていること。`s.to_lean_string()` (`s: &str`) が `LeanString::from(s)` と
  同じく呼び出しの無い inline の経路を持つこと。
- テスト: 既存の `tests/property.rs` (整数・浮動小数点・`bool`・`char` の `*_to_lean_string` / `*_to_lean_str`、`String` の
  `string_to_lean_string` / `string_to_lean_str`)、`tests/alloc_string.rs` (`"..".to_lean_string()` を使うテスト)、
  `tests/out_of_memory.rs` (`try_to_lean_string` / `try_to_lean_str` のエラー経路) がそのまま通ること。`str` に対する
  `to_lean_string()` / `to_lean_str()` を任意の入力で比べる property test は無いので、`string_to_lean_string` に `s.as_str()` の版を足す。

## 関連

- [#06](./06-int-to-repr-registers.md): 整数変換 (`into_repr`) がレジスタで値を返せず展開されない
- [#18](./18-ref-conversion-via-clone.md): blanket impl の `&LeanStr` / `&LeanString` の腕が clone してから変換する
