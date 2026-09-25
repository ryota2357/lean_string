# 07: 足りない API を埋める

種別: API 追加 / 実装量: 小〜中 (項目ごと) / 設計判断: 中 (どこまで揃えるか)

`alloc::String` / `str` / `Arc<str>` と比べて足りないものを洗い出した。
全部入れる必要はないが、片方にだけあるものは先に埋めたい。

このクレートには impl を生成するマクロが無く、`PartialEq` の 20 impl も `From` の 20 impl も
すべて手で書いている。1 つ足すごとに 2〜4 ブロック書くことになる。

## A. 対になっていないもの (優先)

### A-1. `Extend<&LeanString>` / `Extend<&LeanStr>` が無い

所有した値からは extend できるが、参照からはできない。`Extend<&char>` と `Extend<&str>` はあるので揃っていない。
`LeanString` 向けと `String` 向けの両方。

### A-2. 参照からの `From` が一部無い

`From<&LeanString> for LeanString`、`From<&LeanStr> for LeanStr`、`From<&LeanString> for String`、
`From<&LeanStr> for String` はあるが、`From<&LeanStr> for LeanString` と `From<&LeanString> for LeanStr` が無い。

### 確認して問題なかったもの

- `FromIterator`: 所有したアイテム型 8 種 (`char` / `&char` / `&str` / `Box<str>` / `Cow<str>` /
  `String` / `LeanString` / `LeanStr`) × `{LeanString, LeanStr}` はすべて揃っていて、
  `{LeanString, LeanStr} → String` もある。`&String` からの `FromIterator` は std にも無い。
- `#[must_use]`: 値を返して副作用の無い inherent メソッドにはすべて付いている。
  `ToLeanString::to_lean_string` と `ToLeanStr::to_lean_str` には無いが、std の `ToString::to_string` にも
  付いていないので、揃えなくてもよい。
- `repeat` / `to_lowercase` / `to_uppercase` / `to_ascii_*case` は両方の型にあり、
  どちらも自分の型を返す。

## B. `String` にあって無いメソッド

| メソッド | メモ |
| --- | --- |
| `reserve_exact` / `try_reserve_exact` | いちばん目立つ。`HeapBuffer::with_exact_capacity` と `realloc` はすでにあり、`Repr::reserve` が `amortized_growth` しか使っていないだけ |
| `as_mut_str` / `try_as_mut_str` | `ensure_modifiable()` の後に `as_mut_ptr` から `&mut str` を作ればよく、必要な部品は揃っている。CoW なので失敗しうる。`try_as_mut_str` を基本にして panic 版も用意する形になる |
| `into_boxed_str` | `Box::from(self.as_str())` の 1 行 |
| `drain(range)` | `Drain` ガード型が必要なので実装量は中くらい。`tests/alloc_string.rs` に `test_drain*` がコメントアウトされて残っている |
| `replace_range(range, &str)` | 同上。`test_replace_range*` が 12 個コメントアウトされている |
| `extend_from_within(range)` | std では 1.87 で安定化 |
| `String::from_utf8(Vec<u8>)` + `FromUtf8Error` | lean_string の `from_utf8` は `&[u8]` を取って `core::str::Utf8Error` を返す。意図した違いだが、どこにも書かれていない |

入れないもの: `into_bytes`、`as_mut_vec`、`from_raw_parts`、`into_raw_parts`、`leak`、
`DerefMut` / `AsMut<str>` / `BorrowMut<str>` / `IndexMut`。理由を doc に書く件は [06](./06-doc-fixes.md) の D。

## C. `From<LeanString>` / `From<LeanStr>` が無い変換先

`String` にはあって、こちらに無いもの。どれも中身のコピーになるが、書くのは機械的。

- `Cow<'_, str>`
- `Box<str>`
- `Vec<u8>`
- `Rc<str>` / `Arc<str>`
- 逆向きの `From<&mut str> for LeanString` / `LeanStr` (std には `From<&mut str> for String` がある)

## D. デコード系に `try_` 版が無い

`from_utf8` / `from_utf8_lossy` / `from_utf16` / `from_utf16_lossy` / `from_utf16le` / `_lossy` /
`from_utf16be` / `_lossy` (両型) は確保に失敗すると panic する。このクレートは他のほとんどの操作に
`try_` 版を用意しているので、ここだけ揃っていない。

`from_utf16*` は `Result` を返すので特に紛らわしい (lone surrogate なら `Err`、OOM なら panic)。

選択肢:

- 案A: `# Panics` を書くだけ ([06](./06-doc-fixes.md) の B)。実装は要らない。
- 案B: `try_from_utf8` などを足す。エラー型は `Utf8Error` と `ReserveError` の両方を表せる必要があり、
  `ToLeanStringError` のような enum を 3 種類ほど用意することになる。
- 案C: 既存の `FromUtf16Error` に確保失敗の種類を足し、`from_utf16*` は OOM も `Err` で返すようにする。
  `FromUtf16Error` は中の `kind` が非公開の構造体なので、種類を増やしても破壊的変更にはならない。

案C がよい。`from_utf16*` の食い違いがいちばん目立ち、`FromUtf16Error` をそのまま使える。
`from_utf8` 系は入力を検証するだけで確保のタイミングも単純なので、案A で足りる。

`to_lowercase` / `to_uppercase` に `try_` 版が無いのは意図したもの
('Σ' の変換で `str::to_lowercase()` に頼っており、その確保失敗を `ReserveError` にできない。
コード中の NOTE に理由が書いてある)。

## E. `LeanStr` 側

- 共有されているかどうかを調べる手段が無い。`Repr::is_unique` は `pub(crate)`。
  CoW の型として「今共有されているか」を聞けないのは欠けていると言えるが、公開すると
  「聞いた直後に共有されうる」問題があるので、設計を考える必要がある
  (`ecow` は `is_unique` を `&mut self` にしてこれを防いでいる)。
- `capacity` / `with_capacity` は `Box<str>` / `Arc<str>` にも無いので、足りないものには含めない。

## F. `str` のメソッドが `String` を返す

`replace` / `replacen` は `Deref<Target = str>` 経由で呼べるが `String` を返す。
`[LeanString]::concat()` / `join()` も `Borrow<str>` 経由で `String` を返す。

`LeanString` を返す版を用意するかは方針次第。`compact_str` と `ecow` は用意している。
`join` / `concat` については [11](./11-deferred.md) に別の項目がある。優先度は低い。

## G. 効率の問題

### G-1. `FromIterator<&str>` が長さを見積もらない

```
LeanString::from_iter(["a", "b", "c", "d"]):  alloc=1 realloc=2
String::from_iter(["a", "b", "c", "d"]):      alloc=1 realloc=0
```

std は `[&str; N]` などに対して長さを合計してから 1 回だけ確保する。
lean_string の `FromIterator<&str>` は `LeanString::new()` から `push_str` を繰り返すだけで、`Extend<&str>` も同じ。

[03](./03-append-writer.md) の writer が入れば `size_hint` を使えるが、`&str` のイテレータの
`size_hint` は要素数であって長さではない。先に長さを合計する仕組みが必要で、
[11](./11-deferred.md) の `join` / `concat` と同じものになる。

### G-2. `from_utf8_lossy` に、全部正しい UTF-8 だったときの fast path が無い

`LeanString::from_utf8_lossy` は `with_capacity` の後に `utf8_chunks` のループに入る
(`LeanStr` 版はそれを呼んで `into_lean_str()` する)。先頭に

```rust
if let Ok(s) = core::str::from_utf8(buf) { return Self::from(s); }
```

を置けば、正しい UTF-8 (実際の入力のほとんど) は `InlineBuffer::new` のレジスタ経路か、
長さの決まったコピー 1 回で済む。検証のコストは変わらない。

## 進め方

1. A-1 / A-2 (`Extend<&_>` と参照からの `From`)。既存の impl に委譲するだけ。
2. C (`From<LeanString> for Box<str>` など)。機械的。
3. B の `reserve_exact` / `try_reserve_exact`。部品は揃っている。
   `Repr::reserve` に「amortize するかどうか」の引数を足すか、別の経路にするかを決める必要がある。
4. D (`try_` 版か `# Panics`)。案C なら `from_utf16*` から。
5. G-2 (`from_utf8_lossy` の fast path)。小さく、効果も見込める。
6. 残り (`as_mut_str` / `drain` / `replace_range` / F) は要望が出たら。
   `drain` と `replace_range` のテストは `tests/alloc_string.rs` にコメントアウトされた状態で移植済み。

各項目にはテストと doctest を付ける。
