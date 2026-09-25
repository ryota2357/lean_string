# 06: ドキュメントの誤りと抜け

種別: doc / 実装量: 小 (項目ごと) / 設計判断: 小

どれも rustdoc (と `#![doc = include_str!("../README.md")]`) に出る。機械的な修正が多いので、まとめて直すとよい。

## A. 事実と違う記述

### `clear` の容量

`LeanString::clear` の doc に

> If the [`LeanString`] is unique, this method will not change the capacity.

とあるが、unique な static バッファでは成り立たない。

```
unique static clear: cap 29 -> 0
```

static バッファの `capacity()` は今の長さを返すので、`clear()` で長さが 0 になれば容量も 0 になる。
この挙動自体は正しい (1 バイトでも追加すれば必ず確保が起きるので、「追加の確保なしで持てるバイト数」は 0)。
`capacity` / `with_capacity` / `shrink_to_fit` / `shrink_to` の doc にはこの例外が書いてあり、
`clear` だけ抜けている。

static バッファに対する `clear` の doctest も足して、この挙動を仕様として固定する。

### `try_shrink_to_fit` / `try_shrink_to` / `try_pop` の失敗条件

この 3 つの doc は「`capacity` が大きすぎると失敗する」と書いているが、どれも容量を増やさないので、
実際に失敗するのは確保に失敗したときだけ。`try_reserve` の doc から写したものと思われる。

### README

- サンプルの `"This is a not long but can't store inlined"` は英語として不自然で、
  直前のコメント「More than 16 bytes」とも合っていない。実際の文字列は 42 バイト。
- 特徴の箇条書きのうち「Strings larger than 16 bytes are stored on the heap」だけ 32-bit の注記が無い。
  上の 2 つには付いている。

## B. panic するのに `# Panics` が無い関数

どれも `unwrap_with_msg` を通して、確保に失敗すると panic する。

| 対象 | 備考 |
| --- | --- |
| `from_utf8` / `from_utf8_lossy` (両型) | |
| `from_utf16` / `from_utf16_lossy` (両型) | |
| `from_utf16le` / `_lossy` / `from_utf16be` / `_lossy` (両型) | `Result` を返すのに、OOM では panic する。いちばん紛らわしい |
| `From<&str>` / `<String>` / `<&String>` / `<Box<str>>` / `<Cow>` / `<char>` | doc コメント自体が無い |
| `From<LeanString> for LeanStr` / `From<LeanStr> for LeanString` | |
| `Clone` (両型) | 参照カウントが `isize::MAX` を超えると "reference count overflow" で panic する。「O(1) で確保は起きない」とは書いてあるが panic の記述が無い |

`from_utf16*` の 8 つは特に問題で、lone surrogate なら `Err`、OOM なら panic と扱いが分かれている。
このクレートは他の操作にはすべて `try_` 版を用意しているのに、デコード系には無い。
`# Panics` を書くだけにするか、`try_from_utf16*` を足すかは [07](./07-api-gaps.md) の D と合わせて決める。

## C. `#[track_caller]` の付け方が揃っていない

`#[track_caller]` が付いているのは 12 個の `From` impl と `unwrap_with_msg` だけ。
panic しうる inherent メソッド (`with_capacity` / `reserve` / `push` / `push_str` / `insert` /
`insert_str` / `remove` / `retain` / `truncate` / `split_off` / `shrink_to*` / `pop` / `repeat` /
`to_*case` / `make_ascii_*case` / `into_lean_str` / `into_lean_string`) には付いていない。

そのため OOM で panic したとき、表示される位置が利用者のコードではなく `src/lib.rs` の中になる。
機械的に足せる。

## D. あえて提供していない API の理由が書かれていない

`DerefMut` / `AsMut<str>` / `BorrowMut<str>` / `IndexMut` / `into_bytes` / `as_mut_vec` /
`from_raw_parts` / `leak` は、CoW を壊すか、失敗しうる操作を失敗しない形で公開することになるので入れていない。
その理由がどこにも書かれておらず、利用者にはコンパイルエラーしか見えない。

crate レベルの doc に「提供していない `String` の API とその理由」という節を足す。あわせて次の 2 点も書く。

- `From<String>` / `From<Box<str>>` は O(n) であること。heap のレイアウトがデータの前に
  参照カウントのヘッダを置くので、バッファを引き継げない ([09](./09-mutability-conversion.md) の E)。
- `&LeanString` は `str::split` などの `Pattern` として使えないこと。
  `core::str::pattern::Pattern` が unstable なので実装できない。`s.split(lean.as_str())` と書けばよい。

## E. 表記の揺れと誤字

| 場所 | 内容 |
| --- | --- |
| `as_static_str` (両型) | `even if if created with` と `if` が重複しており、末尾のピリオドも無い |
| `try_remove` / `try_retain` / `try_insert` / `try_insert_str` / `try_truncate` / `try_to_ascii_*case` (両型) / `try_make_ascii_*case` の 11 か所 | `but return an [\`ReserveError\`]` は動詞と冠詞が誤り。`try_with_capacity` などは `but returns a` で正しい |
| `try_with_capacity` | `out of memory`。他はすべて `out-of-memory` |
| `LeanString::is_empty` | `has a length of 0, \`false\` otherwise` で末尾のピリオドが無い。`LeanStr` 側は `length of zero` + ピリオドで、表記が揃っていない |
| `LeanString::clear` の doctest | "This is **a** example of" が 2 か所 |

## 検証

- `cargo test --doc` が通ること。A の修正には doctest を付ける。
- `RUSTDOCFLAGS="-D warnings" cargo doc --all-features` が通ること。
- C を入れたら、panic するテストでバックトレースの位置が呼び出し側になることを 1 つだけ手で確認する。

## 依存

なし。
