# Changelog

<!-- Ref: https://keepachangelog.com/ -->

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
