# Changelog

All notable changes to this package will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

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
  - `package:guatiao/form.dart`: `check`, `layout`, `isVisible`.
  - `libraryDirectory`, for an app that ships its own copies of the
    libraries and cannot set its own environment.
