# 比較演算をどこまで速くできるか

調査日: 2026-09-09 / 対象: `448a538`

`LeanString` / `LeanStr` の `PartialEq` / `Ord` はどちらも
`self.as_str().eq(other.as_str())` である。ここを速くする手が 3 つあり、
効く場面と必要な前提が違う。選択肢と推奨は [plans/07](../plans/07-equality-policy.md) に書いた。

## 1. 現状の codegen

`probe_eq(&LeanString, &LeanString)` の asm は
[codegen-baseline.md](./codegen-baseline.md) の §6 のとおり、33 命令で
「両辺の len を branchless に復元 → len 比較 → データポインタを cmov で選択 → `bcmp`」。
無駄な分岐も再読み込みもない。`as_str()` 同士を比較するだけの実装として、
これ以上削るところはない。

## 2. `str` と `Arc<T>` は何をしているか

同じ環境で比較対象の codegen も測った (rustc 1.94.1)。

### `str` 同士

```asm
probe_str_eq:
	cmpq	%rcx, %rsi          # len == len ?
	jne	.LBB23_1
	callq	*bcmp@GOTPCREL(%rip)  # 同じポインタでも全長を舐める
```

`&str` は fat pointer なので「ポインタと長さが同じなら比較は済んでいるはず」と
考えたくなるが、core の str/slice 比較は**長さしか見ない**。データポインタが
一致していても `bcmp` が全長を走る (glibc の `memcmp`/`bcmp` にも自己比較の
早期 return はない)。一般の文字列ではポインタ一致が稀で、チェックが無駄になるためと考えられる。

### `Arc<T>` 同士

`Arc` には条件付きでポインタ比較の近道がある。ただし適用範囲が直感と違う。

```asm
probe_arc_string_eq:                  # Arc<String>
	movq	(%rdi), %rax
	movq	(%rsi), %rcx
	cmpq	%rcx, %rax          # ★ポインタ一致なら
	je	.LBB6_1
	...
.LBB6_1:
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

`bench/benches/apis.rs` の `eq` (別々に構築した 2 値) と `eq/cloned` (一方を clone した値)。
criterion 中央値、`--warm-up-time 0.5 --measurement-time 1.5`。

| len | eq: LeanString | eq/cloned: LeanString | eq: `String` | eq/cloned: `String` |
| --- | --- | --- | --- | --- |
| 0 | 2.4520 ns | 2.3893 ns | 86.124 ns | 86.940 ns |
| 1 | 2.2543 ns | 2.0018 ns | 1.6352 ns | 1.6568 ns |
| 15 | 2.1949 ns | 2.3842 ns | 1.5034 ns | 1.2175 ns |
| 16 | 2.1893 ns | 2.3729 ns | 1.7785 ns | 1.6825 ns |
| 17 | 2.7725 ns | 2.4328 ns | 1.3889 ns | 1.7226 ns |
| 256 | 4.7258 ns | 5.8939 ns | 4.2563 ns | 3.5055 ns |

読み取れること。

- `eq/cloned` でバッファを共有するのは len 17 以上だけ。16 以下は inline なので
  clone はビット単位のコピーであり、`eq` と `eq/cloned` は同じものを測っている。
  実際 16 以下では両者に系統的な差が無い。
- 共有している len 256 でも `eq` (4.73 ns) より `eq/cloned` (5.89 ns) が速くならない。
  当然で、現状ポインタ一致の近道は入っていないため、どちらも 256 バイトを舐める。
- LeanString が `String` より 0.5〜0.7 ns 遅いのは、len を復元する分の命令が
  余計に載っているため。これは 16 バイト表現の構造的なコストで、
  比較の実装を変えて消えるものではない。
- `String` の len 0 が 86 ns なのは `String` 側の性質。空の `String` はデータポインタが
  dangling なので、`memcmp` がそのアドレスに対して毎回ペナルティを踏んでいると思われる。
  `LeanString` は空文字列も inline なので影響を受けない (2.45 ns)。

**この規模の差はプロセス間のばらつきと同程度**である。別の実行では同じ構成で
`eq/current/1` が 2.25 ns と 2.94 ns の両方を観測した。コード配置とアラインメントで
1 ns 弱は動くので、以降の判断はこの表の絶対値ではなく、
同一バイナリ内での A/B (近道の有無を cfg で切り替えるなど) に基づくべきである。

**このベンチには「一致しない 2 値」の点が無い**。`samples()` から作った同一内容の
2 値しか測っていないので、常に全長を舐める最悪ケースだけを見ている。
近道の導入を判断するには「先頭バイトで不一致になる最速ケース」が必要で、
そこが近道のコストを最も強く受ける。ベンチの追加が前提条件になる。

## 4. 3 つの案

### 案1: 共有バッファのポインタ近道

`char_str` (v0.0.4, main にマージ済み) が `Repr::content_eq` / `content_cmp` として持っている。

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

導入時の計測 (Apple arm64) として、共有バッファ同士の等価比較が 17 バイトで約 -45%、
64 バイトで約 -60%、4 KiB で約 -99%、共有でない場合はおおむね中立だが
先頭バイトで不一致になる最速ケースが最大 0.1〜0.2 ns 悪化、と記録されている。

設計上、注意すべき点が 2 つある。

- **長さの比較を先に置くのは正しさのため**。共有された heap / static バッファは
  ハンドルごとに論理長が異なりうる。64-bit の `truncate_unchecked` は共有バッファに対して
  ハンドル側の `TextLen` ワードだけを縮め (repr.rs:783-790)、`StaticBuffer::set_len` も同様。
  `content_cmp` の「ポインタ一致だが長さが違う ⇒ 接頭辞関係なので長さが順序を決める」も
  同じ理由による。ここで `Equal` を返すと誤り。
- **inline には効かない**。inline バッファでは `as_bytes()` のポインタが `self` 自身の
  アドレスになるので、別インスタンス同士でポインタが一致することはない。
  つまりこの近道は heap と static にしか効かず、inline 同士では 1 比較ぶんの
  純粋な追加コストになる。

`ecow` も同じ論点で止まっている。issue #35 は completed としてクローズされているが、
実装は `ptr-eq` ブランチに残ったままで main には入っていない
([related-crates.md](./related-crates.md) の該当節を参照)。
`ecow` の実装は「両辺とも spilled (ヒープ) のときだけ」という条件でこの問題に対処していた。

### 案2: inline 表現の正規形 + ワード比較

inline バッファの「使っていないバイトは必ずゼロ」を不変条件にすれば、
2 つの inline `Repr` は `[usize; 2]` として丸ごと比較できる。長さはタグバイトに
含まれているので、長さの復元も比較も要らない。

```rust
if self.last_byte() < HeapMarker && other.last_byte() < HeapMarker {
    let this  = unsafe { ptr::read(self  as *const Self as *const [usize; 2]) };
    let other = unsafe { ptr::read(other as *const Self as *const [usize; 2]) };
    this == other
} else {
    self.content_eq(other)
}
```

**現行の LE 64-bit の `InlineBuffer::new` (src/repr/inline_buffer.rs:21-70) は、
すでに正規形を作っている**。各 arm を追うと:

| arm | 第 1 ワード | 第 2 ワード |
| --- | --- | --- |
| `len == 16` | データ 8 バイト | データ 8 バイト (余りなし) |
| `len >= 8` | データ 8 バイト | `(tail >> ((16 - len) * 8)) \| tag` — 上位がゼロ埋め |
| `len >= 4` | `head \| (tail << ((len - 4) * 8))` — 上位はゼロ | `tag` のみ |
| `len >= 2` | 同上 (u16 版) | `tag` のみ |
| `len == 1` | `*src as u64` — 上位はゼロ | `tag` のみ |
| `len == 0` | `0` | `tag` のみ |

非 LE のフォールバックも `[0u8; 16]` から始まるので正規形。`InlineBuffer::empty()` と
`Repr::new_with` の inline arm も同じ。

正規形を崩すのは `InlineBuffer::set_len` (inline_buffer.rs:133-139) だけで、
これは**バイト 15 しか書き換えないので、縮めたときに古いバイトが残る**。
縮める経路は `truncate_unchecked` / `pop` / `remove` / `retain` / `clear` で、
いずれも `impl Repr<Mutable>` にある。

ここが効く。**`Repr<Immutable>` には変更 API が一切無い**ので、非正規形の inline バッファが
`LeanStr` に届く経路は `Repr::<Mutable>::into_immutable` の非 heap 早期 return
(repr.rs:904-910、`mem::transmute` でそのまま渡す) だけ。そこで 1 回正規化すれば、
`LeanStr` 同士の比較にはこの案をそのまま適用できる。

`char_str` の `charlie/canonical-inline-equality` ブランチが同じ設計で、
`InlineBuffer::make_canonical(&mut self, len)` をマスク付きのワード書き込み
(memset 呼び出しを避ける) で実装し、`make_exact` (lean_string の `into_immutable` 相当) から
呼んでいる。計測値は残っていない。

`len == 16` のときバイト 15 が文字列の実データになる点は問題にならない。
有効な UTF-8 の末尾バイトは必ず `0xC0` 未満で `HeapMarker` (0xD0) を下回るため、
判別子の判定は正しく inline を返す。

### 案3: 短い文字列の `bcmp` 呼び出しを避ける

論理長が `MAX_INLINE_SIZE` 以下のとき、両端からの重ね合わせロードで比較する。
構築側の `InlineBuffer::new` と同じ手を比較側に使う形。

```rust
// 概念コード。両辺は同じ長さであることが確定している。
fn short_content_eq(this: &[u8], other: &[u8]) -> bool { ... }
```

案2 と違って正規形を要求しないので、heap / static の短い文字列にも、
`&str` / `String` / `Cow` との比較にも効く。案2 が `LeanStr` 同士の inline に
限られるのと補完関係にある。

`char_str` には未マージのブランチが 2 本あり、条件の書き方が違う。

- `charlie/inline-string-equality`: 帯ごとの重ね合わせロードを短絡評価 (`&&`) で繋ぐ。
  `content_eq_str(&self, other: &str)` も足して、`PartialEq<str/&str/String/Cow>` の
  すべてを通している。lean_string の `PartialEq` は 20 impl あるので、
  同型同士だけ直すと取りこぼしが大きいという指摘は同じく当てはまる。
- `charlie/inline-equality`: 先頭と末尾のワードを XOR して `(head | tail) == 0` を
  見る形。分岐が 1 つになる。

どちらも計測値は残っていない。2 本に分かれているということは、
どちらが速いか決着していないと読むのが自然。

## 5. 判断材料の整理

| | 効く相手 | inline 同士への影響 | 追加の不変条件 | 実装量 |
| --- | --- | --- | --- | --- |
| 案1 ポインタ近道 | 共有された heap / static | 1 比較ぶん遅くなる | なし | 小 |
| 案2 正規形 + ワード比較 | inline 同士 (まず `LeanStr`) | 最速 | `into_immutable` で正規化 | 中 |
| 案3 短い文字列の直接比較 | 16 バイト以下すべて | 速くなる | なし | 中 |

- 案1 だけを見ると「既定の `==` の実行時間を予測可能に保つか、共有時の最良ケースを
  取りにいくか」という方針の問題になり、判断が難しい。
- 案3 は追加の不変条件が無く、効く範囲がいちばん広い。`bcmp` の呼び出しが消えることは
  asm で確定できるので、判断が早い。
- 案2 は `LeanStr` に限れば `into_immutable` の 1 か所を触るだけで済み、
  文字列 interning や AST のシンボル比較のような、`LeanStr` がいちばん使われる場面に効く。

したがって案3 → 案2 → 案1 の順に測るのがよい。詳細は
[plans/07](../plans/07-equality-policy.md) に書いた。
