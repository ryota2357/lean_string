# 改善タスク一覧

`8f7fa75` (v0.7.0 + bench deps 更新) 時点のコードを調べて、実行可能な単位に分割したもの。
コード参照の行番号はこの時点のもの。

調査の記録は [`research/`](../research/) にある。

- [research/codegen-baseline.md](../research/codegen-baseline.md) — 現行コードの asm 実測。
  複数のプランがここの測定を出発点にしている。
- [research/equality.md](../research/equality.md) — 比較演算の設計調査。
- [research/related-crates.md](../research/related-crates.md) — 類似クレートの棚卸し。

## 一覧

| プラン | 種別 | 実装量 | 設計判断 | 一言 |
| --- | --- | --- | --- | --- |
| [01 inline のレジスタ構築](./01-inline-register-construction.md) | perf | 中 | 小 | inline 構築が 5 倍速くなることを実測済み |
| [02 中サイズのコピー](./02-medium-copy.md) | perf | 小 | 小 | heap 経路と追記経路の memcpy 呼び出しを外す |
| [03 `push_str` の fast path](./03-push-str-fast-path.md) | perf | 小〜中 | 中 | 判別子の 3 回読みと、呼び出し側で展開されない問題 |
| [04 追記用の内部 writer](./04-append-writer.md) | perf + 内部 API | 中 | 中 | `Extend` / `FromIterator` の 1 要素あたりのコスト |
| [05 cold への値渡し](./05-drop-cold-abi.md) | perf | 小 | 小 | drop に残る 16 バイトのスタックコピー |
| [06 `retain` の共有維持](./06-retain-shared-fast-path.md) | perf (準バグ) | 小 | 小 | 何も削らなくても共有が解ける |
| [07 比較の方針](./07-equality-policy.md) | 方針 + doc / 小 API | 小 | 中 | ポインタ近道を `==` に入れるか |
| [08 fallible な変換の panic](./08-fallible-to-lean-string.md) | バグ修正 | 小 | 小 | `try_to_lean_string` が確保失敗で panic する |
| [09 static の容量ドキュメント](./09-static-capacity-docs.md) | doc | 小 | 中 | 「最小容量は 16」が static バッファで成り立たない |
| [10 API の欠落](./10-api-gaps.md) | API 追加 | 小〜中 | 中 | `FromIterator` の非対称、`reserve_exact` ほか |
| [11 テストと契約](./11-test-hardening.md) | テスト + doc | 小〜中 | 小 | realloc 失敗の経路が未テスト |
| [12 保留項目](./12-deferred.md) | 記録 | - | - | プラン化しなかったものと、確認済みで問題なかったもの |

## 推奨順序

```
08 fallible な変換 ─┐
06 retain          ─┼─→ (独立。いつでも)
09 static の容量   ─┘

01 inline のレジスタ構築 ──(soft)──→ 02 中サイズのコピー
                                        ↓
                          03 push_str の fast path ──→ 04 追記用 writer

05 cold への値渡し   (独立)
07 比較の方針        (独立)
10 API の欠落        (独立)
11 テストと契約      (独立。11-A は早いほうがよい)
```

**08 → 11(A) → 01 → 05 → 06 → 09 → 02 → 03 → 04** を推す。

根拠:

1. **08 (fallible な変換)** は挙動の誤りなので最初に。修正は小さい。
2. **11 の A (realloc 失敗のテスト)** を早めに入れる。`into_exact` の失敗時復元は
   このクレートでもっとも入り組んだ unsafe で、そこが 1 度もテストされていない。
   後続のタスクが `reserve` 周辺を触るので、先に網を張っておきたい。
3. **01 (inline のレジスタ構築)** は効果が最大で、プロトタイプで実測済み
   (inline 構築が 7.5 ns → 1.4 ns)。独立して入れられる。
4. **05 (cold への値渡し)** も独立で確度が高い。drop の asm 検証は
   他の変更と混ざらないうちにやると差分が読みやすい。
5. **06 / 09** は小さく、他と衝突しない。
6. **02 → 03 → 04** はこの順。02 と 03 は `push_str` の同じ行を触り、
   04 は 03 の `push_str` を grow 経路として使う。
7. **07 (比較の方針)** と **10 (API の欠落)** はコードの触る場所が他と重ならないので
   いつでも。07 は方針を決めること自体が成果物。

ファイルの競合: 02/03 は `src/repr.rs` の `push_str` 周辺、01 は
`src/repr/inline_buffer.rs`、05 は `src/repr/heap_buffer.rs`。上の順序なら
rebase のコストは小さい。

## 各プランの構成

おおむね次の形で書いてある。

- **背景 / 現象**: 何が問題か。asm やベンチの実測があるものはそれを引用する
- **方針**: 具体的な変更内容。選択肢がある場合は案を並べて推奨を書く
- **検証方針**: テスト / Miri / loom / asm / criterion のどれで何を確認するか
- **依存**: 他のプランとの前後関係

## 検証の共通事項

- **asm** は `cargo asm` で見る。手順は
  [research/codegen-baseline.md](../research/codegen-baseline.md) の冒頭にある。
  LTO は切っておくと、下流から見たときのインライン化の効き方が観察できる。
- **ベンチ** は `bench/` に `apis.rs` (from / clone / reserve / push_str /
  push_str_after_clone / eq / eq_cloned など current vs prev vs std) と
  `comparison.rs` (他クレートとの比較) がある。
  なお `lean_string_prev` は直前のリリース版を指すので、リポジトリのコードが
  そのリリースから進んでいない時期には `current` と `prev` が同じソースになる。
  今回の測定時点 (v0.7.0 とリポジトリが同一) がそれで、そのとき観測される差は
  コード配置の偶然でしかない。詳細は
  [12-deferred.md](./12-deferred.md) の「ベンチの読み方」を参照。
- 「意味もなく速くなる」は採用理由にならない。コード上・asm 上の明確な改善を
  第一の根拠にして、ベンチは裏取りに使う。効果が誤差の範囲だった場合は、
  見送った記録を残す。
- **Miri** は CI で 64/32-bit × LE/BE の 4 ターゲットを strict provenance で回している。
  unsafe を触るタスク (01, 02, 03, 04, 05, 06) はローカルでも
  最低 `x86_64` と `i686` の 2 ターゲットを通してから push する。
- **loom** (`RUSTFLAGS="--cfg loom" cargo test --test loom --features loom -- --test-threads=1`)
  は atomic に触れるタスク (03, 05) で必須。
