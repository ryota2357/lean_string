# 比較をどこまで速くできるか

対象: `8bf3fee`

`LeanString` / `LeanStr` の `PartialEq` / `Ord` はどちらも `self.as_str().eq(other.as_str())` になっている。
これを速くする方法は 3 つあり、それぞれ効く場面と必要な前提が違う。
選択肢と推奨は [plans/05](../plans/05-equality-policy.md) にまとめた。

## 1. 今の codegen

`probe_eq(&LeanString, &LeanString)` は [codegen-baseline.md](./codegen-baseline.md) の §6 のとおり 33 命令で、
両辺の長さを分岐なしで復元し、長さを比べ、データポインタを選んで `bcmp` を呼ぶ。
余計な分岐も読み直しも無く、`as_str()` 同士を比べる実装としてはこれ以上削れない。

## 2. `str` と `Arc<T>` の比較

比較のために std の型の codegen も見た。

### `str` 同士

```asm
probe_str_eq:
	cmpq	%rcx, %rsi          # 長さが同じか
	jne	.LBB23_1
	callq	*bcmp@GOTPCREL(%rip)  # ポインタが同じでも全体を比べる
```

`&str` は fat pointer なので、ポインタと長さが同じなら比べるまでもない、と考えたくなるが、
core の str / slice の比較は長さしか見ない。データポインタが同じでも `bcmp` が全体を比べる
(glibc の `memcmp` / `bcmp` にも、同じアドレスなら即座に返すような処理は無い)。
普通の文字列ではポインタが一致することはまれで、確認するだけ無駄になるからだと思われる。

### `Arc<T>` 同士

`Arc` には条件付きでポインタを先に比べる処理がある。ただし、どの型で効くかは直感と違う。

```asm
probe_arc_string_eq:                  # Arc<String>
	movq	(%rdi), %rax
	movq	(%rsi), %rcx
	cmpq	%rcx, %rax          # ポインタが同じなら
	je	.LBB6_1
	...
.LBB6_1:
	movb	$1, %al             # すぐに true を返す
	retq
```

```asm
probe_arc_str_eq:                     # Arc<str>
	movq	8(%rdi), %rdx
	cmpq	8(%rsi), %rdx       # 長さを比べるだけ
	jne	.LBB5_1
	callq	*bcmp@GOTPCREL(%rip)  # ポインタは比べない
```

`Arc<T>` の `PartialEq` は `T: Eq` のとき `Arc::ptr_eq(self, other) || **self == **other` に特殊化されるが、
その特殊化に使うマーカー (`MarkerEq`) の実装が `impl<T: Eq> MarkerEq for T`、つまり `Sized` が前提になっている。
そのため `Arc<str>` のような unsized の `T` では効かず、`Arc<String>` では効く。

つまり「`Arc<str>` はポインタを先に比べる」というのは、今の rustc では成り立たない。
`LeanString` が std の型より劣っているという話ではなく、std の中でも近道がある型と無い型が混在している。

## 3. ベンチマーク

`bench/benches/apis.rs` の `eq` (別々に作った 2 つの値) と `eq/cloned` (片方を clone した値)。
criterion の中央値、`--warm-up-time 0.5 --measurement-time 1.5`。

| len | eq: LeanString | eq/cloned: LeanString | eq: `String` | eq/cloned: `String` |
| --- | --- | --- | --- | --- |
| 0 | 2.4520 ns | 2.3893 ns | 86.124 ns | 86.940 ns |
| 1 | 2.2543 ns | 2.0018 ns | 1.6352 ns | 1.6568 ns |
| 15 | 2.1949 ns | 2.3842 ns | 1.5034 ns | 1.2175 ns |
| 16 | 2.1893 ns | 2.3729 ns | 1.7785 ns | 1.6825 ns |
| 17 | 2.7725 ns | 2.4328 ns | 1.3889 ns | 1.7226 ns |
| 256 | 4.7258 ns | 5.8939 ns | 4.2563 ns | 3.5055 ns |

ここから分かること:

- `eq/cloned` でバッファが共有されるのは len 17 以上だけ。16 以下は inline なので clone はただのコピーで、
  `eq` と `eq/cloned` は同じものを測っている。実際、16 以下では両者に一貫した差が無い。
- 共有している len 256 でも、`eq/cloned` (5.89 ns) は `eq` (4.73 ns) より速くならない。
  ポインタで判定する近道が無いので、どちらも 256 バイトを比べている。
- LeanString が `String` より 0.5〜0.7 ns 遅いのは、長さを復元する命令の分。
  16 バイトの表現からくるコストなので、比較の実装を変えても消えない。
- `String` の len 0 が 86 ns なのは `String` 側の事情。空の `String` のデータポインタは dangling なので、
  `memcmp` がそのアドレスで毎回ペナルティを受けているのだと思われる。
  `LeanString` は空文字列も inline なので影響を受けない (2.45 ns)。

ただ、この程度の差はプロセス間のばらつきと同じくらいある。同じ構成で `eq/current/1` が
2.25 ns のときも 2.94 ns のときもあった。コード配置やアラインメントで 1 ns 近く動くので、
判断はこの表の絶対値ではなく、同じバイナリの中での A/B (近道の有無を cfg で切り替えるなど) で行う。

また、このベンチには「内容が違う 2 つの値」が無い。`samples()` から作った同じ内容の 2 値だけを
測っているので、常に全体を比べる最悪のケースしか見ていない。近道を入れるかどうかを決めるには、
近道のコストがいちばん目立つ「先頭のバイトで不一致になる」ケースが要る。まずベンチを足す必要がある。

## 4. 3 つの案

### 案1: 共有バッファならポインタで判定する

`char_str` (v0.0.4、main にマージ済み) が `Repr::content_eq` / `content_cmp` として実装している。

```rust
pub(crate) fn content_eq(&self, other: &Self) -> bool {
    let this = self.as_bytes();
    let other = other.as_bytes();
    this.len() == other.len() && (ptr::eq(this.as_ptr(), other.as_ptr()) || this == other)
}

pub(crate) fn content_cmp(&self, other: &Self) -> cmp::Ordering {
    let this = self.as_bytes();
    let other = other.as_bytes();
    if ptr::eq(this.as_ptr(), other.as_ptr()) {
        this.len().cmp(&other.len())
    } else {
        this.cmp(other)
    }
}
```

導入時に測った結果 (Apple arm64) として、共有バッファ同士の比較が 17 バイトで約 45%、64 バイトで約 60%、
4 KiB で約 99% 速くなり、共有していない場合はほぼ変わらないが、先頭のバイトで不一致になるケースは
最大 0.1〜0.2 ns 遅くなった、と記録されている。

設計上の注意点が 2 つある。

- 長さを先に比べるのは正しさのため。共有された heap / static バッファは、ハンドルごとに長さが違うことがある。
  64-bit の `truncate_unchecked` は共有バッファに対してハンドル側の `TextLen` だけを縮め、
  `StaticBuffer::set_len` も同じ。`content_cmp` で「ポインタが同じで長さが違うなら、一方がもう一方の接頭辞なので
  長さで順序が決まる」としているのも同じ理由で、ここで `Equal` を返すと誤りになる。
- inline には効かない。inline バッファでは `as_bytes()` のポインタが `self` 自身のアドレスなので、
  別のインスタンスとは一致しない。効くのは heap と static だけで、inline 同士では比較が 1 つ増えるだけになる。

CoW を活かす操作が増えるほど、この近道が効く場面も増える。`repeat(1)` や、変更が無いときの
`to_lowercase` / `to_ascii_lowercase` などは元のバッファを共有した値を返すので、
変換の前後を比べるコードでは両辺が同じバッファを指す。

`ecow` でも同じ議論があり、結論は出ていない。issue #35 は completed でクローズされているが、
実装は `ptr-eq` ブランチに残ったままで main には入っていない ([related-crates.md](./related-crates.md) の該当する節)。
`ecow` の実装は「両方が spilled (heap) のときだけ」という条件で inline の問題を避けていた。

### 案2: inline 表現を正規形にしてワード単位で比較する

inline バッファの「使っていないバイトは必ず 0」を不変条件にすれば、2 つの inline `Repr` は
`[usize; 2]` としてそのまま比べられる。長さはタグのバイトに入っているので、長さの復元も比較も要らない。

```rust
if self.last_byte() < HeapMarker && other.last_byte() < HeapMarker {
    let this  = unsafe { ptr::read(self  as *const Self as *const [usize; 2]) };
    let other = unsafe { ptr::read(other as *const Self as *const [usize; 2]) };
    this == other
} else {
    self.content_eq(other)
}
```

inline バッファを作る経路は、すでにすべて正規形を作っている。

LE 64-bit の `InlineBuffer::new` の各分岐:

| 分岐 | 第 1 ワード | 第 2 ワード |
| --- | --- | --- |
| `len == 16` | データ 8 バイト | データ 8 バイト (余りなし) |
| `len >= 8` | データ 8 バイト | `(tail >> ((16 - len) * 8)) \| tag`。上位は 0 |
| `len >= 4` | `head \| (tail << ((len - 4) * 8))`。上位は 0 | `tag` のみ |
| `len >= 2` | 同上 (u16 版) | `tag` のみ |
| `len == 1` | `*src as u64`。上位は 0 | `tag` のみ |
| `len == 0` | `0` | `tag` のみ |

LE 64-bit 以外の実装は `[0u8; 16]` から書き始めるので正規形になる。
`InlineBuffer::from_char` は「エンコード結果と 0」「0 とタグ」の 2 ワードを組み立てるので正規形。
`InlineBuffer::empty()` から書き始める `Repr::new_with` の inline 分岐も、書かなかったバイトは 0 のまま残る。
`Repr::with_capacity_init` の inline 分岐は、`init` が返す長さより先に書くことを契約では禁じていないが、
今の唯一の呼び出し元 (`map_case_if_changed`) は返す長さまでしか書かないので、結果は正規形になっている。
案2 を採るなら、この契約に「`len` より先には書かない」を加える。

正規形が崩れるのは `InlineBuffer::set_len` だけで、バイト 15 しか書き換えないので、縮めたときに古いバイトが残る。
縮める処理 (`truncate_unchecked` / `pop` / `remove` / `retain` / `clear`) はどれも `impl Repr<Mutable>` にある。

ここで、`Repr<Immutable>` には変更する API が無いことが効いてくる。正規形でない inline バッファが
`LeanStr` に渡る経路は、`Repr::<Mutable>::into_immutable` の heap でない場合の早期 return
(`mem::transmute` でそのまま渡している) だけ。そこで 1 回正規化すれば、`LeanStr` 同士の比較にこの案をそのまま使える。

`char_str` の `charlie/canonical-inline-equality` ブランチが同じ設計で、
`InlineBuffer::make_canonical(&mut self, len)` をマスク付きのワード書き込み (memset を呼ばないように) で実装し、
`make_exact` (lean_string の `into_immutable` にあたる) から呼んでいる。計測結果は残っていない。

`len == 16` のときバイト 15 が文字列のデータになる点は問題ない。有効な UTF-8 の最後のバイトは必ず
`0xC0` 未満で `HeapMarker` (0xD0) より小さいので、判別子の判定は正しく inline になる。

### 案3: 短い文字列では `bcmp` を呼ばない

長さが `MAX_INLINE_SIZE` 以下なら、先頭と末尾からの重なったロードで比べる。
構築側の `InlineBuffer::new` と同じ方法を比較に使うもの。

```rust
// 概念的なコード。両辺の長さが同じであることは確認済み。
fn short_content_eq(this: &[u8], other: &[u8]) -> bool { ... }
```

案2 と違って正規形を必要としないので、heap / static の短い文字列にも、`&str` / `String` / `Cow` との比較にも効く。
案2 は `LeanStr` 同士の inline に限られるので、互いに補い合う関係にある。

`char_str` にはマージされていないブランチが 2 本あり、条件の書き方が違う。

- `charlie/inline-string-equality`: 長さの帯ごとの重なったロードを `&&` でつなぐ。
  `content_eq_str(&self, other: &str)` も足して、`PartialEq<str/&str/String/Cow>` すべてに使っている。
  lean_string も `PartialEq` の impl が 20 個あるので、同じ型同士だけ直すと効果が限られる点は同じ。
- `charlie/inline-equality`: 先頭と末尾のワードを XOR して `(head | tail) == 0` を見る。分岐が 1 つで済む。

どちらも計測結果は残っていない。2 本に分かれているのは、どちらが速いか決まっていないからだと思われる。

## 5. 判断材料

| | 効く相手 | inline 同士への影響 | 追加の不変条件 | 実装量 |
| --- | --- | --- | --- | --- |
| 案1 ポインタでの判定 | 共有された heap / static | 比較 1 回分遅くなる | なし | 小 |
| 案2 正規形 + ワード比較 | inline 同士 (まず `LeanStr`) | いちばん速い | `into_immutable` で正規化 | 中 |
| 案3 短い文字列の直接比較 | 16 バイト以下すべて | 速くなる | なし | 中 |

- 案1 だけで考えると、「`==` の実行時間を予測しやすく保つか、共有しているときの最良のケースを取りにいくか」
  という方針の問題になり、決めにくい。
- 案3 は追加の不変条件が無く、効く範囲がいちばん広い。`bcmp` の呼び出しが消えることは asm で確認できるので、判断しやすい。
- 案2 は `LeanStr` に限れば `into_immutable` の 1 か所を変えるだけで済み、文字列の interning や AST のシンボル比較のような、
  `LeanStr` がよく使われる場面に効く。

したがって、案3 → 案2 → 案1 の順に測るのがよい。詳しくは [plans/05](../plans/05-equality-policy.md) に書いた。
