# unsafe の根拠が書かれていない、または別の操作の根拠になっている箇所がある

- 種別: safety-doc
- 対象: `src/repr.rs` の `Repr::from_inline` / `from_heap` / `from_static` / `as_*_buffer*` / `make_shallow_clone`, `src/repr/heap_buffer.rs` の `HeapBuffer::header` / `allocation` / `dealloc` / `into_exact` / `release`, `ExactHeader::layout`, `src/repr/inline_buffer.rs` の `InlineBuffer::new`, `src/lib.rs` の `from_utf16le` / `from_utf16le_lossy` / `from_utf16be` / `from_utf16be_lossy`
- 影響: 健全性の問題は見つからなかった。ただし他の箇所では丁寧に `SAFETY:` を書いているクレートなので、根拠の無い箇所はレビューのたびに読み直しが要る。特に `HeapBuffer::header` と `ExactHeader::layout` は、書かれていない不変条件に依存している
- 優先度: 低

## 問題

`5dd94ea` の `src/` の `unsafe` ブロックと `unsafe fn` をすべて読み、根拠の有無を確認した。次の箇所に不足がある (`case.rs` の `split_at_unchecked` のように、`unsafe fn` の本体で `# Safety` をそのまま使うだけのものは除いた)。

### `# Safety` の無い `unsafe fn`

| 場所 | 前提 |
| --- | --- |
| `Repr::as_inline_buffer_mut` / `as_heap_buffer` / `as_heap_buffer_mut` / `as_static_buffer` / `as_static_buffer_mut` | `self` がその種類のバッファであること。本体の `SAFETY:` は "The caller guarantees ..." と書くが、関数側に契約が書かれていない |
| `HeapBuffer::allocation` | `self` が生きた確保を指すこと (`GrowableHeader` ではヘッダの `capacity`、`ExactHeader` では長さを読む)。戻り値は `alloc_layout()` と対になっていて、`dealloc` / `realloc` / `into_exact` / `into_growable` はこの組で `dealloc` / `realloc` を呼ぶ。本体にも `SAFETY:` が無い |

### `SAFETY:` が無い、または別の操作の根拠になっている箇所

| 場所 | 現状 | 書くべき根拠 |
| --- | --- | --- |
| `Repr::from_inline` / `from_heap` / `from_static` の `mem::transmute` | 直後の `SAFETY:` は `hint::assert_unchecked` の根拠で、`transmute` の根拠ではない | サイズが同じこと (transmute 自体が検査)。どのバイト列も有効な `Repr` であること: フィールド 0 の `*const ()` には validity の要件が無い (inline の文字列のバイトはアドレスとして使われず、`as_bytes` / `as_mut_ptr` は最後のバイトが `HeapMarker` 以上のときだけフィールド 0 をポインタとして読む)。最後のバイトは各バッファが有効な `LastByte` を置く ([#36](./36-lastbyte-invariant.md))。`from_heap` は `HeapBuffer<M::Header>` なので `M` とヘッダが一致する |
| `Repr::make_shallow_clone` の `ref_count_overflow` | `unsafe { ptr::read(repr) }.replace_inner(Repr::new())` に `SAFETY:` が無い | 直前の `fetch_add` で増やした 1 つ分を `ptr::read` で作ったハンドルが所有し、`replace_inner` がそれを解放してカウントを元に戻す。`repr` 自身の参照は消費しない |
| `HeapBuffer::header` | safe な `const fn` の中で生ポインタから `&H` を作っているが、`SAFETY:` が無い | (1) `self` が数えられた参照である間は、`ptr` の `header_offset()` バイト前に初期化済みのヘッダがある。(2) `&GrowableHeader` は atomic でない `capacity` を含む。これを他スレッドと共有してよいのは、`capacity` (とヘッダ全体) を書くのが `realloc` / `into_exact` (失敗時の復元を含む) / `into_growable` の生ポインタ経由の `ptr::write` だけで、どれも `&mut self` かつ `is_unique()` (カウント 1 を `Acquire` で観測) のときに限られるから。参照を持つ別のハンドルがあればカウントは 2 以上なので、書き込みと `&H` が同時に存在することはない |
| `HeapBuffer::dealloc` の本体 | `SAFETY:` が無い | `allocation()` と `alloc_layout()` は `self` の確保と同じ組。`# Safety` により他の参照は無い |
| `HeapBuffer::into_exact` の `allocation.add(...)`、`ptr::copy(old_data, new_data, len)`、`realloc` 失敗時の復元ブロック | 散文の説明はあるが `SAFETY:` の形になっていない。`into_growable` の同種のブロックには書かれている | 移動先が古い確保の中にあること (exact のデータ開始位置は growable より前)、`ptr::copy` が重なりを許すこと、復元後のヘッダと長さの prefix が `old_layout` と一致すること |
| `ExactHeader::layout` | `SAFETY:` は `Capacity::new_unchecked` の根拠 ("validated as a `Capacity`") だけ | 正しい `Layout` を返すには「exact バッファのどのハンドルの長さも確保した長さに等しい」ことが要る。`Repr<Immutable>` には長さを縮める操作が無く (truncate 系は `Repr<Mutable>` だけ)、作る経路 (`HeapBuffer::new` / `new_uninit` / `into_exact`) はすべて長さちょうどで確保するので成り立つ。将来 `LeanStr` に `truncate` 相当を足すと、`dealloc` に違う `Layout` を渡す UB になる。この不変条件を `ExactHeader` の doc に書く |
| `HeapBuffer::release` | `fetch_sub(1, Release) ensures all prior accesses from other threads are visible before we might deallocate.` | 不正確。この `fetch_sub(Release)` が保証するのは「このスレッドのそれまでのアクセス」を、最後の参照を落とすスレッドに公開すること。他スレッドのアクセスがこのスレッドに見えるのは、他スレッドの `Release` と `on_last_reference` の `fence(Acquire)` の組による。コメントは 2 つの役割を 1 つにまとめてしまっている |
| `InlineBuffer::new` (LE 64-bit 版) の `len == MAX_INLINE_SIZE` の 2 つのロードと最後の `mem::transmute([w0, w1])` | 他の分岐には `SAFETY:` があるがこの 2 か所に無い | ロードは `len == 16` なので範囲内。transmute は `[u64; 2]` から同じサイズの `InlineBuffer` で、最後のバイトは `0xC0 \| len` か文字列の最後のバイト |
| `LeanString::from_utf16le` / `from_utf16le_lossy` / `from_utf16be` / `from_utf16be_lossy` (と `LeanStr` 側が呼ぶ同じ関数) の `buf.align_to::<u16>()` (4 か所) | `SAFETY:` が無い | `u16` はどのビットパターンも有効なので、2 バイトずつを `u16` として読んでよい。前後の端数はパターンで除外している |
| `LeanString::from_utf8_unchecked` / `LeanStr::from_utf8_unchecked` の本体 | `SAFETY:` が無い | `# Safety` で `buf` が有効な UTF-8 ([#36](./36-lastbyte-invariant.md) で doc の書き直しも提案) |

小さいもの: `Repr::tail_word` の `SAFETY:` は「サイズとアラインメント」だけだが、2 語目には常に整数のバイト (inline の文字列、`TextLen`、static の長さ) しか入らず、ポインタも未初期化バイトも入らないことが、const 評価 (`tests/const.rs`) で `usize` として読めることの前提になっている。

## 原因

`realloc` や `into_growable` のように各 `unsafe` に根拠を書いている関数が大半で、上の箇所はその抜け。種類は 3 つ: (1) `transmute` のように、隣の別の `unsafe` の `SAFETY:` が根拠のように見えてしまっているもの、(2) `as_*_buffer*` や `allocation` のように契約が呼び出し側の暗黙の了解になっているもの、(3) `header` や `ExactHeader::layout` のように、型の不変条件 (ヘッダは unique なときしか書かれない、exact バッファの長さは縮まない) に依存しているのに、その不変条件が型の側に書かれていないもの。(3) は将来の変更で壊れやすい。

## 修正例

いくつかを示す。

```rust
/// # Safety
///
/// `self` must be an `InlineBuffer`, i.e. `self.is_inline_buffer()`.
#[inline(always)]
unsafe fn as_inline_buffer_mut(&mut self) -> &mut InlineBuffer { ... }

#[inline(always)]
const fn from_inline(buffer: InlineBuffer) -> Self {
    // SAFETY:
    // - `InlineBuffer` and `Repr` have the same size (checked by `transmute`).
    // - Field 0 of `Repr` is a raw pointer, which has no validity requirement. The text bytes
    //   stored there are never used as an address: it is read as a pointer only when the last
    //   byte is `HeapMarker` or `StaticMarker`.
    // - The last byte of an `InlineBuffer` is a valid `LastByte` (see the invariant of `Repr`).
    let repr: Self = unsafe { mem::transmute(buffer) };
    // SAFETY: We just transmuted from `InlineBuffer`, which is always tagged `InlineMarker`.
    unsafe { hint::assert_unchecked(repr.is_inline_buffer()) };
    repr
}

#[cold]
fn ref_count_overflow<M: Mutability>(repr: &Repr<M>) -> ! {
    // SAFETY: `fetch_add` above counted one more reference than there are handles. The handle
    // created by `ptr::read` owns that count, and `replace_inner` releases it, so the reference
    // count goes back to what it was. `repr` itself keeps its own reference.
    unsafe { ptr::read(repr) }.replace_inner(Repr::new());
    panic!("reference count overflow");
}

const fn header(&self) -> &H {
    // SAFETY:
    // - While `self` is a counted reference, `ptr` points `header_offset()` bytes past an
    //   initialized header of type `H` (written by `allocate_ptr`, `realloc`, `into_exact` or
    //   `into_growable`).
    // - The non-atomic fields of the header (`GrowableHeader::capacity`) are written only through
    //   raw pointers by `realloc`, `into_exact` and `into_growable`, which require `&mut self` and
    //   a unique buffer. So no write happens while another handle can hold this reference.
    unsafe { &*self.ptr.as_ptr().sub(Self::header_offset()).cast() }
}

// SAFETY: `u16` has no invalid bit patterns, so any two bytes can be read as one `u16`.
match (cfg!(target_endian = "little"), unsafe { buf.align_to::<u16>() }) { ... }
```

`ExactHeader` には型の doc として不変条件を書く。

```rust
/// Header of an exact (immutable) heap buffer, which has no capacity field.
///
/// # Invariant
///
/// Every handle of an exact buffer has the length the buffer was allocated for: the allocation
/// is exactly as large as the text, and `ExactHeader::layout` recomputes its `Layout` from the
/// handle's length. Nothing may shorten the length of a `Repr<Immutable>` heap handle.
pub struct ExactHeader { ... }
```

## 確認方法

コードの意味は変わらない。機械的な確認には `cargo clippy --all-features --lib -- -W clippy::undocumented_unsafe_blocks` が使える。`5dd94ea` では 23 件の警告が出て、上の表の `unsafe` ブロックはすべてここに含まれる。残りは 1 つの `SAFETY:` コメントで 2 つの項目をまとめて説明しているもの (`INLINE_BUFFER_FALSE`、`unsafe impl Sync`、2 つめの `unsafe impl LifetimeFree`) と `case.rs` の `split_at_unchecked` の本体で、中身の不足ではない。private な `unsafe fn` の `# Safety` 抜けはこの lint でも `clippy::missing_safety_doc` (pub のみ対象) でも検出されないので、目で確認する。lint を CI で有効にするかは別途判断。
