# 改善タスク一覧

`8bf3fee` 時点のコードを調べ、改善点を作業単位に分けたもの。
コードは行番号ではなく、ファイル名と関数名で参照している。

調査の記録は [`research/`](../research/) にある。

- [research/codegen-baseline.md](../research/codegen-baseline.md) — 現行コードの asm 実測。
  多くのプランがここでの測定を前提にしている。
- [research/equality.md](../research/equality.md) — 比較演算の設計調査。
- [research/related-crates.md](../research/related-crates.md) — 類似クレートの調査。

## 一覧

| プラン | 種別 | 実装量 | 設計判断 | 概要 |
| --- | --- | --- | --- | --- |
| [01 中サイズのコピー](./01-medium-copy.md) | perf | 小 | 小 | heap 構築が len 25 で `String` より 37% 遅い |
| [02 `push_str` の fast path](./02-push-str-fast-path.md) | perf | 小〜中 | 中 | 判別子を 3 回読んでいる。呼び出し側に展開されない |
| [03 追記用の内部 writer](./03-append-writer.md) | perf + 内部 API | 中 | 中 | `Extend` / `FromIterator` / `from_utf16` / 大文字小文字の変換で、1 要素ごとにかかるコスト |
| [04 cold への値渡し](./04-drop-cold-abi.md) | perf | 小 | 小 | drop で 16 バイトをスタックにコピーしている |
| [05 比較の速度](./05-equality-policy.md) | 方針 + perf | 小〜中 | 大 | 効く場面の違う 3 つの案のどれを採るか |
| [06 ドキュメント](./06-doc-fixes.md) | doc | 小 | 小 | `clear` の容量記述の誤り、`# Panics` の欠落ほか |
| [07 API の欠落](./07-api-gaps.md) | API 追加 | 小〜中 | 中 | `reserve_exact`、参照からの `From`、デコード系の `try_` 版ほか |
| [08 整数変換の codegen](./08-integer-codegen.md) | perf | 中 | 中 | 戻り値が 5 回のストアに分かれる。符号付きは展開もされない |
| [09 可変性の変換](./09-mutability-conversion.md) | perf | 中 | 小 | 失敗しうる変換の戻り値が sret になる |
| [10 テストと契約](./10-test-hardening.md) | テスト + doc | 小〜中 | 小 | realloc の失敗がテストされていない |
| [11 保留項目](./11-deferred.md) | 記録 | - | - | プランにしなかったものと、確認して問題なかったもの |

## 推奨順序

```
10-A realloc 失敗のテスト ──→ 09-A 変換の ABI

06 ドキュメント    ─┐
09-B/C 変換の往復  ─┴─→ (独立。いつでも)

04 cold への値渡し  (独立)

01 中サイズのコピー ──→ 02 push_str の fast path ──→ 03 追記用 writer
                                                        ↓
                                                    09-D LeanStr の構築

05 比較の速度       (独立。案3 → 案2 → 案1 の順に測る)
07 API の欠落       (独立)
08 整数変換         (独立)
```

**10(A) → 06 → 04 → 09(B,C) → 01 → 02 → 03** の順がよい。

理由:

1. 10 の A (realloc の失敗のテスト) を最初に入れる。`into_exact` の失敗時の復元は
   このクレートでいちばん込み入った unsafe なのに、一度もテストされていない。
   09-A でその関数の形を変えるので、その前にテストを用意しておく。
2. 06 は小さく、他と衝突しない。
3. 04 (cold への値渡し) はプロトタイプで効果を確認済み (1 つだけの drop が 14 → 10 命令)。
   他の変更と混ざる前にやると、asm の差分が読みやすい。
4. 09 の B と C は小さく、確保回数のテストで固定できる。
5. 01 → 02 → 03 はこの順で。01 と 02 は `src/repr.rs` の `push_str` の同じ箇所を触り、
   03 は 02 の `push_str` を grow の経路に使う。
6. 05 / 07 / 08 は他と触る場所が重ならないので、いつでもよい。
   05 は方針を決めることが主な作業になる。

ファイルの競合: 01 / 02 は `src/repr.rs` の `push_str` 周辺、04 と 09-A は
`src/repr/heap_buffer.rs` を触る。上の順番なら rebase の手間は小さい。

## 各プランの構成

だいたい次の順で書いている。

- 背景 / 現象: 何が問題か。asm やベンチの実測があればそれを載せる
- 方針: 具体的な変更。選択肢があるときは案を並べて、どれがよいかを書く
- 検証: テスト / Miri / loom / asm / criterion のどれで何を確かめるか
- 依存: 他のプランとの前後関係

## 検証について

- asm は probe 用のクレートと `cargo rustc --release --lib -- --emit asm` で見る。
  手順は [research/codegen-baseline.md](../research/codegen-baseline.md) の最初にある。
  LTO を切っておくと、下流から呼んだときにインライン化されるかどうかが分かる。
- ベンチは `bench/` に `apis.rs` (from / clone / reserve / push_str /
  push_str_after_clone / eq / eq_cloned / to_lowercase / to_uppercase など current vs prev vs std) と
  `comparison.rs` (他クレートとの比較) がある。
  `lean_string_prev` は `=0.7.0` を指しているので、v0.7.0 から変わっていない関数では
  `current` と `prev` は実質同じコードになる。そこで出る差はコード配置による偶然。
  詳しくは [11-deferred.md](./11-deferred.md) の「ベンチの読み方」。
- 別のプロセスで測った値同士を 1 ns 未満の単位で比べない。効果を判断するときは、
  同じバイナリの中で cfg を切り替えて A/B を取る。
- 理由は分からないが速くなった、というのは採用の理由にしない。コードや asm の上での明確な改善を
  主な根拠にして、ベンチはその確認に使う。効果が誤差程度なら、見送ったことを記録しておく。
- Miri は CI で 64/32-bit × LE/BE の 4 ターゲットを strict provenance で回している。
  unsafe を触るタスク (01, 02, 03, 04, 05, 08, 09) は、手元でも少なくとも `x86_64` と `i686` で
  通してから push する。
- loom (`RUSTFLAGS="--cfg loom" cargo test --test loom --features loom -- --test-threads=1`) は、
  atomic を触るタスク (02, 04, 09) では必ず回す。
