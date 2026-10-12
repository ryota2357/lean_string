# `LeanString::push` が呼び出し側に展開されず、1 文字ごとに `Repr::push_char` を呼び出す

- 種別: perf
- 対象: `src/repr.rs` の `Repr::push_char`、`src/lib.rs` の `LeanString::push` / `LeanString::try_push`
- 影響: 実行時に決まる `char` を `push` するすべての呼び出し (LTO なしの下流クレート)。ASCII 1 文字の追記が `String::push` の約 3.4 倍の命令数になる。`Extend<char>` などのループからも同じ関数が呼ばれる
- 優先度: 中

## 問題

`5dd94ea`、rustc 1.97.0、`--release` (opt-level 3, codegen-units 1, LTO なし) の下流クレートで
`pub fn probe_push(s: &mut LeanString, c: char) { s.push(c) }` を出力すると、本体は呼び出しだけになる
(x86-64 で 8 命令、aarch64 で 9 命令)。

```asm
probe_push:
	pushq	%rax
	callq	_RNv...lean_string4repr...ReprNtNtB4_10mutability7MutableE9push_char
	testb	%al, %al
	jne	.LBB22_2
	popq	%rax
	retq
```

呼ばれる `Repr::push_char` は x86-64 で 124 命令、aarch64 で 131 命令ある。引数が定数の `s.push('a')` は
展開される (55 命令) ので、展開されないのは実行時の `char` のときだけである。比較として
`String::push` は同じ条件で呼び出し側に展開される (`probe_std_push` 72 命令、grow のときだけ
`do_reserve_and_handle` を呼ぶ)。

callgrind の命令数 (1 回の呼び出しで 1000 文字を `push`。`black_box` で文字を実行時の値にし、
`LeanString::new()` / `String::new()` から始める):

| | `LeanString` | `String` |
| --- | ---: | ---: |
| `push('a')` × 1000 | 61,042 | 18,189 |
| `push('a')` × 1000 (`with_capacity(1000)` から) | 58,232 | 16,195 |
| `push('あ')` × 1000 | 84,759 | 41,728 |

容量が足りている場合でも差は変わらないので、差は確保ではなく 1 回ごとの追記の経路から来ている。

なお `bench/` は `lto = true` でビルドするので、`apis.rs` の `push` ベンチは LTO なしの下流とは条件が違う。

## 原因

`Repr::push_char` は `#[inline]` だが、本体が下流クレートでのインライン化の閾値を超えている
(asm で確認、閾値を超えるという判断は LLVM のコストモデルによるもので推測)。本体には次のものが入っている。

- `len()` の分岐のない復元と、バッファの種類による 3 方向の分岐 (inline / heap / static)
- heap のときの容量の比較と参照カウントのロード
- `write_char` による 1〜4 バイトの 4 方向の分岐
- `set_len` の分岐 (判別子をもう一度読む)。LLVM はこれを `write_char` の 4 つの分岐それぞれに複製している

`push_str` (`99b2573`) と同じように grow の呼び出しを `#[cold]` の入れ子関数に包む形も試したが、
`push_char` は 126 命令のままで展開されなかった。大きさの主因は grow ではなく、4 方向のエンコードと
それに複製された `set_len` である。

## 修正例

ASCII を先に分け、ASCII のときだけ 1 バイトを書く小さな経路を呼び出し側に展開する。非 ASCII は今の本体を
outline したものに任せる。`ecow::EcoString::push` も ASCII を特別扱いしている。

```rust
#[inline]
pub(crate) fn push_char(&mut self, ch: char) -> Result<(), ReserveError> {
    if ch.is_ascii() {
        return self.push_ascii(ch as u8);
    }
    self.push_char_non_ascii(ch)
}

#[inline]
fn push_ascii(&mut self, byte: u8) -> Result<(), ReserveError> {
    let len = self.len();
    if self.spare_capacity() < 1 || !self.is_modifiable() {
        #[cold]
        fn grow(this: &mut Repr<Mutable>) -> Result<(), ReserveError> {
            this.grow_amortized(1)
        }
        grow(self)?;
    }
    // SAFETY:
    // - The buffer is modifiable and has 1 byte of spare capacity, which the condition above or
    //   `grow_amortized` ensures.
    // - `byte` is ASCII, so `0..(len + 1)` is valid UTF-8 after the write.
    unsafe {
        self.as_mut_ptr().add(len).write(byte);
        self.set_len(len + 1);
    }
    Ok(())
}

// 今の `push_char` の本体をそのまま移したもの
#[inline(never)]
fn push_char_non_ascii(&mut self, ch: char) -> Result<(), ReserveError> { ... }
```

試した結果 (同じ条件):

- asm: `probe_push` が x86-64 で 65 命令、aarch64 で 69 命令になり、ASCII の経路に関数呼び出しが
  無くなった。非 ASCII のときだけ `push_char_non_ascii` を呼ぶ。
- callgrind: `push('a')` × 1000 が 61,042 → 36,969、`with_capacity` からは 58,232 → 34,237。
  `push('あ')` × 1000 は 84,759 → 85,788 (1 文字あたり +1 命令) でほぼ変わらない。
- `FromIterator<char>` / `from_utf16` も ASCII 入力では同じ経路を通るので減る
  (`chars().collect()` 1000 文字で 62,549 → 35,540、`from_utf16` 1000 単位で 63,252 → 40,256)。
  ただしループでの追記には [#03](./03-char-loop-append.md) で扱う方法のほうが効く。
- `cargo +nightly test` は全件通過した。
- criterion の `push/*` (LTO あり) では current と prev に差が出なかった (1 バイト文字で 292 ns 対 292 ns)。
  LTO ありのビルドでは下流と条件が違う (クレートをまたいだインライン化の判断が変わる) ためだと
  考えているが、確認していない。効果の確認は LTO なしの下流 (上の probe と callgrind) で行った。

`String` との差 (37 対 18) はまだ残る。これは 1 回ごとに `len()` の復元、判別子の分岐、参照カウントの
ロード、判別子の読み直し (`as_mut_ptr` と `set_len`) を通るためで、`LeanString` の表現から来る分である。

`#[inline(always)]` を `push_char` に付ける案もあるが、124 命令がすべての呼び出し箇所に複製されるので勧めない。

## 確認方法

- asm: 上の `probe_push` の ASCII の経路に関数呼び出しが無いこと (x86-64 と aarch64)。
- callgrind: `LeanString::new()` から `black_box('a')` を 1000 回 `push` する関数の命令数。
- ベンチ: `apis.rs` の `push/*`。ただし `bench/Cargo.toml` は `lto = true` なので差が出にくい。
  下流の典型的な設定を見るなら、ベンチ用の profile で `lto = false` にして比べる必要がある。
- テスト: inline → heap をまたぐ `push`、static / 共有バッファへの `push` (既存のテストで通る)。

## 関連

- `fmt::Write::write_char` が未実装で `push_str` を経由している件は [#20](./20-fmt-write-char.md)。この修正と合わせると
  `write_char` の中でも ASCII の経路が展開される。
- `Extend<char>` / `FromIterator<char>` / `from_utf16*` の 1 文字ずつの追記は [#03](./03-char-loop-append.md)。
