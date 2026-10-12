# バッファが共有されているかを調べる公開 API (`ptr_eq` / `is_unique`) が無い

- 種別: api
- 対象: `src/lib.rs` の `LeanString` / `LeanStr`、`src/repr.rs` の `Repr::is_unique` (private)
- 影響: CoW の挙動 (次の変更で複製が起きるか、2 つの文字列が同じ buffer か) をテストやデバッグで確認する公式の手段が無い。`Arc` には `ptr_eq` / `strong_count` / `get_mut` がある。
- 優先度: 低

## 問題

`5dd94ea` で `LeanString::ptr_eq` / `is_unique` はコンパイルできない (`no method named`)。現状の回避は `Deref` 経由の `str::as_ptr()` の比較で、`reserve` の doc の例もそうしている (`assert_eq!(s.as_ptr(), shared.as_ptr())`)。この方法には落とし穴がある:

- inline の文字列は値自体がバッファなので、`as_ptr()` は常に別のアドレスになる。clone しても「共有」にならない。
- 長さを無視すると、`truncate` 済みの clone や、同じ buffer の先頭を指す別の長さの文字列が「同じ」に見える。
- static の文字列は同じ `&'static str` を指すと一致するが、それは「共有」ではなく、変更すると複製される (`is_unique` 相当の判定は heap だけ)。

内部には `Repr::is_unique(&self)` (`pub(crate)`) がある: heap なら参照カウントが 1、それ以外は常に `true` を返す。static に対して `true` を返すので、そのまま公開すると「変更しても複製されない」という意味にはならない (static は変更時に複製される)。

## 修正例

```rust
impl LeanString {
    /// Returns `true` if both strings share the same heap buffer or point to the same `&'static str`.
    /// Always `false` for inline strings.
    pub fn ptr_eq(this: &Self, other: &Self) -> bool {
        (this.is_heap_allocated() || this.as_static_str().is_some())
            && this.as_ptr() == other.as_ptr()
            && this.len() == other.len()
    }
}
```

`Arc::ptr_eq` に倣って関連関数 (`LeanString::ptr_eq(&a, &b)`) にすれば、`Deref<Target = str>` のメソッドと衝突しない。試作して次のテストが通った (Miri `-Zmiri-strict-provenance` でも): heap の clone は `true`、同じ内容を別に作ると `false`、inline の clone は `false`、static の clone は `true`。`LeanStr` にも同様のものを足す (`LeanString` との混在比較は表現が違うので不要)。

### 公開しない場合の論点

`ptr_eq` を公開しない選択にも理由がある。

- inline の値では、`ptr_eq` が `true` になるのは「同じ値を 2 回借用した場合」だけである。短い文字列でこれは直感に反する。
- 同じことは `a.as_ptr() == b.as_ptr() && a.len() == b.len()` で書ける。
- `==` にポインタの近道が入れば ([#19](./19-shared-buffer-ptr-eq.md))、公開する理由はさらに薄くなる。

上の修正例は、`Arc::ptr_eq` に倣った関連関数として足す案である。この利点 (CoW の挙動をテスト・デバッグで確かめられる、inline で `false` と明示できる) と、上の論点のどちらを重く見るかは、用途の有無次第で決まる。

### `is_unique`

`is_unique` を公開する場合は意味を決めてからにする。案: 「この文字列が heap の buffer を他と共有していない (inline、または参照カウントが 1 の heap)」で、static は `false` とする。そうすると「変更系メソッドが容量に関係なく複製を要求するか」とほぼ等価になる (`reserve` が説明している条件と一致)。名前は `Arc::get_mut` の判定と同じく `is_unique` が自然。ただし用途がテスト・デバッグにほぼ限られるので、必要性が確認できるまでは `ptr_eq` だけでもよい。

## 確認方法

`tests/lean_string.rs` に、上の 4 ケースのテストを足す。`PartialEq` の fast path (同じポインタと長さなら即 `true`) に使えるかは別問題で、このイシューの範囲外 (ベンチマークが必要。[#19](./19-shared-buffer-ptr-eq.md) で扱う)。
