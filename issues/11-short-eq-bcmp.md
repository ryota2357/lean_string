# 16 バイト以下の文字列の `==` が毎回 `bcmp` を呼ぶ

- 種別: perf
- 対象: `src/lib.rs` の `PartialEq` の impl すべて (`LeanString` / `LeanStr` と `LeanString` / `LeanStr` / `str` / `&str` / `String` / `Cow<'_, str>` の組み合わせ 20 個)
- 影響: 短い文字列 (inline に入る長さ) の比較が多いコード。1 回あたり約 1 ns 遅い。空の `String` / `&str` と比べると 16〜26 ns かかる
- 優先度: 低

## 問題

どの `PartialEq` も `self.as_str().eq(other)` で、長さが実行時にしか分からないときは長さを比べたあと必ず `bcmp` を呼ぶ。
長さが 1〜16 バイトでも同じである。

`5dd94ea`、rustc 1.97.0、`--release`、LTO なしの下流クレートから見た asm (x86-64)。`probe_eq_ls_str(a: &LeanString, b: &str) -> bool { a == b }`:

```asm
	movzbl	15(%rdi), %eax          # len の復元 (cmov 2 つ)
	...
	cmpq	%rdx, %r9               # 長さの比較
	jne	.LBB6_1
	cmpb	$-48, %al               # データポインタの選択
	jb	.LBB6_4
	movq	(%rdi), %rdi
.LBB6_4:
	pushq	%rax
	callq	*bcmp@GOTPCREL(%rip)    # 長さが 0〜16 でも呼ぶ
```

`LeanString == LeanString` (33 命令)、`== String`、`== Cow` も同じ形で `bcmp` を呼ぶ。aarch64 も `bl bcmp` になる。
右辺がリテラル (`a == "hello"`) なら長さが定数になるので LLVM が `bcmp` を展開し、呼び出しは消える。

所要時間 (Intel Xeon 2.8 GHz、AVX-512 あり、glibc 2.39、`bench` の release プロファイル (LTO あり))。
1 つのバイナリの中で各案を交互に 400k 回ずつ 25 ラウンド実行し、ラウンドごとの値の最小値を取った (ns)。
「不一致」は最後の 1 バイトだけ違う 2 値。

| 長さ | `LeanString == LeanString` 現状 | 修正例 | `LeanString == &str` 現状 | 修正例 |
| --- | --- | --- | --- | --- |
| 1 | 2.80 | 3.01 | 2.70 | 2.40 |
| 8 | 3.01 | 2.06 | 2.70 | 1.63 |
| 8 (不一致) | 3.22 | 2.07 | 3.02 | 1.78 |
| 15 | 2.94 | 2.11 | 2.70 | 1.76 |
| 16 | 3.03 | 2.12 | 2.70 | 1.77 |
| 17 | 3.02 | 2.92 | 2.56 | 2.70 |
| 32 | 3.15 | 2.92 | 2.60 | 2.70 |
| 256 | 5.55 | 4.99 | 5.62 | 5.41 |

8〜16 バイトで約 1 ns (3 割) 速くなる。17 バイト以上は差が 0.6 ns 以下 (17 / 32 バイトでは ±0.3 ns 以内) で、ラウンド間のばらつき (下記) と区別しにくい。
同じコードでもビルドし直すと 2.07 ns が 2.40 ns に動いたことがある (コード配置の影響と思われる)。それでも現状の 3.0〜3.4 ns より速い。
短い `LeanStr` の識別子 64 個を線形探索する処理 (×64) では、3.10 µs が 2.38 µs になった実行もあれば、2.46 µs と 2.44 µs で差が無かった実行もあった。
長さが違えば `bcmp` まで進まないので、長さがばらばらのデータでは効果が薄い。

### 空の `String` / `&str` との比較

右辺が実行時に空の `String` / `&str` / `Cow` だと、`LeanString == &str` は 16〜26 ns かかった (修正例では 2.07 ns)。
空の `String` と空の `&str` のデータポインタは dangling (`0x1`) で、`bcmp(p, 0x1, 0)` がそこで遅くなるためである。
`bcmp` を直接呼んで確かめた。

| 呼び出し | 時間 |
| --- | --- |
| `bcmp(有効, 有効, 0)` | 2.2 ns |
| `bcmp(有効, 0x1, 0)` | 17.8 ns |
| `bcmp(0x1, 0x1, 0)` | 129 ns |

`LeanString` 自身のポインタは dangling にならない (inline のときは自分のアドレス)。
そのため影響が出るのは、空の `String` / `&str` / `Cow` と比べる場合だけである。
`String == String` の両辺が空のときは std 自体が 100 ns を超える。

## 原因

- `<[u8] as PartialEq>::eq` は、長さが定数でなければ長さを比べて `bcmp` を呼ぶ。`bcmp` は GOT を通した間接呼び出しで、
  glibc の ifunc で選ばれた実装に飛ぶ (この環境では `__memcmp_evex_movbe`)。asm で確認した。
- `__memcmp_evex_movbe` は 32 バイト以下をマスク付きの 1 回のロード (`bzhi` でマスクを作り
  `vmovdqu8 (%rsi),%ymm18{%k2}{z}`) で比べる。gdb で逆アセンブルして確認した。比較そのものは速いので、
  約 1 ns の差は呼び出しの往復と、ラッパー側のポインタ選択と呼び出しの準備の分だと考えられる (推測)。
- 長さ 0 では全ビット 0 のマスクで `0x1` を読む。フォールトは抑止されるが、マップされていないページへの
  マスク付きアクセスはマイクロコードアシストになり、遅くなると考えられる (perf カウンタでは確認していない)。
- 呼び出しのコストは libc と CPU で変わる。aarch64 や musl では測っていない。

## 修正例

長さが `MAX_INLINE_SIZE` 以下なら、先頭と末尾から重なり合うロードを XOR して比べる。
`InlineBuffer::new` がコピーで使っているのと同じ方法である。
文字列の中身だけを見るので、バッファの種類 (inline / heap / static) や相手の型を問わず、20 個の impl すべてに使える。

```rust
// src/repr.rs
/// `a == b` that doesn't call `bcmp` when the length is at most `MAX_INLINE_SIZE`.
#[inline(always)]
pub(crate) fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let n = a.len();
    if n != b.len() {
        return false;
    }
    if n > MAX_INLINE_SIZE {
        return a == b;
    }
    use core::ptr::read_unaligned as load;
    let (a, b) = (a.as_ptr(), b.as_ptr());
    // SAFETY: For `k <= n <= 2 * k`, the two `k`-byte loads at `0` and `n - k` stay within `0..n`
    // and cover it.
    unsafe {
        if n >= 8 {
            let x = load(a as *const u64) ^ load(b as *const u64);
            let y = load(a.add(n - 8) as *const u64) ^ load(b.add(n - 8) as *const u64);
            (x | y) == 0
        } else if n >= 4 {
            let x = load(a as *const u32) ^ load(b as *const u32);
            let y = load(a.add(n - 4) as *const u32) ^ load(b.add(n - 4) as *const u32);
            (x | y) == 0
        } else if n >= 2 {
            let x = load(a as *const u16) ^ load(b as *const u16);
            let y = load(a.add(n - 2) as *const u16) ^ load(b.add(n - 2) as *const u16);
            (x | y) == 0
        } else if n == 1 {
            *a == *b
        } else {
            true
        }
    }
}

// src/lib.rs (各 impl)
impl PartialEq<str> for LeanString {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        repr::str_eq(self.as_str(), other)
    }
}
```

`n == 0` ではロードしないので、dangling ポインタの問題も消える。32-bit では `MAX_INLINE_SIZE == 8` なので、
`n >= 8` の分岐は `n == 8` のときだけ通る。

確認したこと:

- 上の表の「修正例」の列がこの実装の数値である。
- asm: 長さ 16 以下では `bcmp` を呼ばない。分岐は長さの帯ごとに 4 段になる。
- テスト: 同じ内容の値をいろいろな表現で作り、`str` の比較と一致することを proptest で確認した。
  縮めた inline、短い heap、短い static、clone を `truncate` した値を含む。
- Miri: x86_64、i686、powerpc64 で `-Zmiri-strict-provenance` を付けて通した。

コスト: `#[inline]` で呼び出し側に展開されるので、`==` 1 か所あたりのコードが増える。
x86-64 の `LeanString == LeanString` は 33 命令から 65 命令に、`LeanString == &str` は 22 命令から 55 命令になる。
aarch64 ではそれぞれ 34 から 75、23 から 65。短い部分を `#[inline(never)]` の関数に出す案もある。
GOT を通らない直接呼び出しになるが、呼び出しは残る。こちらは測っていない。

試して効かなかったもの:

- inline 同士のときだけ 16 バイトを `u128` で読み、長さを超えるバイトをマスクして比べる案
  (使っていないバイトが 0 である必要がない)。長さに応じたマスクを作る `u128` のシフトが重く、
  長さ 15 以下で 4.05 ns と現状より遅かった。
- 同じ方法で `Ord` から `memcmp` をなくす案。inline 同士で、短いほうの長さでマスクした `u128` をバイトスワップして比べた。
  `cmp` 単体は 6.4 ns (現状 3.1 ns)、短い `LeanStr` 1 万個の `sort_unstable` は 791〜809 µs (現状 679〜690 µs) で、どちらも遅くなった。
  glibc の `memcmp` は短い入力に十分速いので、`Ord` は今のままでよい。
- データポインタを `hint::select_unpredictable` で分岐なしに選ぶ案。差は出なかった。

## 確認方法

- asm: 下流クレートで `fn f(a: &LeanString, b: &str) -> bool { a == b }` の asm を出し、長さ 16 以下の経路に
  `bcmp` の呼び出しが無いことを見る。
- ベンチ: `bench/benches/apis.rs` の `eq` / `eq/cloned` は内容が同じ 2 値しか測っていない。
  不一致のケースと `&str` との比較を足す。

```rust
fn eq_mismatch(c: &mut Criterion) {
    let mut group = c.benchmark_group("eq/mismatch");
    for s in [1, 8, 15, 16, 17, 100].map(ascii) {
        let len = s.len();
        // 最後の 1 バイトだけ違う
        let mut t = s.clone().into_bytes();
        *t.last_mut().unwrap() ^= 1;
        let t = String::from_utf8(t).unwrap();
        duplicate! {
            [
                label         StrTy;
                ["current"]   [lean_string::LeanString];
                ["prev"]      [lean_string_prev::LeanString];
                ["std"]       [String];
            ]
            {
                let lhs = StrTy::from(s.as_str());
                let rhs = StrTy::from(t.as_str());
                group.bench_with_input(BenchmarkId::new(label, len), &len, |b, _| {
                    b.iter(|| black_box(&lhs) == black_box(&rhs))
                });
            }
        }
    }
    group.finish();
}
```

  criterion の値はプロセスごとに 1 ns 近く動くことがある。案どうしの比較は、同じバイナリの中で交互に回して行う。
- テスト: 比較の結果が `str` と一致するテスト ([#27](./27-test-eq-ord-hash.md)) を先に入れておく。

## 関連

- [#10](./10-leanstr-word-eq.md): `LeanStr` 同士は、表現のワード比較でさらに速くできる。
  両方を入れるなら、`LeanStr == LeanStr` はそちらを使い、それ以外の impl はこの `str_eq` を使う。
- [#19](./19-shared-buffer-ptr-eq.md): `str_eq` の長い経路 (`n > MAX_INLINE_SIZE`) にポインタの一致による近道を足せる。
