# `retain` が最初の削除以降、残す文字ごとに `memmove` を呼ぶ

- 種別: perf
- 対象: `src/repr.rs` の `Repr::<Mutable>::retain`
- 影響: 1 文字でも削る `retain` すべて。削った位置より後ろの残す文字 1 つにつき `memmove` の呼び出しが 1 回入り、`String::retain` の約 2 倍の命令数になる。
- 優先度: 高

## 問題

`retain` は最初に削る文字を見つけたあと、残す文字を 1 文字ずつ前に詰める。その詰め方が
`ptr::copy(ptr.add(g.src_idx), ptr.add(g.dst_idx), ch_len)` で、`ch_len` (1..=4) が実行時の値なので
LLVM は `memmove` の呼び出しにする。

`5dd94ea`、rustc 1.97.0、LTO off の probe クレート (`s.retain(|c| c != ' ')`) の x86-64 asm、ループ部分の抜粋:

```asm
.LBB19_30:
	cmpq	%r13, %r15            # dst_idx != src_idx ?
	je	.LBB19_32
	leaq	(%r15,%rbp), %rdi
	movq	%r14, %rdx            # ch_len
	callq	*memmove@GOTPCREL(%rip)
	jmp	.LBB19_32
```

aarch64 でも同じく `bl memmove` がループ内にある。一方 `String::retain` の同じ probe は、
`ch.encode_utf8` で書き戻すため `memmove` を呼ばず、`movb` を 1〜4 個並べるだけになっている。

命令数 (valgrind cachegrind の Ir、1 回あたり。`"lorem ipsum dolor sit amet ".repeat(40)` (1080 バイト)
の clone に `retain(|c| c != ' ')`。clone からの共有バッファのコピー 1 回を含む):

| | Ir/回 | 確保 |
|---|---:|---:|
| `LeanString` (現状) | 34,617 | 1 |
| `String` | 18,105 | 1 |
| `LeanString` (下の修正例) | 16,912 | 1 |

## 原因

`ptr::copy` に定数でない長さを渡しているため。`remove` の `memmove` は 1 回きりなので問題にならないが、
`retain` ではループの中にある。文字はすでに `ch` としてデコードしてあるので、元のバイトを
コピーし直す必要はない (検証済み: asm と Ir)。

## 修正例

同じファイルにある `write_char` (定数長コピーに分岐して `memcpy` を避けるヘルパ) で、デコード済みの `ch` を書き戻す。

```rust
            if predicate(ch) {
                if g.dst_idx != g.src_idx {
                    // SAFETY:
                    // - `g.dst_idx + ch_len <= g.src_idx + ch_len <= len`, so the destination is
                    //   within the initialized bytes.
                    // - `write_char` encodes `ch` into a local buffer first, so overlapping the
                    //   source bytes of `ch` is fine.
                    // - `g.dst_idx` is a character boundary, and the bytes written are the UTF-8
                    //   encoding of `ch`.
                    unsafe { write_char(ptr.add(g.dst_idx), ch) };
                }
                g.dst_idx += ch_len;
            }
```

試した結果: probe の asm からループ内の `memmove` 呼び出しが消え (残る呼び出しは `ensure_modifiable` と
OOM 時の panic だけ)、上の表のとおり 34,617 → 16,912 Ir/回 (`String` より少ない)。
既存のテスト (`tests/lean_string.rs`, `tests/alloc_string.rs`) と、`retain` を unique / 共有 / static の
各バッファで `String` と突き合わせる property テストが通り、述語が途中で panic するテストを含めて
x86_64 の Miri (`-Zmiri-strict-provenance`) でも問題なし。

## 確認方法

- probe (`#[unsafe(no_mangle)] pub fn probe_retain(s: &mut LeanString) { s.retain(|c| c != ' ') }`) の
  x86-64 / aarch64 asm に `memmove` が無いこと。
- `bench/benches/apis.rs` に `retain` のベンチが無いので追加する。例えば空白を含む 1 KiB 程度の文字列を
  `iter_batched` で毎回 clone し (共有) / unique にしてから空白を削るものを、`String` と並べる。
- 述語が panic するテスト (`tests/alloc_string.rs` の `test_retain`) を Miri で回す。
