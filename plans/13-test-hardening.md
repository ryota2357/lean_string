# 13: テストと unsafe 契約の穴を埋める

種別: テスト + doc / 実装量: 小〜中 / 設計判断: 小

## A. realloc 失敗の経路がまったくテストされていない

`tests/out_of_memory.rs` には確保と再確保を 1 回だけ失敗させる仕組みがある。

```rust
static FAIL_NEXT_ALLOCATION: Cell<bool> = const { Cell::new(false) };
static FAIL_NEXT_REALLOCATION: Cell<bool> = const { Cell::new(false) };
```

しかし `FAIL_NEXT_REALLOCATION` を `true` にするのは `without_allocating` ヘルパ
(tests/out_of_memory.rs:56-66) だけで、そこは「再確保が起きなかったこと」を確認する用途。
つまり**再確保が失敗する経路を通るテストが 1 つも無い**。

未検証のまま残っているもの:

| 場所 | 到達経路 | 内容 |
| --- | --- | --- |
| `HeapBuffer::into_exact` の失敗時復元 (heap_buffer.rs:507-522) | unique な heap の `LeanString` に対する `try_into_lean_str()` | このクレートでもっとも入り組んだ unsafe。`ptr::copy` でヘッダを潰した後に元へ戻し、長さ prefix を書き直して `GrowableHeader` を再構成する。誤っていても即座にクラッシュせず、壊れたバッファが生き残る |
| `HeapBuffer::into_growable` の失敗時 (heap_buffer.rs:614-617) | unique な heap の `LeanStr` に対する `try_into_lean_string()` | 同種 |
| `Repr::reserve` の unique heap grow (repr.rs:479-486) | unique な heap に対する `try_reserve` / `try_push_str` | `realloc` が失敗したとき |
| `Repr::shrink_to` のその場縮小 (repr.rs:561-564) | unique な heap に対する `try_shrink_to` | 同上 |

テスト自体は難しくない。unique な heap の `LeanString` に余剰容量を持たせ、
`FAIL_NEXT_REALLOCATION.set(true)` してから `try_into_lean_str()` を呼び、

- `Err(ReserveError)` が返ること
- 返ってきた文字列 (エラーと一緒に返る元の値) の内容が壊れていないこと
- 容量も元のままであること
- drop してもリークしないこと (CI の Miri が検出する)

を確認する。`into_exact` の失敗復元は 32-bit で長さ prefix の有無が変わるので、
Miri の i686 / powerpc ターゲットでも通ることを確認したい。

このテストは [plans/12](./12-mutability-conversion.md) の A で
`into_exact` / `into_growable` の形を変えるときの安全網になるので、先に入れる。

## B. `fmt::Write::write_fmt` の fast path が一度も実行されていない

`impl fmt::Write for LeanString` の `write_fmt` (lib.rs:2508-2522) には
「引数の無いリテラルなら static バッファに差し替える」経路がある。

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

`tests/` に `write!` / `writeln!` が 1 つも無いので、`args.as_str()` が `Some` になる
ケースがテストで一度も通っていない。

足すべきケース:

- 空の inline 文字列に `write!(s, "literal")` → static バッファになること
- 空の static 文字列に対して同じことをした場合
- 空でない文字列に対して (fast path に入らず追記されること)
- **確保済みの空の heap 文字列に対して** — `!self.is_heap_allocated()` のガードが
  効いて容量が保たれること。このガードを外すと確保済み容量が捨てられるので、
  退行テストとして価値がある

`src/traits.rs` の `try_from_fmt` にも同じ `args.as_str()` の fast path があるが、
2 つの呼び出し元はどちらも `format_args!("{s}")` を渡すので `as_str()` は常に `None` になる。
そちらは到達不能な分岐であり、テストで通すことはできない。
**むしろ問題は、到達したときに `from_static_str` が panic しうること**で、
fallible な API の中にある以上これは契約違反になる。到達不能なら消し、
残すなら `try_from_static_str` 相当を使う。どちらにするか決める。

## C. `LeanStr` の trait 実装にテストが無い

`tests/lean_str.rs` はテストが 5 つしかない (`new_empty` / `from_char` /
`from_around_inline_limit` / `from_static_str_around_inline_limit` /
`clone_shares_heap_buffer`)。

- `Hash` / `Ord` / `Borrow<str>` のテストが無い。
  `HashMap<LeanStr, _>` に `&str` で引ける (`Borrow<str>` + `Hash` + `Eq` の整合) ことは
  実装上は正しいが、固定するテストが無い。リポジトリ全体を見ても
  `HashMap` / `BTreeMap` を使ったテストは 1 つも無い。
- `LeanString` 側も `Ord` の直接のテストが無い。

## D. `Extend` の大半にテストが無い

`tests/` で実行されている `extend` は 3 種類だけ。

| impl | テスト |
| --- | --- |
| `Extend<char> for LeanString` | あり (tests/lean_string.rs:757、tests/alloc_string.rs:574) |
| `Extend<&char> for LeanString` | あり (tests/alloc_string.rs:775) |
| `Extend<String> for LeanString` | あり (tests/alloc_string.rs:581) |
| `Extend<&str> for LeanString` | **無し** |
| `Extend<Box<str>> for LeanString` | **無し** |
| `Extend<Cow<str>> for LeanString` | **無し** |
| `Extend<LeanString> for LeanString` | **無し** |
| `Extend<LeanStr> for LeanString` | **無し** |
| `Extend<LeanString> for String` | **無し** |
| `Extend<LeanStr> for String` | **無し** |

[plans/04](./04-append-writer.md) がこれらの実装をまとめて書き換えるので、
先に網を張っておきたい。

## E. static バッファに対する `clear` のテストが無い

`tests/lean_string.rs` の `clear_cow` (737-752) は inline と heap しか見ていない。
static バッファでは `clear()` が容量を 0 にする ([plans/09](./09-doc-fixes.md) の A) ので、
doc の修正と合わせてテストを足す。

`pop` / `truncate` / `push` / `insert` / `shrink_to` については static バッファの
テストが既にある (`pop_from_static` / `pop_from_static_cow` / `truncate_from_static` /
`push_to_static` / `insert_to_static` / `shrink_to_static_buffer`)。`clear` だけが抜けている。

## F. `retain` が何も削らない場合のテストが無い

`retain_cow` (tests/lean_string.rs:535-549) は heap と static のどちらも
「実際に文字が削られる」ケースしか見ていない。
[plans/06](./06-retain-shared-fast-path.md) で追加するテストがここを埋める。

なお述語の呼び出し回数を数えるテスト (`retain_f_apply_count`, :516) は既にあるので、
06 の変更で回数が変わらないことはそれで守られる。

## G. `tests/const.rs` が static バッファを const 評価していない

`from_static_str` は 16 バイト以下を inline バッファにする (repr.rs:110-121) ので、
`tests/const.rs` が使っている "hello world" (11 バイト) と長さ 0..=16 のループは
すべて inline 経路を通る。

長さ 16 超の `from_static_str` から `as_str()` / `len()` を const で取り出す経路
(`StaticBuffer::new` と `tail_word()`、`slice::from_raw_parts` を const 評価で通る) が
固定されていない。`as_bytes` や `StaticBuffer` を将来触ったときに黙って壊れる。

## H. unsafe の契約が書かれていない箇所

安全性の問題は見つかっていないが、根拠が近くに書かれていない場所がある。
このクレートは他の場所では丁寧に書いているので、揃えておきたい。

### `# Safety` が無い `unsafe fn`

| 場所 | 内容 |
| --- | --- |
| `Repr::as_inline_buffer_mut` (repr.rs:371) | 呼び出し元がすべて `is_inline_buffer()` を確認済みであることが前提 |
| `Repr::as_heap_buffer` (repr.rs:380) / `as_heap_buffer_mut` (repr.rs:389) | 同じく `is_heap_buffer()` |
| `Repr::as_static_buffer` (repr.rs:398) / `as_static_buffer_mut` (repr.rs:407) | 同じく `is_static_buffer()` |
| `HeapBuffer::allocation` (heap_buffer.rs:301) | 確保の先頭アドレスを復元する関数。`dealloc` / `realloc` / `into_exact` / `into_growable` の 4 か所がその結果と `alloc_layout()` の一致に依存している。必要な不変条件 (「`self.ptr` は `allocate_ptr` が作った確保の、`header_offset` バイト (+ `has_len_prefix` なら `usize` 1 個) 先を指す」) を書き、`into_exact` と `into_growable` にある議論から参照する |

### `SAFETY:` コメントが無い、または対象がずれている箇所

| 場所 | 内容 |
| --- | --- |
| `HeapBuffer::header` (heap_buffer.rs:312) | **safe fn** の中で生ポインタを参照へ変換している。`&GrowableHeader` を作れるということは非 atomic な `capacity` フィールドに触れられるということで、それが安全なのは「`capacity` は unique なときにしか書かれない」から。その根拠がどこにも書かれていない |
| `Repr::from_inline` / `from_heap` / `from_static` の `transmute` (repr.rs:328/336/344) | 直後の `SAFETY:` コメントは `assert_unchecked` の根拠であって transmute の根拠ではない。とくに `from_inline` は任意の UTF-8 バイト列を `*const ()` フィールドに載せる。生ポインタには妥当性要件が無く、`as_bytes` / `as_mut_ptr` は `last_byte >= HeapMarker` のときしかフィールド 0 をポインタとして読まないので正しいが、その説明が無い |
| `Repr::make_shallow_clone` の `ref_count_overflow` (repr.rs:267-271) | `unsafe { ptr::read(repr) }.replace_inner(Repr::new())` に `SAFETY:` が無い。直前の `fetch_add` で作った参照を消費して、panic で巻き戻る前にカウントを釣り合わせる、というのが意図 |
| `HeapBuffer::dealloc` の本体 (heap_buffer.rs:233)、`realloc` の 32-bit layout 変換 arm (heap_buffer.rs:401) | `SAFETY:` コメントが無い |
| `LeanString::from_utf8_unchecked` (lib.rs:207)、`LeanStr::from_utf8_unchecked` (lib.rs:1430) | 本体に "SAFETY: From `# Safety`, ..." の 1 行が無い。他の同種 API は書いている |

### `LastByte` の網羅性

`Repr` はフィールド 2 を `LastByte` 型で持つ (repr.rs:52)。この enum の variant は
`0x00..=0xD1` しか無いので、バイト 15 が `0xD2..=0xFF` になった `Repr` は即座に UB になる。

書き手を追うと不変条件は保たれている。

- `InlineBuffer::new` / `empty` は `len | 0xC0` (= `0xC0..=0xCF`) を書く。
  ただし `len == MAX_INLINE_SIZE` のときはバイト 15 が文字列の最終バイトそのもので、
  有効な UTF-8 の末尾バイトは必ず `0xC0` 未満。
- `HeapBuffer` の `TextLen` と `StaticBuffer` の `len` は必ず
  `HeapMarker` / `StaticMarker` を OR する。
- `push_str` / `insert_str` / `remove` / `retain` は `as_mut_ptr` 経由で書くが、
  inline のときバイト 15 に届くのは `len == 16` の場合だけで、そこも UTF-8 の末尾バイト。

`src/repr/last_byte.rs` の冒頭コメントは UTF-8 の符号化規則は説明しているが、
「この enum の網羅性が `Repr` の安全性要件になっている」ことは書かれていない。
`Repr` の定義のところに 1 段落足したい。

## 進め方

A → B → D → H の順が効率的。

1. **A** は退行時の被害がもっとも大きい経路をカバーする。
   [plans/12](./12-mutability-conversion.md) の A の前提でもある。
2. **B** は小さく、`write_fmt` のガードという退行しやすい箇所を守る。
   `try_from_fmt` の到達不能な分岐をどうするかの判断もここで済ませる。
3. **D** は [plans/04](./04-append-writer.md) の前提。
4. **H** はコードを変えないので、他のタスクと並行して進められる。

C / E / F / G は気づいたときに足していく。E と F はそれぞれ
[plans/09](./09-doc-fixes.md) と [plans/06](./06-retain-shared-fast-path.md) の中で
自然に片付く。
