# 類似クレートの棚卸し

調査日: 2026-08-16 / lean_string 側の基準: `8f7fa75` (v0.7.0 + bench deps 更新)

対象:

- [ParkMyCar/compact_str](https://github.com/ParkMyCar/compact_str) — `9696af7f` (v0.10.0)。
  24 バイト表現、inline / `&'static str` / heap の 3 種。参照カウントは無く CoW でもない。
- [astral-sh/char_str](https://github.com/astral-sh/char_str) — `93f02a4` (v0.0.2)。
  16 バイト表現、inline / static / 参照カウント付き heap。lean_string と同型の CoW。
- [typst/ecow](https://github.com/typst/ecow) — 16 バイト表現の CoW。
  比較の議論のみ参照 ([equality.md](./equality.md) を参照)。

lean_string との関係:

- `compact_str` と `ecow` は lean_string とは独立に作られた別のクレートで、
  コードの継承関係は無い。参考にするのは考え方であって差分ではない。
- `char_str` は lean_string を fork したクレート (README と `Cargo.toml` に明記されている)。
  表現も buffer の種類も一致するので、差分をほぼそのまま取り込める。

`compact_str` は表現が 24 バイトなので、「表現の大きさに依存しない理屈」だけを取る。

以下、それぞれのクレートを指すときはクレート名で書く。

## すでに lean_string にあるもの

| 内容 | lean_string |
| --- | --- |
| `ToLeanString` / `ToLeanStr` の `str` 特殊化 | `8d450b6` |
| f32/f64 の変換を zmij へ | `dce0824` |
| `Repr` の判別子に `assert_unchecked` を置いて `Result` の Err arm を畳む | `7fde9c5` |
| `Capacity::new` の `#[inline]` と上限の `assert_unchecked` | `2878dfe` |
| 32-bit の長さ上限のドキュメント修正 | `b124daf`、`ae513f7` |
| loom テストから dhat 依存を外す | `bf71af7` |
| 確保を失敗させるアロケータによる OOM テスト | `92aee71` |
| `reserve(0)` を no-op にする | `8c1a40d` |
| Drop の書き戻し除去 | `b94f15a` |
| inline バッファのコピーを固定長にする | `553c73c` |

## まだ取り込んでいないもの

| 内容 | 出典 | プラン |
| --- | --- | --- |
| inline バッファをレジスタで組み立てる | compact_str | [01](../plans/01-inline-register-construction.md) |
| 中サイズのコピーから memcpy 呼び出しを外す | compact_str | [02](../plans/02-medium-copy.md) |
| `push_str` の fast path を小さくする | compact_str | [03](../plans/03-push-str-fast-path.md) |
| 連続追記のための内部 writer | compact_str | [04](../plans/04-append-writer.md) |
| cold 関数へ参照ではなく値を渡す | compact_str | [05](../plans/05-drop-cold-abi.md) |
| 共有バッファ比較のポインタ近道 | char_str / ecow | [07](../plans/07-equality-policy.md) — 方針として不採用寄り |
| fallible な変換が確保失敗で panic する | char_str | [08](../plans/08-fallible-to-lean-string.md) |
| 整数フォーマットのレジスタ組み立て | compact_str | [12](../plans/12-deferred.md) — 16 バイト表現では u64 が収まらず、そのままは持ち込めない |
| `format!` 相当のマクロ / `join` / storage 構築子 / 統合 feature | char_str | [12](../plans/12-deferred.md) — 新 API のため方針判断待ち |

## 覚えておく価値のある知見

各プランの根拠になっているので、要点だけ残す。

### store-to-load forwarding

小さな値をバイト単位や部分ワードのストアで組み立て、直後に読み手がワード単位で
読むと、Intel のストアバッファは転送できない。**ロードが単一のストアに完全に
含まれている場合にしか転送できない**ためで、失敗するとストアがキャッシュに
書かれるまで待つ (12 サイクル程度)。Apple の aarch64 は複数ストアに跨るロードでも
転送できるので、ARM だけで測ると見えない。

これは lean_string でも再現した ([codegen-baseline.md](./codegen-baseline.md) の §1、
`from` ベンチの len 16 だけが 5 倍速い段差)。表現が 2 ワードしかないぶん、
24 バイト表現より対処は簡単で、効果も相対的に大きい。

### 呼び出し境界を跨ぐ値の大きさ

hot な関数の稀な arm を outline したとき、**outline した関数の戻り値の大きさが
hot 側がレジスタに留まれるかを決める**。SysV x86-64 では 16 バイトを超える集約は
sret (隠しポインタ渡し) になり、LLVM は hot 側の結果も同じスタック領域へ
まとめてしまう。compact_str はこれを踏んで、cold 側が返すものを (ptr, capacity) の
2 ワードに削るという対処をしている。

lean_string でも実際に何がレジスタで返るかを測った。`Repr` は 16 バイトだが、
最終バイトを独立したフィールドに切り出した `(*const (), [u8; 7], LastByte)` という
並びのため scalar pair にならず、値で返すと sret になる。一方 `HeapBuffer` は
`(NonNull<u8>, TextLen)` の 2 ワードなのでレジスタ返しになる。

| 型の形 | 戻り方 |
| --- | --- |
| `(NonNull<u8>, usize)` (= `HeapBuffer`) | `rax:rdx` |
| `(*const (), [u8; 7], LastByte)` (= `Repr`) | sret |

つまり lean_string もこの問題を免れていない。cold 側へ渡すもの・返すものを
必要最小限に削るという方針はそのまま当てはまる
([plans/03](../plans/03-push-str-fast-path.md)、[plans/05](../plans/05-drop-cold-abi.md))。
`Repr` がレジスタ返しにならない原因である最終バイトの切り出しは、
`Option<LeanString>` が 16 バイトに収まるための niche でもあるので、
戻り値の都合で変えるものではない。

同じ話の裏返しが「cold 側に `&mut self` を渡すと `*self` のアドレスが escape し、
呼び出しが起きない経路でも値をスタックに実体化させられる」というもの。
これは lean_string にそのまま当てはまっており、
[plans/05](../plans/05-drop-cold-abi.md) の題材になっている。

### `#[cold]` はマイクロベンチでは損に見える

`#[cold]` を外すとマイクロベンチでは 1 ns/op ほど速く見えることがあるが、
生成される命令列は同じで、差は `.text.unlikely` への配置によるもの。
小さなバイナリでは cold 側の呼び出しに余分な i-cache ミスが乗るだけだが、
実アプリケーションでは hot な text を詰めておくことのほうが効く。
マイクロベンチはコストだけを測って利得を測らない。

同種の教訓として、compact_str には「命令数が 61 → 25 に減ったのに実行時間が +39% になった」
という記録がある。命令数と実測は食い違うことがあり、食い違ったら実測を採る。

### 分岐の配置

`#[cold]` は「その arm を通るすべての経路が cold な呼び出しに到達する」ときにだけ
配置のヒントとして効く。到達しない sub-arm が 1 つでもあると効かない。
compact_str はそのために「中身が空の `#[cold]` 関数を呼ぶ」という手を使っている。

lean_string の `Clone` は既に望ましい配置になっている
([plans/12](../plans/12-deferred.md) の「確認して問題なかったもの」参照) ので、
今のところ出番は無い。

## 該当しないことを確認したもの

- `compact_str` の soundness 修正 3 件 (retain 中の不正な UTF-8 参照、未初期化の
  余剰容量の露出、realloc の UB) は、いずれも lean_string の実装形が異なるため非該当。
  ただし「意図しない領域への `&str` / `&[u8]` の生成」という類は Miri で
  見張り続ける価値がある。現行 CI は 4 ターゲットで全 suite を Miri で回しており、
  `PROPTEST_CASES=10000` の property テストも含まれるので、体制としては足りている。
- `compact_str` の整数フォーマットのレジスタ組み立ては、24 バイト表現なら
  u64 の最大 20 桁が inline に収まるという前提に立っている。16 バイトでは成立しない。
- `char_str` の inline 比較高速化の 2 案は、[plans/07](../plans/07-equality-policy.md) で
  ポインタ近道を既定の `==` に入れない方針を採るなら同時に見送りになる。
  いずれも char_str の main にはマージされておらず、計測値も残っていない。
