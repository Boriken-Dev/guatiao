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
  initialised. It is an ordinary value, **and that value is a JSON Schema
  (2020-12)**: `properties`, `required`, `type`, `enum`, `oneOf` and the
  rest, spelled the specification's way, so a schema written out as text is
  a document existing JSON Schema tools already read. What this crate adds
  is `x-` prefixed. Two things are deliberately not the specification's:
  `type: "bytes"`, which a strict meta-schema check refuses, and
  `x-variant-tag`, because JSON Schema has no discriminator keyword.
  Builders, readers, validation and a flat `key -> text` projection.
- **Configuration layering**: three merge strategies, per-path overrides a
  schema can declare through `x-merge`, and provenance recorded per leaf
  path.
- **Three derives**: `ToValue`, `FromValue` and `Schema`, which read one
  declaration so a type cannot describe a value it refuses. For a struct
  with named fields, an enum of unit variants (stored as the variant's
  name, described as a choice), and an enum naming its tag with
  `#[map(tag = "...")]` (stored as a map, described as a tagged variant).
  An enum whose variants carry fields and names no tag is refused, because
  the key that tells variants apart is yours to choose.
- No serialisation: how a value or a schema is written down belongs to a
  layer above, which can carry more than one format.
- Licensed under the Mozilla Public License 2.0. Using the library imposes
  nothing on your own code; modifying its files does. Contributions need a
  one-time contributor licence agreement.

[Unreleased]: https://github.com/Boriken-Dev/guatiao/commits/main
