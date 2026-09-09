# 09: ドキュメントの誤りと欠落

種別: doc / 実装量: 小 (項目ごと) / 設計判断: 小

すべて `#![doc = include_str!("../README.md")]` と rustdoc に出る。
機械的なものが多いので、まとめて片付けるとよい。

## A. 事実として誤っている記述

### `clear` の容量に関する記述

lib.rs:1115 に

> If the [`LeanString`] is unique, this method will not change the capacity.

とあるが、unique な static バッファでは成り立たない。実測:

```
unique static clear: cap 29 -> 0
```

`capacity()` は static バッファに対して現在の長さを返す (repr.rs:427-437) ので、
`clear()` で長さが 0 になれば容量も 0 になる。この挙動自体は正しく
(1 バイトでも足せば必ず確保が起きるので、「追加の確保なしに保持できるバイト数」は 0)、
`capacity` / `with_capacity` / `shrink_to_fit` / `shrink_to` の doc は既にこの例外を
書いている。`clear` だけが取り残されている。

あわせて static バッファに対する `clear` の doctest を足して、挙動を仕様として固定する。

### `try_shrink_to_fit` / `try_shrink_to` / `try_pop` の失敗理由

lib.rs:643, 688, 758 が「`capacity` が大きすぎる場合に失敗する」と書いているが、
この 3 つはいずれも容量を増やさないので、実際に失敗しうるのは確保失敗だけ。
`try_reserve` の記述からのコピーと思われる。

### README の記述

- README.md:43 のサンプル中のコメント `"This is a not long but can't store inlined"` は
  英語として崩れており、直前の「More than 16 bytes」というコメントとも噛み合っていない。
  実際の文字列は 42 バイト。
- README.md:15-17 の 3 つ目の箇条書き「Strings larger than 16 bytes are stored on the heap」に
  32-bit の但し書きが無い。1 つ目と 2 つ目には付いている。

## B. `# Panics` が無い panic しうる関数

いずれも `unwrap_with_msg` (lib.rs:2551) 経由で確保失敗時に panic する。

| 対象 | 備考 |
| --- | --- |
| `from_utf8` / `from_utf8_lossy` (lib.rs:166/187 と `LeanStr` 側 1397/1418) | |
| `from_utf16` / `from_utf16_lossy` (lib.rs:234/266、1457/1474) | |
| `from_utf16le` / `_lossy` / `from_utf16be` / `_lossy` (lib.rs:306-410、1505-1582) | **`Result` を返すのに OOM では panic する**。もっとも紛らわしい |
| `From<&str>` / `<String>` / `<&String>` / `<Box<str>>` / `<Cow>` / `<char>` (lib.rs:2104-2200) | doc コメント自体が無い |
| `From<LeanString> for LeanStr` / `From<LeanStr> for LeanString` (lib.rs:2244/2252) | |
| `Clone` (lib.rs:1272、1760) | 参照カウントが `isize::MAX` を超えると "reference count overflow" で panic する (repr.rs:270)。「O(1) で確保は起きない」とは書いてあるが panic の記述が無い |

`from_utf16*` の 8 つはとくに問題で、「lone surrogate なら `Err`、OOM なら panic」という
非対称になっている。このクレートは他のすべてに `try_` 版を用意しているのに、
デコード系には `try_` 版が無い。`# Panics` を書くだけで済ませるか、
`try_from_utf16*` を足すかは [plans/10](./10-api-gaps.md) の判断と合わせる。

## C. `#[track_caller]` の非対称

現状 `#[track_caller]` が付いているのは 12 個の `From` impl (lib.rs:2106-2254) と
`unwrap_with_msg` だけ。panic しうる inherent メソッド
(`with_capacity` / `reserve` / `push` / `push_str` / `insert` / `insert_str` /
`remove` / `retain` / `truncate` / `split_off` / `shrink_to*` / `pop` / `repeat` /
`into_lean_str` / `into_lean_string`) には付いていない。

そのため OOM で panic したとき、利用者のコードではなく `src/lib.rs:783` を指す。
機械的に足せる。

## D. 意図的に提供していない API の理由が書かれていない

`DerefMut` / `AsMut<str>` / `BorrowMut<str>` / `IndexMut` / `into_bytes` /
`as_mut_vec` / `from_raw_parts` / `leak` はいずれも CoW を壊すか、
失敗しうる操作を infallible な形で公開することになるので入れていない。
その理由がどこにも書かれておらず、利用者はコンパイルエラーだけを見ることになる。

crate レベルの doc に「提供していない `String` の API とその理由」という節を足す。
あわせて次の 2 点も書く。

- `From<String>` / `From<Box<str>>` が O(n) であること。ヒープレイアウトが
  データの前に参照カウントのヘッダを置くので、バッファを奪えない
  ([plans/12](./12-mutability-conversion.md) の E)。
- `&LeanString` が `str::split` などの `Pattern` として使えないこと。
  `core::str::pattern::Pattern` が unstable なので実装できない。
  `s.split(lean.as_str())` と書けばよい。

## E. 表記ゆれ・誤字

| 場所 | 内容 |
| --- | --- |
| lib.rs:1186、1678 (`as_static_str` × 2) | `even if if created with` — `if` が重複、末尾のピリオドも無い |
| lib.rs の 11 か所 (589, 643, 688, 719, 758, 789, 843, 883, 923, 963, 1055) | `but return an [\`ReserveError\`]` — 動詞の一致と冠詞が誤り。134, 1004, 1098 は `but returns a` で正しい |
| lib.rs:133 | `out of memory` — 他はすべて `out-of-memory` |
| lib.rs:444 | `has a length of 0, \`false\` otherwise` — 末尾のピリオドが無い。`LeanStr` 側 (1635) は `length of zero` + ピリオドで揃っていない |
| lib.rs:1124、1137 | doctest の文字列が "This is **a** example of" (2 か所) |

## F. `CHANGELOG` と MSRV

- `CHANGELOG.md` に `[Unreleased]` の節が無い。v0.7.0 以降の 8 コミットのうち
  5 つが利用者から見える機能追加 (`FromIterator` の追加、`#[must_use]`、
  `AsRef<Path>`、`as_static_str`、`from_utf16{le,be}{,_lossy}`) だが記録されていない。
- `Cargo.toml` の `rust-version` が 1.85.1 から 1.88.0 に上がっている
  (`<[T]>::as_chunks` を使うため) が、CHANGELOG に記載が無い。
  README の MSRV バッジは値だけが変わり、理由が分からない。

## G. `arbitrary` feature が暗黙

`Cargo.toml` は `std` と `serde` を `[features]` に明示しているが、`arbitrary` は
optional dependency の暗黙 feature としてしか存在しない。動作はするが、
docs.rs の feature 一覧に説明が出ない。

## 検証方針

- `cargo test --doc` で doctest が通ること。A の修正には doctest を伴わせる。
- `RUSTDOCFLAGS="-D warnings" cargo doc --all-features` が通ること。
- C を入れたら、panic するテストのバックトレース位置が呼び出し側になることを
  1 つだけ手で確認する。

## 依存

なし。他のどのタスクとも衝突しない。
