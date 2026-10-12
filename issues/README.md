# Issues

`main` の `5dd94ea` (v0.8.0 と同じソース) を対象に、性能・正しさ・ドキュメント・API・テストを調べて見つかった問題をまとめたもの。
1 ファイルが 1 つの問題で、問題の内容、原因 (メカニズム)、あれば修正例と確認方法を書いている。
修正例は「直せること」と「どこを直せばよいか」を示すためのもので、そのまま採用することを前提にはしていない。

各ファイルの冒頭に、種別・対象・影響・優先度を書いている。優先度は「効果の大きさ・利用者が困る度合い」と「直すコスト」から付けた目安。

## 測定の条件と方法

特に断りのない数値は次の条件で取った。

- コード: `5dd94ea`。rustc 1.97.0 (stable)。Miri は nightly 1.101.0 (2026-10-11)。
- ホスト: x86-64 Linux (4 コアの共有マシン。測定中もほかの処理で負荷がかかっていた)。aarch64 と i686 は asm の出力だけ見た。
- asm: lean_string を path で依存する下流クレートを作り、確認したい呼び出しを `#[unsafe(no_mangle)]` の 1 行の関数として並べ、
  `cargo rustc --release --lib -- --emit asm` で出力した (`opt-level = 3`、`codegen-units = 1`、**LTO なし**)。
  LTO を切っているのは、`#[inline]` の関数が下流クレートでどう展開されるかを見るため。命令数は各関数の本体の命令を数えたもの。
- 命令数 (Ir): 小さな driver バイナリを valgrind の callgrind / cachegrind で実行して数えた。実行ごとに同じ値になるので、負荷のあるマシンでも比較に使える。
  時間より先にこちらを根拠にしている。
- 確保回数: 確保・再確保・解放を数える `#[global_allocator]` で数えた。
- 時間: criterion (`bench/`、LTO あり) か、比べたい版を同じバイナリに入れて交互に回す自作の計測で取った。
  負荷のあるマシンなので、時間は傾向の確認にだけ使い、1 ns 未満の差やプロセスをまたぐ比較は根拠にしていない
  ([#55](./55-bench-baseline-and-noise.md) も参照)。
- Miri: `-Zmiri-strict-provenance` で、CI と同じ x86_64 / powerpc64 / i686 / powerpc のうち、問題に関係するものを実行した。
  race のテストは `-Zmiri-preemption-rate=1`。

## 一覧

### 不具合

| # | 優先度 | 種別 | タイトル |
| --- | --- | --- | --- |
| [01](./01-write-fmt-static-str-panic.md) | 低 | bug, api | `write!` の fast path と `try_from_fmt` が `from_static_str` の panic をそのまま使っている |

### 性能

| # | 優先度 | 種別 | タイトル |
| --- | --- | --- | --- |
| [02](./02-case-conversion-per-char-push.md) | 高 | perf | `to_lowercase` / `to_uppercase` が最初の非 ASCII 文字以降を 1 文字ずつ `push_char` で書くため、`str` の同名メソッドより 2〜3 倍遅い |
| [03](./03-char-loop-append.md) | 高 | perf | `Extend<char>` / `FromIterator<char>` / `from_utf16*` が 1 文字ずつ `push` し、`String` の 1.7〜3 倍の命令を使う |
| [04](./04-retain-memmove-per-char.md) | 高 | perf | `retain` が最初の削除以降、残す文字ごとに `memmove` を呼ぶ |
| [05](./05-from-utf8-lossy-fast-path.md) | 中 | perf | `from_utf8_lossy` が正しい UTF-8 でも `utf8_chunks` で 1 バイトずつ検証し、`LeanStr` 版は `realloc` も足される |
| [06](./06-int-to-repr-registers.md) | 中 | perf | 整数の変換がスタックのバッファに桁を書いてから読み直し、16 バイトの結果を細切れのストアで書き出す |
| [07](./07-push-char-not-inlined.md) | 中 | perf | `LeanString::push` が呼び出し側に展開されず、1 文字ごとに `Repr::push_char` を呼び出す |
| [08](./08-drop-by-value-stack-copy.md) | 中 | perf | 値で持っている `LeanString` / `LeanStr` の drop が 16 バイトをスタックにコピーする |
| [09](./09-conversion-restore-on-failure.md) | 中 | perf, safety-doc | `into_lean_str` / `into_lean_string` が使われない「失敗時に元へ戻す」仕組みのせいで重く、変換不要な文字列でも大きな関数を呼ぶ |
| [10](./10-leanstr-word-eq.md) | 中 | perf | `LeanStr` 同士の `==` が、2 ワードの比較で済む場面でも長さの復元と `bcmp` を通る |
| [11](./11-short-eq-bcmp.md) | 低 | perf | 16 バイト以下の文字列の `==` が毎回 `bcmp` を呼ぶ |
| [12](./12-ascii-case-inline-fast-path.md) | 中 | perf | inline の文字列の大文字・小文字変換が、確保しないのに `String` (確保あり) より遅い |
| [13](./13-inline-new-generic-registers.md) | 低 | perf | 32-bit / big-endian の `InlineBuffer::new` がスタック上のバッファを経由し、部分的に重なるストアの直後にワードで読み直す |
| [14](./14-lean-str-built-via-lean-string.md) | 低 | perf | `LeanStr` の構築関数のいくつかが `LeanString` で作ってから `into_lean_str` し、heap の結果ごとに `realloc` と全体の移動が入る |
| [15](./15-ensure-modifiable-outlined.md) | 低 | perf | `remove` / `retain` が inline や unique なバッファでも `ensure_modifiable` を関数呼び出しする |
| [16](./16-remove-retain-shared-copy-whole.md) | 低 | perf | 共有 / static バッファの `remove` と `retain` は全体をコピーしてから詰めるので、inline に収まる結果も heap に残る |
| [17](./17-to-lean-string-codegen.md) | 低 | perf | `to_lean_string()` / `to_lean_str()` が `Result<_, ToLeanStringError>` を経由し、結果のコピーが細切れになる。`str` の impl は展開されない |
| [18](./18-ref-conversion-via-clone.md) | 低 | perf | 参照からの `LeanString` ⇄ `LeanStr` 変換が `clone()` してから所有権ごと変換している |
| [19](./19-shared-buffer-ptr-eq.md) | 低 | perf | バッファを共有している 2 つの値の `==` / `cmp` が中身を全部比べる |
| [20](./20-fmt-write-char.md) | 低 | perf | `impl fmt::Write for LeanString` が `write_char` を実装しておらず、1 文字ごとに `memcpy` を呼ぶ |
| [21](./21-split-off-static-and-double-check.md) | 低 | perf | `split_off` が static バッファの後半をコピーし、境界チェックを 2 回する |
| [22](./22-lean-str-collect-single-element.md) | 低 | perf | 要素 1 つの `LeanStr` を `collect::<LeanStr>()` すると realloc が 2 回走る |
| [23](./23-repeat-inline-memcpy.md) | 低 | perf | `repeat` の結果が inline に収まるとき、スタック上のバッファへの短い `memcpy` を log n 回呼ぶ |
| [24](./24-truncate-32bit-shared-needless-copy.md) | 低 | perf, doc | 32-bit で長さが heap にある共有バッファの `truncate` / `pop` は、新しい長さがハンドルに収まってもコピーする |

### テスト

| # | 優先度 | 種別 | タイトル |
| --- | --- | --- | --- |
| [25](./25-realloc-failure-tests.md) | 高 | test | realloc の失敗経路がテストされておらず、テスト用アロケータでは `Layout` の食い違いも検出できない |
| [26](./26-race-tests-unique-path.md) | 高 | test | race テストが「相手の drop で unique になった直後にその場で書き換える」経路をほぼ通らず、`is_unique` の順序を弱めても検出できない。clone のオーバーフロー処理もテストされていない |
| [27](./27-test-eq-ord-hash.md) | 中 | test | 比較・順序・ハッシュのテストが無く、`!=` と `Ord` は一度も検証されていない |
| [28](./28-int-boundary-tests.md) | 中 | test | 整数→文字列のテストが 64-bit 整数の 16 桁以下 (inline 経路) と桁の境目をほとんど踏まない |
| [29](./29-test-32bit-length-on-heap.md) | 中 | test | 32-bit で長さを heap に置く経路の多くが、中身のある文字列でテストされていない |
| [30](./30-test-write-fmt-fast-path.md) | 中 | test | `write!` / `writeln!` が一度もテストされておらず、`write_fmt` の fast path の条件が守られていない |
| [31](./31-test-removal-ops-shared-static.md) | 中 | test | 削除・縮小系の操作に、共有 / static バッファで `String` と突き合わせる property テストが無い |
| [32](./32-test-static-shared-and-panics.md) | 低 | test | static / 共有バッファでの `clear` / `split_off` と、heap 上でのユーザー関数の panic がテストされていない |
| [33](./33-test-trait-impls.md) | 低 | test | `Extend` / `FromIterator` / `AsRef<Path>` / `AsRef<OsStr>` の多くの impl にテストが無い |
| [34](./34-test-miri-property-coverage.md) | 中 | test, infra | property テストがすべて Miri で ignore され、それだけが頼りの unsafe 経路が Miri で検査されていない |
| [35](./35-test-const-static-buffer.md) | 低 | test | `tests/const.rs` が static バッファを const 評価していない |

### unsafe の根拠の文書

| # | 優先度 | 種別 | タイトル |
| --- | --- | --- | --- |
| [36](./36-lastbyte-invariant.md) | 中 | safety-doc, doc | `Repr` の最後のバイトが有効な `LastByte` でなければならない不変条件がどこにも書かれていない |
| [37](./37-safety-comments.md) | 低 | safety-doc | unsafe の根拠が書かれていない、または別の操作の根拠になっている箇所がある |

### ドキュメント

| # | 優先度 | 種別 | タイトル |
| --- | --- | --- | --- |
| [38](./38-track-caller-missing.md) | 中 | bug, doc | panic する公開メソッドに `#[track_caller]` がなく、panic 位置がクレート内部を指す |
| [39](./39-panics-doc-gaps.md) | 中 | doc, safety-doc | panic しうる公開 API の `# Panics` が欠けている・不正確で、`# Errors` もほぼ無い |
| [40](./40-type-level-docs.md) | 中 | doc | `LeanString` 型自体のドキュメントが 1 行しかなく、記憶域・容量・CoW・panic 方針が型の docs から分からない |
| [41](./41-std-differences-docs.md) | 中 | doc | `String` との違い・提供しない API とその理由、所有権を取る変換がコピーであることがどこにも書かれていない |
| [42](./42-conversion-cost-docs.md) | 低 | doc | `into_lean_str` / `into_lean_string` の doc が「in place で realloc」と書くが、実際は常に文字列全体を動かす O(n) の変換である |
| [43](./43-readme-claims.md) | 低 | doc | README (クレートのトップページ) の記述が実際の挙動と食い違う・足りない |
| [44](./44-doc-capacity-shared-and-shrink-to-example.md) | 低 | doc | `capacity()` の doc が共有バッファの場合に触れておらず、`shrink_to()` の例のコメントが逆になっている |
| [45](./45-doc-wording.md) | 低 | doc, infra | rustdoc の文言の不備 (文法・名前の誤り・説明不足) と、CI が doc をビルドしない点 |
| [46](./46-traits-and-errors-docs.md) | 低 | doc, api | `ToLeanString` / `ToLeanStr` とエラー型の docs が最小限で、使い方・挙動の約束が分からない |

### API

| # | 優先度 | 種別 | タイトル |
| --- | --- | --- | --- |
| [47](./47-fallible-decoders.md) | 中 | api | UTF-8 / UTF-16 デコード関数に `try_` 版がなく、OOM を `Result` で受け取れない |
| [48](./48-std-conversions.md) | 中 | api | `LeanString` / `LeanStr` から `Box<str>` / `Rc<str>` / `Arc<str>` / `Vec<u8>` などへの `From` が無い |
| [49](./49-format-macro.md) | 中 | api, perf | `format!` 相当 (`lean_format!` / `LeanString::from_fmt`) が無く、`format_args!(..).to_lean_string()` はリテラルでも確保する |
| [50](./50-drain.md) | 中 | api, test | `LeanString::drain` が無い (`tests/alloc_string.rs` の drain テストはコメントアウトのまま) |
| [51](./51-as-mut-str.md) | 低 | api (設計判断を含む) | `&mut str` を得る手段 (`as_mut_str` / `AsMut<str>` / `DerefMut` / `IndexMut`) が無い |
| [52](./52-sharing-introspection.md) | 低 | api | バッファが共有されているかを調べる公開 API (`ptr_eq` / `is_unique`) が無い |
| [53](./53-str-methods-returning-string.md) | 低 | api (設計判断を含む), doc | `Deref<Target = str>` 由来の `replace` / `replacen` / `join` / `concat` は `LeanString` でなく `String` を返す |
| [54](./54-small-additions.md) | 低 | api | 小さな API の欠け: `INLINE_CAPACITY` 定数、`try_to_uppercase`、`const fn` の不足、`Extend<&LeanString>` |

### ベンチ・CI

| # | 優先度 | 種別 | タイトル |
| --- | --- | --- | --- |
| [55](./55-bench-baseline-and-noise.md) | 中 | infra, perf | `apis.rs` の `prev` は直前のリリースに固定されていて、リリース直後は current と同一コードになり、差はすべてノイズになる |
| [56](./56-bench-coverage-gaps.md) | 中 | infra, test, perf | ベンチが公開 API の大半を測っておらず、`as_str` のベンチは `as_str` を測っていない |
| [57](./57-codegen-claims-unchecked.md) | 中 | infra, test, perf | コードのコメントが前提にしているコード生成 (cmov・`memcpy` なし・レジスタ保持など) を検査する仕組みがなく、CI にベンチも asm 検査もない |
| [58](./58-tests-need-rust-1-98.md) | 低 | test, infra | テストのビルドに Rust 1.98 以降が必要で、MSRV (1.88) との差がどこにも書かれていない |

## 関係の深いもの

- 1 文字ずつの追記: [#02](./02-case-conversion-per-char-push.md)、[#03](./03-char-loop-append.md)、[#07](./07-push-char-not-inlined.md)、[#20](./20-fmt-write-char.md)。
  同じ原因 (書き込み先の判定を 1 文字ごとにやり直す) で、修正も共有できる。
- `LeanStr` の構築と `LeanString` ⇄ `LeanStr` の変換: [#05](./05-from-utf8-lossy-fast-path.md)、[#09](./09-conversion-restore-on-failure.md)、
  [#14](./14-lean-str-built-via-lean-string.md)、[#18](./18-ref-conversion-via-clone.md)、[#22](./22-lean-str-collect-single-element.md)、[#42](./42-conversion-cost-docs.md)。
  [#09](./09-conversion-restore-on-failure.md) は [#25](./25-realloc-failure-tests.md) のテストを先に入れておくと安全に進められる。
- 比較: [#10](./10-leanstr-word-eq.md)、[#11](./11-short-eq-bcmp.md)、[#19](./19-shared-buffer-ptr-eq.md)。どれも [#27](./27-test-eq-ord-hash.md) のテストを先に入れておきたい。
- 参照カウントと並行性: [#08](./08-drop-by-value-stack-copy.md)、[#26](./26-race-tests-unique-path.md)。
- 32-bit で長さを heap に置く経路: [#24](./24-truncate-32bit-shared-needless-copy.md)、[#29](./29-test-32bit-length-on-heap.md)。
- `write!` / `fmt::Write`: [#01](./01-write-fmt-static-str-panic.md)、[#20](./20-fmt-write-char.md)、[#30](./30-test-write-fmt-fast-path.md)、[#49](./49-format-macro.md)。
- ベンチと検証の仕組み: [#55](./55-bench-baseline-and-noise.md)、[#56](./56-bench-coverage-gaps.md)、[#57](./57-codegen-claims-unchecked.md)。
  多くの perf の issue の「確認方法」で足すことを勧めているベンチは、[#56](./56-bench-coverage-gaps.md) にまとめてある。

## 調べて問題が無かったもの

同じところを調べ直さないための記録。

### 構築

- `InlineBuffer::new` (64-bit LE): 2 ワードをレジスタで組み立て、ストア 2 回で終わる (x86-64 / aarch64)。
- `InlineBuffer::new` の分岐の順で `len == 0` を先頭にする案: 空文字列は 17 → 12 命令になるが、長さ 1〜15 がそれぞれ 1〜2 命令増える。
  `LeanString::new()` / `Default` はこの関数を通らないので、変える価値は無い。
- `HeapBuffer::new`: 17〜31 バイトは 16 バイト×2、32〜64 バイトは 16 バイト×4 の固定長コピーになり、65 バイト以上だけ `memcpy` を呼ぶ。
- `new_uninit` / `allocate_ptr` / `layout_for` / `Capacity::new` / `TextLen::new`: 64-bit では範囲チェックが `__rust_alloc` の前の 1 回の比較に畳まれる。
- `from_char` / `from_bool` / `from_static_str` / `with_capacity` / `from_utf8`: レジスタだけで組み立てるか、必要な呼び出しだけになっている。
- 32 bit 以下の整数の変換では heap の分岐がすでに消えている (LLVM が桁数の上限から導いている) ので、ヒントを足す必要は無い。
- serde (`deserialize_str` + `visit_str` / `visit_borrowed_str`) と arbitrary (`&str` に委譲) に問題は無い。

### 追記・変更

- `push_str` の fast path は呼び出し側に展開され、`checked_add` や static の分岐は無い。判別子の読み直しは 2 回残るが、1 つはコピーでバイト 15 が
  上書きされうるため、もう 1 つは参照カウントの `Acquire` ロードのため、どちらも必要。
- 短い `push_str` のコピーを固定長のコピーの組み合わせにする案: callgrind の命令数が増えた (1 バイト×1000 回で 57.2k → 67.2k)。
- `reserve` / `reserve_exact` / `insert` (char) / `insert_str` / `extend_from_within`: 余計な呼び出しや分岐は無い。
- `Extend<&str>` / `FromIterator<&str>` が長さを先に合計しないのは `String` と同じ。
- 1.5 倍の成長 (`Repr::grow_amortized`) は `String` (2 倍) より再確保が多いが、意図した設計で、命令数の差の主因でもない。
- `truncate` / `pop` は 64-bit では確保もせず、共有された heap にも書かない (ハンドルの長さだけを書き換える)。
- 何も削らない `retain` は共有 / static のバッファを保つ。`retain` の述語が panic しても、文字列は有効な UTF-8 で、clone も変わらない (`SetLenOnDrop`)。
- 共有 / static のバッファを `truncate` した後に変更すると、内容が収まれば inline にコピーされる (`remove` / `retain` を除く。[#16](./16-remove-retain-shared-copy-whole.md))。
- `clear`: unique なら容量を保ち、共有なら確保せずに空の inline になり、static なら長さ 0 の static になる。doc のとおり。
- `shrink_to` / `shrink_to_fit`: `String` と同じ意味で、共有なら容量を詰めたコピー、16 バイト以下なら inline になる。
- `to_lowercase` / `to_uppercase` の `'Σ'` と、長さが変わる変換 (`İ`, `ŉ`, `ΐ`, `ﬀ`, Kelvin 記号) は `str` と一致する。

### 所有権・参照カウント

- atomic の順序は `Arc` と同じ (`fetch_add(Relaxed)`、`fetch_sub(Release)` + 最後の参照での `fence(Acquire)`、`is_unique` の `load(Acquire)`)。
  `Weak` にあたるものが無いので、カウント 1 を観測すれば排他になる。
- `Clone` は 12 命令 (判別子の比較、`lock incq`、16 バイトのコピー) で、オーバーフローの処理は `.text.unlikely` にある。
  `ref_count_overflow` は panic の前にカウントを戻す。
- `clone_from` は先に元の参照を増やしてから `self` を手放すので、同じバッファ同士の代入でも安全。
- `Vec<LeanString>` の drop のループは要素のアドレスを cold 関数に渡しており、コピーは無い。
- 共有バッファを手放す経路 (`ensure_modifiable`、`reallocate`、`truncate_unchecked`、`clear`、変換) は新しいバッファを作ってから `release` するので、
  確保に失敗しても共有バッファは無事。
- `FromIterator<LeanString> for LeanStr` の要素 1 つでの realloc 1 回、`ToLeanStr` の `Display` 経由の最後の realloc は、レイアウトの変換のため避けられない。

### 比較・読み出し

- `Repr::len` (64-bit) は分岐が無い (x86-64 は `cmov` 2 つ、aarch64 は `csel`)。`Repr::as_bytes` が x86-64 で分岐になるという NOTE も正しい。
  `hint::select_unpredictable` は分岐を消すが、まだ `const fn` で使えない。const でない版を試したが、速くなるとは言えなかった。
- `Hash` は `str::hash` に委譲しており、`Borrow<str>` / `Eq` / `Ord` と整合している。リテラルとの比較 (`s == "hello"`) は `bcmp` を呼ばない。
- 空の `LeanString` のデータポインタは自分自身を指すので、空の `String` 同士の比較で起きる glibc の遅い経路 ([#11](./11-short-eq-bcmp.md)) には入らない。
- 試して遅くなった案: inline 同士の `Ord` をワード比較にする案、正規形を前提にしないマスク付きの比較を `LeanString` に使う案
  ([#10](./10-leanstr-word-eq.md)、[#11](./11-short-eq-bcmp.md) に記録)。

### unsafe と健全性

- 変更していない `5dd94ea` で、テスト全体が Miri の 4 ターゲット (strict provenance) と loom で通る。race のテストも preemption 付きで通る。
- `Repr` のバイト 15 に書き込む箇所はすべて有効な `LastByte` の値を書いている ([#36](./36-lastbyte-invariant.md) に一覧)。
- `Repr` のフィールド 0 (`*const ()`) に inline の文字列のバイトが入るのは strict provenance でも問題ない (heap / static のときしかポインタとして読まない)。
- `from_utf16le` / `from_utf16be` の `align_to::<u16>()` は健全 (`u16` に無効なビットパターンは無く、奇数長や非整列の場合は別の経路に落ちる)。
- `write_fmt` の fast path で `args.as_str()` を `from_static_str` に渡すのは健全 (`&'static str` が返る)。差し替える前の値は heap でない空文字列なので漏れも無い。
- 最近の変更 (`replace_range`、`extend_from_within`、`reserve_exact`、`Repr::reallocate` / `shrink_to`、`clear` の移動、case 変換、
  `HeapBuffer::new` のコピーの分割、`write_char`) をコードと Miri で確かめ、不具合は見つからなかった。

### ドキュメント

- `cargo test --doc --all-features` と `RUSTDOCFLAGS="-D warnings" cargo doc --all-features --no-deps` は通る (`--document-private-items` だけ警告が出る。[#45](./45-doc-wording.md))。
- doc に書かれた上限 (64-bit の `2^56 - 1`、32-bit の `LeanString` の `2^31 - 16` / `LeanStr` の `2^31 - 12`、`from_static_str` の 32-bit の `2^24 - 1`) は実装と一致する。
- `try_shrink_to_fit` / `try_shrink_to` / `try_pop` の失敗条件と、`clear` の static の容量の記述は実装のとおり。
- `try_*` は失敗したとき `self` を変えない (12 メソッド×共有 heap / static で確認)。ただし doc に書いてあるのは 2 つだけ ([#39](./39-panics-doc-gaps.md))。
- CHANGELOG の 0.8.0 の記述はコードと矛盾しない。

### ベンチ

- `clone` / `to_lowercase` / `to_uppercase` / `eq` のベンチは、ループの外に追い出されていない。
- `push_str/reserved` は `clear()` で容量を保つので、再確保ではなく追記を測れている。
- コメントにあるコード生成の前提 (`len` が分岐なし、`push` / `insert` が `memcpy` を呼ばない、`from_char` がレジスタだけ、ASCII の走査がベクトル化される など) は、
  確かめられたものはすべて今も成り立つ ([#57](./57-codegen-claims-unchecked.md))。
