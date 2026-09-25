# 02: `push_str` の fast path を整理する

種別: perf / 実装量: 小〜中 / 設計判断: 中 (内部 API の形)

## 背景

`Repr::push_str` は毎回 `reserve` を呼ぶ。`reserve` の遅い部分は `outline!` で切り出してあるので
毎回関数呼び出しが起きるわけではないが、
[research/codegen-baseline.md](../research/codegen-baseline.md) の §3 で見たとおり、次の 4 点が残っている。

1. 判別子 (バイト 15) を 3 回読んでいる。fast path の判定で読み、`as_mut_ptr` でまた読み、
   `set_len` でもう一度読んで 3 方向に分岐する。最初の判定でバッファの種類は分かっているのに、
   それを後ろに伝える手段がない。
2. `set_len` に通らない分岐が残っている。`cmpl $209, %eax` は StaticMarker との比較だが、
   `reserve` が `Ok` を返した後の `self` が static であることはない。
3. 追記のコピーが可変長 memcpy の呼び出しになっている ([01](./01-medium-copy.md) で扱う)。
4. 関数が大きく、呼び出し側にインライン展開されない。`Repr::push_str` は 108 命令あり、
   `#[inline]` が付いていても下流クレートからは `callq Repr::push_str` になる。

`Extend<&str>` や `Extend<String>` などは要素ごとにこの関数を呼ぶので、4 のコストは要素数に比例する。

`bench/benches/comparison.rs` の `Grow` (`push_str` を 16 回) では 4 クレート中いちばん遅い。

| | LeanString | CompactString | EcoString | String |
| --- | --- | --- | --- | --- |
| Grow | 306.19 ns | 262.95 ns | 241.26 ns | **199.12 ns** |

## 方針

### 案A: 容量チェックだけを inline に残し、grow を cold に回す

`reserve` を呼ぶ前に、fast path に入れるかをその場で判定する。

```rust
#[inline]
pub(crate) fn push_str(&mut self, string: &str) -> Result<(), ReserveError> {
    if string.is_empty() {
        return Ok(());
    }
    let len = self.len();
    let str_len = string.len();

    // fast path に入れないときだけ reserve を呼ぶ。
    // - StaticBuffer は必ず変換が必要
    // - 共有されている HeapBuffer は、容量に関係なく複製が必要 (CoW)
    if self.is_static_buffer() || !self.is_unique() || len + str_len > self.capacity() {
        self.reserve(str_len)?;
    }

    unsafe { /* 既存のコピーと set_len */ }
    Ok(())
}
```

- `is_unique()` は InlineBuffer でも `true` を返すので、条件はこのまま書ける。
  inline のときの `capacity()` は定数 16 になる。
- `len + str_len` はオーバーフローしないので普通の加算でよい。64-bit では `len <= 2^56 - 1`、
  `str_len <= isize::MAX`。32-bit でも生きているバッファは `len <= 2^31 - 16`、
  `str_len <= 2^31 - 1` なので、和は `2^32 - 17` 以下に収まる。この根拠はコメントに書く。
  オーバーフローを `ReserveError` にする処理は、これまでどおり slow path 側の `reserve` に任せる。
- `is_unique()` の `Acquire` はそのままにする。unique を確認してから書き込むのは今の `reserve` と
  同じで、メモリ順序を緩めるのはこのタスクの範囲外。

ただ、今の asm でも `reserve` の inline 部分はほぼこの条件と同じ比較列になっている (§3 の
`.LBB9_12` の手前まで)。案A だけだと、`checked_add` が普通の加算になる程度の差しか出ないかもしれない。

### 案B: 判別子を 1 回だけ読む

1 と 2 に対処する案。fast path でバッファの種類が決まったら、それを `as_mut_ptr` と `set_len` に渡す。

```rust
// 判定済みの書き込み先。push_str / insert_str / Appender から使う。
struct Writable {
    ptr: *mut u8,
    is_inline: bool,   // set_len の分岐に必要な情報だけ持つ
}
```

`set_len` を `set_len_inline` / `set_len_heap` に分けて、`push_str` から該当する方を直接呼ぶ方法もある。
変更はこちらのほうが小さい。

いちばん手軽なのは、`reserve` が `Ok` を返した直後に
`hint::assert_unchecked(!self.is_static_buffer())` を置くことで、これだけでも 2 は消える。

なお、コピーの後にバイト 15 を読み直すのは避けられない。inline で `len == 16` になるコピーは
バイト 15 を書き換えるので、LLVM は読み直すしかない。

### 案C: fast path を小さな `#[inline]` 関数に分ける

4 に対処する案。

```rust
#[inline]
pub(crate) fn push_str(&mut self, string: &str) -> Result<(), ReserveError> {
    // ここは数命令なので呼び出し側に展開される
    if <fast path の条件> { unsafe { self.push_str_unchecked(string) }; return Ok(()); }
    self.push_str_slow(string)   // #[cold] #[inline(never)]
}
```

cold 側の戻り値は `Result<(), ReserveError>` のままにする。
`Repr` は 16 バイトだが scalar pair ではないので、値で返すと sret になる。

| 型 | 戻り方 |
| --- | --- |
| `(NonNull<u8>, usize)` (= `HeapBuffer`) | `rax:rdx` |
| `(*const (), [u8; 7], LastByte)` (= `Repr`) | sret |

`Repr` がレジスタで返らないのは、最終バイトを別のフィールドにして `Option<Repr>` の niche を
作っているから。この構造は `Option<LeanString>` が 16 バイトに収まる根拠でもある
(`src/repr.rs` の `const _` で固定している) ので、戻り値のために変えるものではない。

slow path が `&mut self` を取っても余計なコストは無い。`push_str` は呼び出し側から
`&mut self` を受け取っているので、アドレスはもともと外に出ている
([04](./04-drop-cold-abi.md) で問題にしているのは、値で受け取ったもののアドレスを新たに外へ出してしまうケース)。

### 進め方

案C を中心に、必要なら案B を足す。案A の条件式はそのまま案C の fast path の条件に使えるので、
3 案は排他ではない。

1. 今の `push_str` / `push` / `Extend<&str>` の asm と criterion の結果を控えておく。
2. 案C を入れて、fast path が呼び出し側に展開されるか確認する。
3. 展開された fast path に判別子の読み直しが残っていれば案B を足す。

`reserve` の中でも `self.len()` を読み直している。`push_str` 側の `let len = self.len();` を
`reserve` の後に移すだけで重複が消える ([11](./11-deferred.md) の「`reserve` が `len()` を読み直す」)。
1 行の変更なので、ここで一緒に試す。

## 検証

- asm: `LeanString::push_str` が呼び出し側に展開され、fast path が
  「判別子 1 回 + 参照カウントのロード 1 回 + 容量比較 + コピー + 長さの更新」になっていること。
  `push_str_slow` が `.text.unlikely` に置かれていること。
- criterion: `apis.rs` の `push_str` (292.84 ns、std は 186.12 ns) と
  `push_str/after_clone` (44.157 ns、std は 56.578 ns)、`comparison.rs` の `Grow`。
- loom: `concurrent_push`。unique 判定の順序は変えないつもりだが、判定の並びは変わるので必ず回す。
- Miri: 4 ターゲット。

改善が誤差程度なら入れず、その結果を記録しておく。

## 同じ形の関数

`Repr::insert_str` も判別子を 3 回読み、`memmove` と `memcpy` を呼んでいる (`probe_insert_str` は 133 命令)。
案B を採るなら一緒に直せる。

## 依存

- [01](./01-medium-copy.md) と `push_str` の同じ箇所を触る。
- [03](./03-append-writer.md) はこの `push_str` を grow 経路に使うので、こちらを先に。
