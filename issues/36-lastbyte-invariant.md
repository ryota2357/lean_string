# `Repr` の最後のバイトが有効な `LastByte` でなければならない不変条件がどこにも書かれていない

- 種別: safety-doc, doc
- 対象: `src/repr.rs` の `Repr` の定義, `src/repr/last_byte.rs`, `src/repr/inline_buffer.rs` の `InlineBuffer::set_len`, `src/lib.rs` の `LeanString::from_utf8_unchecked` / `LeanStr::from_utf8_unchecked`
- 影響: `Repr` に書き込むコードを変更する人が、どの値を最後のバイトに置いてはいけないかをコードから読み取れない。利用者側では `from_utf8_unchecked` の `# Safety` が「何が起きるか」を曖昧にしか書いていない
- 優先度: 中

## 問題

`Repr` は 3 つめのフィールドを `LastByte` 型で持つ。

```rust
pub(crate) struct Repr<M: Mutability>(*const (), [u8; 7], LastByte, PhantomData<M>);
```

`LastByte` は `#[repr(u8)]` の enum で、値は `0x00..=0xD1` だけ (`0x00..=0xBF` が文字列の最後のバイト、`0xC0..=0xCF` が inline の長さ、`0xD0` が `HeapMarker`、`0xD1` が `StaticMarker`)。したがって最後のバイトが `0xD2..=0xFF` の `Repr` は、作った (値として読み書きした) 時点で未定義動作になる。さらに `Option<Repr>` や `Result<Option<Repr>, ReserveError>` はこの空き値をニッチに使っている (`repr.rs` 冒頭の `size_of` の const assert がそれを前提にしている)。`0xD2` が入れば `Some(s)` が `None` に化ける。

この前提はクレートの健全性の中心だが、書かれている場所が無い。

- `Repr` の定義にはコメントが無い。
- `last_byte.rs` の冒頭のコメントは UTF-8 の符号化規則を説明するだけで、「enum に無い値は UB」「`Repr` に書き込む側はこれを守る」とは書いていない。
- `InlineBuffer::set_len` の `# Safety` は「`len` バイトが有効な UTF-8」「`len <= MAX_INLINE_SIZE`」だけ。`len == MAX_INLINE_SIZE` のときは最後のバイトを書かない (そこには文字列の最後のバイトが既に入っている前提) という、まさにこの不変条件に依存する振る舞いが説明されていない。
- `Repr::from_inline` / `from_heap` / `from_static` の `mem::transmute` に根拠が書かれていない ([#37](./37-safety-comments.md) にも記載)。

利用者から見える影響として、`from_utf8_unchecked` の `# Safety` は次のとおり。

```text
This function is unsafe because it does not check that the bytes passed to it are valid
UTF-8. If this constraint is violated, it may cause memory unsafety issues.
```

`String::from_utf8_unchecked` の場合、不正な UTF-8 はライブラリの不変条件違反で、壊れるのは後でその文字列を使ったとき。`LeanString` では、ちょうど `2 * size_of::<usize>()` バイトで最後のバイトが `0xD2..=0xFF` の入力は、構築した瞬間に言語レベルの UB になる。確認した結果 (`5dd94ea`, rustc 1.97.0 stable / nightly 1.101.0 2026-10-11 の Miri, x86_64):

```rust
let mut bytes = [b'a'; 16];
bytes[15] = 0xFF; // 0xD2 でも同じ
let s = unsafe { LeanString::from_utf8_unchecked(&bytes) };
```

- Miri (`-Zmiri-strict-provenance`):
  `Undefined Behavior: constructing invalid value of type lean_string::repr::Repr<...Mutable>: at .2.<enum-tag>, encountered 0xff, but expected a valid enum tag` (`Repr::from_inline` の `mem::transmute`)
- debug ビルド: `Repr::from_inline` の `hint::assert_unchecked` の事前条件チェックで abort (SIGABRT)
- release ビルド: SIGILL

関数の契約上は「不正な UTF-8 は UB」で正しいが、doc の書き方 ("may cause memory unsafety issues") からはこれが読み取れない。

## 原因

不変条件は次のとおり守られている (全書き込み箇所を読んで確認。Miri の全テストも x86_64 / i686 / powerpc64 / powerpc で通過)。根拠が分散しており、まとめて書かれていないだけ。

| バイト 15 (32-bit ではバイト 7) を書く箇所 | 書く値 |
| --- | --- |
| `InlineBuffer::new` (LE 64-bit 版) | `len < 16` なら `0xC0 \| len` (`8 < len < 16` の `tail >> shift` は 8 ビット以上シフトするので最上位バイトは 0)。`len == 16` なら文字列の最後のバイト |
| `InlineBuffer::new` (汎用版) | `0xC0 \| len`。`len == 16` なら `copy_nonoverlapping` で文字列の最後のバイトに上書き |
| `InlineBuffer::from_char` / `empty` | `0xC0 \| len` / `0xC0` |
| `InlineBuffer::set_len` | `len < 16` なら `0xC0 \| len`。`len == 16` なら書かない |
| `TextLen::new` | `size.to_le() \| TAG` (`size <= MAX_LEN` なので最上位バイトは 0、結果は `0xD0`)。32-bit の `ON_THE_HEAP` も最上位バイトは `0xD0` |
| `StaticBuffer::new` / `set_len` | `len.to_le() \| TAG` (`len <= MAX_LENGTH`、結果は `0xD1`) |
| `Repr::as_mut_ptr` 経由の書き込み (`push_str`, `push_char`, `insert_str`, `insert_char`, `extend_from_within`, `replace_range`, `remove`, `retain`, `make_ascii_case`, `to_case`) | inline でバイト 15 に届くのは新しい長さが 16 のときだけで、書くのは最終的な文字列の最後のバイト。`insert_str` / `replace_range` は後ろの部分を先に最終位置へ動かすので、途中の状態でもバイト 15 には最終値しか入らない。`remove` / `retain` は左詰めなのでバイト 15 を書かない |
| `Repr::new_with` の `init` | ローカルの `InlineBuffer` に書くので、`Repr` になるのは `set_len` の後 |

有効な UTF-8 の最後のバイトは ASCII (`< 0x80`) か継続バイト (`0x80..=0xBF`) なので、どれも `0xC0` 未満になる。これが「長さ 16 の inline 文字列でも最後のバイトが長さのタグと衝突しない」理由であり、同時に「不正な UTF-8 を入れると即 UB になりうる」理由でもある。

## 修正例

`Repr` の定義に不変条件を書く。

```rust
/// The representation shared by [`LeanString`] and [`LeanStr`]: two words, whose last byte
/// ([`LastByte`]) tells which buffer it holds.
///
/// | last byte      | buffer       | field 0                     | the rest              |
/// | -------------- | ------------ | --------------------------- | --------------------- |
/// | `0x00..=0xBF`  | InlineBuffer | text (`MAX_INLINE_SIZE` bytes, the last byte is text) |
/// | `0xC0..=0xCF`  | InlineBuffer | text, `len = last byte - 0xC0`                        |
/// | `0xD0`         | HeapBuffer   | pointer to the text         | `TextLen`             |
/// | `0xD1`         | StaticBuffer | pointer to the text         | length                |
///
/// # Invariant
///
/// The last byte is always a valid `LastByte`, i.e. `0x00..=0xD1`. Any other value is undefined
/// behavior as soon as the `Repr` is copied or read, and `0xD2..` are used as niches (e.g.
/// `Option<Repr>::None`). A full inline buffer keeps the last byte of its text there, which is
/// below `0xC0` because valid UTF-8 never ends with a lead byte. So every write into an inline
/// buffer must leave valid UTF-8 in it, and a write to the last byte must be the final last byte
/// of the text, or a length tag written by `InlineBuffer::set_len`.
#[repr(C)]
pub(crate) struct Repr<M: Mutability>(*const (), [u8; 7], LastByte, PhantomData<M>);
```

`InlineBuffer::set_len` の `# Safety` に 1 行足す。

```rust
/// - If `len == MAX_INLINE_SIZE`, the last byte of the buffer must already hold the last byte of
///   the text (this method does not write it).
```

`from_utf8_unchecked` (両型) は std と同じ言い方にし、本体に `SAFETY:` を付ける。

```rust
/// # Safety
///
/// The bytes passed in must be valid UTF-8. Passing invalid UTF-8 is undefined behavior, which
/// may occur immediately, not only when the string is used later.
pub unsafe fn from_utf8_unchecked(buf: &[u8]) -> Self {
    // SAFETY: From `# Safety`, `buf` is valid UTF-8.
    let str = unsafe { str::from_utf8_unchecked(buf) };
    LeanString::from(str)
}
```

`last_byte.rs` の冒頭コメントからも `Repr` の Invariant を参照するとよい。

## 確認方法

コードの変更は doc のみ。`cargo doc --document-private-items` で表示を確認する。不正入力で UB になることの再現は上の 3 行を Miri で走らせればよい (テストには入れない)。
