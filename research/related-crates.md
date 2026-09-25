# 類似クレートの調査

lean_string 側の基準: `8bf3fee`

対象:

| クレート | 調査したコミット | バージョン | 表現 |
| --- | --- | --- | --- |
| [ParkMyCar/compact_str](https://github.com/ParkMyCar/compact_str) | `9696af7f` | 0.10.0 | 24 バイト。inline / `&'static str` / heap。参照カウントなし |
| [astral-sh/char_str](https://github.com/astral-sh/char_str) | `4b401e93` | 0.0.4 | 16 バイト。inline / static / 参照カウント付きの heap |
| [typst/ecow](https://github.com/typst/ecow) | `99333459` | 0.3.1 | 16 バイト (inline は 15 バイトまで) の CoW |

lean_string との関係:

- `compact_str` と `ecow` は lean_string とは別に作られたクレートで、コードの共有は無い。
  参考にするのは考え方で、差分をそのまま持ってくるわけではない。
- `char_str` は lean_string の fork。ただし `Mutability` の型パラメータが入る前に分かれていて、
  `Repr<Mutable>` / `Repr<Immutable>` の型での区別を、実行時のタグ (`ExactHeapMarker` / `HeapMarker`) で
  行っている。ここは lean_string のほうがよいので取り入れない。
- `compact_str` は表現が 24 バイトなので、表現の大きさに依存しない部分だけを参考にする。

マージされていないブランチも見た。`char_str` には `charlie/*` が 8 本、`compact_str` には
`perf/*` が 11 本と `perf-audit` がある。実験の記録として参考になるので、関係するものは下で個別に挙げる。

## 取り入れられそうなもの

| 内容 | 出典 | プラン |
| --- | --- | --- |
| 失敗しうる変換の戻り値が sret になる | char_str (`make_exact` / `make_growable`) | [09](../plans/09-mutability-conversion.md) |
| 共有バッファならポインタで比較を済ませる | char_str (main にマージ済み) | [05](../plans/05-equality-policy.md) 案1 |
| inline 表現の正規形 + ワード単位の比較 | char_str `charlie/canonical-inline-equality` | [05](../plans/05-equality-policy.md) 案2 |
| 短い文字列の比較で `bcmp` を呼ばない | char_str `charlie/inline-string-equality` | [05](../plans/05-equality-policy.md) 案3 |
| 中サイズのコピーで memcpy を呼ばない | compact_str (`copy_medium`) | [01](../plans/01-medium-copy.md) |
| 追記のコピーを固定長にする | char_str `charlie/inline-concat-append-copies` | [01](../plans/01-medium-copy.md) |
| `push_str` の fast path を小さくする | compact_str `perf-audit` | [02](../plans/02-push-str-fast-path.md) |
| 連続した追記のための内部 writer | compact_str (`Extend<char>` / `repr/iter.rs`) | [03](../plans/03-append-writer.md) |
| cold 関数に参照でなく値を渡す | compact_str (`outlined_drop`) | [04](../plans/04-drop-cold-abi.md) |
| unique なときの drop で atomic RMW を避ける | char_str `charlie/unique-string-drop` | [04](../plans/04-drop-cold-abi.md) |
| 整数の桁をレジスタで組み立てる | compact_str `perf/int-formatting-registers` | [08](../plans/08-integer-codegen.md) |
| `len == 0` を長さの分岐の先頭に置く | compact_str `perf/empty-string-create` | [11](../plans/11-deferred.md) |
| 確保 1 回で済む `join` / `concat` | char_str | [11](../plans/11-deferred.md) |
| `format_*!` マクロ | compact_str / char_str / ecow (3 つとも) | [11](../plans/11-deferred.md) |
| `INLINE_CAPACITY` / `new_inline` / `new_heap` | char_str | [11](../plans/11-deferred.md) |
| `get-size2` / `salsa` 連携の feature | char_str | [11](../plans/11-deferred.md) |
| `from_utf8_lossy` の、全部正しい UTF-8 だったときの fast path | compact_str | [07](../plans/07-api-gaps.md) |
| `as_mut_str` / `drain` / `replace_range` など | compact_str / ecow | [07](../plans/07-api-gaps.md) |

## 当てはまらなかったもの

同じところを 2 回調べないための記録。

### compact_str の `(ptr, cap)` をレジスタで返す一連の関数

`compact_str/src/repr/heap.rs` は `alloc_copy` / `alloc` / `alloc_copy_extra` を、
「2 ワードのタプルを返す `#[cold] #[inline(never)]` の本体」と「`#[inline(always)]` の薄いラッパ」に分けている。
24 バイトの `Repr` は SysV では MEMORY クラスになって sret で返り、LLVM がそれを戻り値の領域に
ばらばらにコピーしていたための対処。

lean_string には当てはまらない。`Repr` は 16 バイトで、`Result<Repr<M>, ReserveError>` も
`LastByte` の niche で 16 バイトに収まり、`rax:rdx` で返る。ただしエラー側にタプルを持つ `Result` は
24 バイトになるので、そちらは当てはまる ([09](../plans/09-mutability-conversion.md))。

### compact_str の `InlineBuffer::new` の `[u64; 3]` のレジスタでの組み立て

lean_string の `InlineBuffer::new` にすでに 2 ワード版がある。長さの帯ごとの重ね合わせロードも、
provenance とエンディアンの扱いも同じ。取り入れるものは無い。

### compact_str の「64-bit 整数はすべて inline に収まる」前提

`MAX_SIZE = 24` なら `i64::MIN` の 20 文字が収まるが、`MAX_INLINE_SIZE = 16` では 32 ビットまでしか
収まらない。lean_string では、この判定を書かなくても `u32` の変換から heap の分岐が消えていることを
asm で確認した (LLVM が桁数の計算から導いている)。取り入れるものは無い
([08](../plans/08-integer-codegen.md))。

### compact_str の `into_string()` / `from_string_buffer()`

`String` との O(1) の相互変換。lean_string の heap のレイアウトはデータの前にヘッダを置くので、
`String::from_raw_parts` に渡せる形にならない。構造上できない。

### char_str の `Header { capacity, count }` のフィールド順

どちらのレイアウトでも `count` が `ptr - 8` に来るようにフィールドを並べ、`reference_count()` の
`if is_exact()` を固定オフセットのロードにしている。lean_string は `Mutability` が型パラメータなので、
そもそもこの分岐が無い。

### char_str の実行時の exact/growable タグと release ビルドの `assert!`

`charlie/harden-string-invariants` で入れているもの。lean_string は同じ不変条件を型で保証しているので要らない。

### char_str の `ToCharString` のフォールバック

lean_string の `try_from_fmt` (src/traits.rs) のほうが進んでいる (確保エラーを退避しておく処理と
`args.as_str()` の fast path がある)。`charlie/fix-fallible-char-string-format` の内容はすでに含まれている。

### ecow の 2 倍の成長

lean_string は 1.5 倍 (`amortized_growth`)。意図した違いで、改善点ではない。

### `repeat` の実装

3 つとも `push` のループで、std のような倍々のコピーはしていない。lean_string の `Repr::repeat` は
最終的な長さで 1 回だけ確保して倍々のコピーで埋めているので、こちらのほうが進んでいる。

## 押さえておきたい知見

各プランの根拠になっているので、要点だけ残しておく。

### store-to-load forwarding

小さな値をバイト単位や部分的なワードのストアで組み立て、すぐ後にワード単位で読むと、Intel の
ストアバッファは転送できない。転送できるのはロードが 1 つのストアに完全に含まれている場合だけで、
失敗するとストアがキャッシュに書かれるまで待つ (12 サイクル程度)。Apple の aarch64 は複数のストアに
またがるロードでも転送できるので、ARM で測っているだけでは気づかない。

lean_string では `InlineBuffer::new` と `InlineBuffer::from_char` がレジスタで組み立てているので
この問題は無いが、整数の変換 ([08](../plans/08-integer-codegen.md)) には残っている。

### 関数の境界をまたぐ値の大きさ

hot な関数のまれな分岐を別の関数に切り出したとき、切り出した関数の戻り値の大きさで、
hot 側の値がレジスタに残れるかが決まる。SysV x86-64 では 16 バイトを超える集約型は
sret (隠しポインタ) で返る。

lean_string の型での実測 (フィールド構成を再現したプログラムで確認):

| 型 | サイズ | 返し方 |
| --- | --- | --- |
| `Repr<M>` | 16 | `rax:rdx` |
| `Result<Repr<M>, ReserveError>` | 16 | `rax:rdx` (`LastByte` の niche) |
| `Result<Repr<Immutable>, (Repr<Mutable>, ReserveError)>` | **24** | **sret** |
| `Result<(), ReserveError>` | 1 | レジスタ |
| `HeapBuffer<H>` | 16 | `rax:rdx` |
| `Result<HeapBuffer<Exact>, (HeapBuffer<Growable>, ReserveError)>` | **24** | **sret** |

エラー側に値を持つ `Result` は niche を使えないので 24 バイトになり、レジスタで返せなくなる。
これを扱うのが [09](../plans/09-mutability-conversion.md)。

同じ話の裏返しで、cold 側に `&mut self` を渡すと `*self` のアドレスが外に出て、
呼び出しが起きない経路でも値がスタックに置かれる。これを扱うのが [04](../plans/04-drop-cold-abi.md)。

### `#[cold]` はマイクロベンチでは損に見える

`#[cold]` を外すとマイクロベンチでは 1 ns/op ほど速く見えることがあるが、命令列は同じで、
差は `.text.unlikely` に置かれるかどうかによる。小さなバイナリでは cold 側を呼んだときに
i-cache ミスが増えるだけだが、実際のアプリケーションでは hot なコードを詰めて置けることのほうが効く。

似た教訓として、compact_str には「命令数が 61 → 25 に減ったのに実行時間は 39% 増えた」という記録がある。
命令数と実測は食い違うことがあり、そのときは実測を信じる。

### 分岐の配置

`#[cold]` が配置のヒントとして効くのは、その分岐を通るすべての経路が cold な呼び出しに行き着くときだけで、
そうでない経路が 1 つでもあると効かない。compact_str は中身の無い `#[cold]` 関数を呼ぶ方法 (`cold_path()`) で
これを誘導していて、`perf/clone-branch-layout` で inline の clone が 1.08 ns → 0.64 ns になったと記録している。
同じコミットには「その分岐を `Repr` を返す `#[cold]` 関数として切り出すと 1.05 ns に戻る」という注意もある。

lean_string にも `internal::cold_path()` はあるが、使っているのは heap_buffer.rs の 2 か所だけ。
`Clone` はすでによい配置になっている (12 命令、heap の分岐は `lock incq` を含む 2 命令) ので、今のところ使う場面は無い。

### `#[cold]` だけではインライン化を止められない

char_str は `#[cold] #[inline(never)]` を必ず組にしている。lean_string の `HeapBuffer::release` の
`on_last_reference` は `#[cold]` だけだが、`.text.unlikely` に置かれていることは確認した。今は問題ない。

## ecow の比較の扱い (issue #35)

「共有しているハンドル同士の比較で、先にポインタを見るべきか」は ecow でも議論されている。
結論から言うと、ecow の main には入っていない。

- `src/string.rs` の `PartialEq for EcoString` は `self.as_str().eq(other.as_str())` のまま。
- `EcoVec::ptr_eq` は pub になっていない (`grep -rn "ptr_eq" src/` で何も出ない)。
- [issue #35](https://github.com/typst/ecow/issues/35) は 2023-10-10 に epage が
  「`Arc<str>` は先にポインタを比較するが、普通の参照の比較はそうしない」として提案し、
  2023-12-28 に completed でクローズされている。
- 対応するコミットは `ptr-eq` ブランチの `fa87e6f` (2023-11-27、メッセージに `Fixes #35`) だが、
  main の祖先ではない。変更している `src/dynamic.rs` は 2026-08-28 に `src/bytes.rs` に名前が変わっているので、
  そのままでは当たらない。リリース済みの 0.3.1 (2026-09-02) にも入っていない。

completed でクローズされているからといって、検証して採用されたわけではない。
設計として参考になるのは「ポインタを見る条件をバッファの種類で絞る」(ecow は両方 spilled、つまり heap のときだけ見る)
という点だけ。char_str は長さを比べた後にポインタを比べることで同じ効果を得ている。

## 取り入れるものが無いと確認したもの

- `ecow::EcoString::push` は ASCII を特別扱いしている (`if c.len_utf8() == 1 { self.0.push(c as u8) }`)。
  lean_string の `try_push` は常に `push_str` を通る。[03](../plans/03-append-writer.md) の writer が入れば解消する。
- `ecow::EcoVec::is_unique` は `&mut self` を取る。`is_unique` で確認してから変更するまでの間に
  clone されないことを型で保証するため。lean_string の `Repr::is_unique` は `&self` だが `pub(crate)` で、
  今の呼び出し元はすべて `&mut` を持っている。堅くする余地はあるが、バグではない。
