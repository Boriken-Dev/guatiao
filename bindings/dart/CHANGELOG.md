# Changelog

All notable changes to this package will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- `package:guatiao/time.dart`: dates, times of day, civil date-times,
  instants, spans and exact lengths, each read and written by the
  library's own functions so every language accepts the same text.
  `PlainDate`, `PlainTime` and `PlainDateTime` keep a civil date or time
  to the nanosecond (a `DateTime` would carry a zone); an instant is a
  UTC `DateTime`, an exact length a `Duration`, both to the microsecond;
  `Span` is a calendar length, applied with `PlainDate.add` and
  `PlainDateTime.add`. Text a grammar refuses throws `Unparsed`, with
  the byte offset.
- `redact(schema, value)`: a copy with every field the schema marks
  secret replaced by a placeholder.

## [0.0.0-alpha.1] - 2026-10-19

### Changed

- A retired library is closed once nothing uses it: by the new
  `Registry.collect()`, which returns the keys it closed, or by the
  registry's next load, retire or unload. A `ProviderTable` taken from
  it is valid until then. Migration: close an `Instance` before its
  library is retired, and drop a `ProviderTable` before the next call.
- `unload` also answers `Busy` while another library holds a pinned
  answer from the library through the host's services.
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
- `Ref.borrowedFrom(source, find)`: a read-only view of a node inside
  another `Ref`, which holds `source` and finds the node again on every
  access.

### Fixed

- `guatiao_value` and `guatiao_alloc` are exported from
  `package:guatiao/guatiao.dart`, which the public API names in its
  signatures; a consumer can write `Pointer<guatiao_value>` without
  importing a `src/` file.
- What `intake.resolve` and `intake.at` return no longer outlives the
  schema or value it borrows from: it holds its owner, and reading it after
  the owner is closed throws a `StateError` instead of reading freed memory.
- Every write through a borrowed `Ref` throws, including `remove`,
  `removeAt` and `clear`, which used to change the tree they viewed.
- `intake.unflatten` throws an `ArgumentError` for a `Ref` store that is not
  a map of strings, as it already did for a `Map`; `null` stays the answer
  for nothing stored.
- A parse refusal that is not a syntax error keeps the library's message in
  the thrown exception.
- `ParseError`'s message is located from the struct's size, not a pinned
  offset, and a process with pointers that are not eight bytes wide gets an
  `UnsupportedError` when the first library loads.
- The 3.6.0 test leg no longer fails to compile on a suite that needs
  `NativeCallable.isolateGroupBound`; that suite runs on the stable leg only.

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
