# 11: プランにしなかった項目

調べたものの、単独のプランにするほどではないもの、または他の作業を待つべきものをまとめておく。
再検討するきっかけと、そのとき見る場所も書いておく。

## 小さな codegen の改善

### `len == 0` を長さの分岐の先頭に置く

`InlineBuffer::new` の LE 64-bit 版は `== 16` → `>= 8` → `>= 4` → `>= 2` → `== 1` → else の順に
分岐しているので、空文字列は比較を 5 回通る。`len == 0` を先頭に移すと最後の `else` がちょうど
`len == 1` になるので、増える分岐と減る分岐が相殺して命令数はほぼ変わらない。

プロトタイプで測った (`bench/benches/apis.rs` の `from`、criterion の中央値)。

| len | 今 | プロトタイプ |
| --- | --- | --- |
| 0 | 2.6214 ns | **1.2504 ns** |
| 1 | 2.1345 ns | 1.8587 ns |
| 15 | 1.6517 ns | 1.8677 ns |
| 16 | 1.4600 ns | 1.2923 ns |
| 17 | 16.459 ns | 16.986 ns |
| 256 | 18.255 ns | 18.231 ns |

命令数は 82 から 83 に 1 つ増えた。len 0 が半分になるのは大きいが、len 15 は 13% 遅くなったように見える。
ただこの 2 列は別のプロセスで測ったもので、コード配置やアラインメントの違いで同じくらいの差は出る
([research/equality.md](../research/equality.md) の §3 に同じような観測がある)。

判断するには同じバイナリの中で A/B を取る必要がある。また `LeanString::new()` は const の
`InlineBuffer::empty()` を使うのでこの分岐を通らず、効くのは `LeanString::from("")` や、
実行時に長さが 0 になる `Repr::from_str` だけ、という点も考慮する。パーサや `Default` を多用する
コードではそれなりに通る。

### 確保失敗の分岐が cold になっていない

このクレートは cold 化に気を配っている (`Capacity::new` の `cold_path()`、`layout_for` の `#[cold]`
クロージャ、`reserve` の `outline!`) が、次の 4 か所はそのままになっている。

- `alloc` が null を返したかの判定 (`HeapBuffer` の確保)
- `realloc` が null を返したかの判定
- `layout_for` の `checked_add` のオーバーフロー
- `reserve` の `len.checked_add(additional).ok_or(ReserveError)?`。これは `push_str` の hot path 上にある

まとめて直しても数行。ただ単独での効果は小さいので、[02](./02-push-str-fast-path.md) で asm を見るときに一緒に判断する。

### `reserve` が `len()` を読み直している

`Repr::push_str` は `self.len()` を読んでから `reserve` を呼び、`reserve` の中でまた `self.len()` を読む。
64-bit ではマスク付きのロードと `cmov` の組がもう 1 つ増える。`insert_str` にも同じ重複がある。

`reserve` は長さを変えないので、`push_str` の `let len = self.len();` を `reserve` の後に移しても
意味は変わらない。そうすると判別子のロードが呼び出しをまたいで生き残らなくなり、
callee-saved レジスタが 1 本空く。compact_str は同じ種類の変更で `push_str` が x86 で 54 → 45 命令、
arm で 51 → 38 命令になったと記録している。1 行なので [02](./02-push-str-fast-path.md) の中で試す。

### `ensure_modifiable` の static の分岐が `replace_inner` を使っている

`ensure_modifiable` は static だと分かっているのに `self.replace_inner(next)` を呼び、その中で
`is_heap_buffer()` をもう一度判定している。`*self = next` で同じ意味になる。1 命令程度の話。

## 新しい API (方針が決まってから)

### `join` / `concat`

合計の長さを先に計算して 1 回だけ確保し、直接書き込む。今は `Vec<LeanString>` を `join` すると
確保が 2 回起きる (`[LeanString]::join()` が `String` を作り、そこから `LeanString` を作る) が、これが 1 回になる。

`char_str` の `CharStr::concat` / `join` がそのまま参考になる。`AsRef<str>` の実装が呼ぶたびに
違う長さを返す場合に備えて、途中まで初期化したバッファをガード型で守っているところは真似する価値がある。

[07](./07-api-gaps.md) の G-1 (`FromIterator<&str>` が長さを見積もらない) と同じ仕組みなので、
どちらかを実装するときに両方まとめて考える。

### `format_lean!` / `format_lean_str!` マクロ

`format!` の結果を `LeanString::from` するときにできる中間の `String` を省き、直接書き込むマクロ。
compact_str / char_str / ecow の 3 つとも持っていて、lean_string だけ無い。

`src/traits.rs` の `try_from_fmt` がすでにあるので、実装にはそれを使える。
[03](./03-append-writer.md) の writer が入ればもっと素直に書けるので、03 の後に決めるほうがよい。
あわせて `MAX_INLINE_SIZE` にあたる定数を pub にするかも決める。

### `INLINE_CAPACITY` / `new_inline` / `new_heap`

どの表現で作るかを呼び出し側が選べるコンストラクタ。文字列の interning で
「inline に収まるなら intern しない」という判定を、長さの比較ではなくコンストラクタで書きたい利用者向け。
`char_str` は `pub const INLINE_CAPACITY: usize` / `pub const fn new_inline(&str) -> Option<Self>` /
`pub fn new_heap(&str)` を持っている。具体的な要望が出たら考える。

### 他のクレートとの連携 feature

`get-size2` や `salsa` の実装を feature として持つ案 (`char_str` が持っている)。
`char_str` は `Repr::heap_allocation_size()` を足して、そこから組み立てている。要望が出たら考える。
注意点を 2 つ書いておく。

- CoW で共有しているバッファはデータポインタで重複を除かないと、clone の数だけサイズを重複して数えてしまう。
- `get-size2` は `AtomicI64` / `AtomicU64` を無条件に import するので、64-bit の atomic が無い
  ターゲットではコンパイルできない。このクレートの CI は Miri を powerpc (32-bit) でも回しているので、
  feature を足すならそのターゲットでの扱いを cfg で分ける必要がある。

## テストとベンチ

### ベンチの読み方

`bench/Cargo.toml` の `lean_string_prev` は `=0.7.0` を指している。
リポジトリは v0.7.0 から 23 コミット進んでいるので、`current` と `prev` は別のコードになっている。
ただ変わっていない関数 (`push_str` など) では両者は実質同じコードで、そこで出る差はコード配置と
アラインメントによる偶然。実際 `push_str` は current 292.84 ns / prev 282.67 ns だが、
この関数は v0.7.0 から 1 行も変わっていない。

同じ理由で、別のプロセスで測った値同士を 1 ns 未満の単位で比べてはいけない。
効果を判断するときは、同じバイナリの中で cfg を切り替えて A/B を取る。

### asm の確認を手順にする

どのプランも asm の before/after を比べる前提になっているが、その手順はリポジトリに入っていない。
[research/codegen-baseline.md](../research/codegen-baseline.md) で使った「probe 用クレート + `--emit asm`」を
リポジトリ内のツールにしてもよい。

- `#[unsafe(no_mangle)]` の 1 行のラッパを並べただけのクレート (workspace には入れない)
- シンボルごとに命令数を数えるスクリプト
- x86-64 と aarch64 の両方を出す (リンクしないので、クロスの実行環境は要らない)

ベンチと違って測定のノイズが無く、CI に入れれば「この関数に memcpy の呼び出しが戻ったら落ちる」
といった退行検出にも使える。実装は小さい。ただリポジトリに置くものが増えるので、必要になってからでよい。

## 確認して問題なかったもの

同じところを 2 回調べないための記録。

- `Clone` の分岐の配置。`probe_clone` は 12 命令で、heap の分岐は判別子の比較と `lock incq` の 2 命令。
  どちらの分岐も 16 バイトのコピーに合流し、オーバーフローの処理は `.text.unlikely` にある。
  compact_str には「中身が空の `#[cold]` 関数を呼んで分岐の配置を誘導する」手法があり、
  inline の clone が 1.08 ns → 0.64 ns になったという記録もあるが、lean_string の heap の分岐は
  「めったに通らない分岐」ではないので、そのままは当てはまらない。
- `Vec<LeanString>` の drop。ループは判別子の比較 + `lock decq` + cold 関数の呼び出しで、
  空の repr を書き戻すような無駄は無い。
- `HeapBuffer::release` の `on_last_reference` は `#[cold]` だけで `#[inline(never)]` が無いが、
  `.text.unlikely` に置かれていることを確認した。[04](./04-drop-cold-abi.md) で形を変えるときに両方付ける。
- atomic の順序。`Relaxed` の `fetch_add`、`Release` の `fetch_sub` と最後の参照での `Acquire` fence、
  `is_unique` の `Acquire` のロードは `Arc` と同じ。`Weak` にあたるものが無いので、
  参照を持ったままカウント 1 を観測できれば排他が保証される。loom で確認されている。
- 共有された heap バッファのデータは書き換えられない。共有状態での `truncate` / `pop` はハンドル側の
  `TextLen` しか触らず、32-bit で長さが heap 上にある場合は正しくコピーする経路に入る。
  これが `Send`/`Sync` の根拠になっている。実際に共有が保たれることも確認した。
- オーバーフロー。`reserve` と `insert_str` の `checked_add`、`Repr::repeat` の `checked_mul`、
  `layout_for` の `checked_add`、`amortized_growth` の saturating 演算で、すべて防がれている。
- 32-bit の容量の上限 `2^31 - 16` は正しい。`Capacity::MAX = isize::MAX`、`header_offset = 8`、
  長さの prefix 4 バイト、`Layout::from_size_align` の `size <= isize::MAX - (align - 1)` から
  `2^31 - 1 - 3 - 12 = 2^31 - 16`。
- `from_static_str` の長さの上限 `2^56 - 1` / `2^24 - 1` は `StaticBuffer::MAX_LENGTH` と一致している。
- `retain` の panic 安全性。`SetLenOnDrop` が担っていて、テストもある。
  最初に削る文字が見つかるまではバッファに触らないので、その間に述語が panic しても文字列は変わらない。
- `fmt::Write::write_fmt` で static バッファにする fast path の健全性。`fmt::Arguments::as_str()` は
  `Option<&'static str>` を返すので、`from_static_str` に渡して `'static` として扱ってよい。
- `Repr::len()` の分岐の無い復元。`tail_word()` と `assert_unchecked` で `cmov` の組になっていて、
  compact_str が `asm!` でロードを固定して (`ensure_read`) 得ているのと同じ形になっている。追加の対処は要らない。
- serde の `Deserialize` は `deserialize_str` を使っており、visitor が実装しているもの (`visit_str`) と合っている。
