# 12: プラン化しなかった項目の記録

調べたが、単独のプランを起こすほどではない、または前提となる作業を待つべきと判断した項目。
再訪する条件と、そのときに見る場所をまとめておく。

## 小さな codegen 改善

### `set_len` が static の分岐を残す

`Repr::set_len` (repr.rs:871-891) は `is_static_buffer()` を最初に判定するが、
`push_str` / `insert_str` / `remove` / `retain` から呼ばれる時点では
`reserve` / `ensure_modifiable` の事後条件により static ではありえない。
`push_str` の asm には実際にこの死んだ比較が残っている
([research/codegen-baseline.md](../research/codegen-baseline.md) の §3、
`cmpl $209, %eax` の行)。

`reserve` が `Ok` を返した直後に `hint::assert_unchecked(!self.is_static_buffer())` を
置くだけで消える見込み。ただし [plans/03](./03-push-str-fast-path.md) が
「バッファ種別を後段へ伝える」形を採ると自然に解消するので、03 の中で扱うほうがよい。
03 を見送る場合はこれだけ単独で入れる価値がある。

なお、コピー後にバイト 15 を読み直すこと自体は避けられない。inline のとき
`len == 16` になるコピーはバイト 15 を書き換えるので、LLVM は再ロードせざるを得ない。

### 確保失敗の分岐が `#[cold]` になっていない

このクレートは cold 化に気を配っている (`Capacity::new` の `cold_path()`、
`layout_for` の `#[cold]` クロージャ、`reserve` の `outline!`) が、
次の 4 か所は素のままになっている。

- `alloc` が null を返す判定 (heap_buffer.rs:260-263)
- `realloc` が null を返す判定 (heap_buffer.rs:439-442)
- `layout_for` の `checked_add` オーバーフロー (heap_buffer.rs:284-293)
- `reserve` の `len.checked_add(additional).ok_or(ReserveError)?` (repr.rs:442) —
  これは `push_str` の hot path 上にある

まとめて片付ければ数行。ただし単体の効果は小さいので、
[plans/03](./03-push-str-fast-path.md) の asm を見るときに一緒に判断する。

### `reserve` が `len()` を読み直す

`Repr::push_str` は `self.len()` を読んでから `reserve` を呼び、
`reserve` (repr.rs:441) がまた `self.len()` を読む。64-bit ではマスク付きのロードと
`cmov` の対がもう 1 組載る。`insert_str` (repr.rs:726 と 729) にも同じ重複がある。
[plans/03](./03-push-str-fast-path.md) の対象。

### `ensure_modifiable` の static 分岐が `replace_inner` を使う

`ensure_modifiable` (repr.rs:836-840) は static と分かっているのに
`self.replace_inner(next)` を呼び、その中で `is_heap_buffer()` をもう一度判定している。
`*self = next` で等価。1 命令の話。

## `LeanStr` の構築経路

`LeanStr` の構築は `from(&str)` を除いて、いったん `LeanString` を作ってから
`into_lean_str()` する形になっている (lib.rs:1196-1199 ほか、
`FromIterator` 系 lib.rs:1914-1986、`ToLeanStr` のフォールバック traits.rs:162-166)。

`into_lean_str` は unique な heap バッファに対して `HeapBuffer::into_exact` を通り、
文字列全体の `ptr::copy` + `realloc` + ヘッダの書き直しになる。
`from_utf8_lossy` / `from_utf16` / `from_utf16_lossy` / `collect()` は最終長が
先に分かる (または安く計算できる) ので、`Repr::<Immutable>::new_with` 相当で
直接作れば往復が消える。

[plans/04](./04-append-writer.md) の writer が入ると `collect()` 系は自然に片付くので、
その後に残りを見るのがよい。

## `to_lean_string()` / `to_lean_str()` の相互変換

`traits.rs:75` は `LeanStr` を `LeanString` にするとき

```rust
&LeanStr as s => return Ok(s.clone().try_into_lean_string()?),
```

としている。`clone()` が `fetch_add` するのでバッファが共有状態になり、
`into_mutable` は「共有されている」分岐 (repr.rs:958-969) に入って結局コピーする。
その後 `release()` が `fetch_sub` する。`Repr::<Mutable>::from_str(s.as_str())` にすれば
同じ結果を atomic 操作なしで得られる。`traits.rs:160` の逆方向も同じ。

小さいが確実な改善。どこかのタスクのついでに。

## serde の `deserialize_string` ヒント

`src/features/serde.rs:48` と `:90` は `deserializer.deserialize_string(..)` を呼ぶが、
visitor は `visit_string` / `visit_byte_buf` を実装していない。
ヒントを尊重する形式では `String` を確保してから既定の `visit_string` が
`visit_str` に転送し、`LeanString` へコピーして `String` を捨てることになる。
`LeanString` は常にコピーするので `deserialize_str` のほうが正確なヒント。

効果は `Deserializer` の実装次第なので、`serde_json` でエスケープあり/なしの
両方を測ってから変えるべき。

## 整数フォーマットのレジスタ組み立て

`num_to_repr.rs` は既に LUT で `Repr::new_with` に直書きしており、
`compact_str` とおおむね同等。残る差は次の 2 点。

1. `MAX_INLINE_SIZE = 16` なので、u8〜u32 / i8〜i32 (最大 11 桁) は**必ず inline に収まる**。
   型ごとの桁数上限を const にして `assert_unchecked` で伝えれば、
   `new_with` (repr.rs:136-151) の heap 分岐が型レベルで死ぬ。
   u64 / i64 は最大 20 桁で 16 バイトに収まらないので対象外。
   `compact_str` は 24 バイト表現なので u64 まで inline に収まり、そこを前提にした
   レジスタ組み立てを行っている。lean_string にはそのまま持ち込めない。
2. 桁数計算の方式 (`checked_ilog10` ベース vs 現在の match による分岐) の比較。

`impl_NumToRepr_for_integers` マクロが作る `into_repr` にだけ `#[inline]` が無い
(f32/f64/u128/i128/NonZero 系には付いている) が、この関数は `M: Mutability` で
generic なので MIR はエクスポートされており、実際に下流から呼んだ asm でも
完全にインライン展開されていることを確認した。付け忘れではあるが、
現状の codegen には影響していない。

再訪の条件: [plans/01](./01-inline-register-construction.md) の完了後。
整数の構築も `Repr::new_with` の inline 側を通るので、01 の後のほうが
`from_num` の asm に残るものが見やすい。ベンチは `comparison.rs` の
`Numbers` グループが既にある。

## 新 API (方針判断待ち)

### `format_lean!` / `format_lean_str!` マクロ

`format!` → `LeanString::from` の中間 `String` 確保を省き、直接書き込むマクロ。
[plans/04](./04-append-writer.md) の writer が入れば、そのバックエンドとして使える。
04 の後に判断するのが得。あわせて `MAX_INLINE_SIZE` に相当する定数を
pub にするかも決める (`INLINE_CAPACITY` のような名前で)。

### `join` / `concat`

合計長を先に計算して 1 回だけ確保し、直接書き込む。
`Repr::new_with(total_len, |ptr| ...)` がそのまま使えるので実装は素直。
`Vec<LeanString>` → `join` → 2 回確保が 1 回になる。需要が出たら。

### storage を指定する構築子

`new_inline` / `new_heap` のように、どの表現で作るかを呼び出し側が選べる構築子。
文字列 interning のように「inline に収まるなら intern しない」という判定を
長さ比較ではなく構築子で表現したい利用者向け。具体的な要望が出たら。

### 他クレートとの統合 feature

`get-size2` や `salsa` の実装を feature として持つ案。需要が出たら。
1 点だけ注意を記録しておく。

- CoW の共有バッファはデータポインタで重複排除しないと、clone のぶんだけ
  サイズを二重計上してしまう。
- `get-size2` は `AtomicI64` / `AtomicU64` を無条件に import するため、
  64-bit atomic の無いターゲットでコンパイルが通らない。このクレートの CI は
  Miri を powerpc (32-bit) でも回しているので、feature を足すなら
  そのターゲットでの扱いを cfg で分ける必要がある。

## inline 同士の比較を速くする案

[plans/07](./07-equality-policy.md) でポインタ一致の近道を既定の `==` に入れない方針を
採るなら、次の 2 つも同時に見送りになる。方針が変わったときのために記録しておく。

1. **論理長が 16 以下のとき、両端からのワード単位 XOR で比較する**。
   `bcmp` の呼び出しを避ける。論理長の範囲しか読まないので、
   未使用容量に残ったバイトの影響を受けない。追加の不変条件は不要。
2. **inline 表現を「未使用バイトはゼロ」の正規形に保ち、`[usize; 2]` として比較する**。
   比較は最速になるが、`truncate` などで正規形が崩れうるすべての経路に
   正規化を入れる必要がある。また `Repr` の第 1 フィールドは `*const ()` なので、
   これを `usize` として読む形は provenance の扱いに注意が要る
   (CI が `-Zmiri-strict-provenance` で回っているのでそこで検出はされる)。

どちらも `char_str` の未マージのブランチに実装はあるが、計測値は残されていない。

## テストとベンチの整備

### ベンチの読み方 (今回の測定時点での注意)

`bench/Cargo.toml` の `lean_string_prev` は直前のリリース版を指す。
今回の測定時点ではリポジトリのコードが v0.7.0 から進んでいないため、
`current` と `prev` が同じソースを 2 回コンパイルしたものになっていた。

この状態で測ると `push_str` が current 190 ns / prev 238 ns、
`eq/cloned` の 256 バイトが current 4.68 ns / prev 5.76 ns といった差が
繰り返し再現する。ソースが同一なので、これはコード配置やアラインメントの偶然による差。
この文書に載せた `current` / `prev` の数値をあとから読み返すときは、
その時点でリポジトリが `prev` から進んでいたかどうかを確認すること。
何か手を入れた後の測定では両者は別のコードになるので、この注意は当てはまらない。

### asm 確認の定型化

各プランが「asm を before/after で比較する」ことを前提にしているが、
その手順はリポジトリに入っていない。
[research/codegen-baseline.md](../research/codegen-baseline.md) に書いた
「probe 用クレート + `cargo asm`」を、
リポジトリ内のツールとして持つことを検討してもよい。

- `#[unsafe(no_mangle)] #[inline(never)]` の 1 行ラッパを並べただけの
  クレート (workspace には入れない)
- シンボルごとに `.s` を切り出して命令数を数えるスクリプト
- x86-64 と aarch64 の両方を出す (リンクはしないのでクロスの実行環境は不要)

ベンチと違って測定ノイズが無く、CI に載せれば「この関数に memcpy 呼び出しが
復活したら落ちる」といった退行検出にも使える。実装量は小さい。
ただしリポジトリに増やすものが増えるので、必要性が固まってからでよい。

## 確認して問題なかったもの

同じところを二度調べないための記録。

- **`Clone` の分岐配置**。`probe_clone` の asm はビット単位コピーの arm が
  fall-through になっており、heap の arm は判別子の比較と `lock incq` の 2 命令。
  `compact_str` には「cold 側を空関数呼び出しで作って分岐配置を誘導する」手法があるが、
  ここでは不要。
- **atomic の順序**。`Relaxed` の `fetch_add` (repr.rs:242)、`Release` の `fetch_sub` と
  最終参照時の `Acquire` fence (heap_buffer.rs:214-224)、`Acquire` の load (heap_buffer.rs:190)
  は `Arc` と同一。`Weak` に相当するものが無いので、
  参照を保持したままカウント 1 を観測できれば排他は保証される。loom がカバーしている。
- **共有 heap バッファのデータは書き換えられない**。共有状態での `truncate` / `pop` は
  ローカルの `TextLen` ワードしか触らず (repr.rs:775-786)、32-bit で長さが
  ヒープ上にある場合は正しくコピー経路へ落ちる (repr.rs:787-799)。
  これが `Send`/`Sync` の根拠になっている。実測でも共有が維持されることを確認した。
- **オーバーフロー経路**。`reserve` の `checked_add` (repr.rs:442)、
  `insert_str` の `checked_add` (repr.rs:726)、`repeat` の `checked_mul` (lib.rs:824)、
  `layout_for` の `checked_add` (heap_buffer.rs:284)、`amortized_growth` の
  saturating (heap_buffer.rs:19-23) がすべて塞いでいる。
  `heap_buffer.rs:416` の `wrapping_add` は 2^56 の上限から正当化されており、
  その根拠もコメントに書かれている。
- **32-bit の容量上限 `2^31 - 16`** は正しい。`Capacity::MAX = isize::MAX`、
  `header_offset = 8`、長さ prefix 4 バイト、`Layout::from_size_align` の
  `size <= isize::MAX - (align - 1)` から `2^31 - 1 - 3 - 12 = 2^31 - 16`。
- **`Borrow<str>` / `Hash` / `Eq` の整合**。`HashMap` のキーにして `&str` で
  引ける条件を満たしている。
- **zmij の Miri**。`Cargo.toml` は `zmij = "1.0"` でバージョンを固定していないが、
  現在解決される 1.0.23 で `cargo +nightly miri test --all-features` が
  x86_64 で通ることを確認した (f32/f64 の変換は `tests/property.rs` がカバーしている)。
  以前 Miri で問題があったのは 1.0.22 とのことなので、固定は不要と判断してよい。
  ただし CI の Miri は 4 ターゲットで回すので、BE / 32-bit で落ちる可能性は残る。
- **`retain` の panic 安全性**。`SetLenOnDrop` (repr.rs:661-708) が担っており、
  テストもある。
- **`fmt::Write::write_fmt` の static 化 fast path の健全性**。
  `fmt::Arguments::as_str()` は `Option<&'static str>` を返すので、
  `from_static_str` に渡して `'static` として扱ってよい。
  ただしテストが無い ([plans/11](./11-test-hardening.md) の B)。
