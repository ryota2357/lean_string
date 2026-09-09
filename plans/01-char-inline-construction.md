# 01: `char` からの構築をレジスタ内で行う

種別: perf / 実装量: 小〜中 / 設計判断: 小

## 背景

`&str` からの inline 構築はレジスタ内で完結している
([research/codegen-baseline.md](../research/codegen-baseline.md) の §1) が、
`char` からの構築は ASCII 以外でスタック往復が残る。

原因は `Repr::from_char` (src/repr.rs:91-98) の形。

```rust
pub(crate) fn from_char(ch: char) -> Self {
    let inline = unsafe {
        let mut buffer = [0; 4];
        let str = ch.encode_utf8(&mut buffer);
        InlineBuffer::new(str)
    };
    Repr::from_inline(inline)
}
```

`encode_utf8` がスタック上の 4 バイトバッファへバイト単位で書き、`InlineBuffer::new` が
それを読み直す。ASCII の arm は LLVM が定数畳み込みして 2 ストアで終わるが、
2〜4 バイトの arm では読み直しが残る
([codegen-baseline.md](../research/codegen-baseline.md) の §2)。

4 バイト文字の arm がとくに悪い。1 バイトずつのストア 4 本に対して
`movl -4(%rsp), %esi` の 4 バイトロードが跨がるので、x86 の store-to-load forwarding が
成立しない。x86 のストアバッファは**ロードが単一のストアに完全に含まれている場合にしか
転送できない**ので、失敗するとストアがキャッシュに書かれるまで待つ (12 サイクル程度)。
Apple の aarch64 は複数ストアに跨るロードでも転送できるため、ARM だけで測ると見えない。

## 方針

UTF-8 のエンコード結果を u32 としてレジスタ内で組み立て、第 1 ワードへ直接置く。
`char` は最大 4 バイトなので、第 2 ワードは常に長さタグだけになる。

```rust
#[inline]
#[cfg(all(target_pointer_width = "64", target_endian = "little"))]
pub(crate) fn from_char(ch: char) -> Self {
    let code = ch as u32;
    // encode_utf8 と同じビット演算を、メモリを経由せずに行う。
    // little-endian では u32 の下位バイトが文字列の先頭バイトになるので、
    // 各バイトを 8 ビットずつずらして OR するだけでよい。
    let (w0, len) = if code < 0x80 {
        (code as u64, 1)
    } else if code < 0x800 {
        let b0 = 0xC0 | (code >> 6);
        let b1 = 0x80 | (code & 0x3F);
        ((b0 | (b1 << 8)) as u64, 2)
    } else if code < 0x10000 {
        ...
    } else {
        ...
    };
    let last_byte = ((len as u64) | LastByte::MASK_1100_0000 as u64) << 56;
    // SAFETY: `w0` は有効な UTF-8 の 1..=4 バイトで、残りは 0。
    unsafe { mem::transmute([w0, last_byte]) }
}
```

`InlineBuffer::new` (src/repr/inline_buffer.rs:21-70) が既に採っている「2 ワードを
組み立てて `transmute` する」形と同じで、ゲートも同じ
`cfg(all(target_pointer_width = "64", target_endian = "little"))` にする。
それ以外のターゲットは現行実装をそのまま残す。

置き場所の選択肢が 2 つある。

- 案A: `InlineBuffer::from_char(ch) -> Self` を足し、`Repr::from_char` はそれを呼ぶ。
  2 ワードの組み立てが `inline_buffer.rs` に集まるので、`transmute` の根拠を
  1 か所で説明できる。
- 案B: `Repr::from_char` の中で完結させる。`InlineBuffer` を経由しないぶん短いが、
  `Repr` のバイト表現に依存するコードが `repr.rs` にも増える。

案A を推す。

## 検証方針

- **正しさ**: `char::MAX` を含む全長 (1/2/3/4 バイト) の境界と、
  サロゲート直前後 (`\u{D7FF}`, `\u{E000}`)、`\u{FFFF}` / `\u{10000}` の境界で
  `ch.encode_utf8()` の結果と一致すること。`proptest` で全 `char` を回すのが手軽。
- **asm**: `LeanString::from(char)` の 2〜4 バイト arm から `-N(%rsp)` への
  ストア/ロードが消え、各 arm が 2 ストアで終わること。x86-64 と aarch64 の両方。
- **Miri**: 4 ターゲット (64/32-bit × LE/BE)。BE と 32-bit はフォールバック経路の確認を兼ねる。
- **criterion**: `bench/benches/apis.rs` の `from` には `char` の点が無いので足す。
  ASCII / 2 バイト / 3 バイト / 4 バイトの 4 点。
  `LeanString::push(char)` は `push_str` 経由なのでこのタスクの影響を受けない。

## 波及先

`Repr::from_char` の呼び出し元は `From<char>` と `ToLeanString` / `ToLeanStr` の
`&char` arm (src/traits.rs:71, 152)。`push(char)` と `Extend<char>` は
`Repr::push_str` を通るので [plans/04](./04-append-writer.md) の対象。

## 依存

なし。単独で実施できる。
