# Changelog

All notable changes to this package will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- **The web**: `loads` and `dumps` through the module the page loaded,
  which carries the serde surface (an application's own build); the same
  numbers, errors and positions as a native library.
- The package, taken out of package:guatiao, where it was
  `package:guatiao/serde.dart`: the same functions, over the
  `guatiao_serde` library, which it finds through
  `GUATIAO_SERDE_LIBRARY`, then `GUATIAO_LIBRARY` (a directory,
  or a file exporting its functions, else the file beside it).
