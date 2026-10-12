# panic しうる公開 API の `# Panics` が欠けている・不正確で、`# Errors` もほぼ無い

- 種別: doc, safety-doc
- 対象: `src/lib.rs` の `LeanString::{from_utf8, from_utf8_lossy, from_utf8_unchecked, from_utf16, from_utf16_lossy, from_utf16le, from_utf16le_lossy, from_utf16be, from_utf16be_lossy}` と `LeanStr` の同名メソッド、`impl Clone for LeanString` / `LeanStr`、`truncate` / `try_truncate` / `try_pop`、`to_lowercase` / `to_uppercase`、各 `try_*` の `# Errors`
- 影響: `Result` を返すのに OOM で panic するデコード関数や、「アロケーションしない」と書かれた `clone` の refcount overflow panic が文書化されておらず、`no_panic` 性や OOM 方針を気にする利用者が API 契約を読み誤る。また `truncate` の記述は 64-bit では起こりえない状況を挙げている。
- 優先度: 中

## 問題

### 1. `# Panics` が無い (実際に panic する)

内部で `with_capacity` / `push` / `From<&str>` を通じて `unwrap_with_msg` を呼ぶのに `# Panics` セクションが無い公開関数 (`src/lib.rs` の `pub fn` を走査して確認):

- `LeanString` / `LeanStr` の `from_utf8`, `from_utf8_lossy`, `from_utf8_unchecked`, `from_utf16`, `from_utf16_lossy`, `from_utf16le`, `from_utf16le_lossy`, `from_utf16be`, `from_utf16be_lossy`。
  - 特に `from_utf8` / `from_utf16*` は `Result` を返すため、OOM でも `Err` になると読める。実際は panic する。
  - 失敗するアロケータ (次の 1 回の確保を失敗させる `#[global_allocator]`) で確認した (`5dd94ea`, rustc 1.97.0)。いずれも `PANIC: Cannot allocate memory to hold LeanString`:
    `LeanString::from_utf8`, `from_utf8_lossy`, `from_utf16`, `from_utf16_lossy`, `LeanStr::from_utf8`。
- `Clone::clone` / `clone_from` (`LeanString`, `LeanStr`): impl のドキュメントは「O(1)」「メモリ確保なし」とだけ書く。`Repr::make_shallow_clone` は `fetch_add` の旧値が `isize::MAX` を超えると `panic!("reference count overflow")` する (`Arc` は abort)。実際に起こすには `isize::MAX` 個の参照が必要で現実には起きないが、panic しうることは書かれていない (CHANGELOG 0.3.0 には「Panic on reference count overflow」とある)。panic する経路は、`From<&LeanString> for LeanString` など `clone` を呼ぶ impl 全部に及ぶ。
- トレイト impl: `From<&str>`, `From<&mut str>`, `From<String>`, `From<&String>`, `From<Cow<str>>`, `From<Box<str>>`, `From<LeanString> for LeanStr` (`into_lean_str`) など (`LeanStr` 側も)、`FromIterator`, `Extend`, `Add<&str>`, `AddAssign<&str>`, `fmt::Write::{write_str, write_fmt}`、`serde` の `Deserialize`、`arbitrary` の `Arbitrary`。これらはすべて OOM で panic する (`Writer::write_str` は `Ok(())` を返すので `fmt::Error` にもならない)。impl に個別のドキュメントは付かないので、型のドキュメント側にまとめて書く必要がある ([#40](./40-type-level-docs.md) 参照)。

### 2. 記述が不正確

- `LeanString::truncate` の `# Panics` は「2. The system is out-of-memory when cloning the `LeanString`」とするが、64-bit では `truncate` は一切アロケーションしない。`Repr::truncate_unchecked` は、長さが各ハンドル (`Repr`) 側に保持される場合は長さを書き換えるだけで済ませる (長さを heap 側に保存する 32-bit の `2^24 - 2` 超の文字列だけ、共有されていれば複製する。32-bit でのコピーの条件は [#24](./24-truncate-32bit-shared-needless-copy.md) を参照)。確認: 共有中のヒープ文字列 (50 バイト程度)を 20 / 5 に `truncate`、`pop`、`clear` した際のアロケーション回数はすべて 0 (カウント付き `#[global_allocator]`)。`try_truncate` / `try_pop` に失敗アロケータを仕掛けても失敗は発生しなかった (64-bit では常に `Ok`)。
  - 一方 `pop` の `# Panics` は 32-bit 限定として正しく書かれている。`truncate` / `try_truncate` / `try_pop` / `split_off` (中で `try_truncate` を呼ぶ) も同じ書き方に揃えるのがよい。
- `to_lowercase` / `to_uppercase` (`LeanString`, `LeanStr`) は「OOM なら panic」と書くが、小文字化で `'Σ'` (final sigma) を含む場合は `Repr::to_case` が `str::to_lowercase` に処理を丸ごと渡し、その OOM は `handle_alloc_error` で **プロセスが abort** する。確認: 最初の確保を失敗させると catch できる panic だが、`Σ` を含む文字列で 2 回目の確保を失敗させると `memory allocation of 31 bytes failed` で abort した。`to_lowercase` のコード内の NOTE が `try_to_lowercase` を作れない理由として触れているだけで、利用者向けのドキュメントには無い。

### 3. `# Errors` セクション

`clippy::missing_errors_doc` は `Result` を返す公開関数 32 個で警告する (`try_with_capacity`, `from_utf8`, `from_utf16*`, `try_reserve*`, `try_shrink_to*`, `try_push*`, `try_pop`, `try_insert*`, `try_remove`, `try_retain`, `try_replace_range`, `try_extend_from_within`, `try_truncate`, `try_split_off`, `try_repeat`, `try_to_ascii_*`, `try_make_ascii_*`、`LeanStr` の同等のもの)。`# Errors` があるのは `try_into_lean_str` / `try_into_lean_string` と `traits.rs` の `try_to_lean_string` / `try_to_lean_str` だけで、他は「…returns a `ReserveError`」という散文で済ませている。std (`Vec::try_reserve` など) は `# Errors` を使う。

さらに、`ReserveError` が返る条件 (アロケータ失敗、長さ・容量の上限超過 `2^56 - 1` / `2^31 - 16`、`usize` オーバーフロー) と「失敗時に `self` は変化しない」ことは `try_push_str` と `try_extend_from_within` にしか書かれていない。実際には `try_*` 全体で成り立つ: 共有ヒープ文字列と静的文字列それぞれに対し、`try_reserve`, `try_reserve_exact`, `try_push`, `try_push_str`, `try_insert`, `try_insert_str`, `try_replace_range`, `try_remove`, `try_make_ascii_uppercase`, `try_extend_from_within`, `try_retain`, `try_split_off` で次の確保を失敗させ、すべて `Err` かつ元の文字列 (と、共有していた clone) が不変であることを確認した。

## 原因

メソッドを追加するたびに `# Panics` / `# Errors` を足す運用がなく、`From` / `Clone` / `Extend` などトレイト impl は doc コメントを持たないため落ちる。`truncate` のテキストは `remove` / `insert` の文面をそのまま流用したように見える (推測)。

## 修正例

- 上記の関数に `# Panics` を足す。例 (`from_utf8`):

  ```rust
  /// # Panics
  ///
  /// Panics if allocating the string fails. Unlike the validation error, this is not reported in the
  /// returned `Result`.
  ```

  デコード関数に `try_` 版を足すかどうかは [#47](./47-fallible-decoders.md) を参照。
- `impl Clone` のドキュメントに `# Panics`: 「Panics if the reference count overflows `isize::MAX`.」
- `truncate` / `try_truncate` / `try_pop` / `split_off` の記述を 32-bit 限定の条件つきに直す (`pop` の書き方に合わせる): 「On 32-bit architectures, this method needs to clone ... if (not unique) and (length > 2^24 - 2)」。`try_truncate` / `try_pop` には「64-bit では常に `Ok`」と書いてよい。
- `to_lowercase` / `to_uppercase` に「文字列に `Σ` を含む場合、確保の失敗は panic ではなく abort になる」と書く。
- `try_*` に `# Errors` を足す。共通の文を型のドキュメントに置き、各 `try_*` からそこへリンクしてもよい。「失敗時に `self` は変化しない」は型レベルの保証として一箇所に書く。

## 確認方法

- `cargo clippy --all-features -- -W clippy::missing_errors_doc -W clippy::missing_panics_doc` で `# Errors` の欠落を確認 (`missing_panics_doc` は関数本体の直接の panic にしか反応しないので、`unwrap_with_msg` 経由のものは検出されない点に注意)。
- OOM による panic / abort は `tests/out_of_memory.rs` の失敗アロケータ (`FAIL_NEXT_ALLOCATION`) の仕組みで再現できる。`from_utf8` など `Result` を返す関数が `catch_unwind` で panic として観測されること、`try_pop` / `try_truncate` が 64-bit で確保を発生させないことを、このファイルにテストとして足せる。
