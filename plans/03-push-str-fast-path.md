# 03: `push_str` の fast path 整理

種別: perf / 実装量: 小〜中 / 設計判断: 中 (内部 API の形)

## 背景

`Repr::push_str` (src/repr.rs:567-593) は毎回 `reserve` を呼ぶ。`reserve` は cold 部分を
outline 済みなので関数呼び出しが毎回起きるわけではないが、
[research/codegen-baseline.md](../research/codegen-baseline.md) の §3 で測ったとおり
次の 3 つが残っている。

1. **判別子の読み直しが 3 回**。fast path の判定で `last_byte` を読み、`as_mut_ptr` で
   もう一度読み、`set_len` でさらにもう一度読んで 3 分岐する。`push_str` は最初の判定で
   バッファ種別を知っているのに、それを後段へ渡す手段がない。
2. **追記のコピーが可変長 memcpy の呼び出し** ([plans/02](./02-medium-copy.md) の題材)。
3. **関数全体が大きく、呼び出し側でインライン化されない**。`Repr::push_str` は
   106 命令あり、`#[inline]` が付いていても下流クレートからの呼び出しでは
   `callq Repr::push_str` になる。

`Extend<&str>` / `Extend<String>` などは 1 要素ごとにこの関数を呼ぶので、
3 の影響は要素数に比例する。

## 方針

### 案A: 容量チェックだけを inline に残し、grow を cold へ

`reserve` を呼ぶ前に fast path の条件を直接判定する形。

```rust
#[inline]
pub(crate) fn push_str(&mut self, string: &str) -> Result<(), ReserveError> {
    if string.is_empty() {
        return Ok(());
    }
    let len = self.len();
    let str_len = string.len();

    // fast path が成立しないときだけ reserve へ。
    // - StaticBuffer は必ず変換が要る
    // - 共有 HeapBuffer は容量に関係なく detach が要る (CoW)
    if self.is_static_buffer() || !self.is_unique() || len + str_len > self.capacity() {
        self.reserve(str_len)?;
    }

    unsafe { /* 既存の copy + set_len */ }
    Ok(())
}
```

- `is_unique()` は InlineBuffer でも `true` を返す (repr.rs:221-228) ので条件は素直に書ける。
  inline のときの `capacity()` は定数 16 に畳まれる。
- `len + str_len` は素の加算でよい。64-bit では `len <= 2^56 - 1`、
  `str_len <= isize::MAX` なので和は wrap しない。32-bit でも生きたバッファの
  `len <= 2^31 - 16` と `str_len <= 2^31 - 1` から和は `2^32 - 17 < usize::MAX`。
  この根拠はコメントに書く。`checked_add` による `ReserveError` 化は従来どおり
  slow path の `reserve` が担う。
- `is_unique()` の `Acquire` は維持する。fast path で unique を観測してから書き込むのは
  既存の `reserve` 内と同じパターンで、順序を緩める変更はこのタスクではやらない。

この案は `compact_str` が採った形とほぼ同じだが、CoW のぶん条件に `is_unique()` が
増える。**現行の asm を見るかぎり、`reserve` の inline 側はすでにこの条件とほぼ同じ
比較列に畳まれている** (§3 の `.LBB1_9` までの部分) ので、案A 単独では
「`checked_add` が素の加算になる」程度の差しか出ない可能性が高い。

### 案B: 判別子を 1 回だけ読む内部形

上の 1 に直接効かせる案。fast path でバッファ種別が確定した後、`as_mut_ptr` と
`set_len` に「もう判定済み」であることを伝える。

```rust
// 判定済みの書き込み先。push_str / insert_str / Appender から使う。
struct Writable {
    ptr: *mut u8,
    is_inline: bool,   // set_len の分岐に必要な情報だけを持つ
}
```

または、`set_len` を種別ごとの `set_len_inline` / `set_len_heap` に分け、
`push_str` が確定した側を直接呼ぶ形でもよい。後者のほうが変更が小さい。

### 案C: fast path を薄い `#[inline]` 関数に切り出す

3 に直接効かせる案。`push_str` を

```rust
#[inline]
pub(crate) fn push_str(&mut self, string: &str) -> Result<(), ReserveError> {
    // ここは数命令に収まるので呼び出し側へ展開される
    if <fast path 条件> { unsafe { self.push_str_unchecked(string) }; return Ok(()); }
    self.push_str_slow(string)   // #[cold] #[inline(never)]
}
```

の形にして、slow 側 (grow を伴う経路) を outline する。

このとき cold 側の戻り値は `Result<(), ReserveError>` に留める。
**`Repr` は 16 バイトだが scalar pair ではないので、値で返すと sret になる**。
実測で確認した分類:

| 型の形 | 戻り方 |
| --- | --- |
| `(NonNull<u8>, usize)` (= `HeapBuffer`) | `rax:rdx` |
| `(*const (), [u8; 7], LastByte)` (= `Repr`) | sret |

`Repr` がレジスタ返しにならないのは、最終バイトを独立したフィールドに切り出して
`Option<Repr>` の niche を作っているため。この構造は `Option<LeanString>` が
16 バイトに収まることの根拠でもあるので、戻り値の都合で変えるものではない。

なお slow path は `&mut self` を取るが、`push_str` 自体が呼び出し側から
`&mut self` を受け取っている以上そのアドレスはすでに外へ出ているので、
ここで参照を渡すことによる追加のコストは無い
([plans/05](./05-drop-cold-abi.md) が問題にしているのは、値で受け取った
オブジェクトのアドレスを新たに escape させてしまう場合)。

### 推奨

**案C を軸に、案B を必要なぶんだけ**。案A の条件式は案C の fast path 条件としてそのまま
使えるので、実質 3 案は排他ではない。まず案C で「呼び出し側に展開される小さな
fast path」を作り、その asm を見てから案B の判別子削減が必要かを決める。

順序としては:

1. 現状の `push_str` / `push` / `Extend<&str>` の asm と criterion を基準として控える。
2. 案C を入れ、fast path が呼び出し側に展開されるかを確認する。
3. 展開された fast path に判別子の読み直しが残っていたら案B を足す。

## 検証方針

- **asm**: `LeanString::push_str` が呼び出し側に展開され、fast path が
  「判別子 1 回 + 参照カウントのロード 1 回 + 容量比較 + コピー + 長さ更新」に
  収まること。`push_str_slow` が cold セクションへ配置されていること。
- **criterion**: `apis.rs` の `push_str` (連続 push)、`push_str/after_clone`
  (CoW detach 込み)、`comparison.rs` の `Grow`。
  現状の値は `push_str` が 190 ns 前後 (`push_str("abcdefgh")` × 16、std は 147〜165 ns)、
  `push_str/after_clone` が 39 ns 前後 (std は 49 ns)。
- **loom**: `concurrent_push` シナリオ。unique 判定の順序は変えない想定だが、
  fast path が新しい判定順序を持つので必ず回す。
- **Miri**: 全ターゲット。

改善が誤差レベルなら採用を見送り、その結果を記録として残す。

## 依存

- [plans/02](./02-medium-copy.md) と `push_str` の同じ行を触る。
- [plans/04](./04-append-writer.md) はこの `push_str` を grow 経路として使うので、
  こちらを先に。
