# 比較演算にポインタ一致の近道を入れるか

調査日: 2026-08-16 / 対象: `8f7fa75` (v0.7.0 + bench deps 更新)

CoW である `LeanString` / `LeanStr` は clone がバッファを共有するので、
「同じバッファを指す 2 つのハンドルの比較」は内容を見るまでもなく結果が決まる。
これを `PartialEq` / `Ord` に組み込むべきかを検討した記録。結論は
[plans/07](../plans/07-equality-policy.md) にまとめてある。

## 1. 現状の codegen

`probe_eq(&LeanString, &LeanString)` の asm は
[codegen-baseline.md](./codegen-baseline.md) の §6 のとおり、
「両辺の len を branchless に復元 → len 比較 → データポインタを cmov で選択 → `bcmp`」。
無駄な分岐も再読み込みもない。`as_str()` 同士を比較するだけの実装として、
これ以上削るところはない。

## 2. `str` と `Arc<T>` は何をしているか

同じ環境で比較対象の codegen も測った。

### `str` 同士

```asm
probe_str_eq:
	cmpq	%rcx, %rsi          # len == len ?
	jne	.LBB19_1
	callq	*bcmp@GOTPCREL(%rip)  # 同じポインタでも全長を舐める
```

`&str` は fat pointer なので「ポインタと長さが同じなら比較は済んでいるはず」と考えたく
なるが、core の str/slice 比較は**長さしか見ない**。データポインタが一致していても
`bcmp` が全長を走る (glibc の `memcmp`/`bcmp` にも自己比較の早期 return はない)。
一般の文字列ではポインタ一致が稀で、チェックが無駄になるためと考えられる。

### `Arc<T>` 同士

`Arc` には条件付きでポインタ比較の近道がある。ただし**適用範囲が直感と違う**。

```asm
probe_arc_string_eq:                  # Arc<String>
	movq	(%rdi), %rax
	movq	(%rsi), %rcx
	cmpq	%rcx, %rax          # ★ポインタ一致なら
	je	.LBB7_1
	...
.LBB7_1:
	movb	$1, %al             # ★即 true
	retq
```

```asm
probe_arc_str_eq:                     # Arc<str>
	movq	8(%rdi), %rdx
	cmpq	8(%rsi), %rdx       # len 比較のみ
	jne	.LBB5_1
	callq	*bcmp@GOTPCREL(%rip)  # ポインタ比較なし
```

`Arc<T>` の `PartialEq` は `T: Eq` のとき `Arc::ptr_eq(self, other) || **self == **other`
に特殊化されるが、その特殊化マーカ (`MarkerEq`) の実装が `impl<T: Eq> MarkerEq for T`
すなわち `Sized` 前提なので、**`Arc<str>` のような unsized な `T` には効かない**。
`Arc<String>` には効く。

つまり「`Arc<str>` はポインタを先に比べる」という前提は現在の rustc では成立しない。
`LeanString` が std の何かと比べて劣っているという話ではなく、std 側にも
「近道が入る型と入らない型が混在している」というのが実情。

## 3. ベンチマーク

`bench/benches/apis.rs` の `eq` (別々に構築した 2 値) と `eq/cloned` (一方を clone した値)
を実行した結果 (criterion 中央値、`--warm-up-time 0.5 --measurement-time 1.5`)。

| len | eq: LeanString | eq/cloned: LeanString | eq: `String` |
| --- | --- | --- | --- |
| 0 | 2.3711 ns | 2.4451 ns | 108.56 ns |
| 1 | 2.4279 ns | 2.3697 ns | 1.6244 ns |
| 15 | 2.4106 ns | 2.3676 ns | 1.6449 ns |
| 16 | 2.3735 ns | 2.3758 ns | 1.6226 ns |
| 17 | 2.4852 ns | 2.6895 ns | 1.6087 ns |
| 256 | 5.3800 ns | 4.6788 ns | 4.4291 ns |

読み取れること。

- **`eq/cloned` でバッファを共有するのは len 17 以上だけ**。16 以下は inline なので
  clone はビット単位のコピーであり、`eq` と `eq/cloned` は同じものを測っている。
- 共有している len 256 で `eq` 5.38 ns → `eq/cloned` 4.68 ns の差が出ているが、
  これはポインタ一致による近道ではなく (現状そんな近道はない)、
  同じ 256 バイトのバッファを 2 回読むためのキャッシュ局所性による。
- `String` の len 0 が 108 ns なのは `String` 側の性質。空の `String` はデータポインタが
  dangling なので、`memcmp` がそのアドレスに対して毎回ペナルティを踏んでいると思われる。
  `LeanString` は空文字列も inline なので影響を受けない (2.37 ns)。
- LeanString が `String` より 0.7 ns 程度遅いのは、len を復元する分の命令が
  余計に載っているため。これは 16 バイト表現の構造的なコストで、
  比較の実装を変えて消えるものではない。

## 4. 他のクレートでの扱い

### `ecow` (typst/ecow) — [issue #35](https://github.com/typst/ecow/issues/35)

2023-10-10 に epage から「`Arc<str>` は先にポインタを比較するが、通常の参照の等価比較は
それをしない」という趣旨で提案された (ベンチマークへのリンク付き)。
issue は enhancement ラベル付きでクローズされている。

対応するコミットは**存在するが main には入っていない**。
`ptr-eq` ブランチの `fa87e6f` ("Add pointer equality comparison for `EcoString`",
2023-11-27, メッセージに `Fixes #35`) が

- `DynamicVec` の `PartialEq` に「**両辺とも spilled (ヒープ) のとき**だけ
  `EcoVec::ptr_eq` を先に見る」近道を入れ、
- `EcoVec::ptr_eq` を **pub API として公開**する

という内容。inline 変種を条件から外して、inline 同士の比較には一切コストを乗せない
作りになっている。このブランチは 2 年以上マージされておらず、
現在の `ecow` の `PartialEq for EcoString` は `self.as_str().eq(other.as_str())` のまま、
`ptr_eq` も公開されていない。

この経緯は「近道自体は書けるが、既定の `==` に組み込むかは別の判断」という
本タスクの論点と一致している。設計として参考になるのは次の 2 点。

1. **近道を適用する条件をバッファ種別で絞る**。inline 同士では絶対に一致しないので、
   その組み合わせを条件から外せば、もっとも速い経路に命令を足さずに済む。
2. **`ptr_eq` を pub にする**。既定の比較を変えなくても、必要な利用側は
   自分で前置できるようになる。

### `compact_str`

比較は `as_str()` 同士のまま。ポインタ一致の近道は入っていない。
そもそも clone がバッファを共有しない (CoW ではない) ので、動機が無い。

### `char_str` (astral-sh/char_str)

`Repr` に `content_eq` / `content_cmp` を持ち、同型同士と相互比較の `PartialEq`/`Ord` から
呼んでいる。

```rust
pub(crate) fn content_eq(&self, other: &Self) -> bool {
    let this = self.as_bytes();
    let other = other.as_bytes();
    // 共有された growable / static バッファはハンドルごとに論理長が異なりうる。
    this.len() == other.len() && (ptr::eq(this.as_ptr(), other.as_ptr()) || this == other)
}

pub(crate) fn content_cmp(&self, other: &Self) -> cmp::Ordering {
    let this = self.as_bytes();
    let other = other.as_bytes();
    // データポインタが一致するなら共通の接頭辞は同一なので、長さが順序を決める。
    if ptr::eq(this.as_ptr(), other.as_ptr()) {
        this.len().cmp(&other.len())
    } else {
        this.cmp(other)
    }
}
```

導入時の計測 (Apple arm64) として、共有バッファ同士の等価比較が 17 バイトで約 -45%、
64 バイトで約 -60%、4 KiB で約 -99%、共有でない場合はおおむね中立だが
先頭バイトで不一致になる最速ケースが最大 0.1〜0.2 ns 悪化、と記録されている。

設計上、注意すべき点が 2 つある。

- `content_cmp` の「ポインタ一致だが長さが違う ⇒ 接頭辞関係」の扱い。
  ここで `Equal` を返すと誤り。`truncate` などで「同じポインタ・異なる長さ」の
  ハンドルが作れるため、長さの比較が順序になる。
- inline バッファでは `as_bytes()` のポインタが `self` 自身のアドレスになるので、
  **別インスタンス同士でポインタが一致することはない**。つまりこの近道は
  heap と static にしか効かず、inline 同士では 1 比較ぶんの純粋な追加コストになる。

`char_str` には未マージのブランチが 2 本あり、どちらもこの
「inline に効かない」問題への別々の答えになっている (どちらも計測値は残っていない)。

- 論理長が inline 上限以下のとき、両端からのワード単位 XOR で比較する
  (`bcmp` の呼び出しを避ける)。論理長の範囲しか読まないので、
  未使用容量に残ったゴミの影響を受けない。
- inline 表現を「未使用バイトは必ずゼロ」という正規形に保ち、
  `[usize; 2]` としてまるごと比較する。比較は最速になるが、
  正規形を崩しうる経路 (`truncate` などの後) すべてで正規化が必要になる。

## 5. 判断材料の整理

- 現状の比較は codegen としては十分に良く、削れる無駄は無い。
- 近道が効くのは heap / static のハンドルが同じバッファを指す場合だけ。
  16 バイト以下は inline なので、短い文字列のワークロードでは一切効かない。
- 近道を既定の `==` に入れると、「もっとも速い不一致ケース」に 1 比較ぶんの
  コストが乗る。`char_str` の計測ではその劣化は 0.1〜0.2 ns 程度。
- 一方で、共有ハンドル同士の比較が支配的なワークロードでは効果が大きい (長い文字列で顕著)。
- 呼び出し側が自分で近道を書くには「ポインタと長さが一致すれば同一の内容である」という
  保証が要るが、これは現在どこにも文書化されていない。

つまり争点は速度そのものより「既定の `==` の実行時間を予測可能に保つか、
共有時の最良ケースを取りにいくか」という方針の問題になる。
選択肢と推奨は [plans/07](../plans/07-equality-policy.md) に書いた。
