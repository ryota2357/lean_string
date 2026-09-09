# 06: `retain` が何も削らないときに共有を解かないようにする

種別: perf (準バグ) / 実装量: 小 / 設計判断: 小

## 現象

`Repr::retain` (src/repr.rs:664-670) は本体に入る前に無条件で `ensure_modifiable` を呼ぶ。

```rust
pub(crate) fn retain(&mut self, mut predicate: impl FnMut(char) -> bool) -> Result<(), ReserveError> {
    // We will modify the buffer, we need to make sure it.
    self.ensure_modifiable()?;
    ...
```

そのため、述語が 1 文字も落とさない場合でも共有バッファのコピーが発生する。
共有された heap バッファに対する実測:

```
retain(|_| true) kept sharing: false
truncate kept sharing:         true
```

`s.retain(|c| c.is_ascii())` を「すでに全部 ASCII の文字列」に対して呼ぶと、
何も変わらないのにバッファが 1 本増える。CoW 型としては望ましくない。

`truncate` は同じ状況で共有を維持できている (`truncate_unchecked` が
「長さだけを縮めるなら共有のままでよい」を判定している、repr.rs:781-792) ので、
挙動としても非対称になっている。

`remove` は必ず 1 文字消すので該当しない。

## 方針

「最初に述語が false を返す位置まで走査し、そこで初めて `ensure_modifiable` する」形にする。

```rust
pub(crate) fn retain(&mut self, mut predicate: impl FnMut(char) -> bool) -> Result<(), ReserveError> {
    // 1 文字も落ちなければ書き込みは不要なので、共有バッファのまま走査する。
    let mut idx = 0;
    let len = self.len();
    while idx < len {
        let ch = /* self.as_str()[idx..] の先頭文字 */;
        if !predicate(ch) {
            break;
        }
        idx += ch.len_utf8();
    }
    if idx == len {
        return Ok(());   // 何も落ちなかった。確保なし・共有維持
    }

    // ここから先は既存の実装。ただし走査済みの `0..idx` は複写不要なので、
    // src_idx = dst_idx = idx から再開する。
    self.ensure_modifiable()?;
    ...
}
```

注意点。

- **述語の呼び出し回数を変えない**。素朴に書くと、`ensure_modifiable` の後に
  「落とすと決めた文字」をもう一度述語に渡してしまう。走査で見つけた位置と文字を
  引き継ぎ、その文字については述語を呼ばずに落とす。`String::retain` も
  各文字につき述語を 1 回しか呼ばないので、ここは合わせる必要がある。
- **述語が panic したときの状態**。走査フェーズではバッファを触っていないので、
  panic しても文字列は無傷。既存の `SetLenOnDrop` (repr.rs:671-719) は
  書き込みフェーズに入ってから初めて必要になる。
- `ensure_modifiable` がバッファを差し替えるので、走査で使ったポインタは
  必ず取り直す。走査の結果として持ち越すのはバイト位置 `idx` と文字 `ch` だけにする。

## テスト

- 共有バッファに対する `retain(|_| true)`: 確保が起きないこと、
  両方のハンドルの `as_ptr()` が変わらないこと、内容が不変であること。
  `tests/out_of_memory.rs` の `without_allocating` ヘルパがそのまま使える。
- static バッファに対する `retain(|_| true)`: heap 化しないこと。
- 述語の呼び出し回数が文字数と一致すること (呼び出し回数を数えるクロージャで確認)。
  現状もこの性質は満たしている (35 文字の文字列で 35 回) ので、退行テストになる。
- 最初の文字だけ落とす / 最後の文字だけ落とす / 途中で落とす、の 3 ケースで
  `String::retain` と結果が一致すること。
- 述語が途中で panic したとき、走査フェーズなら不変、書き込みフェーズなら
  既存の `SetLenOnDrop` の保証どおりであること。

## 補足: 同種の見直し先

`ensure_modifiable` を無条件に呼ぶ、または `reserve` 経由で必ず unique 化する経路が
他にもある。今回あわせて確認しておくとよい。

- `Repr::remove` (repr.rs:624-661): 必ず 1 文字消すので現状のままで正しい。
- `Repr::insert_str` (repr.rs:725-759): 空文字列の早期 return が既にあり (repr.rs:731)、
  `reserve(0)` が no-op なので問題ない。
- `LeanString::clear` / `truncate` / `pop`: 共有を維持できている。

## 依存

なし。単独で実施できる。
