# テストのビルドに Rust 1.98 以降が必要で、MSRV (1.88) との差がどこにも書かれていない

- 種別: test, infra
- 対象: `tests/property.rs` の `create_from_utf16le_bytes` / `create_from_utf16be_bytes`、`.github/workflows/ci.yaml` の `msrv` ジョブ
- 影響: 1.98 より古い stable で `cargo test` を実行すると、`tests/property.rs` がコンパイルできず、他のテストも含めてテスト全体が実行されない。MSRV (`rust-version = "1.88.0"`) で動作を確かめたい利用者や貢献者がつまずく
- 優先度: 低

## 問題

`tests/property.rs` は `from_utf16le` / `from_utf16be` とその lossy 版の期待値に、`String::from_utf16le` / `from_utf16be` / `from_utf16le_lossy` / `from_utf16be_lossy` を使っている。これらは Rust 1.98.0 で安定化された (`str_from_utf16_endian`)。

`5dd94ea` で、rustc 1.97.0 (stable) の `cargo test` は次のエラーで止まる。

```
error[E0658]: use of unstable library feature `str_from_utf16_endian`
error: could not compile `lean_string` (test "property") due to 4 previous errors
```

クレート本体の MSRV は 1.88.0 で、CI の `msrv` ジョブは `cargo hack check --rust-version --all-features` で確かめている。`check` はテストをビルドしないので、テストが MSRV でビルドできないことは CI では検出されない。CI の `test` ジョブは最新の stable で回るので通っている。テストに必要なツールチェーンは README にも CI にも書かれていない。

## 原因

期待値の計算に、MSRV より新しい std の API を使っている (コードとコンパイルエラーで確認)。

## 修正例

期待値を `char::decode_utf16` (1.9 で安定化) で計算するヘルパに置き換える。std の 4 関数と同じく、奇数バイトなら検証つきの版はエラー、lossy 版は末尾に U+FFFD を 1 つ足す。

```rust
// `String::from_utf16le` and friends are stable only since Rust 1.98. Decode with `char::decode_utf16`
// instead, so that the tests build with older toolchains too.
fn utf16_units(bytes: &[u8], from_bytes: fn([u8; 2]) -> u16) -> impl Iterator<Item = u16> + '_ {
    bytes.chunks_exact(2).map(move |c| from_bytes([c[0], c[1]]))
}

fn expected_from_utf16(bytes: &[u8], from_bytes: fn([u8; 2]) -> u16) -> Option<String> {
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    char::decode_utf16(utf16_units(bytes, from_bytes)).collect::<Result<_, _>>().ok()
}

fn expected_from_utf16_lossy(bytes: &[u8], from_bytes: fn([u8; 2]) -> u16) -> String {
    let mut string: String = char::decode_utf16(utf16_units(bytes, from_bytes))
        .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect();
    if !bytes.len().is_multiple_of(2) {
        string.push(char::REPLACEMENT_CHARACTER);
    }
    string
}
```

呼び出し側は次のように置き換える (`be` も同様に `u16::from_be_bytes` を渡す)。

```rust
-    let string = String::from_utf16le(input);
+    let string = expected_from_utf16(input, u16::from_le_bytes).ok_or(());
...
-    let string = String::from_utf16le_lossy(input);
+    let string = expected_from_utf16_lossy(input, u16::from_le_bytes);
```

試した結果: rustc 1.97.0 で `PROPTEST_CASES=20000 cargo test --test property utf16` の 3 件が通り、`cargo clippy --tests --all-features` と `cargo fmt --check` も通る。rustc 1.88.0 でも `cargo test --all-features --no-run` でテスト全体がビルドできた (修正前は同じエラーで止まる)。

別案として、テストに必要なツールチェーン (1.98 以降) を README や CI に明記するだけにする方法もある。ただし上のヘルパは小さく、MSRV でテストを回せるようになるので、置き換えを勧める。

## 確認方法

- 古いツールチェーン (例えば `cargo +1.88 test --all-features --no-run`) でテストがビルドできること。
- 必要なら CI の `msrv` ジョブにテストのビルドを足す (例: `cargo hack test --rust-version --all-features --no-run`)。これで MSRV より新しい API がテストに入ったときに検出できる。dev-dependencies (`proptest` など) の MSRV が上がると落ちるようになる点には注意する。
