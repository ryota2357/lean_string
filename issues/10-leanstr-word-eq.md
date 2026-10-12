# `LeanStr` 同士の `==` が、2 ワードの比較で済む場面でも長さの復元と `bcmp` を通る

- 種別: perf
- 対象: `src/lib.rs` の `impl PartialEq for LeanStr`、`src/repr.rs` の `Repr::<Mutable>::into_immutable`、`src/repr/inline_buffer.rs` の `InlineBuffer`
- 影響: `LeanStr` を識別子やキーとして比べるコード (シンボル表、AST、重複除去など)。inline 同士の比較が約 3.1 ns から約 1.2 ns になる
- 優先度: 中

## 問題

`LeanStr == LeanStr` は `self.as_str().eq(other.as_str())` である。両辺の長さを 16 バイトの表現から復元し
(cmov 4 つ)、長さを比べ、データポインタを選んで `bcmp` を呼ぶ。asm は `LeanString == LeanString` と同じ 33 命令になる
(`5dd94ea`、rustc 1.97.0、x86-64)。

inline の `Repr` は、長さもタグとして最後のバイトに入っている。使っていないバイトが 0 に揃っていれば、
2 つの inline の値は 16 バイトが一致するときに限り等しい。つまり `[usize; 2]` の比較 1 回で判定できる。
それなのに、今は inline でも `bcmp` を呼んでいる。

時間 (ns)。Intel Xeon 2.8 GHz、glibc 2.39、`bench` の release プロファイル (LTO あり) で測った。
1 つのバイナリの中で交互に 400k 回ずつ 25 ラウンド回し、その最小値を取った。

| ケース | 現状 | 修正例 |
| --- | --- | --- |
| inline、等しい (長さ 1 / 8 / 15 / 16) | 3.12 / 3.12 / 3.11 / 3.22 | 1.18 / 1.19 / 1.18 / 1.18 |
| inline、先頭が不一致 (長さ 8 / 16) | 3.32 / 3.32 | 1.19 / 1.18 |
| inline、末尾が不一致 (長さ 8 / 16) | 3.31 / 3.29 | 1.19 / 1.18 |
| heap、別の値で等しい (長さ 17 / 32 / 256) | 3.19 / 3.16 / 5.81 | 3.00 / 2.99 / 5.75 |
| heap、先頭が不一致 (長さ 17 / 256) | 3.47 / 3.30 | 3.29 / 3.29 |
| heap、clone 同士 (長さ 17 / 32 / 256) | 3.05 / 3.03 / 5.88 | 2.07 / 2.06 / 2.06 |
| 短い識別子 64 個の線形探索 ×64 (µs、3 回の実行) | 2.46〜3.10 | 2.23〜2.28 |

heap 側で遅くなる場面は無かった。clone 同士で速くなるのは、修正例にポインタの一致による近道
([#19](./19-shared-buffer-ptr-eq.md)) も入れたからである。

## 原因

ワード比較には「inline の `[len, 15)` バイトが 0」という正規形が要る。`Repr<Immutable>` では、1 か所を除いてこれがすでに成り立っている。

`Repr<Immutable>` の inline 値を作る経路:

- `InlineBuffer::new`: LE 64-bit 版はレジスタで 2 ワードを組み立て、余りのビットを 0 にしている。それ以外の版は `[0u8; 16]` から書き始める。
- `InlineBuffer::from_char`、`InlineBuffer::empty`、`INLINE_BUFFER_TRUE` / `INLINE_BUFFER_FALSE`。
- `Repr::new_with` の inline 分岐: `empty()` に `len` バイトだけを書く。`from_num` の整数、`repeat`、`to_ascii_case` がここを通る。
- `make_shallow_clone`: 16 バイトをそのままコピーする。
- `Repr::<Mutable>::into_immutable`: 正規形にならない。heap でなければ `mem::transmute` でそのまま渡す。

`Repr<Mutable>` は縮めると正規形が崩れる。`InlineBuffer::set_len` は最後のバイトだけを書き換えるので、
`truncate` / `pop` / `remove` / `retain` / `clear` / 短い文字列での `replace_range` の後は古いバイトが残る。
それが `into_lean_str` (`From<LeanString> for LeanStr`、`ToLeanStr`、`LeanStr::to_lowercase` などの内部) を通って、
そのまま `LeanStr` に入る。`Repr<Immutable>` には変更する操作が無いので、ここで 1 回正規化すれば十分である。

確かめた方法: 正規化をしない版でワード比較を試した。`truncate` した `LeanString` から作った `LeanStr` と、
同じ内容を `LeanStr::from` で作った値が等しくないと判定された (proptest で失敗した)。

もう 1 つ注意点がある。`LeanStr` でも、16 バイト以下の static バッファはありうる。
`LeanString::from_static_str(長い文字列)` を `truncate` してから `into_lean_str` すると、static のまま渡る。
そのため「inline でなければ 17 バイト以上」とは言えない。片方だけが inline のときは、ワード比較で `false` を返してはいけない。
最初の試作はここで間違えた。

## 修正例

`into_immutable` で inline を正規化し、`LeanStr` の `eq` で、両辺が inline ならワード比較を使う。

```rust
// src/repr/inline_buffer.rs
impl InlineBuffer {
    /// Zeroes the unused bytes `len..MAX_INLINE_SIZE - 1`, without `memset`.
    #[inline]
    pub(super) fn make_canonical(&mut self) {
        const W: usize = size_of::<usize>();
        let words = self as *mut Self as *mut [usize; 2];
        // SAFETY: `InlineBuffer` is two words long and aligned as `usize`.
        let [w0, w1] = unsafe { words.read() };
        let (w0, w1) = (usize::from_le(w0), usize::from_le(w1));
        let len = ((w1 >> (usize::BITS - 8)) as usize).wrapping_sub(LastByte::MASK_1100_0000 as usize);
        if len >= MAX_INLINE_SIZE {
            return; // full: there is no unused byte
        }
        let keep = |n: usize| if n >= W { usize::MAX } else { (1 << (n * 8)) - 1 };
        let m0 = keep(len);
        let m1 = keep(len.saturating_sub(W)) | (0xFF << (usize::BITS - 8));
        // SAFETY: same as above.
        unsafe { words.write([(w0 & m0).to_le(), (w1 & m1).to_le()]) };
    }
}

// src/repr.rs, Repr::<Mutable>::into_immutable
if !self.is_heap_buffer() {
    let mut this = self;
    if this.is_inline_buffer() {
        // `Repr<Immutable>` keeps inline buffers canonical, see `Repr::<Immutable>::content_eq`.
        unsafe { this.as_inline_buffer_mut() }.make_canonical();
    }
    return Ok(unsafe { mem::transmute::<Repr<Mutable>, Repr<Immutable>>(this) });
}

// src/repr.rs
impl Repr<Immutable> {
    /// NOTE: Relies on inline buffers of `Repr<Immutable>` being canonical: the bytes
    /// `len..MAX_INLINE_SIZE - 1` are zero. Every constructor guarantees it, and
    /// `Repr::<Mutable>::into_immutable` canonicalizes.
    #[inline]
    pub(crate) fn content_eq(&self, other: &Self) -> bool {
        if self.is_inline_buffer() && other.is_inline_buffer() {
            let words = |r: &Self| [r.0.addr(), r.tail_word()];
            return words(self) == words(other);
        }
        let (a, b) = (self.as_bytes(), other.as_bytes());
        a.len() == b.len() && (ptr::eq(a.as_ptr(), b.as_ptr()) || a == b)
    }
}

// src/lib.rs
impl PartialEq for LeanStr {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.0.content_eq(&other.0)
    }
}
```

- 長さ 16 では最後のバイトも文字列のデータで、マスクは不要である (`len >= MAX_INLINE_SIZE` で抜ける)。
  正しい UTF-8 の最後のバイトは `0xC0` 未満なので、タグと混ざることはない。
- `make_canonical` はマスクとシフトだけで、`memset` を呼ばない。`into_lean_str` の inline 経路は
  レジスタだけの命令が約 25 個増える (x86-64、関数全体で 219 から 249 命令)。heap の経路は変わらない。
- `content_eq` の inline 同士の経路は x86-64 で `retq` を含めて 14 命令になる (タグのバイトのロード 2 つ、ワードのロード 4 つ、
  XOR、OR、`sete`)。aarch64 では `ldrb` 2 つと `ldr` 4 つ、`cmp` / `ccmp` / `cset` になる。それ以外の経路は今の `==` と同じで、ポインタの比較が 1 つ加わる。
- ポインタの比較を入れない版 (`self.as_bytes() == other.as_bytes()` に落とす) も測った。
  inline の結果は同じで、clone 同士では速くならない。

確認したこと: 同じ内容をいろいろな経路で作った `LeanStr` について、全組み合わせの結果が `str` の比較と一致するのを
proptest で確かめた。経路は `truncate` / `pop` / `clear` + `push_str` / `replace_range` / `retain` / 短い heap /
短い static からの `into_lean_str`、`to_lowercase`、`to_ascii_lowercase`、`repeat(1)` である。
Miri (`-Zmiri-strict-provenance`) でも x86_64、i686、powerpc64 で確かめた。

試して効かなかったもの:

- 先にワードの一致を見て、一致しなければバッファの種類で分ける順序。inline は同じ速さだったが、
  heap 同士で別の値のとき が 3.2〜3.5 ns から 4.9〜6.3 ns に遅くなった。原因は突き止めていない。
  inline かどうかを先に判定する上の形にすると、この遅れは出ない。

`LeanString` には広げていない。広げるなら、縮める操作のたびに `InlineBuffer::set_len` で正規化が要る。
また `push_str` などの伸ばす操作の経路と `set_len` を分ける必要もある。
`LeanString` には、正規形を要らない [#11](./11-short-eq-bcmp.md) の方法が使える。

## 確認方法

- 正規形の単体テスト (`src/repr.rs` の `#[cfg(test)]`): すべての構築経路と `into_immutable` の後で、`Repr<Immutable>` が
  inline なら `[len, 15)` が 0 であることを見る。`debug_assert!` を `content_eq` に入れてもよい。

```rust
#[cfg(test)]
fn is_canonical<M: Mutability>(r: &Repr<M>) -> bool {
    !r.is_inline_buffer() || {
        let bytes = unsafe { &*(r as *const Repr<M> as *const [u8; MAX_INLINE_SIZE]) };
        bytes[r.len().min(MAX_INLINE_SIZE - 1)..MAX_INLINE_SIZE - 1].iter().all(|&b| b == 0)
    }
}
```

- 公開 API のテスト: 縮めた `LeanString` から作った `LeanStr` と、`LeanStr::from` で作った値が等しいこと
  ([#27](./27-test-eq-ord-hash.md) の `eq_hash_across_representations` は `into_lean_str` も通る)。
- asm: 下流クレートで `fn f(a: &LeanStr, b: &LeanStr) -> bool { a == b }` を見て、inline 同士の経路に `bcmp` も
  長さの復元も無いこと。
- Miri: `r.0.addr()` は provenance を捨ててアドレスだけを読む。CI の `-Zmiri-strict-provenance` で確認する。
