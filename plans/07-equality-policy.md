# 07: 比較の速度をどこまで取りにいくか

種別: 方針決定 + perf / 実装量: 小〜中 (案ごと) / 設計判断: 大

調査は [research/equality.md](../research/equality.md) にまとめてある。ここでは選択肢と推奨を書く。

## 論点

現状の `PartialEq` / `Ord` はどちらも `self.as_str().eq(other.as_str())` で、
codegen としては無駄がない ([research/codegen-baseline.md](../research/codegen-baseline.md) の §6)。
それでも `String` より 0.6〜0.7 ns 遅く、これは 16 バイト表現から長さを復元する
命令ぶんの構造的なコストで、比較の実装を変えて消えるものではない。

| len | LeanString | LeanString (clone 同士) | `String` |
| --- | --- | --- | --- |
| 1 | 2.2543 ns | 2.0018 ns | 1.6352 ns |
| 15 | 2.1949 ns | 2.3842 ns | 1.5034 ns |
| 16 | 2.1893 ns | 2.3729 ns | 1.7785 ns |
| 17 | 2.7725 ns | 2.4328 ns | 1.3889 ns |
| 256 | 4.7258 ns | 5.8939 ns | 4.2563 ns |

この差はプロセス間のばらつきと同程度なので、絶対値を根拠にしてはいけない
(詳細は [research/equality.md](../research/equality.md) の §3)。
確かなのは「clone 同士でも速くならない」ことだけで、これは近道が入っていない以上当然。

速くする手は 3 つあり、効く場面が違う。排他ではないので、どれを採るかを個別に決める。

## 案1: 共有バッファのポインタ近道

CoW なので clone はバッファを共有する。「同じバッファの同じ範囲を指す 2 つのハンドル」の
比較は内容を見ずに決まる。

```rust
pub(crate) fn content_eq(&self, other: &Self) -> bool {
    let this = self.as_bytes();
    let other = other.as_bytes();
    // 共有された heap / static バッファはハンドルごとに論理長が異なりうるので、
    // 長さの比較を先に置く必要がある。
    this.len() == other.len() && (ptr::eq(this.as_ptr(), other.as_ptr()) || this == other)
}
```

- 長さの比較を先に置くのは正しさのためであって最適化ではない。64-bit の
  `truncate_unchecked` は共有 heap バッファに対してハンドル側の `TextLen` だけを縮める
  (repr.rs:783-790) し、`StaticBuffer::set_len` も同様。つまり「同じポインタ・異なる長さ」の
  ハンドルが作れる。
- inline 表現では `as_bytes()` のポインタが `self` 自身のアドレスになるので、
  別インスタンス同士で一致することはない。**この近道は heap と static にしか効かず、
  inline 同士では 1 比較ぶんの純粋な追加コスト**になる。
- `Ord` 側は「ポインタ一致だが長さが違う ⇒ 接頭辞関係なので長さが順序を決める」。
  ここで `Equal` を返すと誤り。

近道を入れれば共有された長い文字列の比較が 1 比較まで落ちるが、
逆に「最速の不一致ケース」に 1 比較ぶんの命令が乗る。

## 案2: inline 表現の正規形 + ワード比較

inline バッファの「使っていないバイトは必ずゼロ」を不変条件にすれば、
2 つの inline `Repr` は `[usize; 2]` として丸ごと比較できる。長さはタグバイトに
含まれているので、長さの比較も要らない。ロード 2 本と 128 ビット比較 1 回になる。

**現行の LE 64-bit の `InlineBuffer::new` (src/repr/inline_buffer.rs:21-70) は
すでに正規形を作っている**。各 arm を追うと、

- `len >= 8`: `w1 = (tail >> ((16 - len) * 8)) | last_byte` で `[len, 15)` がゼロ埋めされる
- `len >= 4` / `>= 2` / `== 1` / `== 0`: `w1 = last_byte` で第 2 ワードはタグ以外ゼロ、
  `w0` も上位バイトがマスクされる
- `len == 16`: 余りバイトが無い

非 LE のフォールバックも `[0u8; 16]` から始まるので正規形。`InlineBuffer::empty()` と
`Repr::new_with` の inline arm も同じ。

正規形を崩すのは `InlineBuffer::set_len` (inline_buffer.rs:133-139) だけで、
これはバイト 15 しか書き換えないので、縮めたときに古いバイトが残る。
縮める呼び出し元は `truncate_unchecked` / `pop` / `remove` / `retain` / `clear` で、
いずれも `impl Repr<Mutable>` にある。

ここが効いてくる。**`Repr<Immutable>` には変更 API が一切無い**ので、
非正規形の inline バッファが `LeanStr` に届く経路は
`Repr::<Mutable>::into_immutable` の非 heap 早期 return (repr.rs:904-910) だけ。
そこで 1 回正規化すれば、`LeanStr` 同士の比較にはこの案をそのまま適用できる。

- `LeanStr` に限れば: 追加の不変条件は `into_immutable` の 1 か所だけ。
- `LeanString` にも広げるなら: `set_len` の縮小経路にマスク付きのワード書き込みを足す。
  `truncate` / `pop` / `remove` / `retain` の hot path に乗るが、
  `push_str` (伸ばす側) には乗らない。別途測ってから決める。

注意点として、`Repr` の第 1 フィールドは `*const ()` なので、これを `usize` として読む形は
provenance の扱いに注意が要る。inline バッファの第 1 ワードは素のデータバイトなので
問題ないはずだが、CI が `-Zmiri-strict-provenance` で回っているのでそこで確認する。

## 案3: 短い文字列の `bcmp` 呼び出しを避ける

論理長が `MAX_INLINE_SIZE` 以下のとき、両端からの重ね合わせロードで比較する。
`InlineBuffer::new` の構築側と同じ手を比較側に使う形で、`bcmp` の関数呼び出しが消える。

案2 と違って正規形を要求しないので、**heap / static の短い文字列にも、
`&str` / `String` / `Cow` との比較にも効く**。案2 が `LeanStr` 同士の inline に限られるのと
補完関係にある。`PartialEq` は 20 impl あるので、同型同士だけ直すと取りこぼしが大きい。

## 選択肢の組み合わせ

| | 効く相手 | inline 同士への影響 | 追加の不変条件 |
| --- | --- | --- | --- |
| 案1 ポインタ近道 | 共有された heap / static | 1 比較ぶん遅くなる | なし |
| 案2 正規形 + ワード比較 | inline 同士 (まず `LeanStr`) | 最速 | `into_immutable` で正規化 |
| 案3 短い文字列の直接比較 | 16 バイト以下すべて | 速くなる | なし |

推奨: **案3 → 案2 (`LeanStr` 限定) → 案1** の順に測る。

1. 案3 が最初。追加の不変条件が無く、効く範囲がいちばん広い。
   `bcmp` の呼び出しが消えることは asm で確定できるので、判断が早い。
2. 案2 は `LeanStr` に限れば安い。`into_immutable` の 1 か所を正規化するだけで、
   `LeanStr` の比較が最速になる。文字列 interning や AST のシンボル比較のような、
   `LeanStr` がいちばん使われる場面に効く。`LeanString` への拡張は別に測る。
3. 案1 は最後。効くのは共有された長い文字列だけで、inline 同士には
   純粋なコストになる。案3 を入れた後だと inline 側が速くなっているぶん、
   相対的な劣化が見えやすくなる。

## `ptr_eq` を pub にするか

案1 を既定の `==` に入れないと決めた場合でも、`Arc::ptr_eq` / `Rc::ptr_eq` に相当する
小さな pub API を足せば、利用側が自分で前置できる。

```rust
impl LeanString {
    /// 2 つの文字列が同じバッファの同じ範囲を指しているなら `true`。
    ///
    /// `true` なら内容は必ず等しい。`false` でも内容が等しいことはある。
    /// clone 同士や、同じ `&'static str` から作った文字列同士で `true` になる。
    /// インライン化されている文字列 (`2 * size_of::<usize>()` バイト以下) では、
    /// 同じオブジェクトを 2 回借りた場合を除いて常に `false` になる。
    pub fn ptr_eq(&self, other: &Self) -> bool {
        core::ptr::eq(self.as_ptr(), other.as_ptr()) && self.len() == other.len()
    }
}
```

決めるべきこと:

- 命名。`ptr_eq` (`Arc`/`Rc` 踏襲) が第一候補。
- 「ポインタと長さが一致すれば内容も一致する」を pub な保証として書くかどうか。
  書くなら、将来 `as_ptr()` (`Deref<Target = str>` 経由) の返す値を変えられなくなる。
  実際にはバッファ種別ごとに次のとおり成立している:
  - inline: ポインタは `self` のアドレスなので、別インスタンス同士では一致しない。
  - heap: 共有ハンドルは同じポインタ。unshare や realloc で変わる。
  - static: 同じ `&'static str` から作れば同じポインタ (実測で確認)。
    リンカのマージにより「別のリテラルが同じポインタ」になることはありうるが、
    その場合も内容は等しい。
- `LeanStr` にも生やすか。不変で共有が本旨なので、むしろこちらのほうが使われる。
  両方に生やすのが一貫する。

## 検証方針

- asm: 案3 で `bcmp` の呼び出しが消えること。案2 で `LeanStr` 同士の比較が
  ロード 2 本 + 比較になること。案1 で「最速の不一致ケース」に何命令乗るか。
- criterion: `apis.rs` の `eq` / `eq/cloned` と `comparison.rs` の `Eq` / `Eq/cloned`。
  現状 `eq` の長さリストは 0/1/15/16/17/256。案1 と案2 の判断には
  「先頭バイトで不一致になる最速ケース」と「末尾 1 バイトだけ違うケース」の
  2 つを足す必要がある。不一致位置を変えた点も足す。
- 正しさ (案1): `truncate` で「同じポインタ・異なる長さ」を作ったハンドル同士の
  `==` と `cmp` が正しいこと。static バッファ同士も同様。
- 正しさ (案2): すべての構築経路と、`into_immutable` を通った後の `Repr<Immutable>` が
  `[len, 15)` をゼロにしていることを proptest で確認する。
  `LeanString` にも広げる場合は `truncate` / `pop` / `remove` / `retain` の後も同様。
- Miri: 案2 は `*const ()` フィールドを `usize` として読むので
  `-Zmiri-strict-provenance` で必ず確認する。

## 依存

なし。ただし案2 を採る場合、[plans/01](./01-char-inline-construction.md) の
`from_char` も正規形を作る必要がある (レジスタ組み立てなら自然に満たす)。
