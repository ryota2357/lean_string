# 05: 比較をどこまで速くするか

種別: 方針決定 + perf / 実装量: 小〜中 (案による) / 設計判断: 大

調査の詳細は [research/equality.md](../research/equality.md) にある。ここでは選択肢と推奨をまとめる。

## 現状

`PartialEq` / `Ord` はどちらも `self.as_str().eq(other.as_str())` で、生成されるコードに無駄はない
([research/codegen-baseline.md](../research/codegen-baseline.md) の §6)。
それでも `String` より 0.6〜0.7 ns 遅い。これは 16 バイトの表現から長さを復元する命令の分で、
比較の実装を変えても消えない。

| len | LeanString | LeanString (clone 同士) | `String` |
| --- | --- | --- | --- |
| 1 | 2.2543 ns | 2.0018 ns | 1.6352 ns |
| 15 | 2.1949 ns | 2.3842 ns | 1.5034 ns |
| 16 | 2.1893 ns | 2.3729 ns | 1.7785 ns |
| 17 | 2.7725 ns | 2.4328 ns | 1.3889 ns |
| 256 | 4.7258 ns | 5.8939 ns | 4.2563 ns |

この程度の差はプロセス間のばらつきと同じくらいなので、絶対値を根拠にはできない
([research/equality.md](../research/equality.md) の §3)。はっきり言えるのは
「clone 同士でも速くならない」ことだけで、ポインタを見る近道が無いので当然そうなる。

速くする方法は 3 つあり、それぞれ効く場面が違う。排他ではないので、個別に採否を決める。

## 案1: 共有バッファならポインタで判定する

clone はバッファを共有するので、同じバッファの同じ範囲を指す 2 つのハンドルは、中身を見なくても等しいと分かる。

```rust
pub(crate) fn content_eq(&self, other: &Self) -> bool {
    let this = self.as_bytes();
    let other = other.as_bytes();
    // 共有された heap / static バッファはハンドルごとに長さが違うことがあるので、
    // 長さの比較を先にする必要がある。
    this.len() == other.len() && (ptr::eq(this.as_ptr(), other.as_ptr()) || this == other)
}
```

- 長さを先に比べるのは最適化ではなく、正しさのため。64-bit の `truncate_unchecked` は
  共有された heap バッファに対してハンドル側の `TextLen` だけを縮めるし、`StaticBuffer::set_len` も同じ。
  つまり「ポインタは同じで長さが違う」ハンドルが作れる。
- inline のときは `as_bytes()` のポインタが `self` 自身のアドレスなので、別のインスタンスとは
  一致しない。この近道が効くのは heap と static だけで、inline 同士では比較が 1 つ増えるだけになる。
- `Ord` では「ポインタが同じで長さが違う」なら一方がもう一方の接頭辞なので、長さで順序が決まる。
  ここで `Equal` を返すと誤り。

入れれば共有された長い文字列の比較は比較 1 回で終わるが、先頭で不一致になるいちばん速いケースには
比較が 1 つ増える。

`repeat(1)` や、変更が無いときの `to_lowercase` / `to_ascii_lowercase` なども元のバッファを
共有した値を返すので、変換の前後を比べるコードではこの近道が効く。

## 案2: inline 表現を正規形にしてワード単位で比較する

inline バッファの「使っていないバイトは必ず 0」を不変条件にすると、2 つの inline `Repr` は
`[usize; 2]` としてそのまま比較できる。長さはタグのバイトに入っているので、長さの比較も要らない。
ロード 2 本と 128 ビットの比較 1 回で済む。

inline バッファを作る経路は、すでにすべて正規形を作っている。LE 64-bit の `InlineBuffer::new` は
どの分岐でも余ったバイトを 0 にしているし、それ以外のターゲット向けの実装は `[0u8; 16]` から始める。
`InlineBuffer::from_char` と `InlineBuffer::empty()`、`Repr::new_with` の inline 分岐も同様
(`Repr::with_capacity_init` については [research/equality.md](../research/equality.md) の案2 を参照)。

正規形が崩れるのは `InlineBuffer::set_len` だけで、バイト 15 しか書き換えないため、縮めたときに
古いバイトが残る。縮める処理 (`truncate_unchecked` / `pop` / `remove` / `retain` / `clear`) は
どれも `impl Repr<Mutable>` にある。

ここで、`Repr<Immutable>` には変更する API が無いことが効いてくる。正規形でない inline バッファが
`LeanStr` に渡る経路は、`Repr::<Mutable>::into_immutable` の heap でない場合の早期 return だけ。
そこで 1 回正規化すれば、`LeanStr` 同士の比較にこの案をそのまま使える。

- `LeanStr` だけに適用するなら、追加の不変条件は `into_immutable` の 1 か所だけ。
- `LeanString` にも広げるなら、`set_len` の縮める側にマスク付きのワード書き込みを足す。
  `truncate` / `pop` / `remove` / `retain` が少し重くなるが、`push_str` (伸ばす側) には影響しない。
  広げるかどうかは測ってから決める。

`Repr` の第 1 フィールドは `*const ()` なので、それを `usize` として読むときは provenance に注意する。
inline バッファの第 1 ワードはただのデータなので問題ないはずだが、CI の `-Zmiri-strict-provenance` で確認する。

## 案3: 短い文字列では `bcmp` を呼ばない

長さが `MAX_INLINE_SIZE` 以下なら、先頭と末尾からの重ね合わせロードで比較する。
構築側の `InlineBuffer::new` と同じ方法を比較に使うもので、`bcmp` の呼び出しが無くなる。

案2 と違って正規形を必要としないので、heap / static の短い文字列にも、
`&str` / `String` / `Cow` との比較にも効く。案2 は `LeanStr` 同士の inline に限られるので、
お互いに補い合う関係にある。`PartialEq` は 20 個の impl があるので、同じ型同士だけ直すと効果が限られる。

## 組み合わせ

| | 効く相手 | inline 同士への影響 | 追加の不変条件 |
| --- | --- | --- | --- |
| 案1 ポインタでの判定 | 共有された heap / static | 比較 1 回分遅くなる | なし |
| 案2 正規形 + ワード比較 | inline 同士 (まず `LeanStr`) | いちばん速い | `into_immutable` で正規化 |
| 案3 短い文字列の直接比較 | 16 バイト以下すべて | 速くなる | なし |

推奨: **案3 → 案2 (`LeanStr` のみ) → 案1** の順に測る。

1. 案3 を最初に。追加の不変条件が無く、効く範囲がいちばん広い。
   `bcmp` の呼び出しが消えることは asm で確認できるので、判断もしやすい。
2. 案2 は `LeanStr` に限れば安い。`into_immutable` の 1 か所で正規化するだけで、
   `LeanStr` 同士の比較がいちばん速くなる。文字列の interning や AST のシンボル比較のように、
   `LeanStr` がよく使われる場面で効く。`LeanString` に広げるかは別に測る。
3. 案1 は最後に。効くのは共有された長い文字列だけで、inline 同士では遅くなる。
   案3 を入れた後だと inline 側が速くなっている分、遅くなった分が目立ちやすい。

## `ptr_eq` を pub にするか

案1 を `==` に入れないと決めた場合でも、`Arc::ptr_eq` / `Rc::ptr_eq` のような小さな pub API を
用意すれば、使う側で先に判定できる。

```rust
impl LeanString {
    /// 2 つの文字列が同じバッファの同じ範囲を指しているなら `true` を返す。
    ///
    /// `true` なら内容は必ず等しい。`false` でも内容が等しいことはある。
    /// clone 同士や、同じ `&'static str` から作った文字列同士では `true` になる。
    /// inline に格納されている文字列 (`2 * size_of::<usize>()` バイト以下) では、
    /// 同じオブジェクトを 2 回借用した場合を除いて常に `false` になる。
    pub fn ptr_eq(&self, other: &Self) -> bool {
        core::ptr::eq(self.as_ptr(), other.as_ptr()) && self.len() == other.len()
    }
}
```

決めること:

- 名前。`Arc` / `Rc` に合わせて `ptr_eq` が第一候補。
- 「ポインタと長さが同じなら内容も同じ」を公開の保証にするか。保証するなら、
  `as_ptr()` (`Deref<Target = str>` 経由) が返す値を将来変えられなくなる。
  今はバッファの種類ごとに次のとおり成り立っている。
  - inline: ポインタは `self` のアドレスなので、別のインスタンスとは一致しない。
  - heap: 共有しているハンドルは同じポインタ。共有解除や realloc で変わる。
  - static: 同じ `&'static str` から作れば同じポインタ。リンカが同じ内容のリテラルをまとめて
    別のリテラルが同じポインタになることはあるが、その場合も内容は等しい。
- `LeanStr` にも付けるか。不変で共有するための型なので、むしろこちらのほうが使われる。
  両方に付けるのが自然。

## 検証

- asm: 案3 で `bcmp` の呼び出しが消えること。案2 で `LeanStr` 同士の比較がロード 2 本と比較に
  なること。案1 で、先頭で不一致になるケースに何命令増えるか。
- criterion: `apis.rs` の `eq` / `eq/cloned` と `comparison.rs` の `Eq` / `Eq/cloned`。
  今の `eq` は同じ内容の 2 値しか測っていない。案1 と案2 を判断するには
  「先頭バイトで不一致」と「末尾 1 バイトだけ違う」ケースを足す必要がある。不一致の位置を変えた点も足す。
- 正しさ (案1): `truncate` で「ポインタは同じで長さが違う」ハンドルを作り、`==` と `cmp` が正しいこと。
  static バッファ同士でも確認する。
- 正しさ (案2): すべての構築経路と、`into_immutable` を通った後の `Repr<Immutable>` で
  `[len, 15)` が 0 になっていることを proptest で確認する。
  `LeanString` にも広げるなら `truncate` / `pop` / `remove` / `retain` の後も確認する。
- Miri: 案2 は `*const ()` のフィールドを `usize` として読むので、`-Zmiri-strict-provenance` で必ず確認する。

## 依存

なし。
