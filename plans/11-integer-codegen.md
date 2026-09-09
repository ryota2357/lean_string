# 11: 整数変換の戻り値がレジスタに載らない

種別: perf / 実装量: 中 / 設計判断: 中 (素直な対処が効かないことを確認済み)

## 現象

`Repr::from_num` は LUT で桁を作り、`Repr::new_with` (src/repr.rs:136-151) の
`init` クロージャからスタック上の 16 バイトバッファへ直接書く。桁を作る部分は良く、
u32 / i64 では `String` より速い。

| | LeanString | CompactString | std |
| --- | --- | --- | --- |
| `u32` | **11.374 ns** | 15.447 ns | 14.839 ns |
| `i64` | **11.959 ns** | 15.575 ns | 21.308 ns |
| `u64::MAX` | 27.221 ns | **17.202 ns** | 26.989 ns |
| `f64` | 25.189 ns | 24.923 ns | 112.64 ns |

`u64::MAX` だけ大きく負けているのは、20 桁が 16 バイトに収まらず必ずヒープ確保に
なるため。24 バイト表現の CompactString は inline に収まる。これは表現の大きさに
由来する構造的な差で、実装では埋められない。

一方、**戻り値を作る最後の 16 バイト転送が 8 回のストアに分割される**のは実装の問題。

```asm
	movq	-16(%rsp), %rcx       # ワード単位で 2 回ロード
	movq	-8(%rsp), %rdx
	...
	movb	%cl, (%rax)           # ★ここから 16 バイトを 8 回に分けてストア
	movb	%r8b, 7(%rax)
	movw	%cx, 5(%rax)
	movl	%esi, 1(%rax)
	movl	%edx, 8(%rax)
	movb	%cl, 14(%rax)
	movw	%dx, 12(%rax)
	movb	%dil, 15(%rax)
```

ロード自体は 2 本にまとまっているので、分割されているのはストア側だけである。
`LeanString::from(&str)` の inline 経路が同じ `Repr` へ `movq` 2 本で書けている
([research/codegen-baseline.md](../research/codegen-baseline.md) の §1) のと対照的で、
違いは値の出どころにある。`from_str` は 2 つの u64 を計算して持っているのに対し、
`new_with` の値はバイト単位のストアで作られたバッファから来るため、LLVM が
`Repr` のフィールド境界 (バイト 8 と 15) ごとに分解したまま再合成できていない。

## 試して効かなかった対処

**素直な 2 案はどちらも効かなかった**ので、記録しておく。

### 案X: `new_with` の中で読み直しをワード単位に強制する

`InlineBuffer::empty()` へ書いた後、`ptr::read` で `[usize; 2]` として読み直してから
`Repr::from_inline` へ渡す形。

結果は**生成された asm がバイト単位で完全に一致**した。LLVM は挿入した
`ptr::read` を見通して、もとのバイト分解へ戻している。分割の原因は読み直しの
書き方ではなく、`Repr` のフィールド分解のほうにある。

### 案Y: `new_with` の inline arm を `InlineBuffer::new` 経由にする

スタック上の `[u8; 16]` に書いてから、その先頭 `len` バイトを `&str` として
`InlineBuffer::new` に渡す形。`InlineBuffer::new` は 2 ワードをレジスタで組み立てるので、
理屈のうえでは 2 ストアに収まるはず。

結果は**全体としては悪化した**。`InlineBuffer::new` の長さ分岐が乗って
`into_repr` が大きくなり、下流クレートからインライン化されなくなる。

```
before: probe_from_num = 106 命令 (into_repr が完全に展開されている)
after:  probe_from_num = 22 命令 + callq into_repr  (sret 経由で受け取る)
```

`impl_NumToRepr_for_integers` が作る `into_repr` に `#[inline]` を足しても
変わらなかった。桁を作る部分と長さ分岐の両方を抱えた関数は、インライン化の
閾値には収まらない。

## 残っている案

### 案A: 桁数の上限を型ごとに `assert_unchecked` で伝える

`MAX_INLINE_SIZE = 16` なので、u8〜u32 / i8〜i32 (符号込みで最大 11 文字) は
**必ず inline に収まる**。型ごとの桁数上限を const にして
`new_with` の中で `hint::assert_unchecked(len <= MAX_INLINE_SIZE)` を置けば、
`new_with` の heap 分岐が型レベルで死ぬ。

分割ストアそのものは消えないが、u64 / i64 / usize / isize 以外では
確保の分岐と `Result` の Err arm が丸ごと消える。実装量が小さく、退行の危険も低い。
まずここから測るのがよい。

### 案B: 桁を直接 2 つの u64 に組み立てる

`new_with` を使わず、LUT からの 2 バイトずつの書き込みをレジスタ内のシフト + OR に
置き換える。分割ストアの原因を根本から取り除ける唯一の案。

- 対象は桁数が 16 以下に収まる型に限る (u8〜u32 / i8〜i32 と対応する `NonZero`)。
  u64 / i64 / usize / isize (64-bit) は 20 桁になりうるので現行の経路を残す。
- 桁は下位から作るので、レジスタ内では「書き込み位置に応じてシフトして OR」になる。
  現行のループが `curr` を減らしながら書いているのと同じ順序で書ける。
- little-endian 前提の話なので、`InlineBuffer::new` と同じ cfg でゲートする。

実装量は中。案A を測ってから判断する。

### 案C: 何もしない

`u32` / `i64` は既に `String` より速く、`u64::MAX` の差は表現の大きさに由来する
構造的なもの。分割ストアは 6 命令ぶんで、`Numbers` ベンチで有意差が出ない可能性がある。
案A を入れても効果が誤差の範囲なら、そこで打ち切って記録を残す。

## 検証方針

- **asm**: `probe_from_num` の末尾が `movq` 2 本になること (案B)、
  または確保の分岐が消えること (案A)。`into_repr` が下流から見て
  インライン展開されたままであることを毎回確認する — ここが今回いちばん壊れやすい。
- **criterion**: `comparison.rs` の `Numbers` グループ。u32 / i64 / u64::MAX / f64。
  `u64::MAX` は変わらないはずなので、劣化していないことの確認になる。
- **テスト**: `tests/property.rs` に整数の `to_lean_string()` と
  `to_string()` の等価性プロパティがあるか確認し、無ければ全整数型ぶん足す。
  境界 (`MIN` / `MAX` / 0 / ±1 / 桁上がり) を明示的に列挙するテストも足す。
- **Miri**: 4 ターゲット。

## 補足

`impl_NumToRepr_for_integers` が作る `into_repr` にだけ `#[inline]` が無い
(f32/f64/u128/i128/NonZero 系には付いている)。現状はこの関数が `M: Mutability` で
generic なため MIR がエクスポートされており、下流から呼んだ asm でも完全に
インライン展開されていることを確認した。付け忘れではあるが、
現状の codegen には影響していない。案B を入れる場合はインライン化が境目になるので、
そのとき明示的に付ける。

## 依存

なし。単独で実施できる。
