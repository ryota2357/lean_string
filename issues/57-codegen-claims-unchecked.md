# コードのコメントが前提にしているコード生成 (cmov・`memcpy` なし・レジスタ保持など) を検査する仕組みがなく、CI にベンチも asm 検査もない

- 種別: infra, test, perf
- 対象: `src/repr.rs` (`Repr::len`, `Repr::as_bytes`, `Repr::last_byte`, `Repr::is_modifiable`, `write_char`), `src/repr/heap_buffer.rs` (`HeapBuffer::new`, `Capacity::new_unchecked`), `src/repr/inline_buffer.rs` (`InlineBuffer::new`, `InlineBuffer::from_char`), `src/case.rs` (`all_bytes`), `.github/workflows/ci.yaml`
- 影響: `perf:` コミット (`c5c62e9`, `5b98e5b`, `12948a6`, `99b2573` ほか) の効果は「`memcpy` 呼び出しがない」「レジスタだけで組み立てる」といったコード生成の性質に依存している。rustc/LLVM の更新や無関係な変更で壊れても、テストは全部通り、CI にベンチもないので誰も気づかない。wall-clock のベンチはノイズが大きく ([#55](./55-bench-baseline-and-noise.md) 参照)、この種の退行の検出に向かない。
- 優先度: 中

## 問題

### 1. コード生成を前提にしたコメントが 11 か所ある

`grep` で見つかったもの (`5dd94ea`):

| 場所 | 主張 |
|---|---|
| `Repr::len` (64bit) | 「compiled to a single branchless instruction, such as `cmov`」 |
| `Repr::as_bytes` | x86 では `cmov` ではなく分岐になる (`-x86-cmov-converter-force-mem-operand`) |
| `Repr::last_byte` | `assert_unchecked` でバイト読みとワード読みの同一性を伝えると codegen が良くなる |
| `Repr::is_modifiable` | `is_unique()` (atomic load) を最後に書かないと、その後の検査でバッファが再読み込みされる |
| `write_char` | 実行時長の `copy_nonoverlapping` は `memcpy` 呼び出しになる。定数長 4 分岐にすると呼び出しも stack 往復もなくなる |
| `HeapBuffer::new` | `16..=64` バイトは定数長 2 コピーで `memcpy` を避ける。`> 64` だけ `memcpy` |
| `Capacity::new_unchecked` | `assert_unchecked` が `layout_for` のオーバーフロー・サイズ検査を畳む |
| `InlineBuffer::new` (64bit LE) | 「Assemble entirely in registers」 |
| `InlineBuffer::new` (その他) | 長さを半分ずつ定数長でコピーし、`memcpy` を避ける。`len == 16` を先に剥がして長さバイトの上書きを見せる |
| `InlineBuffer::from_char` | 2 ワードをレジスタで組み立てる。`encode_utf8` + `Self::new` ではスタック往復になる |
| `all_bytes` | 合計して `CHUNK_SIZE` と比較する形がベクトル化される (LLVM issue 96395 の回避) |

これらはどれも「コードを読んでも成否が分からない」類の主張で、`perf:` コミットの根拠でもある。しかし検査は人が `cargo asm` で見る以外に存在しない。CI (`.github/workflows/ci.yaml`) は fmt/clippy/MSRV/test/miri/loom だけで、`bench/` はルートの workspace に含まれず一度もビルドされない (ベンチ自体が壊れても検出されない)。

### 2. 現状では主張は成立している (検証済み)

`5dd94ea`, `rustc` stable, LTO なしの probe crate (`#[unsafe(no_mangle)]` の 1 行ラッパーを `cargo rustc --release --lib -- --emit asm`)。x86-64 と aarch64 で確認:

| probe | x86-64 | aarch64 | 結論 |
|---|---|---|---|
| `LeanString::len` | 10 命令、条件分岐 0 (`cmovb` + `cmovae`) | 10 命令、分岐 0 (`csel` ×2) | 成立 (厳密には cmov 2 個 + 算術。コメントの「single branchless instruction」は最後の選択のこと) |
| `LeanString::is_empty` | 12 命令、分岐 0 | 12 命令、分岐 0 | 成立 |
| `LeanString::as_str` | 12 命令、条件分岐 1 (`jb`) | 12 命令、分岐 0 (`csel`) | 成立 (x86 だけ分岐、aarch64 は `csel` というコメントどおり) |
| `LeanString::from(char)` | 56 命令、`call` なし、スタックなし | 43 命令 | 成立 (2 ワードをレジスタで組み立て、`mov` 2 つで書き出す) |
| `LeanString::push(char)` | 123 命令、`memcpy` なし (`call` は grow と panic のみ) | 121 命令、`memcpy` なし | 成立 |
| `LeanString::from(&str)` | 101 命令、`memcpy` は 1 回 (`>= 65` 分岐のみ)、`16..=31` は 16B の `movups` コピー 2 回、`32..=64` は 32B (`movups` 2 本) のコピー 2 回 | 125 命令、`memcpy` 1 回 | 成立 |
| `from(&str)` (i686) | `memcpy` は 1 回のみ | - | 成立 |
| `to_lowercase` | `pcmpgtb`, `pmovmskb` などを含み、チャンク処理がベクトル化されている | - | 成立 |

`Repr::last_byte` / `Repr::is_modifiable` / `Capacity::new_unchecked` の主張 (ある式を足す・順序を変えると codegen が悪化する、という種類) は、単一の asm を見ても成否が分からないので個別には確認していない。これらは変更前後の比較 (下の修正例 B の命令数による A/B など) でしか守れない。

壊れたら検出できるかも確かめた。`write_char` の `match` を実行時長の `ptr::copy_nonoverlapping(src, dst, encoded.len())` に置き換えたコピーで probe を作り直すと、`probe_push` に `memcpy` 呼び出しが 1 つ現れ、後述のチェックが `memcpy=1/0` で失敗した (確認済み)。

### 3. 副産物: 主張の外にある観察

- `LeanString::from(&str)` は inline 長の入力でも `push`/`pop` ×5 (callee-saved レジスタの退避) を行う。ヒープ経路の関数と同居しているため。`from` 自体の最適化の話で、ここでは検査対象にしていない。
- `to_lowercase` は 835 命令、`into_lean_str` は 219 命令が `#[inline]` で呼び出し側に丸ごと展開される (x86-64, LTO なし)。コードサイズの点で別途検討に値する。

## 原因

コード生成に対する検査がない (`tests/` は動作のみ、`bench/` は wall-clock のみで CI 非対象)。それぞれの主張は書かれた時点で `cargo asm` により手で確認されたと考えられるが (推測)、その手順も結果も残っていない。

## 修正例

### A. asm 検査 (推奨・ノイズなし)

`bench/asm-probe/` (または `tests/codegen/`) に probe crate を置く。LTO なし (`opt-level = 3`, `codegen-units = 1`)。`lib.rs` は `#[unsafe(no_mangle)] pub fn probe_*` の 1 行ラッパーで、下流が `#[inline]` から得るコードを見る。例:

```rust
use lean_string::{LeanString, LeanStr};
#[unsafe(no_mangle)] pub fn probe_len(a: &LeanString) -> usize { a.len() }
#[unsafe(no_mangle)] pub fn probe_as_str(a: &LeanString) -> &str { a.as_str() }
#[unsafe(no_mangle)] pub fn probe_from_char(c: char) -> LeanString { LeanString::from(c) }
#[unsafe(no_mangle)] pub fn probe_push(s: &mut LeanString, c: char) { s.push(c) }
#[unsafe(no_mangle)] pub fn probe_from_str(s: &str) -> LeanString { LeanString::from(s) }
```

検査スクリプト `check_asm.py` (抜粋; 実際に x86-64 / aarch64 の `--emit asm` で動かして通った。閾値は現在値に余裕を持たせた値で、「命令数・条件分岐数・`memcpy` 呼び出し数・呼び出し総数」の上限):

```python
CHECKS = [
    # symbol           insns  cond-br  memcpy  calls
    ("probe_len",         12,     0,      0,     0),   # Repr::len: 分岐なし
    ("probe_is_empty",    14,     0,      0,     0),
    ("probe_as_str",      16,     1,      0,     0),   # Repr::as_bytes: 分岐 1 (x86)
    ("probe_from_char",   60,     4,      0,     0),   # from_char: call なし
    ("probe_push",       130,    99,      0,     3),   # write_char: memcpy なし (grow + panic 2)
    ("probe_from_str",   130,    20,      1,     4),   # HeapBuffer::new: memcpy は > 64 の 1 か所
]
# 各 symbol の本体を .s から切り出し、命令数 / 条件分岐 (x86: ^j(?!mp), aarch64: ^b\.\w+|cbn?z|tbn?z) /
# memcpy・memmove への call / call 総数を数えて上限と比較する。超えたら exit 1。
```

見るのは `memcpy` の有無、`call` の数、分岐の数のような質的な性質だけにして、命令列そのものや厳密な命令数は比較しない (rustc のバージョンで揺れるため)。rustc を更新したときに閾値を見直す前提にする。CI は `cargo rustc --release --lib -- --emit asm` (x86-64) と `--target aarch64-unknown-linux-gnu` (target を `rustup target add` するだけで、`--emit asm` にリンクは不要) を実行して `check_asm.py` に渡す。i686 (`rustup target add i686-unknown-linux-gnu`) も `memcpy` 1 回の確認用に加えられる。

### B. 命令数 (Ir) による A/B (補助)

wall-clock と違い、命令数は完全に再現する。小さなドライバ (`#[inline(never)]` の関数が 1000 回ループ) を `valgrind --tool=callgrind --toggle-collect='drv::*'` で動かす。load average 12 の状態で `push_str` を 3 回測って同一の値になり、`clone` / `push` / `push_str` は current と prev (同一コード) でも完全に一致した (確認済み。例: `clone` 50B: 16015 Ir/1000 回、`push_str` ×16: 248904、`push('あ')` ×16: 217070、`from` 16B: 17 Ir/call、17B: 183、32B / 64B: 188、65B: 205。glibc malloc 込み)。

CI では、ベース (`origin/main`) と PR の両方をビルドして同じシナリオの Ir を並べて表示する形にすれば、rustc のバージョンに依存せず、閾値のチューニングも不要になる:

```
ubuntu-latest: sudo apt-get install -y valgrind
for rev in base head: cargo build --release (bench/ir) && ./run.sh <scenario>  -> Ir を表にして PR にコメント
```

失敗させるのではなく差を表示するだけ (informational) にしておけば、ノイズによる誤検知はない。

### C. 最低限

CI に `cd bench && cargo bench --no-run` を足して、ベンチがビルドできることだけを保証する (今は何も保証されていない)。

## 確認方法

- 上記 A のスクリプトが現在の `main` で通ること (x86-64, aarch64 で確認済み)。
- `write_char` の定数長コピーを実行時長に戻す、`HeapBuffer::new` の `if !(N..=4 * N).contains(&len)` を常に `copy_nonoverlapping(src, dst, len)` にする、など、コメントが警告している変更を入れて、チェックが失敗すること (`write_char` で確認済み)。
- `Repr::len` を `if self.is_heap_buffer() {..}` の分岐版に変えて、`probe_len` の条件分岐数の上限が効くこと。

## 関連

- [#55](./55-bench-baseline-and-noise.md): wall-clock の比較がノイズに埋もれるため、ここの仕組みが補助になる。
- [#56](./56-bench-coverage-gaps.md): `as_str` ベンチは `as_str` を測っておらず、この asm 検査が代わりの安全網になる。
