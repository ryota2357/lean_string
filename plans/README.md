# 改善タスク一覧

`448a538` 時点のコードを調べて、実行可能な単位に分割したもの。
コード参照の行番号はこの時点のもの。

調査の記録は [`research/`](../research/) にある。

- [research/codegen-baseline.md](../research/codegen-baseline.md) — 現行コードの asm 実測。
  複数のプランがここの測定を出発点にしている。
- [research/equality.md](../research/equality.md) — 比較演算の設計調査。
- [research/related-crates.md](../research/related-crates.md) — 類似クレートの棚卸し。

## 一覧

| プラン | 種別 | 実装量 | 設計判断 | 一言 |
| --- | --- | --- | --- | --- |
| [01 `char` からの構築](./01-char-inline-construction.md) | perf | 小〜中 | 小 | 非 ASCII の `char` にスタック往復が残る |
| [02 中サイズのコピー](./02-medium-copy.md) | perf | 小 | 小 | heap 構築が len 25 で `String` より 37% 遅い |
| [03 `push_str` の fast path](./03-push-str-fast-path.md) | perf | 小〜中 | 中 | 判別子の 3 回読みと、呼び出し側で展開されない問題 |
| [04 追記用の内部 writer](./04-append-writer.md) | perf + 内部 API | 中 | 中 | `Extend` / `FromIterator` / `from_utf16` の 1 要素あたりのコスト |
| [05 cold への値渡し](./05-drop-cold-abi.md) | perf | 小 | 小 | drop に残る 16 バイトのスタックコピー |
| [06 `retain` の共有維持](./06-retain-shared-fast-path.md) | perf (準バグ) | 小 | 小 | 何も削らなくても共有が解ける |
| [07 比較の速度](./07-equality-policy.md) | 方針 + perf | 小〜中 | 大 | 効く場面の違う 3 案。どれを採るか |
| [08 テストのツールチェイン](./08-test-toolchain.md) | バグ修正 | 小 | 小 | `cargo test` が MSRV の 10 リリース先を要求する |
| [09 ドキュメント](./09-doc-fixes.md) | doc | 小 | 小 | `clear` の容量記述の誤り、`# Panics` の欠落ほか |
| [10 API の欠落](./10-api-gaps.md) | API 追加 | 小〜中 | 中 | `LeanStr::repeat` が黙って `String` を返す |
| [11 整数変換の codegen](./11-integer-codegen.md) | perf | 中 | 中 | 戻り値が 8 回のストアに分割される |
| [12 可変性の変換](./12-mutability-conversion.md) | perf | 中 | 小 | fallible な変換の戻り値が sret になる |
| [13 テストと契約](./13-test-hardening.md) | テスト + doc | 小〜中 | 小 | realloc 失敗の経路が未テスト |
| [14 保留項目](./14-deferred.md) | 記録 | - | - | プラン化しなかったものと、確認済みで問題なかったもの |

## 推奨順序

```
08 テストのツールチェイン ──→ (最初。これが無いとテストが走らない)

13-A realloc 失敗のテスト ──→ 12-A 変換の ABI

06 retain          ─┐
09 ドキュメント    ─┼─→ (独立。いつでも)
12-B/C 変換の往復  ─┘

01 char の構築      (独立)
05 cold への値渡し  (独立)

02 中サイズのコピー ──→ 03 push_str の fast path ──→ 04 追記用 writer
                                                        ↓
                                                    12-D LeanStr の構築

07 比較の速度       (独立。案3 → 案2 → 案1 の順に測る)
10 API の欠落       (独立)
11 整数変換         (独立)
```

**08 → 13(A) → 06 → 09 → 05 → 01 → 12(B,C) → 02 → 03 → 04** を推す。

根拠:

1. **08 (テストのツールチェイン)** が最初。`cargo test` が現状ではローカルで
   走らない (rustc 1.98.0 以降が要る) ので、これを直さないと以降の検証ができない。
2. **13 の A (realloc 失敗のテスト)** を早めに入れる。`into_exact` の失敗時復元は
   このクレートでもっとも入り組んだ unsafe で、そこが 1 度もテストされていない。
   12-A がその関数の形を変えるので、先に網を張っておきたい。
3. **06 / 09** は小さく、他と衝突しない。06 は挙動の非対称 (`truncate` は共有を維持するのに
   `retain` は解く) なので早めに。
4. **05 (cold への値渡し)** はプロトタイプで確認済み (単発 drop が 14 → 10 命令)。
   asm の検証は他の変更と混ざらないうちにやると差分が読みやすい。
5. **01 (`char` からの構築)** も独立で、効果の見込みが立っている。
6. **12 の B と C** は小さく、確保回数のテストで固定できる。
7. **02 → 03 → 04** はこの順。02 と 03 は `src/repr.rs` の `push_str` の同じ行を触り、
   04 は 03 の `push_str` を grow 経路として使う。
8. **07 / 10 / 11** はコードの触る場所が他と重ならないのでいつでも。
   07 は方針を決めること自体が成果物。

ファイルの競合: 02/03 は `src/repr.rs` の `push_str` 周辺、01 は
`src/repr/inline_buffer.rs` と `src/repr.rs` の `from_char`、05 と 12-A は
`src/repr/heap_buffer.rs`。上の順序なら rebase のコストは小さい。

## 各プランの構成

おおむね次の形で書いてある。

- **背景 / 現象**: 何が問題か。asm やベンチの実測があるものはそれを引用する
- **方針**: 具体的な変更内容。選択肢がある場合は案を並べて推奨を書く
- **検証方針**: テスト / Miri / loom / asm / criterion のどれで何を確認するか
- **依存**: 他のプランとの前後関係

## 検証の共通事項

- **asm** は probe 用クレートと `cargo rustc --release --lib -- --emit asm` で見る。
  手順は [research/codegen-baseline.md](../research/codegen-baseline.md) の冒頭にある。
  LTO は切っておくと、下流から見たときのインライン化の効き方が観察できる。
- **ベンチ** は `bench/` に `apis.rs` (from / clone / reserve / push_str /
  push_str_after_clone / eq / eq_cloned など current vs prev vs std) と
  `comparison.rs` (他クレートとの比較) がある。
  `lean_string_prev` は `=0.7.0` を指すので、v0.7.0 から変わっていない関数については
  `current` と `prev` が実質同じコードになる。そこで観測される差はコード配置の偶然。
  詳細は [14-deferred.md](./14-deferred.md) の「ベンチの読み方」を参照。
- **別プロセスの測定同士を 1 ns 未満の粒度で比べない**。効果を判断するときは、
  同一バイナリ内で cfg を切り替えた A/B を用意する。
- 「意味もなく速くなる」は採用理由にならない。コード上・asm 上の明確な改善を
  第一の根拠にして、ベンチは裏取りに使う。効果が誤差の範囲だった場合は、
  見送った記録を残す。
- **Miri** は CI で 64/32-bit × LE/BE の 4 ターゲットを strict provenance で回している。
  unsafe を触るタスク (01, 02, 03, 04, 05, 07, 11, 12) はローカルでも
  最低 `x86_64` と `i686` の 2 ターゲットを通してから push する。
- **loom** (`RUSTFLAGS="--cfg loom" cargo test --test loom --features loom -- --test-threads=1`)
  は atomic に触れるタスク (03, 05, 12) で必須。
