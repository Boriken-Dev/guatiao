# Changelog

All notable changes to this package will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- **A build hook that builds the library from source**: the crate
  `guatiao-intake`, with cargo, for the target being built, bundled with the
  app; nothing when package:guatiao's hook builds the application's own
  crate or nothing. Dart 3.10 or newer.
- **The web**: every function through the module the page loaded
  (`guatiao_intake.wasm`, or an application's own build).
- The package, taken out of package:guatiao, where it was
  `package:guatiao/intake.dart`: the same functions, over the
  `guatiao_intake` library, which it finds through
  `GUATIAO_INTAKE_LIBRARY`, then `GUATIAO_LIBRARY` (a directory,
  or a file exporting its functions, else the file beside it).
