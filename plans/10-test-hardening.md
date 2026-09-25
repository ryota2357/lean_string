# 10: テストと unsafe の契約の抜けを埋める

種別: テスト + doc / 実装量: 小〜中 / 設計判断: 小

## A. realloc の失敗がまったくテストされていない

`tests/out_of_memory.rs` には、次の確保・再確保を 1 回だけ失敗させる thread local のフラグがある。

```rust
static FAIL_NEXT_ALLOCATION: Cell<bool> = const { Cell::new(false) };
static FAIL_NEXT_REALLOCATION: Cell<bool> = const { Cell::new(false) };
```

ただ `FAIL_NEXT_REALLOCATION` を立てているのは `without_allocating` ヘルパだけで、これは
「再確保が起きなかったこと」を確かめるためのもの。再確保が失敗する経路を通るテストは 1 つも無い。

テストされていない経路:

| 場所 | 通る操作 | 内容 |
| --- | --- | --- |
| `HeapBuffer::into_exact` の失敗時の復元 | unique な heap の `LeanString` の `try_into_lean_str()` | このクレートでいちばん込み入った unsafe。`ptr::copy` でヘッダを上書きした後に元に戻し、長さの prefix を書き直して `GrowableHeader` を作り直す。間違っていてもその場では落ちず、壊れたバッファが残る |
| `HeapBuffer::into_growable` の失敗時 | unique な heap の `LeanStr` の `try_into_lean_string()` | 同様 |
| `Repr::reserve` の unique な heap の grow | `try_reserve` / `try_push_str` | `realloc` が失敗したとき |
| `Repr::shrink_to` のその場での縮小 | `try_shrink_to` | 同上 |

4 つとも公開 API から通せる。試しにテストを書いたところ、`cargo test` でも Miri
(`x86_64` と `i686`、strict provenance) でも通り、UB もリークも報告されなかった。

ただし、確認できることは操作によって違う。

`try_reserve` と `try_shrink_to` は `&mut self` を取るので、失敗した後の状態をそのまま調べられる。

```rust
// unique な heap、容量 20 に 18 バイト
FAIL_NEXT_REALLOCATION.set(true);
assert!(s.try_reserve(1000).is_err());
assert_eq!(s.as_ptr(), before);      // ポインタは変わらない
assert_eq!(s.capacity(), 20);        // 容量もそのまま
assert_eq!(s, "0123456789abcdefgh"); // 内容も無事
```

`GlobalAlloc::realloc` の「失敗したら元のブロックはそのまま」という契約が、このクレートの経路で
守られていることを端から端まで確かめるテストになる。

一方 `try_into_lean_str` / `try_into_lean_string` は `self` を値で取り、失敗したら `ReserveError` しか
返さない (中では `mem::replace` して失敗時に復元している)。呼び出し側からは文字列の中身を調べられず、
確認できるのは「1 回だけ drop されてリークしない」ことだけ。いちばん込み入った unsafe が、
いちばん確かめにくい場所にある。

- まずは Miri のリーク検出と、realloc が実際に失敗したこと (フラグが消費されたこと) で固定する。
- [09](./09-mutability-conversion.md) の A で `&mut self` を取る形に変えれば、`try_reserve` と
  同じように中身も確認できるようになる。これも 09-A をやる理由の 1 つ。

`into_exact` の失敗時の復元は 32-bit で長さの prefix の有無が変わるので、
Miri の i686 / powerpc でも通ることを確認する。

09 の A で `into_exact` / `into_growable` の形を変えるので、その前にこのテストを入れておく。

## B. `fmt::Write::write_fmt` の fast path が一度も通っていない

`impl fmt::Write for LeanString` の `write_fmt` には、引数の無いリテラルなら static バッファに
差し替える経路がある。

```rust
match args.as_str() {
    Some(s) => {
        if self.is_empty() && !self.is_heap_allocated() {
            *self = LeanString::from_static_str(s);
        } else {
            self.push_str(s);
        }
        Ok(())
    }
    None => fmt::write(self, args),
}
```

`tests/` に `write!` / `writeln!` が 1 つも無いので、`args.as_str()` が `Some` になる場合をテストで通っていない。

足すケース:

- 空の inline 文字列に `write!(s, "literal")` すると static バッファになること
- 空の static 文字列でも同じになること
- 空でない文字列では fast path に入らず、追記されること
- 確保済みで空の heap 文字列では `!self.is_heap_allocated()` の条件で fast path に入らず、容量が残ること。
  この条件を消すと確保済みの容量を捨ててしまうので、退行テストとして役に立つ

`src/traits.rs` の `try_from_fmt` にも同じ `args.as_str()` の fast path があるが、こちらは通ることがない。

`fmt::Arguments::as_str()` が `Some` を返すのは、`format_args!` 全体がコンパイル時に固定の文字列になる
場合だけ。`format_args!("{}", "literal")` のようにリテラルを直接埋めれば `Some` になるが、
`format_args!("{s}")` のように変数を埋めると、`s` が実行時に `&'static str` であっても `None` になる。
`try_from_fmt` の 2 つの呼び出し元は、どちらも `match_type!` の最後の arm で generic な
`T: fmt::Display` の変数を埋めているので、常に `None` になる。

気になるのは、もし通った場合に `from_static_str` が panic しうること。失敗を `Err` で返す API の中なので、
これは契約違反になる。通らないなら消し、残すなら `try_from_static_str` 相当を使う。どちらにするか決める。

## C. `Hash` / `Borrow` / `Ord` のテストが無い

- `HashMap<LeanStr, _>` を `&str` で引ける (`Borrow<str>` + `Hash` + `Eq` が整合している) ことは
  実装上は正しいが、テストが無い。リポジトリ全体で `HashMap` / `HashSet` / `BTreeMap` / `BTreeSet` を
  使ったテストが 1 つも無い。両方の型について。
- `Ord` / `PartialOrd` は `LeanString` と `LeanStr` のどちらもテストが無い。`.cmp(` や `.sort()` を
  使うテストがリポジトリに無い。[05](./05-equality-policy.md) の案1 を採ると `Ord` の実装を変えることになり、
  「ポインタが同じで長さが違うなら長さで順序が決まる」という間違えやすい規則が入るので、その前にテストを入れておきたい。

## D. `Extend` の多くにテストが無い

| impl | テスト |
| --- | --- |
| `Extend<char> for LeanString` | あり (`tests/lean_string.rs` の `extend_char`、`tests/alloc_string.rs`) |
| `Extend<&char> for LeanString` | あり (`tests/alloc_string.rs`) |
| `Extend<&str> for LeanString` | あり (`tests/alloc_string.rs`) |
| `Extend<LeanString> for LeanString` | `FromIterator<LeanString> for LeanString` のテスト経由 |
| `Extend<Box<str>> for LeanString` | **無し** |
| `Extend<Cow<str>> for LeanString` | **無し** |
| `Extend<String> for LeanString` | **無し** |
| `Extend<LeanStr> for LeanString` | **無し** |
| `Extend<LeanString> for String` | **無し** |
| `Extend<LeanStr> for String` | **無し** |

[03](./03-append-writer.md) でこれらの実装をまとめて書き換えるので、その前にテストを入れておきたい。

## E. テストの薄い公開 API

| 対象 | 統合テスト | doctest |
| --- | --- | --- |
| `from_utf16le` / `be` とその lossy (両型) | `tests/property.rs` のみ | あり |
| `AsRef<Path>` (両型) | **無し** | **無し** (doc コメント自体が無い) |
| `FromIterator` の 14 impl | 一部だけ (下の表) | 無し |

`FromIterator` のうちテストがあるのは次の 4 つだけ。

| impl | テスト |
| --- | --- |
| `FromIterator<char> for LeanString` | あり (イテレータが panic したときの解放も) |
| `FromIterator<&str> for LeanString` | あり |
| `FromIterator<LeanString> for LeanString` | あり (1 要素の再利用、共有の解除、空) |
| `FromIterator<String> for LeanString` | `tests/property.rs` のみ |
| 残り (`LeanStr` 向けの 8 つ、`String` 向けの 2 つ、`Box<str>` / `Cow` 向け) | **無し** |

[09](./09-mutability-conversion.md) の B で `FromIterator<LeanStr> for LeanStr` を書き換えるので、そこは先にテストを足す。

## F. static バッファの `clear` のテストが無い

`tests/lean_string.rs` の `clear_cow` は inline と heap しか見ていない。
static バッファでは `clear()` で容量が 0 になる ([06](./06-doc-fixes.md) の A) ので、doc の修正と一緒にテストを足す。

`pop` / `truncate` / `push` / `insert` / `shrink_to` には static バッファのテストがすでにある。`clear` だけ無い。

## G. `tests/const.rs` が static バッファを const 評価していない

`from_static_str` は 16 バイト以下なら inline バッファにするので、`tests/const.rs` の
"hello world" (11 バイト) や長さ 0..=16 のループはすべて inline の経路を通る。

17 バイト以上の `from_static_str` から const で `as_str()` / `len()` を取り出す経路
(`StaticBuffer::new` と `tail_word()`、const 評価での `slice::from_raw_parts`) はテストされていない。
`as_bytes` や `StaticBuffer` を変更したときに気づかず壊れるおそれがある。

## H. unsafe の契約が書かれていない箇所

安全性の問題は見つかっていないが、根拠が近くに書かれていない箇所がある。
このクレートは他の場所では丁寧に書いているので、揃えておきたい。

### `# Safety` の無い `unsafe fn`

| 場所 | 内容 |
| --- | --- |
| `Repr::as_inline_buffer_mut` | 呼び出し元がすべて `is_inline_buffer()` を確認済みであることが前提 |
| `Repr::as_heap_buffer` / `as_heap_buffer_mut` | 同じく `is_heap_buffer()` |
| `Repr::as_static_buffer` / `as_static_buffer_mut` | 同じく `is_static_buffer()` |
| `HeapBuffer::allocation` | 確保の先頭アドレスを求める関数。`dealloc` / `realloc` / `into_exact` / `into_growable` の 4 か所が、その結果と `alloc_layout()` が一致することに依存している。前提 (「`self.ptr` は `allocate_ptr` が作った確保の先頭から `header_offset` バイト (`has_len_prefix` なら `usize` 1 つ分を加えた) 先を指す」) を書き、`into_exact` と `into_growable` の中の説明からそこを参照する |

### `SAFETY:` コメントが無い、または対象がずれている箇所

| 場所 | 内容 |
| --- | --- |
| `HeapBuffer::header` | safe fn の中で生ポインタを参照にしている。`&GrowableHeader` を作れるということは atomic でない `capacity` に触れるということで、それが安全なのは「`capacity` は unique なときしか書かれない」から。この根拠がどこにも書かれていない |
| `Repr::from_inline` / `from_heap` / `from_static` の `transmute` | 直後の `SAFETY:` コメントは `assert_unchecked` の根拠で、`transmute` の根拠ではない。特に `from_inline` は任意の UTF-8 のバイト列を `*const ()` のフィールドに入れる。生ポインタには validity の要件が無く、`as_bytes` / `as_mut_ptr` は `last_byte >= HeapMarker` のときしかフィールド 0 をポインタとして読まないので正しいが、その説明が無い |
| `Repr::make_shallow_clone` の `ref_count_overflow` | `unsafe { ptr::read(repr) }.replace_inner(Repr::new())` に `SAFETY:` が無い。直前の `fetch_add` で増やした参照を消費して、panic で巻き戻る前にカウントを戻す、という意図 |
| `HeapBuffer::dealloc` の本体 | `SAFETY:` コメントが無い |
| `HeapBuffer::realloc` の 32-bit のレイアウト変換 | `realloc` の他の部分には丁寧に書いてあるのに、ここだけ `SAFETY:` が無い |
| `LeanString::from_utf8_unchecked` / `LeanStr::from_utf8_unchecked` | 本体に "SAFETY: From `# Safety`, ..." の 1 行が無い。同種の他の API には書いてある |
| `from_utf16le` / `_lossy` / `from_utf16be` / `_lossy` の `buf.align_to::<u16>()` | `SAFETY:` コメントが無い。`u16` には無効なビットパターンが無いので健全だが、その説明が無い |

### `LastByte` の網羅性

`Repr` はフィールド 2 を `LastByte` 型で持っている。この enum には `0x00..=0xD1` しか無いので、
バイト 15 が `0xD2..=0xFF` の `Repr` は即 UB になる。

書き込む側をすべて追ったところ、この不変条件は守られている。

| 書き込む箇所 | 書く値 |
| --- | --- |
| `InlineBuffer::new` (LE64 版と汎用版の両方) | `0xC0 + len`。`len == 16` のときは文字列の最後のバイト |
| `InlineBuffer::from_char` | `0xC0 + len` (`len` は 1..=4) |
| `InlineBuffer::empty` | `Length00` (`0xC0`) |
| `InlineBuffer::set_len` | `0xC0 + len` (`len < 16` のときだけ。16 なら何もしない) |
| `TextLen::new` | 上位バイトに `HeapMarker` (`0xD0`) を OR。32-bit の `ON_THE_HEAP` の経路も同じ |
| `HeapBuffer::set_len` / `realloc` / `into_exact` / `into_growable` | 同じ `TextLen` を引き継ぐだけで、作り直さない |
| `StaticBuffer::new` / `set_len` | 上位バイトに `StaticMarker` (`0xD1`) を OR |

`push_str` / `insert_str` / `remove` / `retain` / `make_ascii_*case` は `as_mut_ptr` 経由で書くが、
inline でバイト 15 に届くのは `len == 16` のときだけで、そこも UTF-8 の最後のバイトになる。
有効な UTF-8 の最後のバイトは必ず `0xC0` 未満。

`src/repr/last_byte.rs` の冒頭のコメントは UTF-8 の符号化規則を説明しているが、
「この enum の網羅性が `Repr` の安全性の前提になっている」ことは書かれていない。
`Repr` の定義に 1 段落足す。上の表がそのまま根拠になる。

## 進め方

A → B → D → H の順がよい。

1. A は、壊れたときの被害がいちばん大きい経路を押さえる。[09](./09-mutability-conversion.md) の A の前提でもある。
2. B は小さく、`write_fmt` の条件という壊れやすい箇所を守れる。
   `try_from_fmt` の通らない分岐をどうするかもここで決める。
3. D は [03](./03-append-writer.md) の前提。
4. H はコードを変えないので、他のタスクと並行して進められる。

C / E / F / G は気づいたときに足していく。F は [06](./06-doc-fixes.md) の中で一緒に片付く。
