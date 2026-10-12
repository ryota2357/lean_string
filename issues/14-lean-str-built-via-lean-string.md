# `LeanStr` の構築関数のいくつかが `LeanString` で作ってから `into_lean_str` し、heap の結果ごとに `realloc` と全体の移動が入る

- 種別: perf
- 対象: `src/lib.rs` の `LeanStr::from_utf16` / `from_utf16_lossy` / `from_utf16le` / `from_utf16le_lossy` /
  `from_utf16be` / `from_utf16be_lossy`、`LeanStr::to_lowercase` / `to_uppercase`
  (`Repr::<Mutable>::to_case` と `LeanString::into_lean_str` の組み合わせ)、`impl FromIterator<_> for LeanStr`、
  `impl ToLeanStr for T` の `Display` へのフォールバック
- 影響: 結果が heap に載る長さ (64-bit で 17 バイト以上) のとき、確保 1 回で済む場面で、確保の後に `realloc` 1 回と
  文字列全体の `ptr::copy` が追加で走る。よくある入力 (ASCII の UTF-16、長さの変わらない大文字・小文字変換) ほど無駄がはっきりする
- 優先度: 低

## 問題

これらは `LeanString` で作ってから `into_lean_str()` している。

```rust
pub fn from_utf16(buf: &[u16]) -> Result<Self, FromUtf16Error> {
    LeanString::from_utf16(buf).map(|x| x.into_lean_str())
}
pub fn to_lowercase(&self) -> Self {
    Repr::to_case(self, Case::Lower)
        .unwrap_with_msg()
        .map_or_else(|| self.clone(), |lower| LeanString(lower).into_lean_str())
}
```

`into_lean_str()` は unique な heap なら `Repr::into_immutable` → `HeapBuffer::into_exact` で変換する。
`LeanString` の heap は `GrowableHeader` (参照カウント + 容量、64-bit で 16 バイト)、`LeanStr` の heap は
`ExactHeader` (参照カウントだけ、8 バイト) をデータの前に置くので、`into_exact` はヘッダの大きさの差の分だけ
文字列全体を `ptr::copy` で前に詰めてから `realloc` で縮める。容量がちょうど長さと同じでも省けない (コードで確認)。

確保を数える `#[global_allocator]` で測った (`5dd94ea`、x86_64、release、48 バイト / 48 単位の入力)。

| 操作 | alloc | realloc | 最小 |
| --- | --- | --- | --- |
| `LeanStr::from_utf8(妥当な 48 バイト)` (比較用) | 1 | 0 | 1 / 0 |
| `LeanStr::from_utf16(ASCII 48 単位)` | 1 | 1 | 1 / 0 |
| `LeanStr::from_utf16_lossy` / `from_utf16le` (同) | 1 | 1 | 1 / 0 |
| `LeanStr::from_utf16(CJK 24 単位 = 72 バイト)` | 1 | 4 | 1 / 0 |
| `LeanStr::to_lowercase(ASCII 大文字 48 バイト)` | 1 | 1 | 1 / 0 |
| `LeanStr::to_ascii_lowercase` (比較用、`Repr::new_with` で直接作る) | 1 | 0 | 1 / 0 |
| `LeanStr::to_lowercase("Σ" × 20)` | 3 | 1 | 2 / 0 |

大文字・小文字変換は、確保回数と cachegrind の Ir (1 回あたり) でも `LeanString` 版と比べた (256 バイト)。

| 入力 (256 バイト) | 型 | 確保 | `realloc` | Ir |
|---|---|---:|---:|---:|
| ランダムな印字可能 ASCII | `LeanString::to_lowercase` | 1 | 0 | 926 |
| 同上 | `LeanStr::to_lowercase` | 1 | 1 | 1,202 |
| `"Éé"` + ASCII | `LeanString::to_lowercase` | 1 | 0 | 33,438 |
| 同上 | `LeanStr::to_lowercase` | 1 | 1 | 33,714 |

`LeanStr::repeat` と `LeanStr::to_ascii_lowercase` / `to_ascii_uppercase` は `Repr::new_with` で
`Repr<Immutable>` を直接作っており、この往復は無い。`LeanStr::from_utf8_lossy` も同じ形だが、そちらは
[#05](./05-from-utf8-lossy-fast-path.md) で扱う。

## 原因

`Repr::with_capacity` や `push_*`、`Repr::to_case` が `Repr<Mutable>` 専用なので、長さが先に分かる (か、安く分かる)
場合でも `LeanString` 側の実装を流用している。`LeanStr` 向けに直接作るには、長さを先に求めて `Repr::new_with` を使う必要がある。
`into_exact` はヘッダの大きさが違う 2 つのレイアウトの間の移動なので、長さがちょうどでも `ptr::copy` と `realloc` は避けられない。

## 修正例

### `to_lowercase` / `to_uppercase`: ASCII だけが変わるなら長さは変わらない

最初に変わる文字より後ろが ASCII だけなら、結果の長さは元と同じなので、`to_ascii_case` と同じ書き方で
exact なバッファを直接作れる。

```rust
// src/repr.rs
impl Repr<Immutable> {
    pub(crate) fn to_case_immutable(&self, case: Case) -> Result<Option<Self>, ReserveError> {
        let text = self.as_str();
        let Some((unchanged, changed)) = case::split_at_first_change(text, case) else {
            return Ok(None);
        };
        if !changed.is_ascii() {
            // 今までどおり LeanString で作ってから変換する
            return match Repr::<Mutable>::to_case(text, case)? {
                Some(mapped) => mapped.into_immutable().map(Some).map_err(|(mut repr, err)| {
                    unsafe { repr.drop_in() };
                    err
                }),
                None => Ok(None),
            };
        }
        // SAFETY: same as `to_ascii_case`.
        unsafe {
            Repr::new_with(text.len(), |dst| {
                let dst = dst.as_mut_ptr().cast::<u8>();
                ptr::copy_nonoverlapping(unchanged.as_ptr(), dst, unchanged.len());
                let dst = dst.add(unchanged.len());
                for (i, &b) in changed.as_bytes().iter().enumerate() {
                    dst.add(i).write(match case {
                        Case::Lower => b.to_ascii_lowercase(),
                        Case::Upper => b.to_ascii_uppercase(),
                    });
                }
            })
        }
        .map(Some)
    }
}

// src/lib.rs
pub fn to_lowercase(&self) -> Self {
    self.0.to_case_immutable(Case::Lower).unwrap_with_msg().map_or_else(|| self.clone(), LeanStr)
}
```

試した結果: ASCII 大文字 48 バイトの `to_lowercase` が alloc 1 / realloc 0 になる。全テスト通過 (doc test の
"ὈΔΥΣΣΕΎΣ" / "tschüß" は非 ASCII の経路を通る)。`changed.is_ascii()` の走査が 1 回増えるが、
`Repr::<Mutable>::to_case` の `split_ascii_prefix` で同じ判定をしているので、判定を共有する形に整理する余地がある。

より広く効かせる案 (未実装): 大文字・小文字変換は非 ASCII でもほとんどの入力 (`"Éé"`、キリル文字など) で長さが
変わらない。`Repr::<M>::new_with` の変種として、`init` が「全部書けたか」を返し、書けなかったら確保を
(中身を読まずに) 解放して `None` を返す
`try_new_with(len, init: impl FnOnce(&mut [MaybeUninit<u8>]) -> bool) -> Result<Option<Self>, ReserveError>` を足し、
`to_case` の書き込み処理 (`unchanged` のコピー、ASCII の変換、文字ごとの変換) を書き込み先が足りなくなったら止まる形にして
`try_new_with(text.len(), ...)` の中で呼ぶ。長さがちょうどなら完成、伸びたり縮んだりしたら今の `Repr<Mutable>` の経路で
作り直す (そのときだけ今と同じコストになる)。`to_case` を `impl<M: Mutability> Repr<M>` に移せば、`LeanString` でも
1.5 倍の伸長で余った容量が残らなくなる。文字ごとの変換ループそのものの遅さは [#02](./02-case-conversion-per-char-push.md) を参照。

推奨しない案: `LeanStr` のときだけ変換結果を `Repr::<Immutable>::from_str` でコピーして元を解放する。`realloc` が
`malloc` + `free` に、`ptr::copy` が `memcpy` になるだけで、ほぼ改善しない。

### `from_utf16*`: 先に UTF-8 の長さを数える (未実装)

UTF-16 の各単位から UTF-8 の長さは安く求まる (`< 0x80` は 1、`< 0x800` は 2、サロゲート以外は 3、正しいサロゲートの組は
2 単位で 4)。1 回目の走査で長さを数え (lossy でない版はここで孤立サロゲートを検出してエラーにできる)、
`Repr::<Immutable>::new_with` で書き込めば 1 回の確保で済む。`LeanString::from_utf16` も容量を UTF-16 の単位数で見積もって
いるので、非 ASCII の入力では拡張が続く (CJK 24 単位で `LeanString` は realloc 3 回、`LeanStr` はそれに `into_exact` の 1 回が
加わって 4 回)。長さを先に数える方式なら両方に効く。1 文字ずつ `push` するループの遅さは [#03](./03-char-loop-append.md) を参照。

### 変えないもの

- `impl FromIterator<_> for LeanStr` (`LeanString::from_iter(iter).into_lean_str()`) と、`ToLeanStr` の `Display` への
  フォールバック (`try_from_fmt` → `try_into_lean_str`) も最後に `into_exact` が入る (例: 48 バイトを 1 回の `write_str` で
  書く `Display` で alloc 1 / realloc 1、10 バイトの `LeanString` 100 個を集めると `LeanStr` は realloc 12 回、`LeanString` は 11 回)。
  ただし書き込まれる長さが先に分からないので、exact なレイアウトにするための最後の詰め直しは避けられない
  (`String::into_boxed_str` と同じ)。要素 1 つの `collect::<LeanStr>()` は [#22](./22-lean-str-collect-single-element.md) を参照。
- `to_lowercase` の "Σ" の経路 (`text.to_lowercase()` の `String` を経由) は alloc 3 / realloc 1。`LeanStr` のときに
  `Repr::<Immutable>::from_str(&text.to_lowercase())` で作れば alloc 2 / realloc 0 にできるが、まれな経路なので優先度は低い。

## 確認方法

- 確保回数のテスト: 上の表の各行を、確保を数える `#[global_allocator]` で固定する。長さの変わらない入力の
  `LeanStr::to_lowercase` と、ASCII の `LeanStr::from_utf16` で realloc 0 回になること。
- 長さが伸びる入力 (`"ŉ"` の `to_uppercase`、`"İ"` の `to_lowercase`)、縮む入力 (`"\u{212A}"` (Kelvin) の `to_lowercase`)、
  "Σ"、不正な UTF-16 で、結果が `str` / `String` の同名メソッドと一致すること (既存のテストと proptest)。
- `bench/benches/apis.rs` の `to_lowercase` / `to_uppercase` に `LeanStr` の行を足す。

## 関連

- [#02](./02-case-conversion-per-char-push.md): 大文字・小文字変換の文字ごとのループ
- [#03](./03-char-loop-append.md): `from_utf16*` の 1 文字ずつの `push`
- [#05](./05-from-utf8-lossy-fast-path.md): `LeanStr::from_utf8_lossy` の同じ `realloc`
- [#09](./09-conversion-restore-on-failure.md): `into_immutable` / `into_exact` の失敗時の扱いとコード生成
- [#42](./42-conversion-cost-docs.md): `into_lean_str` のコストの doc
