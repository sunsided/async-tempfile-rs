# Changelog

All notable changes to this project will be documented in this file.
This project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.7.0] - 2025-02-22

[0.7.0]: https://github.com/sunsided/async-tempfile-rs/releases/tag/v0.7.0

### Added

- Added the `drop_async` functions to provide controlled and `async` dropping.

## [0.6.0] - 2024-06-30

[0.6.0]: https://github.com/sunsided/async-tempfile-rs/releases/tag/v0.6.0

### Added

- [#7](https://github.com/sunsided/async-tempfile-rs/pull/7):
  Added `TempDir` for automatically deleted temporary directories.

### Internal

- Refactored temporary file name generation.
- Run tests on Linux, MacOS and Windows.

### Changed

- Changed function signatures to take `Borrow<Path>` instead of `PathBuf`.

## [0.5.0] - 2023-12-06

[0.5.0]: https://github.com/sunsided/async-tempfile-rs/releases/tag/0.5.0

### Changed

- The `new` and `new_in` functions now do not rely on the `uuid` feature anymore
  for the generation of temporary file names.
- The `uuid` feature is now not enabled by default anymore.

### Internal

- Some unnecessary heap allocations were removed.

## [0.4.0] - 2023-06-16

[0.4.0]: https://github.com/sunsided/async-tempfile-rs/releases/tag/0.4.0

### Added

- Added `uuid` as a default crate feature and feature gated all `uuid` crate related functionality.
- Added the `new_with_name` and `new_with_name_in` methods to use a provided file name.
- Added the `new_with_uuid` and `new_with_uuid_in` methods to use a provided UUID
  as the file suffix.
- The library now explicitly declares `allow(unsafe_code)`.

## 0.3.0 - 2023-06-12

### Added

- Added the `open_ro` method to create a new clone in read-only mode.

## 0.2.0 - 2022-10-22

### Added

- Added the functionality to create both borrowed and owned `TempFile` instances
  from an existing file. Previously, only borrowed instances were possible this way.
- The `TempFile` methods are now returning a crate specific error type.

## 0.1.0 - 2022-10-22

### Internal

- 🎉 Initial release.
