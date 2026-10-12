# `Extend<char>` / `FromIterator<char>` / `from_utf16*` が 1 文字ずつ `push` し、`String` の 1.7〜3 倍の命令を使う

- 種別: perf
- 対象: `src/lib.rs` の `impl Extend<char> for LeanString`、`impl FromIterator<char> for LeanString`
  (`&char` 版と `LeanStr` 版はこれに委譲)、`LeanString::from_utf16_units`、`LeanString::from_utf16_units_lossy`
  (`from_utf16` / `from_utf16_lossy` / `from_utf16le*` / `from_utf16be*` と `LeanStr` の同名関数が使う)
- 影響: `chars().collect::<LeanString>()`、`s.extend(iter_of_chars)`、UTF-16 からの変換。
  1000 文字で `String` の約 1.7〜3 倍の命令数になる。再確保の回数も `String` より多い
- 優先度: 高

## 問題

上の関数はどれも「容量を見積もって確保し、1 文字ごとに `LeanString::push` を呼ぶ」形になっている。

```rust
// impl Extend<char> for LeanString
let _ = self.try_reserve(lower_bound);
for ch in iter {
    self.push(ch);
}
```

`5dd94ea`、rustc 1.97.0、x86-64、LTO なしの下流クレートで asm を見ると、
`s.extend(t.chars())` のループ本体は UTF-8 のデコードの後に毎回 `callq ...push_char` を呼ぶ
(`Repr::push_char` は 124 命令で展開されない。[#07](./07-push-char-not-inlined.md))。`push_char` の中では毎回、`len()` の復元、
バッファの種類による分岐、heap なら容量と参照カウントのロード、`as_mut_ptr` と `set_len` での
判別子の読み直しが行われる。`String` の `Extend<char>` は `push` が展開され、ループ内の関数呼び出しは容量が足りないときの
`do_reserve_and_handle` だけになる。

callgrind の命令数 (1 回の呼び出しあたり。入力はループの外で作る。ascii は 1000 バイト、
ja は日本語 1000 文字 = 3000 バイト):

| 操作 | `LeanString` | `String` |
| --- | ---: | ---: |
| `ascii.chars().collect()` | 62,549 | 20,976 |
| `ja.chars().collect()` | 100,419 | 58,162 |
| `LeanString::new()` に `extend(ja.chars())` | 100,475 | 58,162 |
| `from_utf16` (ascii, 1000 単位) | 63,252 | 26,219 |
| `from_utf16` (ja, 1000 単位) | 87,167 | 48,885 |
| `from_utf16_lossy` (ja) | 90,170 | 49,222 |
| `LeanStr::from_utf16` (ja) | 89,141 | - |

確保回数 (確保を数える `#[global_allocator]` で計測):

| 操作 | `LeanString` | `String` |
| --- | --- | --- |
| `from_utf16` 日本語 24 文字 (72 バイト) | alloc 1, realloc 3 | alloc 1, realloc 2 |
| `from_utf16` 日本語 960 文字 (2880 バイト) | alloc 1, realloc 3 | alloc 1, realloc 2 |
| `LeanStr::from_utf16` 日本語 24 文字 | alloc 1, realloc 4 | - |
| `chars().collect()` ASCII 55 文字 | alloc 1, realloc 3 | alloc 1, realloc 2 |
| `chars().collect()` 日本語 24 文字 | alloc 1, realloc 4 | alloc 1, realloc 2 |

## 原因

1. 1 文字ごとに追記先を判定し直している。書き込み先 (inline か heap か、共有されていないか) は
   ループの最初に一度決まれば変わらないが、`push` は毎回 `self` のメモリから判別子と長さを読み、
   heap なら参照カウントを `Acquire` でロードする。書き込みは `self` (inline のとき) か `self` から
   読んだポインタの先に行われ、`Acquire` のロードもあるので、LLVM は長さや判別子をレジスタに
   保持したままループを回せない (asm で毎回のロードを確認。理由は推測)。
2. `push_char` が展開されないので、1 文字ごとに関数呼び出しが入る ([#07](./07-push-char-not-inlined.md))。
3. 再確保の回数が多いのは、容量の見積もりが文字数 (UTF-16 なら単位数、`Chars` なら
   `size_hint().0 = (バイト数 + 3) / 4`) で、非 ASCII では UTF-8 のバイト数より小さいため。
   見積もりの方法は `String::from_utf16` / `String::from_iter` と同じだが、`LeanString` は 1.5 倍で
   伸びる (`Repr::grow_amortized`) ので `String` (2 倍) より 1〜2 回多くなる。`LeanStr` 版は
   `LeanString` で作ってから `into_lean_str` するので、もう 1 回 `realloc` が足される。

## 修正例

文字をスタック上の小さなバッファにエンコードし、いっぱいになったら `push_str` でまとめて追記する。
1 文字あたりの処理はローカル変数だけで済み、`self` に触るのは 60 バイトごとの `push_str` だけになる。
unsafe はスタックのバッファへの書き込みと `from_utf8_unchecked` だけで、`Repr` の不変条件には触れない。

```rust
impl Repr<Mutable> {
    /// Appends every char of `iter`.
    ///
    /// The chars are encoded into a buffer on the stack and appended with `push_str` when it is
    /// almost full, so that the per-char work doesn't touch `self`.
    pub(crate) fn extend_chars(
        &mut self,
        iter: impl Iterator<Item = char>,
    ) -> Result<(), ReserveError> {
        const CHUNK: usize = 64;
        let mut buf = [0u8; CHUNK];
        let mut n = 0;
        for ch in iter {
            // SAFETY: `n <= CHUNK - 4` here, so `buf[n..]` has room for any char.
            unsafe { write_char(buf.as_mut_ptr().add(n), ch) };
            n += ch.len_utf8();
            if n > CHUNK - 4 {
                // SAFETY: `buf[..n]` is a sequence of whole UTF-8 encoded chars.
                self.push_str(unsafe { str::from_utf8_unchecked(&buf[..n]) })?;
                n = 0;
            }
        }
        // SAFETY: Same as above.
        self.push_str(unsafe { str::from_utf8_unchecked(&buf[..n]) })
    }
}

// impl Extend<char> for LeanString
fn extend<T: IntoIterator<Item = char>>(&mut self, iter: T) {
    let iter = iter.into_iter();
    let (lower_bound, _) = iter.size_hint();
    let _ = self.try_reserve(lower_bound);
    self.0.extend_chars(iter).unwrap_with_msg();
}
```

`FromIterator<char>` と `from_utf16_units_lossy` も同じ形にする。エラーで止まる `from_utf16_units` は
`map_while` でエラーを記録しながら渡せばよい。

```rust
let mut lone_surrogate = false;
let chars = char::decode_utf16(units).map_while(|c| match c {
    Ok(c) => Some(c),
    Err(_) => {
        lone_surrogate = true;
        None
    }
});
ret.0.extend_chars(chars).unwrap_with_msg();
if lone_surrogate {
    return Err(FromUtf16Error { kind: FromUtf16ErrorKind::LoneSurrogate });
}
```

試した結果 (同じ条件の callgrind):

| 操作 | 現状 | 修正例 | `String` |
| --- | ---: | ---: | ---: |
| `ascii.chars().collect()` | 62,549 | 13,401 | 20,976 |
| `ja.chars().collect()` | 100,419 | 49,791 | 58,162 |
| `extend(ja.chars())` | 100,475 | 49,845 | 58,162 |
| `from_utf16` (ascii) | 63,252 | 19,144 | 26,219 |
| `from_utf16` (ja) | 87,167 | 40,666 | 48,885 |
| `from_utf16_lossy` (ja) | 90,170 | 45,717 | 49,222 |
| `LeanStr::from_utf16` (ja) | 89,141 | 42,653 | - |

どれも `String` より少なくなった。まとめて追記するので再確保も減る (`collect` ASCII 55 文字は
realloc 3 → 0、日本語 24 文字は 4 → 2、`from_utf16` 日本語 24 文字は 3 → 2)。
criterion (`bench/` の設定なので LTO あり。`current` が修正例、`prev` は 0.8.0 = `main` と同じコード。
負荷のある共有マシンなので値は粗い) でも同じ傾向だった。

| ベンチ | current (修正例) | prev | std |
| --- | ---: | ---: | ---: |
| `collect/chars` ascii 256 バイト | 934 ns | 3.27 µs | 1.26 µs |
| `collect/chars` ja 152 文字 (2 回) | 623 / 694 ns | 1.27 / 1.09 µs | 1.02 µs / 949 ns |
| `from_utf16` ascii 256 単位 | 463 ns | 1.81 µs | 1.58 µs |
| `from_utf16` ja 152 単位 (2 回) | 572 / 712 ns | 995 ns / 1.22 µs | 762 / 812 ns |

`collect/chars` ja は最初の計測だけ current 2.02 µs、prev 1.75 µs と逆になったが、同じバイナリで
測り直した 2 回は上のとおりだった。

`cargo +nightly test` は全件通過し、Miri (`-Zmiri-strict-provenance`, x86_64) で
`tests/alloc_string.rs` の utf16 / utf8_lossy / extend / from_iter / push のテストも通った。

注意点:

- イテレータが途中で panic すると、スタックのバッファに残っていた最大 60 バイト分の文字は追記されない。
  文字列は有効なまま (UTF-8 として正しい) だが、`String` (panic までの文字がすべて入る) とは
  観測できる結果が変わる。気にするなら、`Drop` で残りを `push_str` するガードを置く
  (このとき確保の失敗は無視するしかない)。
- `from_utf16` は `#[inline]` なので、修正後は呼び出し側の asm が大きくなる
  (x86-64 の `probe_from_utf16` が 103 → 392 命令)。`from_utf16_units` / `from_utf16_units_lossy` を
  `#[inline]` にしない (または `extend_chars` を `#[inline(never)]` にする) ことで抑えられる。

他の案:

- 書き込み先のポインタ・長さ・容量をローカル変数に持って直接書く writer。中間のコピーは無くなるが、
  inline バッファでは書き込みがバイト 15 (判別子と長さ) を上書きするので、panic したときに
  `set_len` で必ず確定させるガードが要る。上の案で `String` を下回ったので、そこまでは要らないと考える。
- `from_utf16` で UTF-8 の長さを先に数え (`1 + (u >= 0x80) + (u >= 0x800 && u & 0xF800 != 0xD800)` の和)、
  正確な容量で 1 回だけ確保する案も試した。realloc は 0 回になったが、callgrind の命令数は
  上の案より増えた (ascii 19,144 → 28,417、ja 40,666 → 49,218)。数える処理は SSE2 でベクトル化されて
  いたので、増えた理由は確認できていない。既定の方法としては勧めない。ただし `LeanStr::from_utf16` では、
  正確な長さが分かれば `Repr::<Immutable>::new_with` で直接作れて `into_lean_str` の `realloc` を
  無くせるので、そちらを検討する余地はある。

小さな関連: `from_utf16le_lossy` / `from_utf16be_lossy` は奇数バイトのとき、作り終えた文字列に
`+ "\u{FFFD}"` で 3 バイト足す。容量はコード単位数で見積もっているので、ASCII の入力ではちょうど
埋まった後にこの追記が来て必ず `realloc` になる (ASCII 55 単位 + 1 バイトで alloc 1, realloc 1)。
見積もりに 3 を足して `from_utf16_units_lossy` に渡し、末尾の U+FFFD も同じバッファに書けば無くなる。

## 確認方法

- callgrind / ベンチ: 今は char 列の追記のベンチが無いので `bench/benches/apis.rs` に足す。

  ```rust
  fn char_samples() -> [(&'static str, String); 2] {
      [("ascii", ascii(256)), ("ja", "日本語のテキストを変換するテストです。".repeat(8))]
  }

  fn collect_chars(c: &mut Criterion) {
      let mut group = c.benchmark_group("collect/chars");
      for (name, text) in char_samples() {
          duplicate! {
              [
                  label         StrTy;
                  ["current"]   [lean_string::LeanString];
                  ["prev"]      [lean_string_prev::LeanString];
                  ["std"]       [String];
              ]
              group.bench_with_input(BenchmarkId::new(label, name), &name, |b, _| {
                  b.iter(|| black_box(text.as_str()).chars().collect::<StrTy>())
              });
          }
      }
      group.finish();
  }

  fn from_utf16(c: &mut Criterion) {
      let mut group = c.benchmark_group("from_utf16");
      for (name, text) in char_samples() {
          let units: Vec<u16> = text.encode_utf16().collect();
          duplicate! {
              [
                  label         StrTy;
                  ["current"]   [lean_string::LeanString];
                  ["prev"]      [lean_string_prev::LeanString];
                  ["std"]       [String];
              ]
              group.bench_with_input(BenchmarkId::new(label, name), &name, |b, _| {
                  b.iter(|| StrTy::from_utf16(black_box(&units)).unwrap())
              });
          }
      }
      group.finish();
  }
  ```

- asm: `pub fn probe_extend_chars(s: &mut LeanString, t: &str) { s.extend(t.chars()) }` のループに
  `push_char` の呼び出しや判別子 (`15(%reg)`) のロードが無いこと。
- テスト: `tests/property.rs` には `collect_from_chars` はあるが `extend` 側のプロパティが無いので足す
  (inline / heap / static / 共有の各状態から `extend(chars)` して `String` と一致すること)。
  バッファの境界 (60 バイト前後、4 バイト文字で境界をまたぐ) を含む入力も入れる。
  `from_utf16` の途中に孤立サロゲートがある入力でエラーになることは既存のテストで確認できる。
- Miri: 4 ターゲット。

## 関連

- `LeanString::push` が展開されない件は [#07](./07-push-char-not-inlined.md)。この修正はそちらと独立に効く。
- `Repr::to_case` の非 ASCII 部分 (`c.to_lowercase().try_for_each(|l| mapped.push_char(l))`) も同じ
  1 文字ずつの追記なので、`extend_chars` を使える ([#02](./02-case-conversion-per-char-push.md))。
- `LeanStr` の `FromIterator` / `from_utf16*` / `from_utf8_lossy` は `LeanString` を作ってから
  `into_lean_str` するので、heap のときは必ず `realloc` が 1 回足される
  ([#14](./14-lean-str-built-via-lean-string.md)、`from_utf8_lossy` は [#05](./05-from-utf8-lossy-fast-path.md))。
