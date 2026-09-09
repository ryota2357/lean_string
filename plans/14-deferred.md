# 14: プラン化しなかった項目の記録

調べたが、単独のプランを起こすほどではない、または前提となる作業を待つべきと判断した項目。
再訪する条件と、そのときに見る場所をまとめておく。

## 小さな codegen 改善

### `len == 0` を長さ分岐の先頭へ

`InlineBuffer::new` の LE 64-bit 版 (src/repr/inline_buffer.rs:37-68) の分岐は
`== 16` → `>= 8` → `>= 4` → `>= 2` → `== 1` → else の順で、空文字列は 5 回の比較を通る。
`len == 0` を先頭に上げると最後の `else` がちょうど `len == 1` になるので、
足した分岐と消える分岐が相殺して命令数は変わらない。

プロトタイプで測った (`bench/benches/apis.rs` の `from`、criterion 中央値):

| len | 現行 | プロトタイプ |
| --- | --- | --- |
| 0 | 2.6214 ns | **1.2504 ns** |
| 1 | 2.1345 ns | 1.8587 ns |
| 15 | 1.6517 ns | 1.8677 ns |
| 16 | 1.4600 ns | 1.2923 ns |
| 17 | 16.459 ns | 16.986 ns |
| 256 | 18.255 ns | 18.231 ns |

命令数は 82 → 83 と 1 命令増えた。len 0 が半分になるのは大きいが、
**len 15 が 13% 悪化して見える**。ただしこの 2 つは別プロセスの測定で、
コード配置とアラインメントの差が同程度の幅で乗りうる
([research/equality.md](../research/equality.md) の §3 に同種の観測がある)。

判断には同一バイナリ内での A/B が要る。`LeanString::new()` は const の
`InlineBuffer::empty()` を通るのでこの分岐に来ず、効くのは
`LeanString::from("")` や実行時に長さ 0 になる `Repr::from_str` だけである点も
考慮に入れる。パーサや `Default` を多用するコードでは頻度が高い。

### 確保失敗の分岐が `#[cold]` になっていない

このクレートは cold 化に気を配っている (`Capacity::new` の `cold_path()`、
`layout_for` の `#[cold]` クロージャ、`reserve` の `outline!`) が、
次の 4 か所は素のままになっている。

- `alloc` が null を返す判定 (heap_buffer.rs:260-263)
- `realloc` が null を返す判定 (heap_buffer.rs:439-442)
- `layout_for` の `checked_add` オーバーフロー (heap_buffer.rs:284-293)
- `reserve` の `len.checked_add(additional).ok_or(ReserveError)?` (repr.rs:452) —
  これは `push_str` の hot path 上にある

まとめて片付ければ数行。ただし単体の効果は小さいので、
[plans/03](./03-push-str-fast-path.md) の asm を見るときに一緒に判断する。

### `#[cold]` と `#[inline(never)]`

`#[cold]` だけでは inline 化を禁止できない。`HeapBuffer::release` の
`on_last_reference` (heap_buffer.rs:216) は `#[cold]` のみだが、
実測では `.text.unlikely` に配置されていることを確認したので現状は問題ない。
[plans/05](./05-drop-cold-abi.md) で関数の形を変えるときに、対で付けておくとよい。

### `reserve` が `len()` を読み直す

`Repr::push_str` は `self.len()` を読んでから `reserve` を呼び、
`reserve` (repr.rs:451) がまた `self.len()` を読む。64-bit ではマスク付きのロードと
`cmov` の対がもう 1 組載る。`insert_str` (repr.rs:736) にも同じ重複がある。

`reserve` は長さを変えないので、`push_str` 側の `let len = self.len();` を
`reserve` の後に移すだけでも意味が変わらない。そうすると判別子のロードが
呼び出しを跨いで生存しなくなり、callee-saved レジスタが 1 本空く。
compact_str は同種の変更で `push_str` が x86 で 54 → 45 命令、arm で 51 → 38 命令に
なったと記録している。1 行なので [plans/03](./03-push-str-fast-path.md) の中で試す。

### `ensure_modifiable` の static 分岐が `replace_inner` を使う

`ensure_modifiable` (repr.rs:846-850) は static と分かっているのに
`self.replace_inner(next)` を呼び、その中で `is_heap_buffer()` をもう一度判定している。
`*self = next` で等価。1 命令の話。

## serde の `deserialize_string` ヒント

`src/features/serde.rs:48` と `:90` は `deserializer.deserialize_string(..)` を呼ぶが、
visitor は `visit_string` / `visit_byte_buf` を実装していない。
ヒントを尊重する形式では `String` を確保してから既定の `visit_string` が
`visit_str` に転送し、`LeanString` へコピーして `String` を捨てることになる。
`LeanString` は常にコピーするので `deserialize_str` のほうが正確なヒント。

効果は `Deserializer` の実装次第なので、`serde_json` でエスケープあり/なしの
両方を測ってから変えるべき。

## 新 API (方針判断待ち)

### `join` / `concat`

合計長を先に計算して 1 回だけ確保し、直接書き込む。`Vec<LeanString>` を
`join` すると現状は 2 回確保する (`[LeanString]::join()` が `String` を作り、
そこから `LeanString` を作る) ところが 1 回になる。

`char_str` が `CharStr::concat` / `join` として実装しており、そのまま設計図になる。
`AsRef<str>` の実装が呼ぶたびに違う長さを返す場合に備えて、部分的に初期化された
バッファをガード型で守っている点は写す価値がある。

[plans/10](./10-api-gaps.md) の G-1 (`FromIterator<&str>` が長さを見積もらない) と
同じ機構なので、どちらかを実装するときに両方まとめて考える。

### `format_lean!` / `format_lean_str!` マクロ

`format!` → `LeanString::from` の中間 `String` 確保を省き、直接書き込むマクロ。
compact_str / char_str / ecow の 3 つともこれを持っており、lean_string だけ無い。

`src/traits.rs` の `try_from_fmt` が `args.as_str()` の fast path込みで既にあるので、
バックエンドとしてはそれを使える。[plans/04](./04-append-writer.md) の writer が入れば
さらに素直になる。04 の後に判断するのが得。
あわせて `MAX_INLINE_SIZE` に相当する定数を pub にするかも決める。

### `INLINE_CAPACITY` / `new_inline` / `new_heap`

どの表現で作るかを呼び出し側が選べる構築子。文字列 interning のように
「inline に収まるなら intern しない」という判定を長さ比較ではなく構築子で
表現したい利用者向け。`char_str` が
`pub const INLINE_CAPACITY: usize` / `pub const fn new_inline(&str) -> Option<Self>` /
`pub fn new_heap(&str)` として持っている。具体的な要望が出たら。

### 他クレートとの統合 feature

`get-size2` や `salsa` の実装を feature として持つ案 (`char_str` が持っている)。
`char_str` は `Repr::heap_allocation_size()` を足してそこから組み立てている。
需要が出たら。1 点だけ注意を記録しておく。

- CoW の共有バッファはデータポインタで重複排除しないと、clone のぶんだけ
  サイズを二重計上してしまう。
- `get-size2` は `AtomicI64` / `AtomicU64` を無条件に import するため、
  64-bit atomic の無いターゲットでコンパイルが通らない。このクレートの CI は
  Miri を powerpc (32-bit) でも回しているので、feature を足すなら
  そのターゲットでの扱いを cfg で分ける必要がある。

## テストとベンチの整備

### ベンチの読み方

`bench/Cargo.toml` の `lean_string_prev` は `=0.7.0` を指す。
リポジトリは v0.7.0 から 8 コミット進んでいるので、`current` と `prev` は
別のコードになっている。ただし変わっていない関数 (`push_str` など) については
両者は実質同じコードであり、そこで観測される差はコード配置とアラインメントの偶然。
実際、`push_str` は current 292.84 ns / prev 282.67 ns だが、
この関数は v0.7.0 から 1 行も変わっていない。

同じ理由で、**別プロセスの測定同士を 1 ns 未満の粒度で比べてはいけない**。
効果を判断するときは、同一バイナリ内で cfg を切り替えた A/B を用意する。

### asm 確認の定型化

各プランが「asm を before/after で比較する」ことを前提にしているが、
その手順はリポジトリに入っていない。
[research/codegen-baseline.md](../research/codegen-baseline.md) に書いた
「probe 用クレート + `--emit asm`」を、リポジトリ内のツールとして持つことを検討してもよい。

- `#[unsafe(no_mangle)]` の 1 行ラッパを並べただけのクレート (workspace には入れない)
- シンボルごとに命令数を数えるスクリプト
- x86-64 と aarch64 の両方を出す (リンクはしないのでクロスの実行環境は不要)

ベンチと違って測定ノイズが無く、CI に載せれば「この関数に memcpy 呼び出しが
復活したら落ちる」といった退行検出にも使える。実装量は小さい。
ただしリポジトリに増やすものが増えるので、必要性が固まってからでよい。

## 確認して問題なかったもの

同じところを二度調べないための記録。

- `Clone` の分岐配置。`probe_clone` は 12 命令で、heap の arm は判別子の比較と
  `lock incq` の 2 命令。どちらの arm も 16 バイトのビット単位コピーへ合流し、
  オーバーフロー処理は `.text.unlikely` にある。
  compact_str には「中身が空の `#[cold]` 関数を呼んで分岐配置を誘導する」手法があり、
  inline clone が 1.08 ns → 0.64 ns になったという記録もあるが、
  lean_string の heap arm は「稀な arm」ではないのでそのままは当てはまらない。
- `Vec<LeanString>` の drop glue。ループ本体は判別子比較 + `lock decq` + cold 呼び出しで、
  空 repr の書き戻しは無い。
- atomic の順序。`Relaxed` の `fetch_add` (repr.rs:249)、`Release` の `fetch_sub` と
  最終参照時の `Acquire` fence (heap_buffer.rs:213-225)、`Acquire` の load (heap_buffer.rs:190)
  は `Arc` と同一。`Weak` に相当するものが無いので、
  参照を保持したままカウント 1 を観測できれば排他は保証される。loom がカバーしている。
- 共有 heap バッファのデータは書き換えられない。共有状態での `truncate` / `pop` は
  ローカルの `TextLen` ワードしか触らず (repr.rs:781-792)、32-bit で長さが
  ヒープ上にある場合は正しくコピー経路へ落ちる。
  これが `Send`/`Sync` の根拠になっている。実測でも共有が維持されることを確認した。
- オーバーフロー経路。`reserve` の `checked_add` (repr.rs:452)、
  `insert_str` の `checked_add` (repr.rs:736)、`repeat` の `checked_mul` (lib.rs:1014)、
  `layout_for` の `checked_add` (heap_buffer.rs:284)、`amortized_growth` の
  saturating (heap_buffer.rs:19-23) がすべて塞いでいる。
- 32-bit の容量上限 `2^31 - 16` は正しい。`Capacity::MAX = isize::MAX`、
  `header_offset = 8`、長さ prefix 4 バイト、`Layout::from_size_align` の
  `size <= isize::MAX - (align - 1)` から `2^31 - 1 - 3 - 12 = 2^31 - 16`。
- `from_static_str` の長さ上限 `2^56 - 1` / `2^24 - 1` は
  `StaticBuffer::MAX_LENGTH` (static_buffer.rs:17-22) と一致している。
- `Borrow<str>` / `Hash` / `Eq` の整合。`HashMap` のキーにして `&str` で
  引ける条件を満たしている。
- `retain` の panic 安全性。`SetLenOnDrop` (repr.rs:671-719) が担っており、
  テストもある。
- `fmt::Write::write_fmt` の static 化 fast path の健全性。
  `fmt::Arguments::as_str()` は `Option<&'static str>` を返すので、
  `from_static_str` に渡して `'static` として扱ってよい。
- `Repr::len()` の branchless 復元。`tail_word()` と `assert_unchecked` の組み合わせで
  `cmov` の対に畳まれており、compact_str が `asm!` によるロードの固定
  (`ensure_read`) で得ているのと同じ形になっている。追加の手当ては要らない。
