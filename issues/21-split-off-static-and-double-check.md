# `split_off` が static バッファの後半をコピーし、境界チェックを 2 回する

- 種別: perf
- 対象: `src/lib.rs` の `LeanString::try_split_off` (`split_off` も)
- 影響: `from_static_str` で作った文字列を `split_off` すると、後半が 17 バイト以上なら heap 確保が起きる (確保しなくてよい)。
  どのバッファでも、文字境界チェックが 2 回走り、その間に unwind 用の landing pad ができる。
- 優先度: 低

## 問題

```rust
    pub fn try_split_off(&mut self, at: usize) -> Result<Self, ReserveError> {
        let other = LeanString(Repr::from_str(&self.as_str()[at..])?);
        self.try_truncate(at)?;
        Ok(other)
    }
```

1. `self` が `&'static str` を持っているとき、その後半 `&s[at..]` も `&'static str` なので、`from_static_str` で
   コピーせずに持てる。今は `Repr::from_str` で毎回コピーする。
2. `&self.as_str()[at..]` のスライスで `at` の境界チェックをしたあと、`try_truncate` (`Repr::truncate`) がもう一度
   `is_char_boundary` で assert する。2 回目の assert は `other` を作ったあとなので、panic したときに `other` を
   drop するための landing pad (`_Unwind_Resume`、`panic_in_cleanup`) も生成される。

`5dd94ea`、rustc 1.97.0、LTO off の probe (`s.split_off(at)`) の x86-64 asm: 194 命令。`cmpb $-65, (...)` の
境界チェックが 2 か所、`panic_fmt` (truncate の assert) と `_Unwind_Resume` を含む。

確保回数とcachegrind の Ir (1 回あたり、65 バイトの文字列の clone を `split_off(10)`):

| | 現状 | 下の修正例 |
|---|---|---|
| static | 確保 1 回、267 Ir | 確保 0 回、79 Ir |
| 共有 heap | 確保 1 回、306 Ir | 確保 1 回、302 Ir |
| unique heap (生成を含む) | 確保 2 回、517 Ir | 確保 2 回、513 Ir |

## 原因

`try_split_off` が static バッファを区別していないことと、境界チェック済みの `at` で公開 API の `try_truncate` を
呼んでいること (asm で確認)。

## 修正例

```rust
    pub fn try_split_off(&mut self, at: usize) -> Result<Self, ReserveError> {
        // Panics here if `at` is not a char boundary or out of bounds.
        let tail = &self.as_str()[at..];
        let other = match self.as_static_str() {
            // A suffix of a `&'static str` is a `&'static str` too.
            // SAFETY: `at` was checked by the slicing above.
            Some(s) => Repr::from_static_str(unsafe { s.get_unchecked(at..) })?,
            None => Repr::from_str(tail)?,
        };
        let other = LeanString(other);
        if at < self.len() {
            // SAFETY: `at` is a char boundary and less than `len()`, checked above.
            unsafe { self.0.truncate_unchecked(at) }?;
        }
        Ok(other)
    }
```

(`Repr::truncate_unchecked` を `pub(crate)` にする。`at < self.len()` の判定は、32-bit で長さが heap にある共有バッファで
`at == len` のときに無駄なコピーをしないため。)

試した結果: probe の asm で境界チェックが 1 回になり、`panic_fmt` / `_Unwind_Resume` が消えた (命令数は static の分岐が
増えたぶん 194 → 217)。Ir と確保回数は上の表のとおり。`tests/alloc_string.rs` の `split_off` 系と、
`split_off` を unique / 共有 / static で `String` と突き合わせる property テストが通る。

## 確認方法

- static バッファの `split_off` で、返り値の `as_static_str()` が `Some` になること (後半が 17 バイト以上のとき) を
  確かめるテストを足す。
- probe の asm で `cmpb $-65` (境界チェック) が 1 か所、`_Unwind_Resume` が無いこと。
