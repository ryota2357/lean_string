# 整数→文字列のテストが 64-bit 整数の 16 桁以下 (inline 経路) と桁の境目をほとんど踏まない

- 種別: test
- 対象: `tests/property.rs` の `prop_test_integer_to_lean_s!`、`src/repr/num_to_repr.rs` の `impl_NumToRepr_for_integers!` / `DigitCount`
- 影響: `u64` / `i64` / `usize` / `isize` (と 32-bit ターゲットの `u32` / `i32`) の短い値の変換を壊しても、どのテストも落ちない。
- 優先度: 中

## 問題

`tests/property.rs` は `u8` / `i8` を全数で、`u16` 以上を proptest (`any::<T>()`、256 ケース) で
`to_string()` と比べている。proptest の整数は全範囲から一様に取るので、値はほぼ最大桁数になる。
同じ設定で生成された値の桁数を数えると次のようになった (`5dd94ea`、proptest 1.12)。

| 型 | 桁数の分布 (256 ケース) |
| --- | --- |
| `u64` | 17 桁: 2、18 桁: 13、19 桁: 109、20 桁: 132 |
| `u32` | 7 桁: 1、8 桁: 1、9 桁: 58、10 桁: 196 |

`u64` が 16 桁以下になる確率は 1 ケースあたり約 0.05% で、`u64` / `i64` を inline (16 バイト以内) で
作る経路は proptest ではまず通らない。`9` → `10`、`99999999` → `100000000` のような桁の境目や
`MIN` / `MAX` も列挙されていない。他のテストでは `tests/alloc_string.rs` に `1` / `-1` / `200` / `2`
(すべて `i32`) があるだけである。

実際に `DigitCount for u64` の `10..=99 => 2` を `=> 3` に書き換えても、`cargo +nightly test --release`
は全件通った (doctest を含む)。debug ビルドなら `into_repr` の `debug_assert_eq!(curr, 0)` で検出できる
はずの誤りだが、その値を通すテストが無い。

`DigitCount` は型ごとに 20 個近い範囲を手で書いた `match` なので、こうした書き間違いが入りやすい場所でもある。

## 修正例

境目の値を列挙するテストを足す。各型について `0`、`MIN`、`MAX`、`MIN + 1`、`MAX - 1`、
`10^k - 1`、`10^k`、`10^k + 1` (符号付きはそれぞれの符号反転も) を `to_string()` と比べ、
`NonZero` 版と `to_lean_str` も同時に見る。

```rust
// tests/num_boundaries.rs
use core::num::NonZero;
use lean_string::{ToLeanStr, ToLeanString};

/// `0`, `MIN`, `MAX`, and `10^k - 1`, `10^k`, `10^k + 1` (and their negations) for every `k`.
macro_rules! boundaries {
    ($ty:ty) => {{
        let mut v: Vec<$ty> = vec![0, <$ty>::MIN, <$ty>::MAX, <$ty>::MIN + 1, <$ty>::MAX - 1];
        let mut p: $ty = 1;
        loop {
            #[allow(unused_comparisons)]
            for x in [p - 1, p, p + 1] {
                v.push(x);
                if <$ty>::MIN < 0 {
                    v.push((0 as $ty).wrapping_sub(x));
                }
            }
            match p.checked_mul(10) {
                Some(q) => p = q,
                None => break,
            }
        }
        v
    }};
}

macro_rules! test_boundaries {
    ($($ty:ident),* $(,)?) => {$(
        #[test]
        fn $ty() {
            for n in boundaries!($ty) {
                let expected = n.to_string();
                assert_eq!(n.to_lean_string(), expected);
                assert_eq!(n.to_lean_str(), expected);
                if let Some(nz) = NonZero::new(n) {
                    assert_eq!(nz.to_lean_string(), expected);
                    assert_eq!(nz.to_lean_str(), expected);
                }
            }
        }
    )*};
}

test_boundaries!(u8, i8, u16, i16, u32, i32, u64, i64, u128, i128, usize, isize);
```

上の書き換え (`u64` の `10..=99 => 3`) はこのテストで `u64` と `usize` が落ちることを確認した。
数百件しかないので Miri でも 1 ターゲット 30 秒程度で回る (powerpc64 / i686 で確認)。
`u16` / `i16` は 65536 件なので、proptest をやめて `u8` / `i8` と同じ全数テストにしてもよい。

あわせて、proptest 側も桁数が偏らないように、一様な値ではなく「桁数を先に選んでからその範囲の値を選ぶ」
戦略を使うとよい (例: `(1..=20u32).prop_flat_map(|d| 10u64.pow(d - 1)..=10u64.saturating_pow(d) - 1)`)。

## 確認方法

`cargo test --test num_boundaries` (debug と `--release`)、および
`MIRIFLAGS=-Zmiri-strict-provenance cargo +nightly miri test --target <t> --test num_boundaries` を
CI の 4 ターゲットで。`num_to_repr.rs` を変更するときは、境目を 1 つずらす変異 (例: `DigitCount` の
範囲の 1 つの桁数を変える) で落ちることを一度確かめる。

## 関連

- [#06](./06-int-to-repr-registers.md) (整数の変換をレジスタで組み立てる案)。その試作では、16 バイトちょうどの
  inline 文字列 (例: `-999999999999999`) で長さのタグの扱いを誤った版を一度作ったが、既存のテストは
  (`cargo +nightly test`、debug ビルドでも) すべて通り、このテストだけが検出した
  (`Repr::from_inline` の `assert_unchecked` の debug チェックで落ちる)。
