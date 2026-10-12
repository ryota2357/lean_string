# バッファを共有している 2 つの値の `==` / `cmp` が中身を全部比べる

- 種別: perf
- 対象: `src/lib.rs` の `impl PartialEq for LeanString` (と `LeanString` / `LeanStr` を両辺に取る他の impl)、`impl Ord for LeanString`、`impl Ord for LeanStr`
- 影響: clone や、変更が無いときに元のバッファを共有して返す操作の結果を、元の値と比べるコード。長さ 17 以上で、比較が長さに比例する時間から一定時間になる
- 優先度: 低

## 問題

`LeanString` / `LeanStr` の clone は heap バッファを共有する。static バッファの値は `&'static str` と同じポインタを持つ。
変更が無いときに元のバッファを共有して返す操作も多い。`repeat(1)`、`to_lowercase` / `to_uppercase` / `to_ascii_*case` (変化が無いとき)、
`LeanStr` の `to_lean_str()` (`ToLeanStr`) などがそうである。

同じバッファの同じ範囲を指していれば内容は必ず等しいのに、今の `==` / `cmp` は `as_str()` 同士を比べるので、
`bcmp` / `memcmp` が全体を読む。

時間 (ns)。`5dd94ea` に試作を足したもの、Intel Xeon 2.8 GHz、`bench` の release プロファイル (LTO あり) で測った。
1 つのバイナリで交互に 25 ラウンド回し、その最小値を取った。「修正例」は長さが 16 を超えるときだけポインタを見る版
(下の修正例を [#11](./11-short-eq-bcmp.md) の `str_eq` に入れたもの) である。

| `LeanString == LeanString` | 現状 | 修正例 |
| --- | --- | --- |
| clone 同士、長さ 17 | 3.10 | 2.42 |
| clone 同士、長さ 32 | 3.30 | 1.97 |
| clone 同士、長さ 256 | 5.66 | 2.36 |
| 別の値、長さ 17 / 32 / 256 で等しい | 3.22 / 3.21 / 5.81 | 2.86 / 2.89 / 4.98 |
| 別の値、長さ 32 の先頭が不一致 | 3.72 | 3.30 |

| `LeanString::cmp` (常にポインタを見る版) | 現状 | ポインタを見る版 |
| --- | --- | --- |
| clone 同士、長さ 17 / 32 / 256 | 3.20 / 3.17 / 5.57 | 2.11 / 2.04 / 1.99 |
| 別の値で等しい、長さ 8 / 17 / 32 | 3.10 / 3.16 / 3.26 | 3.33 / 3.19 / 3.25 |
| 先頭が不一致、長さ 8 / 17 | 3.68 / 3.81 | 3.80 / 3.98 |

## 原因

`str` / `[u8]` の比較は長さしか見ず、データポインタが同じでも全体を比べる (`probe_std_str_eq` の asm で確認)。
普通の `&str` ではポインタが一致することはまれなので、core がそうしないのは妥当である。
しかし `LeanString` / `LeanStr` は参照カウントでバッファを共有する型なので、一致はよく起こる。
std の `Arc<T>` も、`T: Eq` なら `Arc::ptr_eq` を先に見るように特殊化されている。

正しく判定するための注意点が 2 つある。

- 長さを先に比べなければならない。64-bit の `Repr::truncate_unchecked` は、共有された heap バッファに対して
  ハンドル側の長さだけを縮める。`StaticBuffer::set_len` も同じである。そのため「ポインタは同じで長さが違う」ハンドルがありうる
  (`tests/lean_string.rs` の `truncate_share_buffer` がそれを作っている)。
- `cmp` では、ポインタが同じなら短いほうがもう一方の接頭辞なので、順序は長さの比較で決まる。ここで `Equal` を返すと誤りになる。

inline の値では、データポインタは値そのもののアドレスなので、別の値と一致することはない。
inline 同士でポインタの比較を常に入れると、その分が無駄になる。測ると差はほとんど無かったが、
`LeanStr` の短い識別子 64 個の線形探索では、常にポインタを見る版が 3.25〜3.83 µs になった。
現状は 2.46〜3.10 µs で、繰り返し測っても遅いほうに出た。長さが inline の上限を超えるときだけ見る版 ([#11](./11-short-eq-bcmp.md) の方法と組み合わせたもの) は 2.16 µs だった。
原因 (ループのコード配置か分岐か) は突き止めていない。

## 修正例

ポインタを見るのは長さが `MAX_INLINE_SIZE` を超えるときだけにする。短い文字列は比べても安いので、近道の意味が無い。
`str` の中身だけで判定できる (ポインタと長さが同じ 2 つの `&str` は必ず等しい) ので、相手の型を問わず使える。

```rust
// src/repr.rs: #11 の str_eq を使う場合、長い経路だけを変える
if n > MAX_INLINE_SIZE {
    return ptr::eq(a.as_ptr(), b.as_ptr()) || a == b;
}

// str_eq を入れない場合の単独の形 (この形そのものは測っていない)
#[inline]
pub(crate) fn str_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && ((a.len() > MAX_INLINE_SIZE && ptr::eq(a.as_ptr(), b.as_ptr())) || a == b)
}

#[inline]
pub(crate) fn str_cmp(a: &str, b: &str) -> cmp::Ordering {
    if ptr::eq(a.as_ptr(), b.as_ptr()) {
        // One is a prefix of the other: handles sharing a buffer may have different lengths.
        a.len().cmp(&b.len())
    } else {
        a.cmp(b)
    }
}
```

`cmp` については、常にポインタを見る版だけを測った (上の表)。clone でない値では +0.1〜0.2 ns で、ばらつきと同程度である。
`==` と同じように、短いときは見ない形にもできる。`LeanStr` 同士の `==` については [#10](./10-leanstr-word-eq.md) の
`content_eq` が、inline 同士でないときにポインタを見る形になっている。

公開 API としての `ptr_eq` は [#52](./52-sharing-introspection.md) で扱う。

確認したこと: `truncate` で「ポインタが同じで長さが違う」ハンドルを heap と static の両方で作り、`==` と `cmp` が
`str` と一致することを試作で確かめた (proptest、Miri `-Zmiri-strict-provenance` を x86_64 / i686 / powerpc64 で実行)。

## 確認方法

- テスト: [#27](./27-test-eq-ord-hash.md) の `shared_buffer_with_different_lengths`。heap と static の両方で、
  ポインタが同じで長さが違う 2 値に対し `assert_ne!` と `cmp` の向きを確認する。
- ベンチ: `apis.rs` の `eq/cloned` (長さ 17 / 100) で、clone 同士が長さによらず一定になること。
  先頭のバイトで不一致になるケース ([#11](./11-short-eq-bcmp.md) の `eq/mismatch`) が遅くならないことも見る。
