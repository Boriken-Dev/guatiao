# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Workspace scaffold: the `guatiao` crate and its `guatiao-derive`
  proc-macro companion, feature flags `derive`, `c-header` and `load`, and the
  test workflow.
- **The value model, as plain C structs.** Null, bool, number, string,
  bytes, list and map; one node type; `{ptr, len}` views and
  `{ptr, len, cap}` owned containers, each carrying the allocator that
  made it. A number is stored as the exact text that declared it, so
  `1.10` reads back as `1.10` and a 200-digit integer survives; map
  iteration is insertion order; nothing coerces between kinds. A C, C++ or
  Dart consumer reads a whole tree through the committed header with no
  call into any library.
- **Building from Rust names no allocator.** `Map::new()` hands back a
  `Map` you grow with `set`; `map.into()` makes it a value at the one
  point something wants one. `map.set("port", 5900)?` is the whole call,
  because `set` takes anything a value can be made from. A container that
  must live in a host's arena says so with the `_in` constructors, which
  stay fallible because a foreign allocator refusing is recoverable.
- **A value owns its tree** and frees it on drop, like any other Rust
  value; handing one to something else is a move. A value crossing a
  boundary is forgotten, and the far side frees it with
  `guatiao_value_free` — the same walk, which is iterative because a tree
  may have arrived from a caller this crate cannot see.
- **Reading from Rust**: `ReadValue` for a lookup that composes, and
  `TryFrom<&Value>` for every scalar, so `try_into` is the conversion.
- **The schema**, which is how a provider says what it needs to be
  initialised. It is an ordinary value with a documented key vocabulary,
  so a consumer in any language reads one by walking a map. Builders,
  readers, validation and a flat `key -> text` projection.
- **Configuration layering**: three merge strategies, per-path overrides a
  schema can declare through `x-merge`, and provenance recorded per leaf
  path.
- **Three derives**: `ToValue`, `FromValue` and `Schema`, which read one
  declaration so a type cannot describe a value it refuses.
- No serialisation: how a value or a schema is written down belongs to a
  layer above, which can carry more than one format.
- Licensed under the Mozilla Public License 2.0. Using the library imposes
  nothing on your own code; modifying its files does. Contributions need a
  one-time contributor licence agreement.

[Unreleased]: https://github.com/Boriken-Dev/guatiao/commits/main
