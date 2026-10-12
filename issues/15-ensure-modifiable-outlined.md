# `remove` / `retain` が inline や unique なバッファでも `ensure_modifiable` を関数呼び出しする

- 種別: perf
- 対象: `src/repr.rs` の `Repr::<Mutable>::ensure_modifiable` と、その呼び出し元 `Repr::<Mutable>::remove`, `Repr::<Mutable>::retain`
- 影響: `remove` を呼ぶたび (とくに inline の短い文字列) に、ほぼ常に何もしない関数への呼び出しが 1 回入る。`retain` は呼び出し 1 回ぶんだけ。
- 優先度: 低

## 問題

`ensure_modifiable` には `#[inline]` が無く、ジェネリックでもないので `lean_string` 側で 1 つの関数として
コード生成され、利用側からは常に呼び出しになる。中身は「static か共有ならコピー」という判定で、
inline や unique な heap では何もしない。

`5dd94ea`、rustc 1.97.0、LTO off の probe (`s.remove(i)`) の x86-64 asm 抜粋 (境界チェックのあと):

```asm
	movq	%rbx, %rdi
	callq	*_RNv...ensure_modifiable@GOTPCREL(%rip)
	testb	%al, %al
	jne	.LBB16_26
	cmpb	$-48, 15(%rbx)          # 呼び出しをまたいだので判別子を読み直す
	movq	%rbx, %rdi
	je	.LBB16_11
```

aarch64 も `bl ...ensure_modifiable` が同じ位置にある。`retain` も最初に削る文字を見つけたあとで同じ関数を呼ぶ。

`push_str` などは「容量か共有の判定はインライン、コピーは `#[cold]` の関数」という形になっている
(`Repr::push_str` の `grow`)。`ensure_modifiable` だけがこの形になっていない。

cachegrind の Ir (1 回あたり、`LeanString::from("hello world")` を作って `remove(3)` する。
文字列の生成と `black_box` を含む): 現状 111、下の修正例 90。比較として `String` は 191 (確保と解放を含む)。

## 原因

`ensure_modifiable` が非インラインの通常の関数であるため (asm で確認)。判定そのものは
判別子の比較と (heap なら) 参照カウントの読み込みだけで、呼び出しのほうが重い。

## 修正例

判定をインラインにし、コピーだけを `#[cold]` の関数に出す。

```rust
    #[inline]
    fn ensure_modifiable(&mut self) -> Result<(), ReserveError> {
        #[cold]
        #[inline(never)]
        fn copy(this: &mut Repr<Mutable>) -> Result<(), ReserveError> {
            let copied = Repr::from_str(this.as_str())?;
            this.replace_inner(copied);
            Ok(())
        }
        if !self.is_modifiable() {
            return copy(self);
        }
        Ok(())
    }
```

試した結果: `probe_remove` から通常経路の呼び出しが消え、`copy` の呼び出しは static / 共有の分岐の先だけになった
(命令数は 137 → 139 でほぼ同じ。呼び出しが分岐に置き換わった)。Ir は上のとおり 111 → 90。
`tests/lean_string.rs`、`tests/alloc_string.rs` は通る。

なお、static / 共有バッファでの `remove` / `retain` のコピーのしかた自体にも無駄がある。それは
[#16](./16-remove-retain-shared-copy-whole.md) に分けた。そちらの修正で
`remove` の非 modifiable 経路が `ensure_modifiable` を使わなくなれば、呼び出し元は `retain` だけになる。

## 確認方法

- probe (`s.remove(i)`) の x86-64 / aarch64 asm で、inline / unique の経路に `ensure_modifiable` の呼び出しが無いこと。
- `remove` / `retain` のベンチは `bench/benches/apis.rs` に無い。inline (例: 11 バイト) と heap の `remove` を `String` と並べるベンチを足すと効果を見られる。
