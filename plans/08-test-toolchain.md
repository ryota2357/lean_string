# 08: テストが MSRV でも現行の安定版でもコンパイルできない

種別: バグ修正 / 実装量: 小 / 設計判断: 小

## 現象

`cargo test` がライブラリの MSRV より 10 リリース新しい rustc を要求する。

```
$ cargo test --all-features        # rustc 1.94.1
error[E0658]: use of unstable library feature `str_from_utf16_endian`
  --> tests/property.rs:81:18
error[E0658]: use of unstable library feature `str_from_utf16_endian`
  --> tests/property.rs:93:18
error[E0658]: use of unstable library feature `str_from_utf16_endian`
  --> tests/property.rs:105:18
error[E0658]: use of unstable library feature `str_from_utf16_endian`
  --> tests/property.rs:117:18
error: could not compile `lean_string` (test "property") due to 4 previous errors
```

`create_from_utf16le_bytes` と `create_from_utf16be_bytes` が
`String::from_utf16le` / `from_utf16le_lossy` / `from_utf16be` / `from_utf16be_lossy` を
オラクルとして使っている。これらは `str_from_utf16_endian`
([rust-lang/rust#116258](https://github.com/rust-lang/rust/issues/116258)) の一部で、
安定化されたのは 1.98.0。

| rustc | `String::from_utf16le` |
| --- | --- |
| 1.88.0 (`Cargo.toml` の `rust-version`) | unstable |
| 1.94.1 | unstable |
| 1.96.0 | unstable |
| 1.97.0 | unstable |
| 1.98.0 | stable |

ライブラリ自体は 1.88.0 でビルドできる (`cargo +1.88.0 check --all-features` が通る) ので、
利用者への影響は無い。影響を受けるのは、1.98.0 より前の rustc でこのリポジトリの
テストを走らせようとする人だけ。

CI が緑なのは、`rust-toolchain.toml` が `channel = "stable"` で、
GitHub Actions の runner が 1.98.0 以降の安定版を引くため。
つまり**この状態は CI では検出できない**。

MSRV ジョブ (`cargo hack check --rust-version --all-features`) も
ライブラリしか見ないので、同じくすり抜ける。

## 方針

### 対処1 (必須): オラクルを自前で書く

`String::from_utf16le` の 4 つは std での実装が短いので、テスト内に同等の関数を置く。
たとえば LE の場合:

```rust
fn utf16le_oracle(buf: &[u8]) -> Result<String, ()> {
    let (chunks, []) = buf.as_chunks::<2>() else { return Err(()) };
    let units = chunks.iter().copied().map(u16::from_le_bytes);
    char::decode_utf16(units).collect::<Result<String, _>>().map_err(|_| ())
}
```

`String::from_utf16le` と挙動が一致していることは、この関数を書いた時点の
新しい安定版で 1 度確認すればよい。オラクルを自前で書くと「実装と同じ間違いをする」
危険があるので、std の実装を写すのではなく `char::decode_utf16` から素直に組み立てる。

代替として `#[cfg(...)]` でこの 2 つの property テストだけ落とす手もあるが、
UTF-16LE/BE のデコードはこのクレートで新しく書いた部分なので、テストを落とすのは避けたい。

### 対処2: テストを MSRV の対象に含めるかを決める

現状の CI は次の 2 段構えになっている。

| ジョブ | コマンド | 対象 |
| --- | --- | --- |
| MSRV | `cargo hack check --rust-version --all-features` | ライブラリのみ |
| Test | `cargo test` ほか | 現行の安定版のみ |

「テストは MSRV でビルドできなくてよい」という方針ならそれを `CONTRIBUTING` なり
CI のコメントなりに明記する。「できるべき」なら MSRV ジョブに
`--all-targets` を足す (今回の 4 か所以外にも引っかかる可能性があるので、
足すなら一度手元で確認してからにする)。

前者を推す。テストのオラクルに新しい std API を使えるのは利点であり、
それを MSRV に縛ると書けるテストが減る。ただし**現状のように黙って壊れるのは避けたい**ので、
どちらを採るにせよ「テストのビルドに必要な rustc」を 1 か所に書く。

## 検証方針

- `cargo +1.88.0 test --all-features` が通ること (対処2 で「MSRV に含める」を選んだ場合)。
- そうでない場合、少なくとも `cargo +<決めた下限> test --all-features` が通ること。
- `cargo clippy --all-targets --all-features` も同じ理由で失敗するので、あわせて確認する。

## 依存

なし。他のどのタスクよりも先に片付けてよい。
