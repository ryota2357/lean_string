# race テストが「相手の drop で unique になった直後にその場で書き換える」経路をほぼ通らず、`is_unique` の順序を弱めても検出できない。clone のオーバーフロー処理もテストされていない

- 種別: test
- 対象: `tests/race_condition.rs`、`tests/loom.rs`、`src/repr/heap_buffer.rs` の `HeapBuffer::is_unique`、`src/repr.rs` の
  `Repr::is_modifiable` を使う変更系メソッド全般 (`push_str`, `remove`, `retain`, `replace_range`, `extend_from_within`, `insert_str`,
  `make_ascii_*case`, `Repr::reallocate` 経由の `reserve_exact` / `shrink_to` など)、`Repr::make_shallow_clone` (`ref_count_overflow`)
- 影響: `is_unique` の `Acquire` を `Relaxed` に変える (別スレッドの読み取りが終わる前に書き換えうる UB) ような退行を、CI の Miri も
  loom も検出しない。`replace_range` など最近の変更系メソッドは race テスト自体が無い。clone の参照カウントのオーバーフロー時に
  カウントを戻して panic する処理は一度も実行されていない
- 優先度: 高

## 問題

### unique 判定の経路を通らない

`HeapBuffer::is_unique` は `load(Acquire) == 1` で、unique と判断したらその場で書き込み・realloc する。この `Acquire` は、他のスレッドが
「内容を読んでから drop (`fetch_sub(Release)`)」したときに、その読み出しがこちらの書き込みより前に起きたことを保証するためのものである。

`tests/race_condition.rs` の冒頭のコメントは、これらのテストが CoW の uniqueness の判定での use-after-free の競合を狙うと書いている。
各テストは次の形をしている。

```rust
let mut one = LeanString::from("abcdefghijklmnopqrstuvwxyz");
let two = one.clone();
let th = thread::spawn(move || drop(two));
assert_eq!(one.remove(0), 'a');
```

狙っている経路は「`two` の drop を `one` 側が `is_unique()` の `Acquire` ロードで観測し、その場で書き換える」もの。しかし実際には、
スレッドの起動に時間がかかるので、`one.remove(0)` の判定はほぼ常に `two` が生きているうちに行われ、コピーする経路に入る。
`5dd94ea`、nightly 1.101.0 (2026-10-11) の Miri、x86_64、`-Zmiri-strict-provenance -Zmiri-preemption-rate=1 -Zmiri-many-seeds=0..32` で、
上と同じ形のテストで `remove` の後にポインタが変わったか (= コピーしたか) を出力すると、32 seed すべてで「コピーした」になった。
その場で書き換える経路は一度も通らなかった。また、相手のスレッドは drop の前に共有バイトを読まないので、仮に順序付けが壊れていても、
書き換えと競合する非 atomic なアクセスが存在しない。

そのため、`HeapBuffer::is_unique` を `load(Relaxed)` に変える退行を入れても、

- `tests/race_condition.rs` の 6 件は Miri (同じフラグ、`-Zmiri-many-seeds=0..64` まで) ですべて通る
- `tests/loom.rs` の 4 件もすべて通る (`RUSTFLAGS="--cfg loom" cargo test --test loom --features loom`。文字列のバイト列は loom の
  追跡対象 (`loom::cell::UnsafeCell`) ではないので、バイト列の読み書きの競合は loom の守備範囲外である。`tests/race_condition.rs` の
  冒頭コメントのとおり)

ことを確認した。また、`replace_range` / `extend_from_within` / `insert_str` / `make_ascii_*case` / `shrink_to` / `reserve_exact` / `clear`
は race テストも loom テストも無い。

なお `release` 側 (`fetch_sub(Release)` + `fence(Acquire)`) は既存のテストで守られている。`release` の `fence(Acquire)` を外すと、
`drop_lean_str_while_mutating_lean_string` が "Data race detected between (1) atomic store ... and (2) deallocation" で落ちる。

### clone のオーバーフロー

`Repr::make_shallow_clone` は `fetch_add` の前の値が `isize::MAX` を超えていたら、`ref_count_overflow` で参照を 1 つ戻してから panic する
(`Arc` は abort する)。この経路を通るテストは無い。64-bit では到達させる現実的な方法が無く、32-bit でも `mem::forget` で 2^31 回 clone
する必要があるので、公開 API からのテストは現実的でない。

## 原因

race テストが、相手スレッドの drop が `is_unique()` の判定より先に起きることを保証しておらず、相手スレッドが drop の前に共有バイトを
読んでいない。オーバーフローは公開 API からは現実的に到達させられないが、クレート内の単体テストも無い。

## 修正例

### 内容を読んでから drop するスレッドとの競合テスト

相手スレッドに「共有バイトを読む → drop → `Relaxed` でフラグを立てる」をさせ、こちらはフラグを `Relaxed` で待ってから変更する。
`Relaxed` のフラグは happens-before を作らないので、相手の読み取りとこちらの書き込みを順序付けるのは参照カウント
(`fetch_sub(Release)` と `is_unique` の `Acquire`) だけになる。順序付けが弱ければ Miri のデータ競合検出が報告する。

`tests/race_condition.rs` に足す (既存のテストもこの helper に置き換えられる)。

```rust
use lean_string::LeanString;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::thread;

fn drop_clone_while(f: impl FnOnce(&mut LeanString)) -> LeanString {
    let mut one = LeanString::with_capacity(64);
    one.push_str("abcdefghijklmnopqrstuvwxyz");
    let two = one.clone();
    let dropped = Arc::new(AtomicBool::new(false));
    let th = thread::spawn({
        let dropped = Arc::clone(&dropped);
        move || {
            // Read the shared bytes, then drop the clone.
            assert_eq!(two, "abcdefghijklmnopqrstuvwxyz");
            drop(two);
            dropped.store(true, Relaxed);
        }
    });
    // Wait until the other thread has dropped its clone. `Relaxed` creates no happens-before
    // edge, so the only synchronization with the read above is the reference count: `f` must
    // see the buffer as unique only through an `Acquire` load, or writing to it is a data race.
    while !dropped.load(Relaxed) {
        thread::yield_now();
    }
    f(&mut one);
    th.join().unwrap();
    one
}

#[test]
fn drop_while_replace_range() {
    let s = drop_clone_while(|s| s.replace_range(1..3, "BC"));
    assert_eq!(s, "aBCdefghijklmnopqrstuvwxyz");
}

#[test]
fn drop_while_extend_from_within() {
    let s = drop_clone_while(|s| s.extend_from_within(..3));
    assert_eq!(s, "abcdefghijklmnopqrstuvwxyzabc");
}

#[test]
fn drop_while_insert_str() {
    let s = drop_clone_while(|s| s.insert_str(1, "__"));
    assert_eq!(s, "a__bcdefghijklmnopqrstuvwxyz");
}

#[test]
fn drop_while_make_ascii_uppercase() {
    let s = drop_clone_while(|s| s.make_ascii_uppercase());
    assert_eq!(s, "ABCDEFGHIJKLMNOPQRSTUVWXYZ");
}

#[test]
fn drop_while_shrink_to_fit() {
    let s = drop_clone_while(|s| s.shrink_to_fit());
    assert_eq!(s, "abcdefghijklmnopqrstuvwxyz");
    assert_eq!(s.capacity(), 26);
}

#[test]
fn drop_while_reserve_exact() {
    let s = drop_clone_while(|s| s.reserve_exact(100));
    assert_eq!(s, "abcdefghijklmnopqrstuvwxyz");
    assert!(s.capacity() >= 126);
}

#[test]
fn drop_while_clear() {
    let s = drop_clone_while(|s| s.clear());
    assert_eq!(s, "");
}
```

容量 64 で作るのは、容量が足りていて共有だけが理由で `!is_modifiable()` になる状態にするため。`push` / `remove` / `retain` /
`truncate` / `into_lean_str` など既存のテストの対象も、同じ helper で書き直すとよい。

試した結果:

- x86_64 で `cargo test`、Miri (`-Zmiri-strict-provenance -Zmiri-preemption-rate=1`) の x86_64 / i686 / powerpc64 / powerpc で通る。
  x86_64 で 16 seed (`-Zmiri-many-seeds=0..16`) でも誤検出は無い。
- 経路: 上と同じポインタの比較で、32 seed のうち約半分がその場で書き換える経路を通った (残りは Miri の弱いメモリのエミュレーションで
  古いカウント 2 を読み、コピーする。どちらも正しい動作)。
- 検出: `is_unique` を `load(Relaxed)` にすると、既定の seed で `drop_while_insert_str` / `drop_while_replace_range` /
  `drop_while_reserve_exact` が `Undefined Behavior: Data race detected between (1) non-atomic read on thread ... and (2) non-atomic write ...`
  などで落ちた。`drop_while_make_ascii_uppercase` / `drop_while_shrink_to_fit` も 16 seed の中で検出した。検出は seed による確率的な
  ものだが、CI の Miri は 4 ターゲットで走るので、退行はどこかで見つかる見込みが高い。
- `drop_while_extend_from_within` と `drop_while_clear` は相手の読んだ範囲 (0..26) に書かないので検出には寄与しない。経路を通すためのテスト。

フラグで待つ代わりに `thread::sleep(Duration::from_millis(10))` で相手を先に走らせる形でも `is_unique` の `Relaxed` 化を検出できた
(Miri の sleep は仮想時計を進めて他のスレッドに実行を譲るだけで、同期は作らない)。ただし sleep を外すと検出されなかったので、待ち合わせ
自体は必要で、フラグで待つ上の形の方が時間に依存しない。

### clone のオーバーフローの単体テスト

参照カウントを直接書き換えられるクレート内の単体テストにする。

```rust
// src/repr.rs
#[cfg(all(test, feature = "std", not(loom)))]
mod refcount_tests {
    use super::*;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    #[test]
    fn clone_reports_reference_count_overflow_and_restores_count() {
        let mut repr = Repr::<Mutable>::from_str("a string longer than the inline limit").unwrap();
        let near_overflow = isize::MAX as usize + 1;
        unsafe { repr.as_heap_buffer() }.reference_count().store(near_overflow, Relaxed);

        let result = catch_unwind(AssertUnwindSafe(|| repr.make_shallow_clone()));

        assert!(result.is_err());
        let count = unsafe { repr.as_heap_buffer() }.reference_count();
        assert_eq!(count.load(Relaxed), near_overflow);
        count.store(1, Relaxed);
        unsafe { repr.drop_in() };
    }
}
```

試した結果: native と Miri (x86_64、strict provenance) で通る。

## 確認方法

- `MIRIFLAGS="-Zmiri-strict-provenance -Zmiri-preemption-rate=1" cargo +nightly miri test --test race_condition` が 4 ターゲットで通ること。
- `HeapBuffer::is_unique` の `Acquire` を一時的に `Relaxed` にしたときに、追加したテストが Miri で落ちることを一度確かめる
  (`-Zmiri-many-seeds=0..16` を付けると確実)。CI は seed 1 つで回しているので、実行順序への依存を減らしたいなら CI の
  "Test (race condition)" ステップに `-Zmiri-many-seeds=0..8` などを足す (ローカルでは 32 seed でも数秒だった)。
- `ref_count_overflow` で panic の前に参照を戻す処理 (`replace_inner`) を消すと、単体テストの
  `assert_eq!(count.load(Relaxed), near_overflow)` が落ちること。
