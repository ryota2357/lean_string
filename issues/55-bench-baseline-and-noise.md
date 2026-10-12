# `apis.rs` の `prev` は直前のリリースに固定されていて、リリース直後は current と同一コードになり、差はすべてノイズになる

- 種別: infra, perf
- 対象: `bench/Cargo.toml` の `lean_string_prev`, `bench/benches/apis.rs`
- 影響: リリース直後 (現在の `5dd94ea`) は `current` と `prev` が同一コードで、`apis.rs` の current/prev 比は意味を持たない。以降も「リリース以降に変更していない関数」は常に同一コードの比較になり、コードレイアウトと測定ノイズしか見えない。読み間違えると、存在しない改善・退行を根拠にしてしまう。
- 優先度: 中

## 問題

`bench/Cargo.toml` は `lean_string_prev = { package = "lean_string", version = "=0.8.0" }` で、`git diff bf326fa 5dd94ea -- src Cargo.toml` (v0.8.0 と `main`) は空。つまり `apis.rs` の `current` / `prev` は同じソースから作られた 2 つのコピーである。

同一コード同士の比がどれくらいぶれるかを測った (`5dd94ea`, x86-64, stable rustc, `flock` 下で
`cargo bench --bench apis -- --measurement-time 1 --warm-up-time 1 --noplot '^(from|clone|as_str|eq)/'` を 3 回、
4 コアの負荷のある共有マシンで測ったので load average は 5〜17。中央値の current/prev 比):

- 58 組のうち 36 組が ±10% 超、18 組が ±20% 超。最小 0.69、最大 1.62。
- 同じ組でも実行ごとに向きが変わる: `from/256` は 0.99 → 1.46、`from/17` は 1.13 → 0.78、`clone/16` は 1.05 → 1.50 (1 回目 → 2 回目)。
- criterion は直前の実行の結果と自動で比較する。コードを一切変えていないのに、2 回目の実行 (`from`, `clone` の 51 測定) では 39 件が "Performance has improved"、1 件が "has regressed" と表示された (3 回目の `as_str` の 18 測定でも 5 件 improved / 6 件 regressed)。

負荷が高い環境なので絶対値は誇張されているが、サブ ns〜十数 ns の操作ではクリーンな環境でもコード配置 (アラインメント、分岐のずれ) による数 % の差は出る。

一方、命令数は決定的である。同じ 2 コピーを callgrind (`valgrind --tool=callgrind --toggle-collect='irdrv::cur::*'`) で、1000 回ループの Ir を測った (load average 12 の状態で 3 回測って完全に同一):

```
scenario         current Ir    prev Ir
clone  (50B)        16015       16015
push_str x16       248904      248904
push 'あ' x16      217070      217070
```

(`from` と `eq` では `prev` 側のラッパーが LTO で `cur` 側と同一関数として畳まれたらしく、`prev` の toggle が 0 を返した。畳まれた原因は未確認だが、同一コードであることの傍証になる。`from` の Ir は `cur` 側で 16B: 17 / 17B: 183 / 32B: 188 / 64B: 188 / 65B: 205 Ir/call で、アロケータ込み。)

## 原因

- `prev` を「最後のリリース」に固定する運用そのもの。`git log -- bench/Cargo.toml` を見ると、リリースのたびに `prev` を新しいバージョンへ上げている (`5dd94ea` が v0.8.0 への更新)。上げた直後は必ず同一コードになる。
- 変更した関数としていない関数が同じ表に並ぶので、どの行が「変更の影響を受けうる行」なのか表から分からない。
- ベンチの読み方 (current/prev が同一コードのときはノイズである、wall-clock は ±10〜20% ぶれる) が `bench/` 配下のどこにも書かれていない (`bench/README` はなく、CI・ルートの `README.md`・`CHANGELOG.md` にも `bench` への言及はない)。

## 修正例

比較対象を「バージョン」ではなく「任意のリビジョン」にする。in-process で current と prev を同じバイナリに入れる現在の構成は、別プロセスの baseline 比較 (`--save-baseline` + `critcmp`) より時間方向のドリフトに強いので維持する。変えるのは `prev` の指し方だけ。

1. git 依存でリビジョンを固定する (動作確認済み: `cargo fetch` で `source = "git+https://github.com/ryota2357/lean_string?rev=a4ad207#<full sha>"` として、registry の `0.8.0` と同じバージョンのまま別パッケージとして解決できた):

   ```toml
   # bench/Cargo.toml
   # 比較したい変更の直前のコミット (PR なら `git merge-base origin/main HEAD`) を指定する。
   lean_string_prev = { package = "lean_string", git = "https://github.com/ryota2357/lean_string", rev = "<sha>" }
   ```

   ローカルの未コミット・未 push の基準を使いたい場合は path 依存でもよいが、バージョンが current と同じだと Cargo が `package collision in the lockfile` で拒否する (確認済み)。基準側の `Cargo.toml` の `version` を `0.8.0-base` のように変えれば通る (確認済み: ビルド成功)。

2. `bench/README.md` を足して、次の 3 点を書く:
   - `prev` が current と同一コードの間は current/prev の差はノイズであること。
   - 判断に使うのは「変更した関数」の行だけで、wall-clock の ±10〜20% は無視すること。サブ ns〜十数 ns の差は、asm・命令数で裏付けること。
   - `prev` の更新手順 (リリース時にバージョンへ、開発中は `rev` へ) を書くこと。

3. 決定的な補助として、callgrind の Ir を取る小さなドライバ (`bench/ir/`) を置く案は [#57](./57-codegen-claims-unchecked.md) に書いた。ノイズ込みの wall-clock より先に Ir/asm を見る運用にすれば、同一コード比較の誤読は構造的に起きなくなる。

案の比較:

| 案 | 長所 | 短所 |
|---|---|---|
| `rev` 固定の git 依存 (推奨) | 同一プロセス A/B を維持、リリース前でも比較対象を選べる | `Cargo.toml` を書き換える手間、オフラインでは使えない |
| `--save-baseline` + `critcmp` | 依存の書き換え不要 | 別プロセス・別時刻の比較なので負荷ドリフトを拾う。サブ ns の差には不向き |
| 現状 (`version = "=x.y.z"`) | 簡単 | リリース直後は同一コード |

## 確認方法

- `bench/Cargo.toml` の `prev` を 1 つ前のコミットに向けて `cargo bench --no-run` が通ること。
- 同一リビジョンを `prev` に指定したとき、README に書いた手順どおり「差はノイズ」と判断できること (上記の 36/58 が ±10% 超という数字を README に載せてもよい)。
