# 10: API の欠落を埋める

種別: API 追加 / 実装量: 小〜中 (項目ごと) / 設計判断: 中 (どこまで揃えるか)

`alloc::String` / `str` と比べて欠けているものを棚卸しした。すべてを入れる必要は無いが、
「非対称になっているもの」は先に埋めたい。

## A. 非対称になっているもの (優先)

### A-1. `FromIterator` の欠落

`Extend` はあるのに `FromIterator` が無い組み合わせがある。

| ある | 無い |
| --- | --- |
| `Extend<LeanStr> for LeanString` (lib.rs:2063) | `FromIterator<LeanStr> for LeanString` |
| `Extend<LeanString> for String` (lib.rs:2071) | `FromIterator<LeanString> for String` |
| `Extend<LeanStr> for String` (lib.rs:2079) | `FromIterator<LeanStr> for String` |

`LeanString` 側は `.collect()` が使えないという素直な不便。コンパイルエラーで確認済み:

```
error[E0277]: a value of type `LeanString` cannot be built from an iterator over elements of type `LeanStr`
   = help: the trait `FromIterator<LeanStr>` is not implemented for `LeanString`
```

`FromIterator<LeanString> for LeanStr` も同様に無い
(`FromIterator<LeanStr> for LeanStr` と `FromIterator<LeanString> for LeanString` はある)。

`String` 向けの 2 つは他クレートの型に対する impl になるが、
`String` は `alloc` の型なので orphan rule 上は問題ない。
入れるかどうかは「`String` への変換をどこまで面倒見るか」の方針次第。

### A-2. `From<LeanString>` の欠落

`From<Cow<'_, str>> for LeanString` (lib.rs:1789) はあるが逆が無い。
`String` が持っていて対応が無いもの:

- `From<LeanString> for Cow<'_, str>`
- `From<LeanString> for Box<str>`
- `From<LeanString> for Vec<u8>`
- `From<LeanString> for Rc<str>` / `Arc<str>`
- `From<&mut str> for LeanString`

### A-3. `Extend<&LeanString>` / `Extend<&LeanStr>`

所有した値からは extend できるが借用からはできない。
`Extend<&'a char>` と `Extend<&'a str>` はあるので、ここも非対称。

## B. `String` にあって無いメソッド

| 欠落 | 所感 |
| --- | --- |
| `reserve_exact` / `try_reserve_exact` | **もっとも目立つ欠落**。`HeapBuffer::with_exact_capacity` と `realloc` は既にあり、`Repr::reserve` が `amortized_growth` しか使っていないだけ。ぴったりの確保を要求する手段が無い |
| `as_mut_str` (fallible 版も) | `ensure_modifiable()` してから `as_mut_ptr`/`len` で `&mut str` を作る。機構は揃っている。CoW なので `try_as_mut_str` を主にし、panic 版を併設する形になる |
| `into_boxed_str` | `Box::from(self.as_str())` で 1 行 |
| `AsRef<Path>` (`feature = "std"`) | `AsRef<OsStr>` (lib.rs:1495) はある。`File::open(s)` が通るようになる |
| `drain(range)` | `Drain` ガード型が要るので実装量は中 |
| `replace_range(range, &str)` | 同上。`tests/alloc_string.rs` の該当ブロックがコメントアウトされたままなのは、この欠落が理由と思われる |
| `from_utf16le` / `from_utf16le_lossy` / `from_utf16be` / `from_utf16be_lossy` | std で 1.83 から stable |

意図的に入れないもの (doc に理由を 1 行書いておくとよい): `into_bytes`、`as_mut_vec`、
`from_raw_parts`、`leak`、`DerefMut` / `AsMut<str>` / `BorrowMut<str>`。
いずれも CoW を壊すか、失敗しうる操作を infallible な形で公開することになる。

## C. `LeanStr` 側の欠落

- **static バッファかどうかを問う手段が無い**。`is_heap_allocated()` はあるが
  `is_static()` / `as_static_str()` に相当するものが無い。static を持てることは
  `LeanStr` の主な売りなので、それを確認・取り出せないのは惜しい。
  `as_static_str(&self) -> Option<&'static str>` の形が素直。
- `LeanStr::repeat` が無い。
- `impl From<&str> for LeanStr` は `unwrap_with_msg` 経由で panic しうるが、
  `# Panics` の記述が無い。`LeanString::from_static_str` は長さ上限を書いているので、
  ここも揃えたい。

## D. `#[must_use]`

現状 `split_off` / `try_split_off` にしか付いていない。std で `#[must_use]` が付いている
対応物: `as_str`、`as_bytes`、`len`、`is_empty`、`capacity`、`repeat`。
lean_string 固有では `is_heap_allocated`、`into_lean_str`、`into_lean_string` も該当する。

## E. `str` のメソッドが `String` を返す件

`to_uppercase` / `to_lowercase` / `replace` / `repeat` などは `Deref<Target = str>` 経由で
呼べるが `String` を返す。`LeanString` を返す版を用意するかは方針判断
(`compact_str` は用意している)。`LeanString::repeat` は既にあるので、
残りを揃えるかどうかという話になる。優先度は低い。

## 進め方

1 つの PR にまとめる必要はない。次の順で片付けるのがよさそう。

1. **A-1 (FromIterator の欠落)** — 既存の `Extend` に委譲するだけで、
   非対称の解消として効果が分かりやすい。
2. **B の `reserve_exact` / `try_reserve_exact`** — 機構が揃っており、
   欠落として目立つ。`Repr::reserve` に「amortize するか否か」の引数を足すか、
   `reserve_exact` 用の経路を分けるかの設計判断が要る。
3. **C の `as_static_str`** — `LeanStr` の価値に直結する。
4. **D (`#[must_use]`)** — 機械的。
5. 残りは需要が出たときに。

各項目に対応するテストと doctest を足すこと。`tests/alloc_string.rs` に
`String` のテストから移植できるものが多いはず (コメントアウトされたブロックが
そのまま候補になる)。
