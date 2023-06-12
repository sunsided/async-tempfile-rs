# Changelog

All notable changes to this project will be documented in this file.
This project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
