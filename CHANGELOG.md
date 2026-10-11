# Changelog

<!-- Ref: https://keepachangelog.com/ -->

## [0.8.0] - 2026-10-11

### Added

- Case conversion methods: `to_lowercase()`, `to_uppercase()`, `to_ascii_lowercase()` and `to_ascii_uppercase()` for `LeanString` and `LeanStr`, and `make_ascii_lowercase()` and `make_ascii_uppercase()` for `LeanString`. The ASCII ones have fallible `try_` variants. If no character changes, they don't allocate: the `to_` methods return a clone, and the `make_` methods leave the string untouched, even if it is shared with its clones.
- `LeanString::replace_range()`, `LeanString::extend_from_within()` and `LeanString::reserve_exact()`, with their fallible `try_` variants.
- `LeanStr::repeat()` and `LeanStr::try_repeat()`.
- `from_utf16le()`, `from_utf16be()`, `from_utf16le_lossy()` and `from_utf16be_lossy()` for `LeanString` and `LeanStr`, to decode UTF-16 from bytes.
- `as_static_str()` for `LeanString` and `LeanStr`, which returns the underlying `&'static str` if the string holds one.
- Conversions from and to more types:
  - `From<&mut str>` for `LeanString` and `LeanStr`.
  - `From<&LeanString>` for `LeanStr` and `From<&LeanStr>` for `LeanString`.
  - `From<LeanString>`, `From<LeanStr>`, `From<&LeanString>` and `From<&LeanStr>` for `Cow<'_, str>`. A string holding a `&'static str` converts into `Cow::Borrowed` without allocating.
  - `FromIterator<LeanStr>` for `LeanString`, `FromIterator<LeanString>` for `LeanStr`, and `FromIterator<LeanString>` and `FromIterator<LeanStr>` for `String`.
  - `AsRef<Path>` for `LeanString` and `LeanStr`, with the `std` feature.

### Changed

- The MSRV is now Rust 1.88.0 (was 1.85.1).
- **Breaking:** `to_lowercase()`, `to_uppercase()`, `to_ascii_lowercase()` and `to_ascii_uppercase()` of `LeanString` and `LeanStr`, and `LeanStr::repeat()`, now return `LeanString` or `LeanStr` instead of `String`. They were previously resolved to the `str` methods through `Deref`.
- **Breaking:** `FromUtf16Error` now matches `std`'s `FromUtf16Error`: it can no longer be constructed outside this crate, and its message tells a lone surrogate from an odd number of bytes.
- Added `#[must_use]` to constructors, getters and methods that return a new string, which may produce new warnings where their result is discarded.
- Modifying a string that shares its heap buffer with its clones now copies it into the inline storage, rather than a new heap buffer, if its contents fit there (e.g. after `truncate()`).
- `retain()` no longer copies a string shared with its clones or created from a `&'static str` when the predicate keeps every character.
- Creating a string short enough to be stored inline is even faster on 64-bit little-endian targets, as it is now assembled in registers instead of copied with `memcpy` (about 2x for lengths near the inline limit on aarch64, and about 5x for most lengths on x86_64).
- Creating a string from a `char` is several times faster, as it is also built in registers.
- `push()` and `insert()` of a `char`, and creating a heap-allocated string slightly over the inline limit from a `&str`, no longer call `memcpy` to copy the bytes.
- `repeat()` fills the result by doubling the copied part instead of appending the string `n` times, which is much faster for a short string repeated many times.
- With the `serde` feature, deserializing `LeanString` and `LeanStr` asks the deserializer for a `&str` rather than an owned `String`, which may save an intermediate allocation with some formats.

### Fixed

- `try_to_lean_string()` and `try_to_lean_str()` panicked on allocation failure for values converted through their `Display` implementation, instead of returning an error.
- A panic in `From<Cow<'_, str>>` for `LeanString` and `LeanStr` now reports the caller's location.
- Corrected the documentation of `capacity()`, `shrink_to_fit()`, `shrink_to()` and `clear()` for strings created from a `&'static str`.

## [0.7.0] - 2026-07-23

### Added

- `LeanStr`, an immutable version of `LeanString`, based on a design originally proposed by [@charliermarsh](https://github.com/charliermarsh) in [#6](https://github.com/ryota2357/lean_string/pull/6). ([#7](https://github.com/ryota2357/lean_string/pull/7))
- `LeanString::into_lean_str()` and `LeanStr::into_lean_string()`, with their fallible `try_` variants, to convert between the two types.
- `ToLeanStr` trait and `ToLeanStrError`, the `LeanStr` counterparts of `ToLeanString` and `ToLeanStringError`.
- `ToLeanString` and `ToLeanStr` implementations for `str`, so `to_lean_string()` and `to_lean_str()` can be called on an unsized `str`, just as `to_string()` can. Until now they only accepted a `&str`.
- The MSRV is now declared (Rust 1.85.1) and verified in CI.

### Changed

- **Breaking:** `reserve(0)` and `try_reserve(0)` are now no-ops, and inserting an empty string does nothing beyond checking the index. Neither clones a string shared with others.
- Replaced the `ryu` dependency with `zmij` for float formatting.
- Creating a string short enough to be stored inline is significantly faster (about 2x for lengths near the inline limit, measured on aarch64-darwin).
- Removed a redundant store on drop, making it cheaper to drop many strings at once, such as a `Vec<LeanString>`.
- Outlined `reserve`'s cold paths, so the common case generates less code at its call sites. ([#5](https://github.com/ryota2357/lean_string/pull/5) by [@charliermarsh](https://github.com/charliermarsh))
- Removed redundant capacity, allocation size and error checks from `From<&str>`, `from_static_str()`, `into_lean_str()` and `into_lean_string()`.

### Fixed

- Corrected the documented capacity limit on 32-bit architecture: it is `2^31 - 16`, not `2^32 - 1`.
- Documented the missing panic condition of `from_static_str()`.

## [0.6.1] - 2026-07-08

Most of the fixes in this release were contributed by [@charliermarsh](https://github.com/charliermarsh). Thank you!

### Fixed

- Several 32-bit correctness bugs: heap length tagging on big-endian targets, and heap base recovery from capacity. ([#4](https://github.com/ryota2357/lean_string/pull/4))
- Undefined behavior from references to uninitialized capacity, and to invalid UTF-8 during `retain` and `remove`.
- Memory leaks in `split_off` and `FromIterator` on early exit.
- Shared buffers are now shrunk to their exact capacity.

### Changed

- `FromIterator<LeanString>` reuses the first one, improving performance.

## [0.6.0] - 2026-04-12

### Added

- `as_str`, `as_bytes` and `len` are now `const fn`. ([#3](https://github.com/ryota2357/lean_string/pull/3) by [@lperlaki](https://github.com/lperlaki))
- `split_off` method.
- `repeat` method.

## [0.5.3] - 2026-03-18

### Changed

- Depend on `serde_core` instead of `serde` for the `serde` feature.

### Fixed

- Use-after-free that could occur under clone-on-write in `reserve`, `ensure_modifiable` and `truncate_unchecked` by switching to a read-only uniqueness check.

## [0.5.2] - 2026-03-12

### Changed

- Various performance improvements: specialized `Write` for static `&str`, faster `len()`, and `#[inline]` on `serde` and `Drop` implementations.

## [0.5.1] - 2025-08-24

### Fixed

- Memory leak in `shrink_to()`.

## [0.5.0] - 2025-05-16

### Added

- `truncate()` method.

### Changed

- Migrated to the Rust 2024 edition.

### Removed

- **Breaking:** the `last_byte` feature.

## [0.4.0] - 2025-01-21

### Changed

- `pop()` no longer clones the buffer, improving performance.

## [0.3.0] - 2024-12-27

### Added

- Support for 32-bit architectures.

### Fixed

- Panic on reference count overflow instead of risking unsound behavior.

## [0.2.0] - 2024-12-22

### Changed

- Performance improvements via specialization for `ToLeanString` and `From<char>`, plus `#[inline]` on several methods.

## [0.1.0] - 2024-12-20

Initial release.

### Added

- `LeanString`: a compact, clone-on-write string with small-string optimization.
- Growable string API: `push`, `push_str`, `pop`, `insert*`, `remove`, `retain`, `shrink_to*`, `with_capacity` and fallible `reserve`.
- Constructors `from_utf8` / `from_utf16` and the `ToLeanString` trait.
- `no_std` support.
- Optional `serde` and `arbitrary` support.
- `Send` and `Sync` implementations, verified with `loom` and `miri`.

[0.8.0]: https://github.com/ryota2357/lean_string/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/ryota2357/lean_string/compare/v0.6.1...v0.7.0
[0.6.1]: https://github.com/ryota2357/lean_string/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/ryota2357/lean_string/compare/v0.5.3...v0.6.0
[0.5.3]: https://github.com/ryota2357/lean_string/compare/v0.5.2...v0.5.3
[0.5.2]: https://github.com/ryota2357/lean_string/compare/v0.5.1...v0.5.2
[0.5.1]: https://github.com/ryota2357/lean_string/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/ryota2357/lean_string/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/ryota2357/lean_string/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/ryota2357/lean_string/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/ryota2357/lean_string/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/ryota2357/lean_string/releases/tag/v0.1.0
