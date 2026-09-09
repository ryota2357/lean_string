# 類似クレートの棚卸し

調査日: 2026-09-09 / lean_string 側の基準: `448a538`

対象:

| クレート | 調査コミット | バージョン | 表現 |
| --- | --- | --- | --- |
| [ParkMyCar/compact_str](https://github.com/ParkMyCar/compact_str) | `9696af7f` | 0.10.0 | 24 バイト、inline / `&'static str` / heap。参照カウント無し |
| [astral-sh/char_str](https://github.com/astral-sh/char_str) | `4b401e93` | 0.0.4 | 16 バイト、inline / static / 参照カウント付き heap |
| [typst/ecow](https://github.com/typst/ecow) | `99333459` | 0.3.1 | 16 バイト (inline は 15 バイト) の CoW |

lean_string との関係:

- `compact_str` と `ecow` は lean_string とは独立に作られた別のクレートで、
  コードの継承関係は無い。参考にするのは考え方であって差分ではない。
- `char_str` は lean_string を fork したクレート。ただし `Mutability` の型パラメータが
  入る前の時点で分岐しており、`Repr<Mutable>` / `Repr<Immutable>` の型レベルの分離を
  実行時のタグ (`ExactHeapMarker` / `HeapMarker`) に置き換えている。
  この点については lean_string のほうが良い形になっているので、そこは取らない。
- `compact_str` は表現が 24 バイトなので、「表現の大きさに依存しない理屈」だけを取る。

未マージのブランチも見た。`char_str` は 8 本の `charlie/*`、`compact_str` は
11 本の `perf/*` と `perf-audit` がある。実験の記録として価値があるので、
該当するものは以下で個別に挙げる。

## まだ取り込んでいないもの

| 内容 | 出典 | プラン |
| --- | --- | --- |
| fallible な変換の戻り値が sret になる | char_str (`make_exact` / `make_growable`) | [12](../plans/12-mutability-conversion.md) |
| 共有バッファのポインタ近道 | char_str (main にマージ済み) | [07](../plans/07-equality-policy.md) 案1 |
| inline 表現の正規形 + ワード比較 | char_str `charlie/canonical-inline-equality` | [07](../plans/07-equality-policy.md) 案2 |
| 短い文字列の `bcmp` を避ける比較 | char_str `charlie/inline-string-equality` | [07](../plans/07-equality-policy.md) 案3 |
| 中サイズのコピーから memcpy 呼び出しを外す | compact_str (`copy_medium`) | [02](../plans/02-medium-copy.md) |
| 追記のコピーを固定長にする | char_str `charlie/inline-concat-append-copies` | [02](../plans/02-medium-copy.md) |
| `push_str` の fast path を小さくする | compact_str `perf-audit` | [03](../plans/03-push-str-fast-path.md) |
| 連続追記のための内部 writer | compact_str (`Extend<char>` / `repr/iter.rs`) | [04](../plans/04-append-writer.md) |
| cold 関数へ参照ではなく値を渡す | compact_str (`outlined_drop`) | [05](../plans/05-drop-cold-abi.md) |
| unique な drop で atomic RMW を避ける | char_str `charlie/unique-string-drop` | [05](../plans/05-drop-cold-abi.md) |
| 整数フォーマットのコンパイル時 inline 判定 | compact_str (`repr/num.rs`) | [11](../plans/11-integer-codegen.md) |
| 整数の桁をレジスタで組み立てる | compact_str `perf/int-formatting-registers` | [11](../plans/11-integer-codegen.md) |
| `len == 0` を長さ分岐の先頭へ | compact_str `perf/empty-string-create` | [14](../plans/14-deferred.md) |
| 1 回の確保で済む `join` / `concat` | char_str | [14](../plans/14-deferred.md) |
| `format_*!` マクロ | compact_str / char_str / ecow (3 クレートすべて) | [14](../plans/14-deferred.md) |
| `INLINE_CAPACITY` / `new_inline` / `new_heap` | char_str | [14](../plans/14-deferred.md) |
| `get-size2` / `salsa` の統合 feature | char_str | [14](../plans/14-deferred.md) |
| `from_utf8_lossy` の全 valid fast path | compact_str | [10](../plans/10-api-gaps.md) |
| `as_mut_str` / `drain` / `replace_range` ほか | compact_str / ecow | [10](../plans/10-api-gaps.md) |

## 該当しないことを確認したもの

同じところを二度調べないための記録。

### compact_str の `(ptr, cap)` レジスタ返し一式

`compact_str/src/repr/heap.rs:35-155` は `alloc_copy` / `alloc` / `alloc_copy_extra` を
「2 ワードのタプルを返す `#[cold] #[inline(never)]` の本体 + `#[inline(always)]` の
薄いラッパ」に分けている。24 バイトの `Repr` が SysV の MEMORY クラスになって
sret 返しになり、LLVM がそれを戻り値スロットへ散らしてコピーしていたための対処。

lean_string には該当しない。`Repr` は 16 バイトで、`Result<Repr<M>, ReserveError>` も
`LastByte` の niche によって 16 バイトに収まる (実測)。したがって `rax:rdx` で返る。
ただしタプル型のエラーを持つ `Result` は 24 バイトになるので、そちらは該当する
([plans/12](../plans/12-mutability-conversion.md))。

### compact_str の `InlineBuffer::new` の `[u64; 3]` レジスタ組み立て

lean_string には 2 ワード版が既にある (`src/repr/inline_buffer.rs:21-70`)。
帯ごとの重ね合わせロードも provenance と endianness の扱いも同じ。取るものは無い。

### compact_str の「64-bit 整数はすべて inline に収まる」前提

`MAX_SIZE = 24` なら `i64::MIN` の 20 文字が収まる。`MAX_INLINE_SIZE = 16` では
32 ビットまでしか収まらない。手法そのものは構造的だが、適用範囲が狭まる
([plans/11](../plans/11-integer-codegen.md))。

### compact_str の `into_string()` / `from_string_buffer()`

`String` との O(1) 相互変換。lean_string のヒープレイアウトはデータの前にヘッダを
置くので、`String::from_raw_parts` に渡せる形にはならない。構造的に不可能。

### char_str の `Header { capacity, count }` のフィールド順

`count` が両レイアウトで `ptr - 8` に来るように並べ、`reference_count()` の
`if is_exact()` を固定オフセットのロードに畳む工夫。lean_string は
`Mutability` が型パラメータなので分岐そのものが存在しない。取るものは無い。

### char_str の実行時 exact/growable タグと release ビルドの `assert!`

`charlie/harden-string-invariants` が入れているもの。lean_string は同じ不変条件を
型レベルで保証しているので不要。

### char_str の `ToCharString` フォールバック

lean_string の `try_from_fmt` (src/traits.rs) のほうが進んでいる
(確保エラーの退避と `args.as_str()` の fast path を持つ)。
`charlie/fix-fallible-char-string-format` の内容は既に包含している。

### ecow の 2 倍成長

lean_string は 1.5 倍 (`amortized_growth`, heap_buffer.rs:19-23)。
意図的な差であって改善ではない。

### `repeat` の実装

4 クレートとも `push` のループで、std の倍々コピーを持っているものは無い。
取れるものは無い (std の手法を自前で入れるかは [plans/04](../plans/04-append-writer.md) の話)。

## 覚えておく価値のある知見

各プランの根拠になっているので、要点だけ残す。

### store-to-load forwarding

小さな値をバイト単位や部分ワードのストアで組み立て、直後に読み手がワード単位で
読むと、Intel のストアバッファは転送できない。ロードが単一のストアに完全に
含まれている場合にしか転送できないためで、失敗するとストアがキャッシュに
書かれるまで待つ (12 サイクル程度)。Apple の aarch64 は複数ストアに跨るロードでも
転送できるので、ARM だけで測ると見えない。

lean_string では `InlineBuffer::new` がレジスタ構築になったことでこれを回避済みだが、
`Repr::from_char` の非 ASCII 経路 ([plans/01](../plans/01-char-inline-construction.md)) と
整数フォーマット ([plans/11](../plans/11-integer-codegen.md)) にはまだ残っている。

### 呼び出し境界を跨ぐ値の大きさ

hot な関数の稀な arm を outline したとき、**outline した関数の戻り値の大きさが
hot 側がレジスタに留まれるかを決める**。SysV x86-64 では 16 バイトを超える集約は
sret (隠しポインタ渡し) になる。

lean_string の実測 (フィールド構成を再現した独立のプログラムで確認):

| 型 | サイズ | 返し方 |
| --- | --- | --- |
| `Repr<M>` | 16 | `rax:rdx` |
| `Result<Repr<M>, ReserveError>` | 16 | `rax:rdx` (`LastByte` の niche) |
| `Result<Repr<Immutable>, (Repr<Mutable>, ReserveError)>` | **24** | **sret** |
| `Result<(), ReserveError>` | 1 | レジスタ |
| `HeapBuffer<H>` | 16 | `rax:rdx` |
| `Result<HeapBuffer<Exact>, (HeapBuffer<Growable>, ReserveError)>` | **24** | **sret** |

エラー側にペイロードを載せた `Result` は niche を使えないので 24 バイトになり、
レジスタ返しから外れる。これが [plans/12](../plans/12-mutability-conversion.md) の題材。

同じ話の裏返しが「cold 側に `&mut self` を渡すと `*self` のアドレスが escape し、
呼び出しが起きない経路でも値をスタックに実体化させられる」というもの。
これは [plans/05](../plans/05-drop-cold-abi.md) の題材になっている。

### `#[cold]` はマイクロベンチでは損に見える

`#[cold]` を外すとマイクロベンチでは 1 ns/op ほど速く見えることがあるが、
生成される命令列は同じで、差は `.text.unlikely` への配置によるもの。
小さなバイナリでは cold 側の呼び出しに余分な i-cache ミスが乗るだけだが、
実アプリケーションでは hot な text を詰めておくことのほうが効く。

同種の教訓として、compact_str には「命令数が 61 → 25 に減ったのに実行時間が +39% になった」
という記録がある。命令数と実測は食い違うことがあり、食い違ったら実測を採る。

### 分岐の配置

`#[cold]` は「その arm を通るすべての経路が cold な呼び出しに到達する」ときにだけ
配置のヒントとして効く。到達しない sub-arm が 1 つでもあると効かない。
compact_str は「中身が空の `#[cold]` 関数を呼ぶ」という手 (`cold_path()`) でこれを誘導し、
`perf/clone-branch-layout` で inline clone が 1.08 ns → 0.64 ns になったと記録している。
同じコミットに「その arm を `Repr` を返す `#[cold]` 関数として outline すると
1.05 ns に戻る」という警告もある。

lean_string にも `internal::cold_path()` はあるが、使っているのは `Capacity::new` と
`allocation()` のみ。`Clone` は既に望ましい配置になっている (12 命令、heap arm は
`lock incq` の 2 命令) ので、いまのところ出番は無い。

### `#[cold]` は inline を禁止しない

`#[cold]` だけでは inline 化を禁止できない。char_str は
`#[cold] #[inline(never)]` を対にしている。lean_string の
`HeapBuffer::release` の `on_last_reference` は `#[cold]` のみだが、
実測では `.text.unlikely` に配置されていることを確認した。現状は問題ない。

## ecow の比較の扱い (issue #35)

「共有ハンドルの比較でポインタを先に見るべきか」は ecow でも議論になっている。
結論から書くと、**ecow の main には入っていない**。

- `src/string.rs:352-359` の `PartialEq for EcoString` は
  `self.as_str().eq(other.as_str())` のまま。
- `EcoVec::ptr_eq` は pub になっていない (`grep -rn "ptr_eq" src/` が何も返さない)。
- [issue #35](https://github.com/typst/ecow/issues/35) は 2023-10-10 に epage が
  「`Arc<str>` は先にポインタを比較するが通常の参照の等価比較はそれをしない」として
  提案し、2023-12-28 に completed としてクローズされている。
- 対応するコミットは `ptr-eq` ブランチの `fa87e6f` (2023-11-27,
  メッセージに `Fixes #35`) にあるが、main の祖先ではない。
  触っている `src/dynamic.rs` は 2026-08-28 に `src/bytes.rs` へ改名されており、
  そのままでは適用できない。リリース済みの 0.3.1 (2026-09-02) にも入っていない。

**「completed でクローズされている」ことを「検証されて採用された」と読んではいけない**。
設計として参考になるのは 1 点だけで、「近道を適用する条件をバッファ種別で絞る」
(ecow は spilled = ヒープのときだけ見る) という考え方。
char_str は長さ比較の後にポインタを比べることで同じ効果を得ている。

## 各クレートに固有で、取れるものが無いことを確認したもの

- `ecow::EcoString::push` は ASCII を特別扱いする
  (`if c.len_utf8() == 1 { self.0.push(c as u8) }`, src/string.rs:174-180)。
  lean_string の `try_push` は常に `push_str` を通る。これは
  [plans/04](../plans/04-append-writer.md) の writer で自然に解消する。
- `ecow::EcoVec::is_unique` は `&mut self` を取る (src/vec.rs:878-888)。
  「`is_unique` の観測から変更までの間に clone が起きないことを型で保証する」ため。
  lean_string の `Repr::is_unique` は `&self` だが `pub(crate)` で、
  現在の呼び出し元はすべて `&mut` を持っている。堅牢化の余地であって、バグではない。
