# 11: テストと unsafe 契約の穴を埋める

種別: テスト + doc / 実装量: 小〜中 / 設計判断: 小

## A. realloc 失敗の経路がまったくテストされていない

`tests/out_of_memory.rs` には確保と再確保を 1 回だけ失敗させる仕組みがある。

```rust
static FAIL_NEXT_ALLOCATION: Cell<bool> = const { Cell::new(false) };
static FAIL_NEXT_REALLOCATION: Cell<bool> = const { Cell::new(false) };
```

しかし `FAIL_NEXT_REALLOCATION` を `true` にするのは `without_allocating` ヘルパだけで、
そこは「再確保が起きなかったこと」を確認する用途。つまり
**再確保が失敗する経路を通るテストが 1 つも無い**。

未検証のまま残っているもの:

| 場所 | 内容 |
| --- | --- |
| `HeapBuffer::into_exact` の失敗時復元 (heap_buffer.rs:507-522) | このクレートでもっとも入り組んだ unsafe。`ptr::copy` でヘッダを潰した後に元へ戻し、長さ prefix を書き直して `GrowableHeader` を再構成する。誤っていても即座にクラッシュせず、壊れたバッファが生き残る |
| `HeapBuffer::into_growable` の失敗時 (heap_buffer.rs:614-617) | 同種 |
| `Repr::reserve` の unique heap grow (repr.rs:469-475) | `realloc` が失敗したとき |
| `Repr::shrink_to` のその場縮小 (repr.rs:550-553) | 同上 |

テストは難しくない。unique な heap `LeanString` に余剰容量を持たせ、
`FAIL_NEXT_REALLOCATION.set(true)` してから `try_into_lean_str()` を呼び、

- `Err(ReserveError)` が返ること
- 返ってきた文字列 (エラーと一緒に返る元の値) の内容が壊れていないこと
- drop してもリークしないこと (CI の Miri が検出する)

を確認する。`into_exact` の失敗復元は 32-bit で長さ prefix の有無が変わるので、
Miri の i686 / powerpc ターゲットでも通ることを確認したい。

## B. `fmt::Write::write_fmt` の fast path が一度も実行されていない

`impl fmt::Write for LeanString` の `write_fmt` (lib.rs:2094-2109) には
「引数の無いリテラルなら static バッファに差し替える」経路がある。
`tests/` に `write!` / `writeln!` が 1 つも無く、この経路の唯一の到達元である
`ToLeanString` のフォールバックは必ず引数付きで呼ぶため、
`args.as_str()` が `Some` になるケースがテストで一度も通っていない。

足すべきケース:

- 空の inline 文字列に `write!(s, "literal")` → static バッファになること
- 空の static 文字列に対して同じことをした場合
- 空でない文字列に対して (fast path に入らず追記されること)
- **確保済みの空の heap 文字列に対して** — `!self.is_heap_allocated()` のガードが
  効いて容量が保たれること。このガードを外すと確保済み容量が捨てられるので、
  退行テストとして価値がある

## C. その他の穴

- `LeanStr` の専用テストが薄い (`tests/lean_str.rs`)。`Hash` / `Ord` / `Borrow` の
  テストが無い。`HashMap<LeanStr, _>` に `&str` で引ける (`Borrow<str>` + `Hash` +
  `Eq` の整合) ことは実装上は正しいが、固定するテストが無い。
- `tests/const.rs` は inline 表現しか const 評価していない。長さ 16 超の
  `from_static_str` から `as_str()` / `len()` を const で取り出す経路
  (`tail_word()` と `slice::from_raw_parts` を const 評価で通る) が固定されていない。
  `as_bytes` を将来触ったときに黙って壊れる。
- **MSRV でテストがコンパイルできない**。`cargo +1.85.1 test --all-features` は
  `tests/const.rs` で失敗する (`core::str::<impl str>::split_at` が 1.85.1 では
  const-stable でない)。CI の MSRV ジョブは `cargo hack check --rust-version --all-features`
  なのでライブラリしか見ておらず、この状態に気づけない。
  ライブラリ自体は 1.85.1 でビルドできることを確認したので利用者への影響は無いが、
  テストを MSRV の対象に含めるかどうかは決めておきたい
  (含めるなら `tests/const.rs` の該当箇所を書き換えるか、`cfg` で分ける)。
- static バッファに対する `clear()` のテストが無い
  ([plans/09](./09-static-capacity-docs.md) と合わせて足す)。
- `Extend<LeanStr> for LeanString`、`Extend<LeanString> for String`、
  `Extend<LeanStr> for String` (lib.rs:2063-2085) にテストが無い。

## D. unsafe の契約が書かれていない箇所

安全性の問題は見つかっていないが、根拠が近くに書かれていない場所がある。
このクレートは他の場所では丁寧に書いているので、揃えておきたい。

| 場所 | 内容 |
| --- | --- |
| `HeapBuffer::allocation` (heap_buffer.rs:301-310) | `unsafe fn` なのに `# Safety` が無い。確保の先頭アドレスを復元する関数で、`dealloc` / `realloc` / `into_exact` / `into_growable` の 4 か所がその結果と `alloc_layout()` の一致に依存している。必要な不変条件 (「`self.ptr` は `allocate_ptr` が作った確保の、`header_offset` バイト (+ `has_len_prefix` なら `usize` 1 個) 先を指す」) を書き、`into_exact` (:487-490) と `into_growable` (:603-605) にある議論から参照する |
| `HeapBuffer::header` (heap_buffer.rs:312-314) | **safe fn** の中で生ポインタを参照へ変換している。`SAFETY:` コメントが無い。`&GrowableHeader` を作れるということは非 atomic な `capacity` フィールドに触れられるということで、それが安全なのは「`capacity` は unique なときにしか書かれない」から。その根拠がどこにも書かれていない |
| `Repr::make_shallow_clone` の `ref_count_overflow` (repr.rs:259) | `unsafe { ptr::read(repr) }.replace_inner(Repr::new())` に `SAFETY:` が無い。直前の `fetch_add` で作った参照を消費して、panic で巻き戻る前にカウントを釣り合わせる、というのが意図 |
| `Repr::from_inline` / `from_heap` / `from_static` の `transmute` (repr.rs:319/327/335) | 直後の `SAFETY:` コメントは `assert_unchecked` の根拠であって transmute の根拠ではない。特に `from_inline` は任意の UTF-8 バイト列を `*const ()` フィールドに載せる。生ポインタには妥当性要件が無く、`as_bytes` / `as_mut_ptr` は `last_byte >= HeapMarker` のときしかフィールド 0 を読まないので正しいが、その説明が無い |
| `HeapBuffer::dealloc` の本体 (heap_buffer.rs:234)、`realloc` の 32-bit layout 変換 arm (heap_buffer.rs:401) | `SAFETY:` コメントが無い |
| `LeanString::from_utf8_unchecked` (lib.rs:204)、`LeanStr::from_utf8_unchecked` (lib.rs:1209) | 本体に `// SAFETY: From `# Safety`, ...` の 1 行が無い。他の同種 API は書いている |

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

A → B → D の順が効率的。A は退行時の被害がもっとも大きい経路をカバーする。
D はコードを変えないので、他のタスクと並行して進められる。
C は気づいたときに足していく。
