# `LeanString::drain` が無い (`tests/alloc_string.rs` の drain テストはコメントアウトのまま)

- 種別: api, test
- 対象: `src/lib.rs` の `LeanString`、`tests/alloc_string.rs` の `test_drain` / `test_drain_start_overflow` / `test_drain_end_overflow`
- 影響: `String::drain(range)` (範囲を取り除きつつ、取り除く文字を iterator で受け取る) が使えない。`replace_range(range, "")` で削除だけはできるが、取り除いた文字を受け取るには先に `split_off` / コピーが必要になる。
- 優先度: 中

## 問題

`5dd94ea` で `LeanString` に `drain` は無い (`no method named drain`)。一方 `String` の `drain` が通るテストは `tests/alloc_string.rs` にコピーされ、`String` のまま `// #[test] fn test_drain()` とコメントアウトされている。同じファイルで 0.8.0 の `replace_range` / `extend_from_within` / `reserve_exact` のテストは有効になっている。`drain` が未実装の API として残っている目印である。

`compact_str` 0.10.0 には `CompactString::drain(range) -> Drain<'_>` がある (ローカルの registry のソースで確認)。

## 原因

`drain` は `&mut self` を借りる `Drain` 型が必要で、CoW の観点では最初に unique にしておく必要がある (iterator が借りている間に buffer が複製されないようにする) ので、他のメソッドより実装が一手間多い。0.8.0 の追加分 (`replace_range` など) から漏れたと思われる (推測)。

## 修正例

`std::string::Drain` と同じ形 (範囲を指す `Chars` と、`Drop` で範囲を取り除く)。`replace_range` の機構と `Repr::ensure_modifiable` が既にあるので追加の unsafe は少ない。試作して、コメントアウトされているテストと同じ内容 (`LeanString` に直したもの) + 共有・static・`rev()`・途中で drop・`mem::forget` の各ケースを通した。通常実行と Miri (`-Zmiri-strict-provenance`) の両方でパスした:

```rust
pub struct Drain<'a> {
    string: NonNull<LeanString>,
    start: usize,
    end: usize,
    iter: str::Chars<'a>,
    _marker: PhantomData<&'a mut LeanString>,
}

impl LeanString {
    pub fn try_drain(&mut self, range: impl RangeBounds<usize>) -> Result<Drain<'_>, ReserveError> {
        let Range { start, end } = slice_range(self.as_str(), range); // 既存の関数 (境界・順序の assert 付き)
        // Drain が借りている間 buffer が動かないように、先に unique にする。
        self.0.ensure_modifiable()?;
        let string = NonNull::from(&mut *self);
        // SAFETY: buffer は unique で、Drain::drop 以外は変更しない。iter は drop の前に使い終わる。
        let iter = unsafe { &*(&self.as_str()[start..end] as *const str) }.chars();
        Ok(Drain { string, start, end, iter, _marker: PhantomData })
    }
    #[track_caller]
    pub fn drain(&mut self, range: impl RangeBounds<usize>) -> Drain<'_> {
        self.try_drain(range).unwrap_with_msg()
    }
}

impl Drop for Drain<'_> {
    fn drop(&mut self) {
        // unique なので、空文字列への replace_range はその場で縮むだけで確保しない。
        unsafe { let _ = self.string.as_mut().0.replace_range(self.start..self.end, ""); }
    }
}
// Iterator<Item = char> + DoubleEndedIterator は iter に委譲。
```

`std` との違い: `std` は `Drain` の生成時に `set_len(start)` して leak amplification を避けるが、この案は生成時に長さを変えず、`mem::forget(drain)` すると取り除かれずに残る (安全だが不完全、試作のテスト `drain_shared_and_static_and_rev` で確認)。`ensure_modifiable` が確保しうるので、`try_drain` と `drain` を両方用意する (他の変更系メソッドと同じ流儀)。

## 確認方法

`tests/alloc_string.rs` のコメントアウトされた 3 テストを `LeanString` に直して有効化し、さらに共有 clone が不変なこと (`drain` 後の元の clone の内容)、静的文字列への呼び出し、`rev()`、途中 drop を足す。`cargo +nightly miri test --test alloc_string drain` で確認する。
