# 03: 連続した追記のための内部 writer

種別: perf + 内部 API / 実装量: 中 / 設計判断: 中 (API の形)

## 背景

`Extend` と `FromIterator` は要素ごとに `push` / `push_str` を呼ぶ。
[research/codegen-baseline.md](../research/codegen-baseline.md) の §4 のとおり、
`Extend<char>` のループでは 1 文字ごとに次の処理が走る。

- `Repr::push_str` の呼び出し (インライン展開されない)
- 長さの復元とオーバーフロー判定
- 判別子による分岐
- 参照カウントのロード (heap のとき。aarch64 では `ldar`)
- 容量の比較
- 1〜4 バイトのための `memcpy` 呼び出し
- `set_len` での判別子による分岐

書き込み先のバッファは最初に 1 回決めれば済むので、これらはループの外に出せる。

同じ作りになっている箇所:

| 場所 | 今の実装 |
| --- | --- |
| `Extend<char>` | `try_reserve(size_hint)` の後、1 文字ずつ `push` |
| `Extend<&char>` | `Extend<char>` に委譲 |
| `Extend<&str>` / `Box<str>` / `Cow` / `String` / `LeanString` / `LeanStr` | 1 要素ずつ `push_str`。`size_hint` は見ない |
| `FromIterator<char>` | `try_with_capacity(size_hint)` の後、1 文字ずつ `push_str` |
| `FromIterator<&str>` ほか | `Extend` に委譲 |
| `LeanString::from_utf16*` (`from_utf16_units` / `from_utf16_units_lossy`) | `with_capacity` の後、1 文字ずつ `push` |
| `LeanString::from_utf8_lossy` | `with_capacity` の後、チャンクごとに `push_str` と `push` |
| `to_lowercase` / `to_uppercase` (両型) の非 ASCII 部分 | `c.to_lowercase().for_each(\|l\| mapped.push(l))` |

`Extend<LeanString>` / `Extend<LeanStr>` for `String` も同じ形だが、`String` 側の処理なので対象外。

`from_utf16` 系は UTF-16 のコード単位数を容量として確保する。BMP の非 ASCII 文字は
UTF-8 で 1 単位あたり 3 バイトになるので、非 ASCII の入力では容量が足りなくなる。
`String::from_utf16` も同じ見積もりをしているので std と揃ってはいるが、
writer があれば再確保の回数は減らせる。日本語 21 文字 (UTF-8 で 63 バイト) で数えると:

```
LeanString::from_utf16: alloc=1 realloc=3
String::from_utf16:     alloc=1 realloc=2
```

## 方針

### 案1 (推奨): 内部用の `Appender`

書き込み先を 1 回だけ決め、ポインタと長さを手元に持って書き進める writer を作る。

```rust
// repr.rs (crate 内部専用)
struct Appender<'a> {
    repr: &'a mut Repr<Mutable>,
    ptr: *mut u8,   // 書き込み先のデータポインタ
    cap: usize,
    len: usize,     // 書き進めた長さ (commit するまで repr には書かない)
}

impl Repr<Mutable> {
    /// `size_hint` バイト分の容量を確保し (0 なら確保しない)、書き込める状態にした
    /// `Appender` を返す。
    fn appender(&mut self, size_hint: usize) -> Result<Appender<'_>, ReserveError>;
}

impl Appender<'_> {
    /// `bytes` は UTF-8 として完結した単位 (char 1 文字のエンコード結果か &str)。
    /// 容量が足りなければ commit → grow → ポインタの取り直し。
    #[inline]
    fn push_bytes(&mut self, bytes: &[u8]) -> Result<(), ReserveError>;
}

impl Drop for Appender<'_> {
    // set_len(self.len) で commit する
    fn drop(&mut self);
}
```

- `push_bytes` の fast path は「容量の比較 + コピー + 長さの加算」だけになる。
  長さの復元、判別子の分岐、参照カウントのロードは `appender()` で 1 回だけ行う。
- grow するときは一度 commit してから `reserve` を呼び、ポインタと容量を取り直す。
  めったに通らないので `#[cold]` にする。
- panic 安全性: イテレータの `next()` やユーザのクロージャが panic しても、`Drop` で `set_len` が
  走るので、それまでに書いた分で有効な状態に戻る。`retain` の `SetLenOnDrop` と同じ考え方。
  長さは `push_bytes` の単位でしか進めないので、UTF-8 の途中で切れることはない。
- SAFETY の根拠: `appender()` の中で `ensure_modifiable` と必要な `reserve` を済ませるので、
  `ptr` への書き込みは `as_mut_ptr` と同じ理由で正しい。`Appender` が生きている間に
  他から `repr` を触れないことは `&'a mut` で保証される。

このタスクで適用するのは上の表の箇所まで。`fmt::Write::write_str` は 1 回の `push_str` なのでそのままにする。

### 案2: `Extend<char>` だけ専用のループにする

案1 が大げさだと分かったときの代わり。実装は小さいが、`Extend<&str>` 系には効かない。

### 案3: pub API にする

今回はやらない。ユーザが writer を直接使いたい場面は、[11](./11-deferred.md) の
`format_lean!` 系マクロを検討するときに考える。案1 の内部 API が固まれば、そのマクロの実装に使える。

## 決めておくこと

- grow の戦略: commit してから `Repr::push_str` / `reserve` に任せる (1.5 倍の amortized growth)
  で足りるはず。writer 独自の倍々戦略は要らない。
- `appender(0)` の初期状態: inline のまま始める場合、`cap` は定数 16、`ptr` は `self` のアドレス。
  static バッファは `appender()` の中で `ensure_modifiable` 相当の処理が inline / heap に変換する。
- inline から heap に移るとき、`ptr` は `self` の中から heap に変わる。`push_bytes` の grow 経路で
  必ず取り直すようにすれば問題ないが、inline から heap をまたぐ extend はテストで必ず通す。
- エラーの扱い: `Extend` は `Result` を返せないので、内部は `Result` で書き、
  `Extend` 側で `unwrap_with_msg` する (今の `push` と同じ)。

## 検証

- テスト: 既存の `Extend` / `FromIterator` のテストに加えて、
  (1) inline から heap をまたぐ extend、(2) grow を何度もまたぐ extend、
  (3) 途中で panic したとき長さが `push_bytes` の境界に戻ること、を足す。
  `tests/property.rs` には `collect::<LeanString>()` のプロパティ (`collect_from_chars`) があるが、
  `extend` 側には無いので足す。
- Miri: 未初期化の領域に生ポインタで書くので 4 ターゲットすべて。
- asm: `fn collect_chars(it: impl Iterator<Item = char>) -> LeanString` のような関数のループから、
  要素ごとの関数呼び出し、参照カウントのロード、判別子の分岐が消えていること。
- criterion: 今は extend 系のベンチが無いので足す
  (char 列の collect、短い `&str` 列の extend、`from_utf16`、非 ASCII を含む `to_lowercase`)。
  先に今の数字を取っておく。
- 確保回数: `from_utf16` の realloc が減ること。確保を数えるグローバルアロケータで測れる
  (`tests/out_of_memory.rs` に似た仕組みがある)。

## 依存

- [02](./02-push-str-fast-path.md): grow 経路で `push_str` / `reserve` を使うので、そちらが決まってから。
  02 を見送ってもこのタスクは成り立つ (grow はめったに通らないので、fast path 化の影響は小さい)。
