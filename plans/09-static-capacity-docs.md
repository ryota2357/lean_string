# 09: static バッファの `capacity()` に関するドキュメント修正

種別: doc (+ 設計判断) / 実装量: 小 / 設計判断: 中 (`capacity()` が何を返すべきか)

## 現象

`Repr::capacity` (src/repr.rs:417-427) は static バッファのとき**現在の長さ**を返す。

```rust
pub(crate) fn capacity(&self) -> usize {
    if self.is_heap_buffer() {
        unsafe { self.as_heap_buffer() }.capacity()
    } else if self.is_static_buffer() {
        unsafe { self.as_static_buffer() }.len()   // ← 長さをそのまま返す
    } else {
        MAX_INLINE_SIZE
    }
}
```

static バッファは `clear` / `truncate` / `pop` で長さを縮められるので、
`capacity()` は 16 未満にも 0 にもなる。実測 (v0.7.0):

```
static:             len=48 cap=48 heap=false
after clear:        len=0  cap=0  heap=false
after truncate(5):  len=5  cap=5  str="Long "
```

一方で 4 か所の doc が「最小容量は `2 * size_of::<usize>()` バイト」と書いている。

| 場所 | 記述 |
| --- | --- |
| `with_capacity` (lib.rs:90-92) | "the minimum capacity of a `LeanString` is `2 * size_of::<usize>()` bytes" |
| `capacity` (lib.rs:296-298) | 同上 |
| `shrink_to_fit` (lib.rs:412-413) | "The resulting capacity is always greater than `2 * size_of::<usize>()` bytes" |
| `shrink_to` (lib.rs:466-467) | 同上 |

`shrink_to_fit` は heap 以外では何もしない (repr.rs:530-533) ので、
`truncate(5)` した static バッファに対して呼んでも容量は 5 のまま。
記述と食い違う。

さらに `clear` (lib.rs:925) の

> If the `LeanString` is unique, this method will not change the capacity.

も、unique な static バッファでは容量が 48 から 0 に変わるので成り立たない。

安全性の問題は無い。`reserve` は static バッファを容量に関係なく変換する
(repr.rs:495-511) ので、`capacity()` の報告値に依存した危ない経路は存在しない。
既存テストも `tests/lean_string.rs` が static バッファの容量が長さと一致することを
意図的に確認している。つまりこれは**実装が意図どおりで doc が追いついていない**ケース。

## 決めること: `capacity()` は static バッファで何を返すべきか

### 案A (推奨): 実装はそのまま、doc を直す

`capacity()` の意味を「追加の確保なしに保持できるバイト数」と読むなら、
static バッファでは現在の長さがまさにその値になる (1 バイトでも足せば必ず確保が起きる)。
実装は正しく、記述だけが古い。

- `capacity` / `with_capacity` の doc に「`&'static str` から作られた文字列は例外で、
  容量は現在の長さに等しい」を足す。
- `shrink_to_fit` / `shrink_to` の「常に `2 * size_of::<usize>()` より大きい」を、
  static バッファの例外を含む記述に直す。
- `clear` の「unique なら容量は変わらない」に static バッファの例外を足す。
- doctest を 1 つ足して、この挙動を仕様として固定する。

### 案B: static バッファでも `MAX_INLINE_SIZE` を下限にする

`capacity()` を `self.as_static_buffer().len().max(MAX_INLINE_SIZE)` にする。

- 利点: 4 か所の doc がそのまま正しくなる。「最小容量」という語の直感に合う。
- 欠点: 嘘になる。長さ 5 の static バッファに対して容量 16 と答えると
  「11 バイト足しても確保は起きない」と読めるが、実際には 1 バイトで確保が起きる。
- 採らない。

### 案C: static バッファの `capacity()` を 0 にする

「書き込める余地は無い」ことを強調する。

- 利点: 「1 バイトでも書けば確保が起きる」がもっとも伝わる。
- 欠点: 既存テストの期待を変える破壊的変更になり、`capacity() >= len()` という
  直感も壊れる。採らない。

案A を推す。実装を変えずに済み、既存テストとも整合する。

## 手順

1. 上記 5 か所の doc を修正する。
2. `capacity` の doc に、3 種類のバッファそれぞれで何が返るかを表か箇条書きで書く。
   - inline: 常に `2 * size_of::<usize>()`
   - heap: 確保した容量
   - static: 現在の長さ
3. doctest を足す。static バッファを `truncate` してから `capacity()` と
   `shrink_to_fit()` を見るもの。
4. `clear` の static バッファのテストを足す (`tests/lean_string.rs` の `clear_cow` は
   inline と heap しか見ていない)。

## ついでに直せるドキュメントの誤り

同じ機会に片付けられるもの。

- `README.md` の "Nich optimized" (25 行目と、比較表の 60-62 行目・71-73 行目の計 6 か所) は
  "Niche optimized" の誤り。`src/lib.rs:1` が `#![doc = include_str!("../README.md")]` で
  README を取り込んでいるので、公開ドキュメントにもそのまま出る。
- `src/lib.rs:616` (`remove` の `# Panics`) の "larger than or equal tothe" に空白が抜けている。
- `remove` の panic メッセージは、どちらの `assert!` が先に発火するかで文面が変わる。
  12 バイトの文字列に `remove(13)` すると
  "index is not a char boundary or out of bounds (index: 13)"、
  `remove(12)` すると "index out of bounds (index: 12, len: 12)" になる。
  doc は両方を 1 つの条件にまとめているので誤りではないが、
  テストは `idx == len` の文面しか固定していない。
  範囲外の文面も固定するか、境界判定の順序を入れ替えるかを決める。

## 依存

なし。`capacity()` の実装を変えないので他のタスクと衝突しない。
