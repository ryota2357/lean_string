# 04: 連続追記のための内部 writer

種別: perf + 内部 API / 実装量: 中 / 設計判断: 中 (API の形)

## 背景

`Extend` 系と `FromIterator` 系は 1 要素ごとに `push`/`push_str` を呼ぶ。
[research/codegen-baseline.md](../research/codegen-baseline.md) の §4 のとおり、
`Extend<char>` のループ本体は 1 文字あたり

- `Repr::push_str` への関数呼び出し (インライン化されない)
- 長さの復元とオーバーフロー判定
- 判別子の分岐
- 参照カウントのロード (heap のとき。aarch64 では `ldar`)
- 容量比較
- 1〜4 バイトのための `memcpy` 呼び出し
- `set_len` での判別子の再分岐

を通る。バッファは一度解決すれば済むので、これらはループの外に出せる。

同じ構造の呼び出し元:

| 場所 | 現状 |
| --- | --- |
| `Extend<char>` (lib.rs:2011) | `try_reserve(size_hint)` 後、1 文字ずつ `push` |
| `Extend<&char>` (lib.rs:2025) | 上へ委譲 |
| `Extend<&str>` / `Box<str>` / `Cow` / `String` / `LeanString` / `LeanStr` (lib.rs:2031-2069) | 1 要素ずつ `push_str`。`size_hint` は使わない |
| `FromIterator<char>` ほか (lib.rs:1899-2009) | 上と同じ経路 |
| `LeanString::try_repeat` (lib.rs:818-831) | 最終長が既知なのに `try_push_str` のループ |

`try_repeat` は最終容量も「新しく作った値なので unique である」ことも呼び出し前から
分かっているので、writer が無くても改善できる。

## 方針

### 案1 (推奨): 内部 `Appender`

バッファを 1 回だけ解決し、ローカルにポインタと長さを持つ writer を作る。

```rust
// repr.rs (crate 内部専用)
struct Appender<'a> {
    repr: &'a mut Repr<Mutable>,
    ptr: *mut u8,   // 解決済みのデータポインタ
    cap: usize,
    len: usize,     // ローカルで進める長さ (commit まで repr には書かない)
}

impl Repr<Mutable> {
    /// `size_hint` 分の容量を確保し (0 なら確保しない)、書き込み可能に解決した
    /// `Appender` を返す。
    fn appender(&mut self, size_hint: usize) -> Result<Appender<'_>, ReserveError>;
}

impl Appender<'_> {
    /// `bytes` は UTF-8 として完結した単位 (char のエンコード結果または &str)。
    /// 入り切らなければ commit → grow → 再解決。
    #[inline]
    fn push_bytes(&mut self, bytes: &[u8]) -> Result<(), ReserveError>;
}

impl Drop for Appender<'_> {
    // set_len(self.len) を commit する
    fn drop(&mut self);
}
```

- fast path の `push_bytes` は「容量比較 + コピー + ローカル長の加算」だけになる。
  長さの復元・判別子分岐・参照カウントのロードは `appender()` の 1 回に集約される。
- grow 経路は commit してから `reserve` を呼び、ポインタと容量を取り直す。
  頻度が低いので `#[cold]` にする。
- panic 安全: イテレータの `next()` やユーザのクロージャが panic しても `Drop` で
  `set_len` が走り、「そこまで書いた分」で有効な状態に戻る。`retain` の
  `SetLenOnDrop` (repr.rs:661-708) と同じ考え方。UTF-8 が途中で切れないよう、
  長さは `push_bytes` の単位でしか進めない。
- SAFETY 契約: `appender()` が `ensure_modifiable` と必要な `reserve` を済ませるので、
  `ptr` への書き込みは `as_mut_ptr` (repr.rs:844-864) と同じ根拠で成立する。
  `Appender` が生きている間に `repr` を触る経路が無いことは `&'a mut` が保証する。

適用先はこのタスクでは `Extend` 系と `FromIterator` 系に限る。
`fmt::Write::write_str` は単発の `push_str` なので現状のまま。

### 案2: `Extend<char>` だけを対象にした専用ループ

案1 の一般化が過剰だった場合の縮退先。実装は最小だが、`Extend<&str>` 系には効かない。

### 案3: pub API 化

見送り。ユーザが writer を直接使いたい動機は
[plans/12](./12-deferred.md) の `format_lean!` 系マクロを検討するときに評価する。
案1 の内部 API が安定すれば、そのときのバックエンドとして使える。

## 設計判断に必要な情報

- **grow 戦略**: commit してから `Repr::push_str` / `reserve` に任せる
  (amortized 1.5 倍、heap_buffer.rs:19-23) で十分か。独自の倍々戦略は不要のはず。
- **`appender(0)` の初期状態**: inline バッファのまま始めるとき、`cap` は 16 の定数、
  `ptr` は `self` 自身のアドレスでよい。static バッファは `appender()` の中で
  `ensure_modifiable` 相当が inline / heap へ変換する。
- **`Appender` が inline バッファを指すときの再解決**: grow で inline → heap へ移ると
  `ptr` が `self` の中から heap へ変わる。`push_bytes` の grow 経路で必ず取り直す形に
  すれば問題ないが、テストで inline → heap を跨ぐ extend を必ず通す。
- **エラーの扱い**: `Extend` は `Result` を返せないので、内部は `Result` で書いて
  `Extend` 側で `unwrap_with_msg` する (既存の `push` と同じ流儀)。

## `try_repeat` の先行対応

writer とは独立に、`LeanString::try_repeat` (lib.rs:818-831) は
「最終長を計算 → `Repr::new_with(total_len, |ptr| ...)` で 1 回確保して直接書く」
形に置き換えられる。`new_with` (repr.rs:136-151) は既にあるので、
このタスクの中で最初に片付けるとよい。

## 検証方針

- **テスト**: 既存の `Extend`/`FromIterator` テストに加えて、
  (1) inline → heap を跨ぐ extend、(2) grow を複数回跨ぐ extend、
  (3) 途中で panic したとき長さが `push_bytes` の境界に落ちること、を追加する。
  `tests/property.rs` に `collect::<LeanString>()` / `extend` と `String` の
  等価性プロパティがあるか確認し、無ければ足す。
- **Miri**: 未初期化の余剰容量へ生ポインタで書くので全ターゲット必須。
- **asm**: `fn collect_chars(it: impl Iterator<Item = char>) -> LeanString` 相当の
  ループ本体から、要素ごとの関数呼び出し・参照カウントのロード・判別子分岐が
  消えていること。
- **criterion**: 現状 bench に extend 系が無いので追加する
  (char 列の collect、短い `&str` 列の extend、`repeat`)。
  ベースラインを取ってから着手する。

## 依存

- [plans/03](./03-push-str-fast-path.md) — grow 経路で `push_str`/`reserve` を使うので、
  そちらの契約が固まってから。03 を見送った場合でも本タスクは成立する
  (grow は cold なので fast path 化の恩恵は薄い)。
