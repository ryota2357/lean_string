# `impl fmt::Write for LeanString` が `write_char` を実装しておらず、1 文字ごとに `memcpy` を呼ぶ

- 種別: perf
- 対象: `src/lib.rs` の `impl fmt::Write for LeanString`
- 影響: `write_char` を直接呼ぶコードと、`write!(s, "{}", c)` のように `char` を幅指定なしで書式化するコード。
  1 文字あたり `String` の約 4 倍の命令数になる
- 優先度: 低

## 問題

`impl fmt::Write for LeanString` は `write_str` と `write_fmt` だけを実装している。`write_char` は
`fmt::Write` の既定の実装 (`self.write_str(c.encode_utf8(&mut [0; 4]))`) になり、
`LeanString::push_str` を通る。`push_str` のコピーは長さが実行時に決まる `copy_nonoverlapping` なので、
1〜4 バイトのために `memcpy` を呼ぶ。

`5dd94ea`、rustc 1.97.0、x86-64、LTO なしの下流クレートで
`pub fn probe_write_char(s: &mut LeanString, c: char) -> bool { s.write_char(c).is_ok() }` を出力すると、
`<LeanString as fmt::Write>::write_char` (103 命令) を呼び、その中に `callq *memcpy@GOTPCREL(%rip)` がある。
`String` は `write_char` を `self.push(c)` で上書きしており、同じ probe は展開されて `memcpy` を呼ばない。
`LeanString::push` は `c5c62e9` 以降 `memcpy` を使わない (`Repr::push_char` の `write_char` ヘルパ) ので、
`fmt::Write` だけがその経路を使っていない。

`impl Display for char` は幅も精度も指定されていないとき `f.write_char(*self)` を呼ぶので、
`write!(s, "{}", c)` も同じ経路を通る。

callgrind の命令数 (`black_box('a')` を 1000 回):

| 操作 | `LeanString` | `String` |
| --- | ---: | ---: |
| `s.write_char('a')` × 1000 | 74,988 | 18,189 |
| `write!(s, "{}", 'a')` × 1000 | 163,997 | 109,206 |

## 原因

`write_char` の上書きが無いこと (コードで確認)。

## 修正例

```rust
impl fmt::Write for LeanString {
    #[inline]
    fn write_char(&mut self, c: char) -> fmt::Result {
        self.push(c);
        Ok(())
    }
    // write_str / write_fmt は今のまま
}
```

試した結果 (同じ条件の callgrind):

| 操作 | 現状 | `write_char` を追加 | さらに `push` に ASCII の fast path ([#07](./07-push-char-not-inlined.md)) |
| --- | ---: | ---: | ---: |
| `s.write_char('a')` × 1000 | 74,988 | 59,060 | 36,969 |
| `write!(s, "{}", 'a')` × 1000 | 163,997 | 153,067 | 135,974 |

criterion (LTO あり、64 文字、1 回の計測で粗い。`current` は両方の修正を入れたもの) では
`write_char/current` 730 ns、`prev` (`main` と同じコード) 971 ns、`std` 424 ns だった。

`memcpy` の呼び出しは無くなる。`write_char` 単体では `Repr::push_char` (展開されない) を呼ぶ形になるので、
`push` が展開されない件と合わせて直すと効果が大きい。

## 確認方法

- asm: 上の `probe_write_char` に `memcpy` の呼び出しが無いこと。
- ベンチ: `apis.rs` に `write_char` のベンチを足す。

  ```rust
  fn write_char(c: &mut Criterion) {
      use std::fmt::Write;
      let mut group = c.benchmark_group("write_char");
      duplicate! {
          [
              label         StrTy;
              ["current"]   [lean_string::LeanString];
              ["prev"]      [lean_string_prev::LeanString];
              ["std"]       [String];
          ]
          group.bench_function(label, |b| {
              b.iter(|| {
                  let mut s = StrTy::new();
                  for _ in 0..black_box(64) {
                      s.write_char(black_box('a')).unwrap();
                  }
                  s
              })
          });
      }
      group.finish();
  }
  ```

- テスト: `write_char` で 1〜4 バイトの文字を inline → heap をまたいで書き、`String` と一致すること。

## 関連

- `LeanString::push` が呼び出し側に展開されない件は [#07](./07-push-char-not-inlined.md)。
