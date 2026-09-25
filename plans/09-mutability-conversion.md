# 09: 可変性の変換 — ABI と余計な往復

種別: perf / 実装量: 中 (A) + 小 (B〜D) / 設計判断: 小

`LeanString` (Mutable) と `LeanStr` (Immutable) は heap バッファのヘッダが違う
(`GrowableHeader { count, capacity }` と `ExactHeader { count }`)。
両者の変換は `Repr::into_immutable` / `into_mutable` を通り、unique な heap バッファなら
`HeapBuffer::into_exact` / `into_growable` で文字列全体の `ptr::copy` + `realloc` + ヘッダの書き直しになる。
共有されていれば新しく確保する。

ここに 2 種類の無駄がある。A は変換そのものの ABI、B〜D は呼び出し側での余計な往復。

## A. 失敗しうる変換の戻り値が sret になる

4 つの変換はどれも「失敗したら元の値をエラーと一緒に返す」形になっている。

```rust
// src/repr.rs
pub(crate) fn into_immutable(self) -> Result<Repr<Immutable>, (Self, ReserveError)>
pub(crate) fn into_mutable(self)   -> Result<Repr<Mutable>,   (Self, ReserveError)>

// src/repr/heap_buffer.rs
pub(super) unsafe fn into_exact(self)    -> Result<HeapBuffer<ExactHeader>,    (Self, ReserveError)>
pub(super) unsafe fn into_growable(self) -> Result<HeapBuffer<GrowableHeader>, (Self, ReserveError)>
```

エラー側に値を載せると `Result` が `LastByte` の niche を使えなくなり、16 バイトから
24 バイトになってレジスタで返せなくなる。フィールド構成を再現したプログラムで確認した。

| 型 | サイズ | 返し方 |
| --- | --- | --- |
| `Repr<M>` | 16 | `rax:rdx` |
| `Result<Repr<M>, ReserveError>` | 16 | `rax:rdx` |
| `Result<Repr<Immutable>, (Repr<Mutable>, ReserveError)>` | **24** | **sret** |
| `HeapBuffer<H>` | 16 | `rax:rdx` |
| `Result<HeapBuffer<Exact>, ReserveError>` | 16 | `rax:rdx` |
| `Result<HeapBuffer<Exact>, (HeapBuffer<Growable>, ReserveError)>` | **24** | **sret** |
| `Result<(), ReserveError>` | 1 | レジスタ |

`probe_into_lean_str` (= `fn(LeanString) -> LeanStr`) の asm にそのまま出ている
([research/codegen-baseline.md](../research/codegen-baseline.md) の §10)。

```asm
	leaq	32(%rsp), %rdi        # sret のポインタ
	callq	*..HeapBuffer..into_exact@GOTPCREL(%rip)
	movq	8(%rsp), %r12
	movq	40(%rsp), %r15        # 結果をスタックから読む
	movq	48(%rsp), %rbp
	cmpl	$1, 32(%rsp)          # 判別子もスタックから
```

### 方針

`&mut self` を取り、`Result<(), ReserveError>` を返すようにする。戻り値は 1 バイトになり、sret は無くなる。

```rust
pub(crate) fn make_immutable(&mut self) -> Result<(), ReserveError>
pub(crate) fn make_mutable(&mut self)   -> Result<(), ReserveError>
pub(super) unsafe fn realloc_into_exact(&mut self)    -> Result<(), ReserveError>
pub(super) unsafe fn realloc_into_growable(&mut self) -> Result<(), ReserveError>
```

`Repr<Mutable>` から `Repr<Immutable>` に型が変わるので `&mut self` では書けない、というのが今の形の
理由だと思われる。ただこの 2 つは同じレイアウトで `mem::transmute` できる型
(`into_immutable` の SAFETY コメントにその根拠がある) なので、`Repr<Mutable>` のまま中身を書き換え、
最後に呼び出し側で `transmute` すればよい。

`HeapBuffer` 側 (`into_exact` / `into_growable`) は `ExactHeader` と `GrowableHeader` で
`header_offset` が違うので、同じやり方は使えない。こちらは `&mut HeapBuffer<Growable>` を取り、
成功したら新しい `ptr` を書き戻して `Result<(), ReserveError>` を返し、呼び出し側で `transmute` する。

意味は変わらない。失敗したときにバッファがその場で有効なまま残るのは、今のコードが手で復元している
状態と同じ。

`LeanString::try_into_lean_str` と `LeanStr::try_into_lean_string` が今やっている
`mem::replace` と失敗時の復元も、その場で変換する形なら要らなくなる。

### 検証

- asm: `probe_into_lean_str` から sret のポインタ引数とスタック経由の受け取りが消えること。
  x86-64 と aarch64 の両方で見る。
- 確保回数: 変換の前後で `alloc` / `realloc` の回数が変わらないこと。
- 失敗時の状態: 確保を失敗させるアロケータで、`try_into_lean_str()` が `Err` を返した後も
  元の文字列が壊れていないこと。この経路は今まったくテストされていない
  ([10](./10-test-hardening.md) の A) ので、先にテストを足してから始める。
- Miri: `into_exact` の失敗時の復元はこのクレートでいちばん込み入った unsafe なので、4 ターゲットすべて。

## B. `FromIterator<LeanStr> for LeanStr` に要素が 1 つだけのとき

確保を数えるグローバルアロケータで測った (48 バイトの文字列)。

| 操作 | alloc | realloc | あるべき値 |
| --- | --- | --- | --- |
| `[LeanString; 1] → LeanString` (collect) | 0 | 0 | 0 / 0 |
| `[LeanStr; 1] → LeanString` (collect) | 0 | 1 | 0 / 1 |
| `[LeanString; 1] → LeanStr` (collect) | 0 | 1 | 0 / 1 |
| **`[LeanStr; 1] → LeanStr` (collect)** | **0** | **2** | **0 / 0** |

今の実装は次のとおり。

```rust
impl FromIterator<LeanStr> for LeanStr {
    fn from_iter<T: IntoIterator<Item = LeanStr>>(iter: T) -> Self {
        let mut iter = iter.into_iter();
        let mut buf = match iter.next() {
            Some(str) => str.into_lean_string(),   // Immutable → Mutable (realloc)
            None => return LeanStr::new(),
        };
        buf.extend(iter);
        buf.into_lean_str()                        // Mutable → Immutable (realloc)
    }
}
```

要素が 1 つだけなら 2 つの変換は打ち消し合うのに、`realloc` を 2 回している。
`FromIterator<LeanString> for LeanString` は最初の要素をそのままバッファに使うので、
1 要素なら確保は起きない。これに揃える。

```rust
let Some(first) = iter.next() else { return LeanStr::new() };
let Some(second) = iter.next() else { return first };   // 追加するものが無いのでそのまま返す
let mut buf = first.into_lean_string();
buf.push_str(&second);
buf.extend(iter);
buf.into_lean_str()
```

`FromIterator<LeanString> for LeanStr` は `LeanString::from_iter(iter).into_lean_str()` で、
1 要素なら `into_lean_str` 1 回。これ以上減らせないので変更しない。

## C. `ToLeanString` / `ToLeanStr` での相互変換

`ToLeanString` は `LeanStr` を `LeanString` にするとき (src/traits.rs)

```rust
&LeanStr as s => return Ok(s.clone().try_into_lean_string()?),
```

としている。`clone()` の `fetch_add` でバッファが共有状態になるので、`into_mutable` は
共有されているときの分岐に入り、結局コピーする。その後 `release()` で `fetch_sub` する。
確保は 1 回。

`Repr::<Mutable>::from_str(s.as_str())` にすれば、atomic 操作なしで同じ結果になる。
確保の回数は変わらない (どちらも 1 回) が、`fetch_add` / `fetch_sub` と共有判定の分岐が無くなる。

逆向き (`ToLeanStr` で `&LeanString` を `LeanStr` にする) も同じ形なので、
`Repr::<Immutable>::from_str(s.as_str())` にできる。

## D. `LeanStr` の構築が `LeanString` を経由している

`LeanStr` の構築の多くは、一度 `LeanString` を作ってから `into_lean_str()` している。

| 場所 | 今の実装 |
| --- | --- |
| `LeanStr::from_utf8_lossy` | `LeanString::from_utf8_lossy(..).into_lean_str()` |
| `LeanStr::from_utf16` / `_lossy` | 同上 |
| `LeanStr::from_utf16le` / `be` とその lossy | 同上 |
| `LeanStr::to_lowercase` / `to_uppercase` | `LeanString::map_case_if_changed(..)` の結果を `into_lean_str()` |
| `FromIterator` 系 | `LeanString::from_iter(iter).into_lean_str()` |
| `ToLeanStr` の Display へのフォールバック | 同上 |

どれも `into_lean_str()` の `realloc` が 1 回余分にかかる。

一方、`LeanStr::repeat` と `LeanStr::to_ascii_*case` は `Repr::new_with` で `Repr<Immutable>` を
直接作っているので、この往復は無い。

`from_utf8_lossy` / `from_utf16` / `from_utf16_lossy` は最終的な長さが先に分かる (か、安く計算できる) ので、
`Repr::<Immutable>::new_with` で直接作れば往復が無くなる。
`to_lowercase` / `to_uppercase` は最終的な長さが分からないので、`Repr::with_capacity_init` が
`Repr<Mutable>` 専用であることがネックになる。`FromIterator` 系と同じく長さが事前に分からないので、
[03](./03-append-writer.md) の writer が `Repr<Immutable>` にも使えるようになれば一緒に片付く。

順番としては 03 の後。writer の形が決まる前に個別に直すと、やり直しになる。

## E. `From<String>` / `From<Box<str>>` はバッファを引き継げない (doc のみ)

`From<String>` と `From<Box<str>>` は中身をコピーしてから元を解放する。heap のレイアウトが
参照カウントのヘッダをデータの前に置いている以上、これは避けられない
(`String::from(Box<str>)` が O(1) なのとは違う)。

変える余地は無いが、doc に書かれていない。`String` からの変換が O(n) なのは利用者にとって意外なので、
[06](./06-doc-fixes.md) の D で `From` impl の doc に書く。

## 検証 (B〜D)

- 確保回数: 上の表の各行を、確保を数えるグローバルアロケータで固定する。
  B と C は退行テストとして特に役に立つ。
- テスト: B の変更で 0 要素 / 1 要素 / 2 要素以上がそれぞれ正しいこと。
  1 要素のときは元のバッファをそのまま共有していること (`as_ptr()` が一致すること) を確認する。
- criterion: 変換のベンチは今無い。B と C は確保の回数が変わるので、ベンチを足すより
  確保回数のテストで固定するほうが確実。

## 依存

- A は [10](./10-test-hardening.md) の A (realloc 失敗のテスト) の後に。
- B と C は独立しているのでいつでもよい。
- D は [03](./03-append-writer.md) の後。
