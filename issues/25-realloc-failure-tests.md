# realloc の失敗経路がテストされておらず、テスト用アロケータでは `Layout` の食い違いも検出できない

- 種別: test
- 対象: `tests/out_of_memory.rs`。テストされていない経路は `src/repr/heap_buffer.rs` の `HeapBuffer::into_exact` (失敗時の復元)、`HeapBuffer::into_growable`、`HeapBuffer::realloc` (`Repr::reserve` / `Repr::shrink_to` から)、`src/repr.rs` の `Repr::into_immutable` / `into_mutable` の共有時の確保失敗
- 影響: このクレートでいちばん込み入った unsafe (`into_exact` の失敗時にバイト列・長さの prefix・ヘッダを書き戻す処理) が一度も実行されていない。壊しても既存のテストは通る
- 優先度: 高

## 問題

`tests/out_of_memory.rs` には次の確保・再確保を 1 回だけ失敗させる `FAIL_NEXT_ALLOCATION` / `FAIL_NEXT_REALLOCATION` があるが、`FAIL_NEXT_REALLOCATION` を立てているのは「再確保が起きなかったこと」を確かめる `without_allocating` だけで、realloc を失敗させるテストは無い (`5dd94ea` で確認)。`try_into_lean_str` / `try_into_lean_string` の確保失敗 (共有されている場合) のテストも無い。

公開 API から通せる未テストの経路:

| 経路 | 通す操作 |
| --- | --- |
| `HeapBuffer::realloc` の失敗 | unique な heap の `try_reserve` / `try_push_str` / `try_shrink_to_fit` |
| `HeapBuffer::into_exact` の失敗時の復元 | unique な heap の `LeanString` の `try_into_lean_str()` |
| 同上、32-bit で容量だけが長さの prefix を持つ場合 | `LeanString::with_capacity((1 << 24) - 1)` に短い文字列を入れて `try_into_lean_str()` |
| 同上、32-bit で長さも heap にある場合 | 長さ `(1 << 24) - 1` の文字列で `try_into_lean_str()` |
| `HeapBuffer::into_growable` の失敗 | unique な heap の `LeanStr` の `try_into_lean_string()` |
| `into_immutable` / `into_mutable` の共有時の確保失敗 | clone した heap の文字列の `try_into_lean_str()` / `try_into_lean_string()` |

さらに、テストを足すだけでは足りない。`FailNextAllocation` は `System` に委譲しているが、`System` の `dealloc` / `realloc` (libc の `free` / `realloc`) は渡された `Layout` を使わないので、ヘッダが壊れて間違った `Layout` で解放されても何も起きない。Miri でも、カスタムの global allocator が `System` に委譲している限り `Layout` の食い違いは検出されない (Miri が `Layout` を照合するのは既定の global allocator を使ったときだけ)。

実際に `into_exact` の失敗時のヘッダの書き戻し (`ptr::write(header, GrowableHeader { .. })`) を消してみると、下の realloc 失敗テストは今のアロケータのままでは Miri (x86_64、strict provenance) で通ってしまった (native も `free` が `Layout` を見ないので同様のはずだが未確認)。`try_into_lean_str` は失敗時に元の値を drop するだけなので、壊れたヘッダは「間違った `Layout` での解放」としてしか表に出ない。

## 修正例

### アロケータ: 生きているブロックのサイズを記録して照合する

ブロックの前にサイズを置く方法は、std の `RawVec` などが渡すポインタが Stacked Borrows 上で確保範囲に絞られているため、Miri で UB になる (試した)。アドレスをキーにした表で照合する。

```rust
// `System` (`free` / `realloc`) ignores the `Layout` it is given, so on its own neither the tests
// nor Miri notice a buffer whose header no longer describes its allocation. Record the size of
// every live block (keyed by address) and abort if `dealloc` / `realloc` passes a different one.
mod sizes {
    use std::sync::atomic::{AtomicUsize, Ordering::*};

    const SLOTS: usize = 1 << 12;
    const EMPTY: usize = 0;
    const TOMBSTONE: usize = usize::MAX;
    static ADDRS: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(EMPTY) }; SLOTS];
    static SIZES: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];

    fn start(addr: usize) -> usize {
        (addr >> 3).wrapping_mul(0x9E37_79B9) % SLOTS
    }

    pub fn insert(addr: usize, size: usize) {
        let mut i = start(addr);
        for _ in 0..SLOTS {
            let cur = ADDRS[i].load(Relaxed);
            if (cur == EMPTY || cur == TOMBSTONE)
                && ADDRS[i].compare_exchange(cur, addr, AcqRel, Relaxed).is_ok()
            {
                SIZES[i].store(size, Release);
                return;
            }
            i = (i + 1) % SLOTS;
        }
        // Table full: leave this block unchecked.
    }

    fn remove(addr: usize) -> Option<usize> {
        let mut i = start(addr);
        for _ in 0..SLOTS {
            let cur = ADDRS[i].load(Acquire);
            if cur == EMPTY {
                return None;
            }
            if cur == addr {
                let size = SIZES[i].load(Acquire);
                ADDRS[i].store(TOMBSTONE, Release);
                return Some(size);
            }
            i = (i + 1) % SLOTS;
        }
        None
    }

    pub fn check(addr: usize, size: usize) {
        if remove(addr).is_some_and(|recorded| recorded != size) {
            // The `Layout` passed back does not match the one the block was allocated with.
            std::process::abort();
        }
    }
}

unsafe impl GlobalAlloc for FailNextAllocation {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if FAIL_NEXT_ALLOCATION.replace(false) {
            return ptr::null_mut();
        }
        // SAFETY: The caller provides a valid layout.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            sizes::insert(ptr.addr(), layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        sizes::check(ptr.addr(), layout.size());
        // SAFETY: `ptr` was allocated by `System` with this layout.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        sizes::check(ptr.addr(), layout.size());
        let new = if FAIL_NEXT_REALLOCATION.replace(false) {
            ptr::null_mut()
        } else {
            // SAFETY: The caller provides a pointer allocated with `layout`.
            unsafe { System.realloc(ptr, layout, new_size) }
        };
        if new.is_null() {
            sizes::insert(ptr.addr(), layout.size()); // the old block stays live
        } else {
            sizes::insert(new.addr(), new_size);
        }
        new
    }
}
```

### テスト

```rust
#[cfg(target_pointer_width = "32")]
const MIN_CAPACITY_FOR_HEAP_LENGTH_LAYOUT_ON_32BIT: usize = (1 << 24) - 1;

/// Runs `f` with the next reallocation armed to fail, and asserts that `f` attempted one.
#[track_caller]
fn with_failing_reallocation<T>(f: impl FnOnce() -> T) -> T {
    FAIL_NEXT_REALLOCATION.set(true);
    let value = f();
    assert!(!FAIL_NEXT_REALLOCATION.replace(false), "no reallocation was attempted");
    value
}

#[test]
fn try_reserve_reallocation_failure_leaves_string_unchanged() {
    let mut string = LeanString::from(TEXT);
    let (ptr, capacity) = (string.as_ptr(), string.capacity());

    let result = with_failing_reallocation(|| string.try_reserve(1000));

    assert_eq!(result, Err(ReserveError));
    assert_eq!(string, TEXT);
    assert_eq!(string.as_ptr(), ptr);
    assert_eq!(string.capacity(), capacity);
    string.push_str("!"); // still unique and usable
    assert_eq!(string, format!("{TEXT}!"));
}

#[test]
fn try_push_str_reallocation_failure_leaves_string_unchanged() {
    let mut string = LeanString::from(TEXT);
    let ptr = string.as_ptr();

    let result = with_failing_reallocation(|| string.try_push_str(TEXT));

    assert_eq!(result, Err(ReserveError));
    assert_eq!(string, TEXT);
    assert_eq!(string.as_ptr(), ptr);
}

#[test]
fn try_shrink_to_reallocation_failure_leaves_string_unchanged() {
    let mut string = LeanString::with_capacity(100);
    string.push_str(TEXT);
    let ptr = string.as_ptr();

    let result = with_failing_reallocation(|| string.try_shrink_to_fit());

    assert_eq!(result, Err(ReserveError));
    assert_eq!(string, TEXT);
    assert_eq!(string.as_ptr(), ptr);
    assert_eq!(string.capacity(), 100);
}

// `into_exact` moves the bytes before `realloc`, so its failure path has to leave a buffer that
// is freed with the right layout. The allocator above checks that when the string is dropped.
#[test]
fn try_into_lean_str_reallocation_failure() {
    let mut string = LeanString::with_capacity(100);
    string.push_str(TEXT);
    let result = with_failing_reallocation(|| string.try_into_lean_str());
    assert_eq!(result, Err(ReserveError));
}

#[test]
fn try_into_lean_str_reallocation_failure_exact_capacity() {
    let string = LeanString::from(TEXT);
    let result = with_failing_reallocation(|| string.try_into_lean_str());
    assert_eq!(result, Err(ReserveError));
}

#[test]
fn try_into_lean_string_reallocation_failure() {
    let str = LeanStr::from(TEXT);
    let result = with_failing_reallocation(|| str.try_into_lean_string());
    assert_eq!(result, Err(ReserveError));
}

#[test]
fn try_into_lean_str_allocation_failure_keeps_shared_buffer() {
    let mut string = LeanString::with_capacity(100);
    string.push_str(TEXT);
    let mut shared = string.clone();
    let ptr = shared.as_ptr();

    FAIL_NEXT_ALLOCATION.set(true);
    let result = string.try_into_lean_str();
    FAIL_NEXT_ALLOCATION.set(false);

    assert_eq!(result, Err(ReserveError));
    assert_eq!(shared, TEXT);
    // The failed conversion released its reference, so `shared` is unique and writes in place.
    without_allocating(|| shared.push('!'));
    assert_eq!(shared.as_ptr(), ptr);
}

#[test]
fn try_into_lean_string_allocation_failure_keeps_shared_buffer() {
    let str = LeanStr::from(TEXT);
    let shared = str.clone();

    FAIL_NEXT_ALLOCATION.set(true);
    let result = str.try_into_lean_string();
    FAIL_NEXT_ALLOCATION.set(false);

    assert_eq!(result, Err(ReserveError));
    assert_eq!(shared, TEXT);
}

#[cfg(target_pointer_width = "32")]
#[test]
fn try_into_lean_str_reallocation_failure_length_prefix_capacity() {
    // Only the capacity needs the length prefix; the exact layout does not have one.
    let mut string = LeanString::with_capacity(MIN_CAPACITY_FOR_HEAP_LENGTH_LAYOUT_ON_32BIT);
    string.push_str(TEXT);
    let result = with_failing_reallocation(|| string.try_into_lean_str());
    assert_eq!(result, Err(ReserveError));
}

#[cfg(target_pointer_width = "32")]
#[test]
fn try_into_lean_str_reallocation_failure_length_prefix() {
    let len = MIN_CAPACITY_FOR_HEAP_LENGTH_LAYOUT_ON_32BIT;
    let mut string = LeanString::with_capacity(len + 100);
    string.push_str(&"a".repeat(len));
    let result = with_failing_reallocation(|| string.try_into_lean_str());
    assert_eq!(result, Err(ReserveError));
}

#[cfg(target_pointer_width = "32")]
#[test]
fn try_into_lean_string_reallocation_failure_length_prefix() {
    let str = LeanStr::from("a".repeat(MIN_CAPACITY_FOR_HEAP_LENGTH_LAYOUT_ON_32BIT).as_str());
    let result = with_failing_reallocation(|| str.try_into_lean_string());
    assert_eq!(result, Err(ReserveError));
}

#[cfg(target_pointer_width = "32")]
#[test]
fn try_reserve_reallocation_failure_length_prefix() {
    let text = "a".repeat(MIN_CAPACITY_FOR_HEAP_LENGTH_LAYOUT_ON_32BIT);
    let mut string = LeanString::with_capacity(text.len());
    string.push_str(&text);
    let ptr = string.as_ptr();
    let result = with_failing_reallocation(|| string.try_reserve(text.len()));
    assert_eq!(result, Err(ReserveError));
    assert_eq!(string.as_ptr(), ptr);
    assert_eq!(string, text); // compare with `==` (memcmp); a per-byte loop over 16 MiB is very slow in Miri
}
```

(`use lean_string::LeanStr` を追加する。)

### 試した結果 (`5dd94ea` に上記を追加)

- native (x86_64): 全 23 テスト通過。
- Miri `-Zmiri-strict-provenance`: x86_64 で 23 テスト、i686 で 27 テスト (32-bit 専用の 4 つを含む。16 MiB のテストも含めて 4 秒程度) が通過。UB もリークも報告されない。
- 検出力の確認: `into_exact` の失敗時のヘッダの書き戻しを消すと、native x86_64 では `sizes::check` で abort、Miri i686 では壊れた容量から長さの prefix があると判定して範囲外のポインタ演算として UB が報告される。照合なしの元のアロケータでは Miri x86_64 で検出されなかった。
- 失敗時のバイト列の書き戻し (`ptr::copy(new_data, old_data, len)`) を消しても通る。`try_into_lean_str` は失敗時に元の値を drop するだけで中身を読まないので、これは公開 API からは観測できない ([#09](./09-conversion-restore-on-failure.md) 参照)。

## 確認方法

上のテストが native と Miri (x86_64 / i686、strict provenance) で通ること。`into_exact` / `into_growable` / `HeapBuffer::realloc` を書き換えたときは、失敗経路を意図的に壊して (ヘッダの書き戻しを消すなど) テストが落ちることも一度確かめる。
