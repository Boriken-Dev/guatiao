# Changelog

All notable changes to this package will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.0.0-alpha.1] - 2026-10-16

### Changed

- The generated bindings follow the C header: the status enum's tag is
  `guatiao_status_code`, and `guatiao_status` is the plain integer
  typedef. No name this package exports changes.
- `pubspec.yaml` declares the platforms the package runs on (Android, iOS,
  Linux, macOS and Windows). `dart:ffi` is not available on the web, so
  pub.dev no longer lists it there.

### Added

- `package:guatiao/intake.dart` wraps the flat `key -> text` projection:
  `resolve`, `flatKeys`, `flatten` and `unflatten`.
- `loads` reads through the `_parse_ex` entry points and throws
  `ParseError` (a `BadValue` and a `FormatException`) carrying `format`,
  `line` and `column`.

### Fixed

- `guatiao_value` and `guatiao_alloc` are exported from
  `package:guatiao/guatiao.dart`, which the public API names in its
  signatures; a consumer can write `Pointer<guatiao_value>` without
  importing a `src/` file.
## [0.0.0-alpha.0] - 2026-09-27

### Added

- First release: `dart:ffi` bindings over the guatiao C ABI.
  - `Value`, `Ref`, `MapRef` and `ListRef`: a value tree built, read and
    changed in place, or converted to and from plain `Map`/`List` data.
    A number keeps its exact text, and `toDart` reads it back as an
    `int`, a `BigInt`, a `double` or the text itself.
  - `Registry` and `Instance`: load a library by file or by scan, list
    providers best first, rank them, read a provider's configuration
    schema, build an instance from a configuration, and take a library
    back out with `retire` or `unload`.
  - `kindTable`: a provider's function table, checked against the size
    and the floor hash the kind's own C header declares.
  - `package:guatiao/serde.dart`: JSON, TOML and YAML.
  - `package:guatiao/intake.dart`: `check`, `layout`, `isVisible`.
  - `libraryDirectory`, for an app that ships its own copies of the
    libraries and cannot set its own environment.
