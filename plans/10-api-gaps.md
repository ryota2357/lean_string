# 10: API の欠落を埋める

種別: API 追加 / 実装量: 小〜中 (項目ごと) / 設計判断: 中 (どこまで揃えるか)

`alloc::String` / `str` / `Arc<str>` と比べて欠けているものを棚卸しした。
すべてを入れる必要は無いが、「非対称になっているもの」は先に埋めたい。

このクレートには impl を生成するマクロが無く、`PartialEq` の 20 impl も
`From` の 20 impl もすべて手書きで並んでいる。1 つ足すと 2〜4 ブロック書くことになる。

## A. 非対称になっているもの (優先)

### A-1. `LeanStr::repeat` が無い

`LeanString::repeat` (lib.rs:998) は `Self` を返すが、`LeanStr` には無い。
`LeanStr` に対して `.repeat(n)` と書くと `Deref` 経由で `str::repeat` が呼ばれ、
**コンパイルが通って `String` が返る**。呼び出し側からは見えない非対称で、
この種の欠落としてはもっとも質が悪い。

### A-2. `Extend<&LeanString>` / `Extend<&LeanStr>` が無い

所有した値からは extend できるが借用からはできない。
`Extend<&char>` (lib.rs:2438) と `Extend<&str>` (lib.rs:2444) はあるので非対称。
`LeanString` 側と `String` 側の両方。

### A-3. 参照からの `From` の欠落

`From<&LeanString> for LeanString` (lib.rs:2202)、`From<&LeanStr> for LeanStr` (2209)、
`From<&LeanString> for String` (2230)、`From<&LeanStr> for String` (2237) はあるが、
`From<&LeanStr> for LeanString` と `From<&LeanString> for LeanStr` が無い。

### A-4. `FromIterator` の欠落は無い

念のため確認した結果、8 種類の所有アイテム型 (`char` / `&char` / `&str` /
`Box<str>` / `Cow<str>` / `String` / `LeanString` / `LeanStr`) × `{LeanString, LeanStr}` は
すべて揃っており、`{LeanString, LeanStr} → String` もある。
`&String` からの `FromIterator` は std にも無いので欠落ではない。

### A-5. `#[must_use]` はほぼ完全

33 か所に付いており、値を返して副作用の無い inherent メソッドは網羅されている。
残る候補は `ToLeanString::to_lean_string` (traits.rs:18) と
`ToLeanStr::to_lean_str` (traits.rs:99) の 2 つだが、std の `ToString::to_string` にも
付いていないので、揃えないという判断もありうる。

## B. `String` にあって無いメソッド

| 欠落 | 所感 |
| --- | --- |
| `reserve_exact` / `try_reserve_exact` | もっとも目立つ欠落。`HeapBuffer::with_exact_capacity` (heap_buffer.rs:336) と `realloc` は既にあり、`Repr::reserve` (repr.rs:446) が `amortized_growth` しか使っていないだけ |
| `as_mut_str` / `try_as_mut_str` | `ensure_modifiable()` (repr.rs:831) してから `as_mut_ptr` (repr.rs:862) で `&mut str` を作る。機構は揃っている。CoW なので失敗しうる。`try_as_mut_str` を主にして panic 版を併設する形になる |
| `into_boxed_str` | `Box::from(self.as_str())` で 1 行 |
| `drain(range)` | `Drain` ガード型が要るので実装量は中。`tests/alloc_string.rs` にコメントアウトされた `test_drain*` がある |
| `replace_range(range, &str)` | 同上。`test_replace_range*` が 12 個コメントアウトされている |
| `extend_from_within(range)` | std で 1.87 から stable |
| `String::from_utf8(Vec<u8>)` + `FromUtf8Error` | lean_string の `from_utf8` は `&[u8]` を取り `core::str::Utf8Error` を返す。意図的な差だが、その旨がどこにも書かれていない |

意図的に入れないもの: `into_bytes`、`as_mut_vec`、`from_raw_parts`、`into_raw_parts`、
`leak`、`DerefMut` / `AsMut<str>` / `BorrowMut<str>` / `IndexMut`。
理由を doc に書く件は [plans/09](./09-doc-fixes.md) の D。

## C. `From<LeanString>` / `From<LeanStr>` の欠落

`String` が持っていて対応が無いもの。いずれも中身のコピーになるが機械的。

- `for Cow<'_, str>`
- `for Box<str>`
- `for Vec<u8>`
- `for Rc<str>` / `Arc<str>`
- 逆向きの `From<&mut str> for LeanString` / `LeanStr` (std は `From<&mut str> for String` を持つ)

## D. `try_` 版の欠落

デコード系の 8 つ (`from_utf8` / `from_utf8_lossy` / `from_utf16` / `from_utf16_lossy` /
`from_utf16le` / `_lossy` / `from_utf16be` / `_lossy`、それぞれ両型) は
確保失敗で panic する。このクレートは他のすべてに `try_` 版を用意しているので、
ここだけ非対称になっている。

`from_utf16*` は `Result` を返す関数なので、とくに紛らわしい
(lone surrogate なら `Err`、OOM なら panic)。

選択肢:

- 案A: `# Panics` を書くだけ ([plans/09](./09-doc-fixes.md) の B)。実装量ゼロ。
- 案B: `try_from_utf8` などを足す。エラー型を `Utf8Error` と `ReserveError` の
  どちらも表せるものにする必要があり、`ToLeanStringError` と同じ形の enum を
  8 つぶん (実際には 3 種類ほど) 用意することになる。
- 案C: 既存の `FromUtf16Error` に `Reserve` の variant を足し、
  `from_utf16*` だけ「OOM も `Err` で返す」に変える。破壊的変更ではない
  (`FromUtf16Error` は non-exhaustive でない enum ではなく構造体なので、
  内部の `kind` を増やすだけで済む)。

案C を推す。`from_utf16*` の非対称がいちばん目立ち、
`FromUtf16Error` の形がそのまま使える。`from_utf8` 系は入力を検証するだけで
確保のタイミングが素直なので、案A で十分。

## E. `LeanStr` 側の欠落

- `LeanStr::repeat` (A-1)。
- 共有されているかどうかを問う手段が無い。`Repr::is_unique` (repr.rs:231) は
  `pub(crate)`。CoW 型として「いま共有されているか」を聞けないのは概念的な欠落だが、
  公開すると「聞いた直後に共有されうる」問題があるので設計判断が要る
  (`ecow` は `is_unique` を `&mut self` にすることでこれを塞いでいる)。
- `capacity` / `with_capacity` は `Box<str>` / `Arc<str>` にも無いので欠落ではない。

## F. `str` のメソッドが `String` を返す件

`to_uppercase` / `to_lowercase` / `to_ascii_uppercase` / `to_ascii_lowercase` /
`replace` / `replacen` は `Deref<Target = str>` 経由で呼べるが `String` を返す。
`[LeanString]::concat()` / `join()` も `Borrow<str>` 経由で `String` を返す。

`LeanString` を返す版を用意するかは方針判断。`compact_str` と `ecow` は用意している。
`join` / `concat` は [plans/14](./14-deferred.md) に別項がある。優先度は低い。

## G. 効率の問題

### G-1. `FromIterator<&str>` が長さを見積もらない

```
LeanString::from_iter(["a", "b", "c", "d"]):  alloc=1 realloc=2
String::from_iter(["a", "b", "c", "d"]):      alloc=1 realloc=0
```

std には `SpecExtendStr for [&str; N]` があり、長さを合計してから 1 回確保する。
lean_string の `FromIterator<&str>` (lib.rs:2311) は `LeanString::new()` から
`push_str` を繰り返すだけ。`Extend<&str>` も同じ。

[plans/04](./04-append-writer.md) の writer が入れば `size_hint` を活かせるが、
`&str` のイテレータでは `size_hint` は要素数であって長さではない。
長さの合計を先に求める形 ([plans/14](./14-deferred.md) の `join`/`concat` と同じ機構) が要る。

### G-2. `from_utf8_lossy` に全 valid の fast path が無い

lib.rs:187 と 1418 はどちらも `with_capacity` してから `utf8_chunks` のループに入る。
先頭に

```rust
if let Ok(s) = core::str::from_utf8(buf) { return Self::from(s); }
```

を置けば、正しい UTF-8 (実際の入力の大半) が `InlineBuffer::new` のレジスタ経路か
1 回のサイズ指定コピーで済む。検証のコストは変わらない。

### G-3. `try_repeat` が n 回のループ

lib.rs:1008 は容量をぴったり確保したあと `try_push_str` を n 回呼ぶ。
再確保は起きないが、std の `str::repeat` は倍々コピーを使う。
[plans/04](./04-append-writer.md) で `Repr::new_with` に置き換える案がある。

## 進め方

1. A-1 (`LeanStr::repeat`) — 黙って `String` を返す状態がいちばん危険。
2. A-2 / A-3 (`Extend<&_>` と参照からの `From`) — 既存の impl に委譲するだけ。
3. C (`From<LeanString> for Box<str>` ほか) — 機械的。
4. B の `reserve_exact` / `try_reserve_exact` — 機構が揃っている。
   `Repr::reserve` に「amortize するか否か」の引数を足すか、
   `reserve_exact` 用の経路を分けるかの設計判断が要る。
5. D (`try_` 版 / `# Panics`) — 案C を採るなら `from_utf16*` から。
6. G-2 (`from_utf8_lossy` の fast path) — 小さく効果が読める。
7. 残り (`as_mut_str` / `drain` / `replace_range` / F) は需要が出たときに。
   `drain` と `replace_range` は `tests/alloc_string.rs` に移植済みのテストが
   コメントアウトされたまま置いてある。

各項目に対応するテストと doctest を足すこと。
