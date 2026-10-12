# ベンチが公開 API の大半を測っておらず、`as_str` のベンチは `as_str` を測っていない

- 種別: infra, test, perf
- 対象: `bench/benches/apis.rs` (`as_str`, `eq`, `from`), `bench/benches/comparison.rs`
- 影響: `HashMap`/`BTreeMap` のキーとしての `Hash`/`Eq`/`Ord`、`LeanStr`、`collect`/`Extend`、変更系 (`insert_str`, `replace_range`, `retain`, `truncate`, `pop`)、UTF-8/16 変換、所有権の受け渡し (`into_lean_str`) は一切ベンチがなく、退行しても検出できない。`as_str` のベンチは LeanString と `String` で同じ ~10 ns を示すだけで、`as_str` の退行を検出できない。
- 優先度: 中

## 問題

### 1. ベンチのない公開操作 (`5dd94ea`)

`apis.rs` が測っているのは `from(&str)`, `from(char)`, `from_static_str`, `ToLeanString` (4 種の数値), `clone`, `reserve`, `push`, `push_str` (4 形), `insert(0, ch)`, `as_str`, `eq` (同一内容のみ), `to_lowercase`, `to_uppercase`。`comparison.rs` は `from`, `from_static_str`, `clone`, CoW write, grow, `as_str`, `eq`, 数値。ここに無いもの:

- 型: `LeanStr` は 1 件もない (`from`, `clone`, `drop`, `as_str`, `eq`, `into_lean_string`)。
- 比較・ハッシュ: `Hash` (`HashMap` キーとして最もよく使われる操作)、`Ord::cmp`、不一致の `eq` (先頭バイトが違う / 長さが違う / 末尾だけ違う)、`== &str` / `== String` との比較。現在の `eq` ベンチは中身が同じ文字列同士 (`eq/cloned` は同一バッファ) だけで、早期 return の経路 (長さ不一致) も `bcmp` の途中で差が出る経路も通らない。
- 変更系: `insert_str`, `replace_range`, `retain`, `remove`, `truncate`, `pop`, `clear`, `extend_from_within`, `split_off`, `shrink_to_fit`, `repeat`, `make_ascii_*` / `to_ascii_*`。`insert` は位置 0 の `char` だけ。
- 構築・変換: `from_utf8`, `from_utf8_lossy`, `from_utf16*`, `From<String>`, `From<Box<str>>`, `From<Cow<str>>`, `into_lean_str` / `into_lean_string` (`LeanString` → `LeanStr` の `into_exact` は再確保が起きうる経路)。
- 反復: `FromIterator<char>`, `FromIterator<&str>`, `Extend<*>`, `fmt::Write` (`write!`), `Add<&str>` / `AddAssign`。
- `ToLeanString` / `ToLeanStr`: 数値 4 種だけ。`bool`, `char`, 小さい整数 (`u8`), `f32`, `&str` / `String`, 汎用の `Display` フォールバック (`try_from_fmt`) が無い。`to_lean_str` も無い。
- 破棄: `Vec<LeanString>` の drop (最後の参照の解放と、inline が混ざった列のスキャン)。単体 drop は戻り値を返す各ベンチの計測に含まれてはいるが、単独では見えない。
- 静的バッファ: 静的文字列に対する `clone`, `push_str` (最初の変更でコピーが起きる)。

asm 側から見ると、これらはいずれも `#[inline]` で下流に展開される (例: `probe_into_lean_str` は 219 命令、`probe_insert_str` は 97 命令、`probe_cmp` は `memcmp` 呼び出し + 34 命令、`probe_hash` は `Hasher::write` 2 回。x86-64, LTO なし)。コードサイズ・コード配置の影響を最も受ける種類の関数なのに、wall-clock の目安すらない。

### 2. パラメータ空間の穴

- `from`: 長さ `[0, 1, 15, 16, 17, 32, 64, 65, 256]`。ヒープ側は `HeapBuffer::new` の分岐 (`16..32` → 16B×2、`32..=64` → 32B×2、`65..` → `memcpy`) をそれぞれ 17 / 32, 64 / 65 でカバーしており、よい。穴は inline 側: `InlineBuffer::new` は長さ `16`, `8..16` (`== 8` を別扱い), `4..8`, `2..4`, `1`, `0` で分岐するが、測っているのは 0, 1, 15, 16 だけで `2..8` と `8` が空いている。`65` より長いところは 256 だけ (`memcpy` のサイズ依存、`1000` 以上や 4 KiB 以上は見えない)。
- `clone` は長さ依存がほぼ無いのでこのままでよいが、`clone` ベンチは `clone` + drop (criterion の `iter` は戻り値の drop も計測に含める) なので、ヒープでは atomic inc + dec の往復を測っている。名前は `clone` だが、clone 単独の値は出ない。一方 `push_str/after_clone` と `CoW write` は `iter_batched` なので出力の drop が計測外で、`from` / `push` などの「drop 込み」ベンチと条件が揃っていない。
- `reserve` の `additional = 8` は空の inline 文字列に対して確保が起きない (容量 16 に収まる) ので、`reserve` 呼び出しのオーバーヘッドしか測っていない。確保を伴うのは 64 だけ。
- `push` / `insert` の `ch` ごとの比較は 1, 2, 3, 4 バイトを測っており十分。

### 3. `as_str` のベンチは `as_str` を測っていない

`apis.rs`:

```rust
b.iter(|| black_box(uut.as_str()))
```

結果は LeanString `current` / `prev` / `String` のどれも 8〜13 ns (長さ 0〜100 を通して、`String` も同様)。`String::as_str` はフィールドを返すだけなので、この数字は `as_str` の中身ではなく、計測形式の定数項である。mini ベンチで切り分けた結果 (同じマシン・load average 約 13、長さ 8 / 100):

```
black_box(uut.as_str())            10.3 ns / 8.3 ns   (現在の形。String でも 12 ns)
uut.as_str()                        0.56 ns / 0.50 ns  (空クロージャ 0.46 ns と同じ)
black_box(&uut).as_str()            2.0 ns             (入力側を black_box)
black_box(&uut).as_str().len()      1.1 ns
```

- `uut.as_str()` を black_box せずに返す形は空クロージャと同じ速さなので、呼び出しがループ外に出ている (loop-invariant として巻き上げられている) と考えられる。asm では未確認。
- `black_box(uut.as_str())` が ~10 ns かかる理由は未確認 (`&str` のペアを `black_box` がメモリ経由で往復させるコストと考えられるが、検証していない)。`String` でも同じ値になるので、`as_str` の実装とは無関係である。

いずれにせよ、現在の形では `Repr::len` / `Repr::as_bytes` (分岐を cmov に変えた箇所) が退行しても出力に現れない。

他のベンチは同種の問題がないかも確認した: `clone` は `uut.clone()` が巻き上げられていない (`black_box(&uut).clone()` と同じ値)。`to_lowercase` / `to_uppercase` も `black_box(&uut).to_*case()` と同じ桁で、巻き上げは起きていない。`eq` は `black_box(&lhs) == black_box(&rhs)` で入力側が不透明なので問題ない。

## 修正例

### `as_str` の計測形式

入力を不透明にして、結果は長さに畳む (`as_str` は `len` とポインタ選択の両方を実行する):

```rust
group.bench_with_input(BenchmarkId::new(label, len), &len, |b, _| {
    b.iter(|| black_box(&uut).as_str().len())
});
```

ポインタ選択の分岐も含めて見たい場合は、先頭バイトを読む (`black_box(&uut).as_str().as_bytes().first().copied()`)。asm 側での検証は [#57](./57-codegen-claims-unchecked.md) の仕組みの方が確実 (`Repr::len` は 10 命令・分岐なし、`as_str` は条件分岐 1 つ)。

### 不足しているベンチ (優先度順の例)

```rust
// eq: 先頭が違う / 長さが違う / 末尾だけ違う。inline と heap の両方 (len = 15, 100)
fn eq_mismatch(c: &mut Criterion) {
    let mut group = c.benchmark_group("eq/mismatch");
    for len in [15, 100] {
        let base = ascii(len);
        let mut first = base.clone().into_bytes(); first[0] ^= 1;
        let mut last = base.clone().into_bytes();  *last.last_mut().unwrap() ^= 1;
        let short = base[..len - 1].to_owned();
        for (kind, other) in [("first", String::from_utf8(first).unwrap()),
                              ("last", String::from_utf8(last).unwrap()),
                              ("len", short)] {
            duplicate! {
                [ label StrTy; ["current"] [lean_string::LeanString]; ["prev"] [lean_string_prev::LeanString]; ["std"] [String]; ]
                {
                    let lhs = black_box(StrTy::from(base.as_str()));
                    let rhs = black_box(StrTy::from(other.as_str()));
                    group.bench_function(BenchmarkId::new(label, format!("{kind}/{len}")), |b| {
                        b.iter(|| black_box(&lhs) == black_box(&rhs))
                    });
                }
            }
        }
    }
    group.finish();
}

// hash: 固定の Hasher (`DefaultHasher`) と、HashMap lookup (キーを 1000 個)
// cmp: eq と同じ組 (一致 / 先頭差 / 長さ差) に対して `black_box(&lhs).cmp(black_box(&rhs))`
// from に追加する長さ: [2, 3, 4, 7, 8, 9]
// extend/collect: "abcdefgh" を 16 個の iterator に対して `StrTy::from_iter(...)` / `s.extend(...)`
// mutate: 長さ 40 の文字列から `truncate(20)`, `pop()` x 8, `insert_str(5, "xyz")`, `replace_range(5..10, "xyz")`,
//         `retain(|c| c != 'a')` を iter_batched で (入力を毎回 clone して unique を保つ)
// convert: `LeanString -> LeanStr -> LeanString` の往復を 15 / 17 / 100 バイトで
// drop: `Vec<LeanString>` (半分 inline 半分 heap の 1000 要素) を iter_batched で作って drop
```

`comparison.rs` にも `Hash` (HashMap lookup), `Eq` の不一致, `collect` を足すと他クレートとの位置づけが分かる。

## 確認方法

- 追加した各ベンチについて、標準の `String` (あるいは `prev`) と同じバイナリ内で並べ、`cargo bench --no-run` が通ること。
- `as_str` の修正後は、`Repr::len` を分岐版に戻した変更 (ミューテーション) を入れて、ベンチが差を検出できるかを確認する。検出できなければ [#57](./57-codegen-claims-unchecked.md) の asm チェックで担保する。
- ベンチを増やしても CI では実行しない (ノイズが大きい)。ただし `bench/` はルートの workspace に含まれず CI でも一度もビルドされていないので、壊れても気づけない。`cd bench && cargo bench --no-run` のビルドだけは CI に入れる価値がある ([#57](./57-codegen-claims-unchecked.md) を参照)。
