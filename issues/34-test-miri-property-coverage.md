# property テストがすべて Miri で ignore され、それだけが頼りの unsafe 経路が Miri で検査されていない

- 種別: test, infra
- 対象: `tests/property.rs` (全テストに `#[cfg_attr(miri, ignore)]`), `src/lib.rs` の `from_utf16le` / `from_utf16le_lossy` / `from_utf16be` / `from_utf16be_lossy` (`align_to::<u16>()`), `src/repr/num_to_repr.rs` の整数の `into_repr`
- 影響: CI の Miri ジョブ 4 つは `tests/property.rs` の 77 件のうち 10 件 (u8 / i8 の全数テスト) しか走らせない。`from_utf16le` / `be` の `align_to` で選ぶ 2 つの経路と、`u16` 以上の整数の書き出し (`Repr::new_with` に渡した長さぶんを生ポインタで後ろから書く) は、Miri ではほぼ検査されていない
- 優先度: 中

## 問題

`tests/property.rs` の `#[property_test]` はすべて `#[cfg_attr(miri, ignore)]` が付いている。Miri の実行結果 (`5dd94ea`, nightly 1.101.0 2026-10-11, x86_64 / i686 / powerpc64 / powerpc のいずれも): `test result: ok. 10 passed; 0 failed; 67 ignored`。

多くの API は `tests/lean_string.rs` などの単体テストがあるので Miri でも通るが、次の 2 つは property テスト以外にほとんど無い。

1. `from_utf16le` / `from_utf16be` とその lossy 版 (両型)。単体テストは無く、Miri で走るのは doctest (両型で 12 件、奇数長の入力は無い) だけ。これらは `buf.align_to::<u16>()` で、バッファが 2 バイト境界にそろっていてエンディアンが一致すれば `&[u16]` として読み、そうでなければ 2 バイトずつ組み立てる。doctest の `&[u8; N]` のアドレスは実行ごとに変わりうるので、どちらの経路を通るかは決まっていない。奇数長で lossy 版が末尾に U+FFFD を付ける分岐 (`([], buf, [_remainder])`) も、アラインが合ったときしか通らない。
2. 整数の `to_lean_string` / `to_lean_str`。`num_to_repr.rs` は `DigitCount::digit_count` で求めた長さで `Repr::new_with` を呼び、`buf_ptr.add(curr)` に後ろから生ポインタで書く。桁数の計算が 1 ずれると範囲外への書き込みになるが、Miri で走るのは u8 / i8 の全数テストと `tests/alloc_string.rs` の `test_simple_types` (`1`, `-1`, `200`, `2`) だけで、`u64::MAX` や `i64::MIN` のような桁数の境界は通らない。(整数の書き出しの境界値テストそのものは [#28](./28-int-boundary-tests.md) の範囲。ここでは Miri で検査されていないことだけを指摘する)

## 原因

proptest は Miri では遅く、全部を走らせると CI が成り立たない。試したところ、`ProptestConfig { cases: 8, failure_persistence: None, .. }` にした 2 テスト (`replace_range` と `from_utf16le_lossy`) で、Miri x86_64 の実行に 2 テスト合わせて約 105 秒 (壁時計、ビルド済み) かかった。`String` の任意値の生成が長い文字列を作るため。

## 修正例

property テストを Miri で回すより、Miri でしか見られない性質 (アラインメント、境界) を狙った決定的なテストを足すのがよい。

`from_utf16le` / `be` は、アラインをそろえたバッファと 1 バイトずらしたバッファの両方で試す。x86_64 で `cargo test`、Miri (`-Zmiri-strict-provenance`) の x86_64 / i686 / powerpc64 / powerpc で通ることを確認した (powerpc 系では LE が組み立ての経路、BE が `&[u16]` の経路になる)。

```rust
use lean_string::{LeanStr, LeanString};

#[repr(C, align(2))]
struct Aligned([u8; 24]);

fn utf16_bytes(text: &str, le: bool) -> Aligned {
    let mut buf = Aligned([0; 24]);
    let mut i = 0;
    for unit in text.encode_utf16() {
        let b = if le { unit.to_le_bytes() } else { unit.to_be_bytes() };
        buf.0[i..i + 2].copy_from_slice(&b);
        i += 2;
    }
    buf
}

#[test]
fn from_utf16le_be_aligned_and_unaligned() {
    let text = "𝄞music";
    let n = text.encode_utf16().count() * 2;

    for le in [true, false] {
        let aligned = utf16_bytes(text, le);
        let mut shifted = Aligned([0; 24]);
        shifted.0[1..n + 1].copy_from_slice(&aligned.0[..n]);

        for bytes in [&aligned.0[..n], &shifted.0[1..n + 1]] {
            let (a, b, c, d) = if le {
                (
                    LeanString::from_utf16le(bytes),
                    LeanStr::from_utf16le(bytes),
                    LeanString::from_utf16le_lossy(bytes),
                    LeanStr::from_utf16le_lossy(bytes),
                )
            } else {
                (
                    LeanString::from_utf16be(bytes),
                    LeanStr::from_utf16be(bytes),
                    LeanString::from_utf16be_lossy(bytes),
                    LeanStr::from_utf16be_lossy(bytes),
                )
            };
            assert_eq!(a.unwrap(), text);
            assert_eq!(b.unwrap(), text);
            assert_eq!(c, text);
            assert_eq!(d, text);
        }

        // An odd number of bytes: an error, or a trailing U+FFFD for the lossy versions.
        for bytes in [&aligned.0[..n + 1], &shifted.0[1..n + 2]] {
            if le {
                assert!(LeanString::from_utf16le(bytes).is_err());
                assert!(LeanStr::from_utf16le(bytes).is_err());
                assert_eq!(LeanString::from_utf16le_lossy(bytes), "𝄞music\u{FFFD}");
                assert_eq!(LeanStr::from_utf16le_lossy(bytes), "𝄞music\u{FFFD}");
            } else {
                assert!(LeanString::from_utf16be(bytes).is_err());
                assert!(LeanStr::from_utf16be(bytes).is_err());
                assert_eq!(LeanString::from_utf16be_lossy(bytes), "𝄞music\u{FFFD}");
                assert_eq!(LeanStr::from_utf16be_lossy(bytes), "𝄞music\u{FFFD}");
            }
        }
    }
}
```

整数は、各型の `MIN` / `MAX` / `0` / `10^k` / `10^k - 1` (符号付きはその負数も) を `to_string()` と比べる決定的なテストを置けば Miri で境界の書き込みを検査できる。試したところ、u16 から isize までの 10 型で x86_64 と i686 の Miri でどちらも通り (Miri の報告で約 14 秒)、UB は報告されなかった (テストの中身は [#28](./28-int-boundary-tests.md) のものを使う。そちらは決定的なテストで Miri でも回る)。

property テスト自体を Miri で回したい場合は、`#[cfg_attr(miri, ignore)]` の代わりに `cfg!(miri)` でケース数を 4〜8 に落とし、`failure_persistence: None` にする (Miri の隔離でファイルに書けないため)。上の測定のとおり 1 テストあたり 1 分前後かかるので、`Repr` の unsafe に触る数個 (`replace_range`, `extend_from_within`, `insert`, `push`) に絞るのが現実的。

## 確認方法

`MIRIFLAGS=-Zmiri-strict-provenance cargo +nightly miri test --target <t>` を 4 ターゲットで走らせ、追加したテストが ignore されずに通ること。
