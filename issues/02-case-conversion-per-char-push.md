# `to_lowercase` / `to_uppercase` が最初の非 ASCII 文字以降を 1 文字ずつ `push_char` で書くため、`str` の同名メソッドより 2〜3 倍遅い

- 種別: perf
- 対象: `src/repr.rs` の `Repr::<Mutable>::to_case`、`src/case.rs`、`src/lib.rs` の `LeanString::to_lowercase` / `to_uppercase` (と `LeanStr` の同名メソッド)
- 影響: 変化のある文字を含む入力すべて。とくに非 ASCII 文字を 1 つでも含む入力 (先頭にアクセント付きの文字がある英文、
  欧州の言語の文章など) では、その文字より後ろの ASCII まで 1 文字ずつ処理され、`str::to_lowercase` の 2〜3 倍の時間がかかる。
  変化が無い入力 (確保しない経路) は速い。
- 優先度: 高

## 問題

`Repr::to_case` は次の順で処理する。

1. `case::split_at_first_change` で最初に変わる文字の位置を探す (変わらなければ `None` を返して clone になる)。
2. `case::split_ascii_prefix` で、そこから続く ASCII の部分を切り出し、`unchanged` のコピー (`memcpy`) と ASCII の変換をまとめて行う。
3. 残り (`rest`、最初の非 ASCII 文字から末尾まで) を `rest.chars()` で 1 文字ずつ回し、`c.to_lowercase().try_for_each(|l| mapped.push_char(l))` で書く。

3 で `rest` に ASCII が続いても 1 文字ずつ処理される。さらに `Repr::push_char` は `#[inline]` だが `to_case` の中では展開されず、別の関数 (probe の x86-64 asm で 124 命令) への呼び出しになる。その中で、
1 文字ごとに判別子の読み込み、容量の判定、`is_modifiable` (heap なら参照カウントの Acquire ロード) を繰り返す。
`mapped` は自分で作ったばかりの unique なバッファなので、これらの判定は不要である。

計測 (`5dd94ea`、rustc 1.97.0、x86_64):

既存ベンチ `to_lowercase` / `to_uppercase` (`bench/benches/apis.rs`、criterion、`--measurement-time 2`、中央値、ns):

| 入力 | `to_lowercase` current | std | `to_uppercase` current | std |
|---|---:|---:|---:|---:|
| ascii/16 | 47 | 29 | 66 | 22 |
| head_non_ascii/16 | 267 | 112 | 281 | 116 |
| non_ascii/16 | 383 | 311 | 493 | 246 |
| ascii/256 | 110 | 55 | 75 | 48 |
| head_non_ascii/256 | 2,946 | 1,025 | 2,255 | 883 |
| non_ascii/256 | 4,770 | 3,357 | 3,716 | 3,591 |

cachegrind の Ir (1 回あたり。どれも確保 1 回、ascii/16 の `LeanString` だけ確保 0 回):

| 入力 | `LeanString::to_lowercase` | `str::to_lowercase` |
|---|---:|---:|
| ascii/16 | 316 | 225 |
| head_non_ascii/256 (`"Éé"` + ASCII) | 33,438 | 16,768 |
| non_ascii/256 (キリル文字) | 35,076 | 26,218 |
| mixed/256 (`"Le Café Élégant à Paris, Straße "` × 8) | 38,167 | 21,800 |

callgrind の内訳 (head_non_ascii/256): `push_char` 38%、`to_case` 本体 33%、`conversions::to_lower` 17%、
`ToLowercase` のイテレータの `next` 10%。`conversions::to_lower` の呼び出し回数は `str` と同じなので、差はほぼ書き込み側にある。

## 原因

- `rest` の中の ASCII を一括で変換していない (コードで確認)。
- 書き込みが、文字ごとに容量と一意性を確かめ直す `push_char` の呼び出しになっている (asm と callgrind で確認)。
- `try_for_each` に渡すクロージャも呼び出しとして残る (callgrind に `Iterator::try_for_each::call` が出る)。

## 修正例

書き込み先のポインタ・長さ・容量をローカルに持ち、容量が足りないときだけ `grow_amortized` する。
`rest` の中でも ASCII が続く部分は `prefix_len_while` で長さを測って一括で変換する。

```rust
    pub(crate) fn to_case(text: &str, case: Case) -> Result<Option<Self>, ReserveError> {
        let Some((unchanged, changed)) = case::split_at_first_change(text, case) else {
            return Ok(None);
        };
        let mut mapped = Repr::with_capacity(text.len())?;
        match Self::write_case(&mut mapped, unchanged, changed, case) {
            Ok(true) => Ok(Some(mapped)),
            Ok(false) => {
                // SAFETY: `mapped` is not accessed again.
                unsafe { mapped.drop_in() };
                // 'Σ': let the standard library handle `Final_Sigma` (今と同じ)
                Repr::from_str(&text.to_lowercase()).map(Some)
            }
            Err(err) => {
                // SAFETY: `mapped` is not accessed again.
                unsafe { mapped.drop_in() };
                Err(err)
            }
        }
    }

    /// Returns `Ok(false)` when it meets 'Σ' while lowercasing.
    #[inline(always)]
    fn write_case(mapped: &mut Self, unchanged: &str, changed: &str, case: Case) -> Result<bool, ReserveError> {
        // `mapped` is a fresh buffer of `text.len()` bytes: not static, unique, and large enough
        // for `unchanged`. Keep its pointer, length and capacity in locals.
        let mut cap = mapped.capacity();
        let mut ptr = unsafe { mapped.as_mut_ptr() };
        unsafe { ptr::copy_nonoverlapping(unchanged.as_ptr(), ptr, unchanged.len()) };
        let mut len = unchanged.len();

        macro_rules! reserve {
            ($n:expr) => {
                if cap - len < $n {
                    unsafe { mapped.set_len(len) };
                    mapped.grow_amortized($n)?;
                    cap = mapped.capacity();
                    ptr = unsafe { mapped.as_mut_ptr() };
                }
            };
        }

        let mut rest = changed.as_bytes();
        while let Some(&b) = rest.first() {
            if b.is_ascii() {
                // An ASCII run: convert it in bulk (vectorized).
                let n = case::prefix_len_while(rest, |b| b.is_ascii());
                reserve!(n);
                let dst = unsafe { ptr.add(len) };
                for (i, &b) in rest[..n].iter().enumerate() {
                    let b = match case {
                        Case::Lower => b.to_ascii_lowercase(),
                        Case::Upper => b.to_ascii_uppercase(),
                    };
                    unsafe { dst.add(i).write(b) };
                }
                len += n;
                rest = &rest[n..];
                continue;
            }
            // SAFETY: `rest` starts at a char boundary of valid UTF-8 and is not empty.
            let c = unsafe { str::from_utf8_unchecked(rest).chars().next().unwrap_unchecked() };
            rest = &rest[c.len_utf8()..];
            macro_rules! write_all {
                ($iter:expr) => {
                    for m in $iter {
                        reserve!(m.len_utf8());
                        unsafe { write_char(ptr.add(len), m) };
                        len += m.len_utf8();
                    }
                };
            }
            match case {
                Case::Lower if c == 'Σ' => return Ok(false),
                Case::Lower => write_all!(c.to_lowercase()),
                Case::Upper => write_all!(c.to_uppercase()),
            }
        }
        // SAFETY: `0..len` was written with valid UTF-8 above, and `len <= cap`.
        unsafe { mapped.set_len(len) };
        Ok(true)
    }
```

(容量の判定は書く文字の長さちょうどで行う。`4` や `12` のような最大値で判定すると、長さの変わらない入力でも
末尾で余計な `realloc` が起きる。最初の試作で `cap - len < 4` としたら、キリル文字 128 文字の入力で `realloc` が 1 回起きた。)

試した結果 (同じバイナリに現状と並べて A/B):

| 入力 | Ir 現状 → 修正例 (`str`) | criterion 中央値 ns 現状 → 修正例 (`str`) |
|---|---|---|
| head_non_ascii/16 | 2,538 → 1,047 (1,408) | 417 → 138 (112) |
| head_non_ascii/256 | 33,438 → 1,504 (16,768) | 1,882 → 167 (900) |
| non_ascii/256 | 35,076 → 32,040 (26,218) | 5,183 → 3,568 (3,173) |
| mixed/256 | 38,167 → 15,909 (21,800) | 5,498 → 1,719 (1,495) |
| ascii/256 | 926 → 910 (465) | 102 → 65 (35) |
| `to_uppercase` head_non_ascii/256 | 32,681 → 1,525 (14,534) | — |

(criterion は LTO ありの `bench` プロファイル。負荷のある共有マシンなので ±20% 程度のノイズを見込む。
ascii/256 は Ir がほぼ同じなので、時間の差はコード配置などによるものと思われる (未検証)。
ascii/16 は [#12](./12-ascii-case-inline-fast-path.md) の inline の高速経路で別に扱う。
`case::prefix_len_while` は今は private なので `pub(crate)` にする。)

なお変化の無い入力は今でも速い (ASCII 小文字 256 バイトの `to_lowercase`: `LeanString` 320 Ir・確保 0、`str` 465 Ir・確保 1)。
修正例はこの経路 (`split_at_first_change` が `None`) を変えない。

`str` の同名メソッドとの比較、`U+0000..U+3000` のすべての Unicode スカラー値を 7 文字ずつ区切った入力、`Σ` を含む入力、
伸びる (`İ`, `ŉ`, `ΐ`)・縮む (Kelvin `K`, `ﬀ`) 入力で `to_lowercase` / `to_uppercase` の結果が `str` と一致し、
短い入力のテストは Miri (`-Zmiri-strict-provenance`) の x86_64 / i686 / powerpc で問題なし。

残る差について:

- non_ascii (キリル文字) はまだ `str` より 10〜20% 多い。`char::to_lowercase()` が返す `ToLowercase` の `next` が
  関数呼び出しのまま残るため (callgrind で確認)。`str` は非公開の `conversions::to_lower` を直接使っている。
- ascii/256 は `str` の約 2 倍の Ir のまま。最初の変化の位置を探す走査、ASCII の長さを測る走査、変換の 3 パスになるため。
  `str` は確保してから 1 パスで変換する。ASCII の部分を `usize` 単位の SWAR で「判定しながら変換」する 1 パス版も試したが、
  ascii/256 は約 910 → 1,178 Ir と悪化した (LLVM がベクトル化した変換ループのほうが速い)。mixed/256 は 15,909 → 12,639 Ir と改善した。

## 確認方法

- `bench/benches/apis.rs` の `case_samples` に、まばらに非 ASCII を含む入力 (上の mixed/256 など) と、
  変化の無い入力 (clone になる経路) を足す。`LeanStr` の行も足す。
- `to_lowercase` / `to_uppercase` の property テスト (`tests/property.rs`) は今のまま有効。伸びる・縮む・`Σ` を含む入力を
  固定の例としても足す。
- probe の asm で、`to_case` から `push_char` の呼び出しが消えていること。

## 関連

- 変換結果が 16 バイト以下に縮んだとき (Kelvin 記号 `U+212A` × 6 = 18 バイト → `"kkkkkk"` 6 バイト、トルコ語の `ı` の `to_uppercase` など)、
  `LeanString` の結果は heap (容量 18) のまま残る。上の修正の最後で `len <= MAX_INLINE_SIZE` なら inline に移せば揃う (まれなので優先度は低い)。
- `LeanStr` の `to_lowercase` / `to_uppercase` は結果を `LeanString` で作ってから `LeanStr` に変換するため、heap の結果ごとに `realloc` と全体の `memmove` が入る。これは [#14](./14-lean-str-built-via-lean-string.md) で扱う。
