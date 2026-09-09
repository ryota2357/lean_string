# 12: 可変性の変換 — ABI と往復

種別: perf / 実装量: 中 (A) + 小 (B〜D) / 設計判断: 小

`LeanString` (Mutable) と `LeanStr` (Immutable) はヒープバッファのヘッダが違う
(`GrowableHeader { count, capacity }` と `ExactHeader { count }`)。
両者の変換は `Repr::into_immutable` / `into_mutable` (repr.rs:904-980) を通り、
unique な heap バッファに対しては `HeapBuffer::into_exact` / `into_growable` を呼んで
文字列全体の `ptr::copy` + `realloc` + ヘッダの書き直しになる。共有されていれば新規確保。

この経路に 2 種類の無駄がある。A は変換そのものの ABI、B〜D は呼び出し側の往復。

## A. fallible な変換の戻り値が sret になる

現在の 4 つの変換はいずれも「失敗したら元の値をエラーと一緒に返す」形をしている。

```rust
// repr.rs:904, 950
pub(crate) fn into_immutable(self) -> Result<Repr<Immutable>, (Self, ReserveError)>
pub(crate) fn into_mutable(self)   -> Result<Repr<Mutable>,   (Self, ReserveError)>

// heap_buffer.rs:473, 587
pub(super) unsafe fn into_exact(self)    -> Result<HeapBuffer<ExactHeader>,    (Self, ReserveError)>
pub(super) unsafe fn into_growable(self) -> Result<HeapBuffer<GrowableHeader>, (Self, ReserveError)>
```

エラー側にペイロードが載ると `Result` が `LastByte` の niche を使えなくなり、
**16 バイトから 24 バイトに膨らんでレジスタ返しから外れる**。
フィールド構成を再現した独立のプログラムで実測した。

| 型 | サイズ | 返し方 |
| --- | --- | --- |
| `Repr<M>` | 16 | `rax:rdx` |
| `Result<Repr<M>, ReserveError>` | 16 | `rax:rdx` |
| **`Result<Repr<Immutable>, (Repr<Mutable>, ReserveError)>`** | **24** | **sret** |
| `HeapBuffer<H>` | 16 | `rax:rdx` |
| `Result<HeapBuffer<Exact>, ReserveError>` | 16 | `rax:rdx` |
| **`Result<HeapBuffer<Exact>, (HeapBuffer<Growable>, ReserveError)>`** | **24** | **sret** |
| `Result<(), ReserveError>` | 1 | レジスタ |

`probe_into_lean_str` (= `fn(LeanString) -> LeanStr`) の asm にそのまま現れている。

```asm
.LBB16_22:
	movq	(%rsp), %rsi
	movq	8(%rsp), %rdx
	leaq	16(%rsp), %rdi        # ★sret のポインタ
	callq	*..HeapBuffer..into_exact..E@GOTPCREL(%rip)
	movq	24(%rsp), %r15        # ★スタックから読み直し
	movq	32(%rsp), %rbp
	cmpl	$1, 16(%rsp)          # ★判別子もスタックから
```

### 方針

`&mut self` を取って `Result<(), ReserveError>` を返す形にする。
戻り値は 1 バイトになり、sret が消える。

```rust
pub(crate) fn make_immutable(&mut self) -> Result<(), ReserveError>
pub(crate) fn make_mutable(&mut self)   -> Result<(), ReserveError>
pub(super) unsafe fn realloc_into_exact(&mut self)    -> Result<(), ReserveError>
pub(super) unsafe fn realloc_into_growable(&mut self) -> Result<(), ReserveError>
```

型が変わる (`Repr<Mutable>` → `Repr<Immutable>`) ので `&mut self` のままでは書けない、
というのが現在の形になっている理由と思われるが、この 2 つは `mem::transmute` で
行き来できる同レイアウトの型 (repr.rs:906-909 の SAFETY コメントがその根拠を書いている)。
`Repr<Mutable>` を受け取って中身を書き換え、最後に呼び出し側が `transmute` する形にできる。

`HeapBuffer` 側 (`into_exact` / `into_growable`) は `ExactHeader` と `GrowableHeader` で
`header_offset` が違うので同じ手は使えない。こちらは
「`&mut HeapBuffer<Growable>` を取り、成功したら新しい `ptr` を書き戻して
`Result<(), ReserveError>` を返す」形にして、呼び出し側で `transmute` する。

意味は変わらない。失敗したときバッファがその場で有効なまま残るのは、
いまのコードが手で復元しているものと同じ状態である。

`LeanString::try_into_lean_str` (lib.rs:1243-1254) と
`LeanStr::try_into_lean_string` (lib.rs:1733-1744) が現在やっている
`mem::replace` + 失敗時の復元も、in-place の形なら不要になる。

### 検証方針

- asm: `probe_into_lean_str` から sret のポインタ引数とスタック経由の
  受け取りが消えること。x86-64 と aarch64 の両方。
- 確保回数: 変換の前後で `alloc` / `realloc` の回数が変わらないこと。
- 失敗時の状態: 確保を失敗させるアロケータで、`try_into_lean_str()` が
  `Err` を返したあと元の文字列が壊れていないこと。
  **この経路は現在まったくテストされていない** ([plans/13](./13-test-hardening.md) の A)
  ので、先にテストを足してから着手する。
- Miri: `into_exact` の失敗時復元はこのクレートでもっとも入り組んだ unsafe なので、
  4 ターゲットすべてで回す。

## B. `FromIterator<LeanStr> for LeanStr` の 1 要素

カウントするグローバルアロケータで測った結果 (48 バイトの文字列):

| 操作 | alloc | realloc | あるべき値 |
| --- | --- | --- | --- |
| `[LeanString; 1] → LeanString` (collect) | 0 | 0 | 0 / 0 |
| `[LeanStr; 1] → LeanString` (collect) | 0 | 1 | 0 / 1 |
| `[LeanString; 1] → LeanStr` (collect) | 0 | 1 | 0 / 1 |
| **`[LeanStr; 1] → LeanStr` (collect)** | **0** | **2** | **0 / 0** |

lib.rs:2396-2406 は次の形になっている。

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

要素が 1 つしかないとき、2 つの変換が打ち消し合うのに `realloc` を 2 回通る。
同じファイルの `FromIterator<LeanString> for LeanString` (lib.rs:2367-2376) は
「最初の要素をそのままバッファとして使う」形になっていて、1 要素なら確保が起きない。
ここを揃える。

```rust
let Some(first) = iter.next() else { return LeanStr::new() };
let Some(second) = iter.next() else { return first };   // 追加なし。そのまま返す
let mut buf = first.into_lean_string();
buf.push_str(&second);
buf.extend(iter);
buf.into_lean_str()
```

`FromIterator<LeanString> for LeanStr` (lib.rs:2390-2394) は
`LeanString::from_iter(iter).into_lean_str()` で、1 要素なら `into_lean_str` 1 回。
これは最小なので変更不要。

## C. `ToLeanString` / `ToLeanStr` の相互変換

src/traits.rs:75 は `LeanStr` を `LeanString` にするとき

```rust
&LeanStr as s => return Ok(s.clone().try_into_lean_string()?),
```

としている。`clone()` が `fetch_add` するのでバッファが共有状態になり、
`into_mutable` は「共有されている」分岐 (repr.rs:962-978) に入って結局コピーする。
その後 `release()` が `fetch_sub` する。実測でも `alloc=1`。

`Repr::<Mutable>::from_str(s.as_str())` にすれば同じ結果を atomic 操作なしで得られる。
確保の回数は変わらない (どちらも 1 回) が、`fetch_add` / `fetch_sub` の対と
共有判定の分岐が消える。

src/traits.rs:156 の逆方向 (`&LeanString` → `LeanStr`) も同じ形で、
`Repr::<Immutable>::from_str(s.as_str())` にできる。

## D. `LeanStr` の構築が `LeanString` を経由する

`LeanStr` の構築は `from(&str)` と `from_static_str` を除いて、いったん `LeanString` を
作ってから `into_lean_str()` する形になっている。

| 場所 | 現状 |
| --- | --- |
| `LeanStr::from_utf8_lossy` (lib.rs:1418) | `LeanString::from_utf8_lossy(..).into_lean_str()` |
| `LeanStr::from_utf16` / `_lossy` (lib.rs:1457/1474) | 同上 |
| `LeanStr::from_utf16le` / `be` とその lossy (lib.rs:1505-1582) | 同上 |
| `FromIterator` 系 (lib.rs:2293-2406) | 同上 |
| `ToLeanStr` の Display フォールバック (traits.rs:158-161) | 同上 |

いずれも `into_lean_str()` の `realloc` が 1 回余分に乗る。

`from_utf8_lossy` / `from_utf16` / `from_utf16_lossy` は最終長が先に分かる
(または安く計算できる) ので、`Repr::<Immutable>::new_with` 相当で直接作れば往復が消える。
`FromIterator` 系は長さが事前に分からないので、[plans/04](./04-append-writer.md) の
writer が `Repr<Immutable>` に対しても使える形になっていれば自然に片付く。

順序としては 04 の後。writer の形が決まる前に個別対応を入れると二度手間になる。

## E. `From<String>` / `From<Box<str>>` がバッファを奪えない (ドキュメントのみ)

lib.rs:2136 と lib.rs:2186 は `String` / `Box<str>` の中身をコピーしてから元を解放する。
ヒープレイアウトが参照カウントのヘッダをデータの前に置く以上これは避けられない
(`String::from(Box<str>)` が O(1) なのとは違う)。

**変更する余地は無いが、ドキュメントに書かれていない**。
`String` からの変換が O(n) であることは利用者にとって意外なので、
[plans/09](./09-doc-fixes.md) で `From` impl の doc として書く。

## 検証方針 (B〜D)

- 確保回数: 上の表の各行を、カウントするグローバルアロケータで固定する。
  B と C は退行テストとして価値が高い。
- テスト: B の変更で 0 要素 / 1 要素 / 2 要素以上の 3 ケースが正しいこと。
  1 要素のとき元のバッファをそのまま共有していること (`as_ptr()` の一致で確認)。
- criterion: 現状 bench に変換系が無い。B と C は確保の回数が変わるので、
  ベンチを足すより確保回数のテストで固定するほうが確実。

## 依存

- A は [plans/13](./13-test-hardening.md) の A (realloc 失敗のテスト) を先に。
- B と C は独立。いつでも実施できる。
- D は [plans/04](./04-append-writer.md) の後。
