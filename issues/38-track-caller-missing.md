# panic する公開メソッドに `#[track_caller]` がなく、panic 位置がクレート内部を指す

- 種別: bug, doc
- 対象: `src/lib.rs` の `LeanString` / `LeanStr` の panic しうる公開メソッド全般 (`reserve`, `push_str`, `insert`, `insert_str`, `remove`, `truncate`, `split_off`, `replace_range`, `extend_from_within`, `repeat`, `to_lowercase` など)、`src/repr.rs` の `Repr::{remove, insert_str, insert_char, truncate, replace_range, extend_from_within}` と `slice_range`、`src/traits.rs` の `ToLeanString::to_lean_string` / `ToLeanStr::to_lean_str`、`FromIterator` / `Extend` / `Add` / `AddAssign` / `fmt::Write` の各 impl
- 影響: 範囲外の index や char 境界違反で panic したとき、`panicked at ...` の位置が利用者のコードではなく `lean_string/src/lib.rs` や `src/repr.rs` になる。`std::string::String` の同名メソッドは呼び出し元を報告するので、`String` から置き換えたときにデバッグ性が落ちる。
- 優先度: 中

## 問題

`#[track_caller]` が付いているのは `From` 系の impl (`char`, `&str`, `&mut str`, `String`, `&String`, `Cow`, `Box<str>`, `LeanString` <-> `LeanStr`) と内部の `unwrap_with_msg` だけである。`impl LeanString` / `impl LeanStr` の公開メソッドには 1 つも付いていない (`src/lib.rs` の `pub fn` を走査して確認した)。`unwrap_with_msg` 自体は `#[track_caller]` だが、報告されるのは「`#[track_caller]` でない最初のフレーム」なので、結局 `lib.rs` 内の `self.try_xxx(..).unwrap_with_msg()` の行になる。

測定条件: `5dd94ea`, rustc 1.97.0 (stable), x86_64 Linux, release。各操作を `catch_unwind` + `panic::set_hook` で呼び、`info.location()` を表示した (下流クレートの probe で確認した)。

```
reserve(usize::MAX)          -> src/lib.rs:586   Cannot allocate memory to hold LeanString
reserve_exact(usize::MAX/2)  -> src/lib.rs:639
with_capacity(usize::MAX)    -> src/lib.rs:131
repeat(usize::MAX)           -> src/lib.rs:1171
insert(10, 'a') ("abc")      -> src/repr.rs:917  index is not a char boundary or out of bounds (index: 10)
truncate(1) ("é")           -> src/repr.rs:1031
remove(5) ("é")              -> src/repr.rs:737
split_off(5) ("é")           -> src/lib.rs:1489  start byte index 5 is out of bounds for string of length 2
replace_range(2..1, "")      -> src/repr.rs:1310 range start is greater than end (start: 2, end: 1)
extend_from_within(2..9)     -> src/repr.rs:1306
Bad.to_lean_string()         -> src/traits.rs:19 (Display が Err を返す型)
```

比較用に同じ操作を `std::string::String` で実行した結果は次のとおり。

```
String::insert / insert_str / truncate / remove / split_off  -> 呼び出し元 (src/bin/stdloc.rs)
String::replace_range / extend_from_within / drain / reserve(MAX) -> 標準ライブラリ内部
```

std も全部が呼び出し元を指すわけではないが、`insert` / `insert_str` / `truncate` / `remove` / `split_off` は呼び出し元を指す。このクレートの 0.8.0 の CHANGELOG にも「`From<Cow<'_, str>>` の panic が呼び出し元の位置を報告するようになった」とあり、`From` 系だけ対応済みで、メソッド側が取り残されている。

副次的に、同種の違反なのに panic メッセージが揃っていない。`split_off` だけは `&self.as_str()[at..]` のスライス由来の `start byte index 5 is out of bounds for string of length 2` で、他は `index is not a char boundary or out of bounds (index: 5)`。

## 原因

公開メソッドが `#[track_caller]` なしで `try_xxx(..).unwrap_with_msg()` や `Repr` の `assert!` を呼んでいるため。`#[track_caller]` は呼び出しチェーン全体 (公開メソッド -> `Repr` メソッド -> `assert!` / `slice_range`) に付けないと伝播しない。

## 修正例

`#[track_caller]` を次に付ける (検証済み。以下のスクリプトで機械的に付与した)。

- `src/lib.rs`: docs に `# Panics` があるか、本体が `unwrap_with_msg` / `self.0.remove/insert*/truncate/replace_range/extend_from_within` / `[at..]` を呼ぶ `pub fn` 全部 (39 個)。`try_` 版も index の assert で panic するので付ける。
- `src/repr.rs`: `remove`, `insert_str`, `insert_char`, `truncate`, `replace_range`, `extend_from_within`, `slice_range`。
- 未検証 (同様に付くはず): `ToLeanString::to_lean_string`, `ToLeanStr::to_lean_str` (trait の provided method), `Add::add`, `AddAssign::add_assign`, `FromIterator::from_iter`, `Extend::extend`, `fmt::Write::write_str` / `write_fmt`。

結果: 上の表の位置がすべて probe 側の呼び出し行 (`src/main.rs:NN`) になった (`to_lean_string` は trait メソッドに付けていないので `traits.rs` のまま)。

性能への影響 (asm、x86-64、rustc 1.97.0、release、LTO なし、`#[inline]` 経由で下流クレートに見える形): `push_str` (70 命令) / `push` (123) / `reserve` (33) / `insert` (150) / `insert_str` (97) / `remove` (137) / `truncate` (49) / `split_off` (194) / `extend` of `&str` (85) の wrapper の命令数は付与前後で同じで、`replace_range` だけ 204 -> 198 に減った。`#[inline]` のおかげで `Location` は panic 側の cold path にだけ materialize される。ホットパスの `push` 系は、念のため修正時に asm を再確認する。

panic メッセージの統一は任意。`split_off` を `Repr` の char 境界 assert と同じ文面にするなら、`try_split_off` の先頭に `assert!(self.is_char_boundary(at), ...)` を置けばよい。

## 確認方法

プロセス全体の panic hook を触るため、専用の integration test ファイルに `#[test]` を 1 つだけ置く。

```rust
// tests/panic_location.rs
use lean_string::LeanString;
use std::{panic, sync::{Arc, Mutex}};

fn panic_file(f: impl FnOnce() + panic::UnwindSafe) -> String {
    let seen = Arc::new(Mutex::new(String::new()));
    let seen2 = seen.clone();
    let prev = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        *seen2.lock().unwrap() = info.location().unwrap().file().to_owned();
    }));
    let _ = panic::catch_unwind(f);
    panic::set_hook(prev);
    let file = seen.lock().unwrap().clone();
    file
}

#[test]
fn panic_location_is_the_caller() {
    let here = file!();
    assert_eq!(panic_file(|| LeanString::from("é").truncate(1)), here);
    assert_eq!(panic_file(|| { LeanString::from("é").remove(5); }), here);
    assert_eq!(panic_file(|| LeanString::from("abc").insert(10, 'a')), here);
    assert_eq!(panic_file(|| LeanString::new().reserve(usize::MAX)), here);
    assert_eq!(panic_file(|| { LeanString::from("abc").repeat(usize::MAX); }), here);
    // ... 他の panic しうるメソッドも同様に
}
```
