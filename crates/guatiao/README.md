# guatiao

[![Crate](https://img.shields.io/crates/v/guatiao.svg)](https://crates.io/crates/guatiao)
[![docs.rs](https://img.shields.io/docsrs/guatiao)](https://docs.rs/guatiao)
[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

**One contract for passing values between languages.**
A value model whose C form is plain structs with pointer-and-length strings,
a schema that describes a value — what a provider needs to be configured, or
a record, capabilities or metadata one library hands another — and a library
envelope through which one library registers any number of providers.
Nothing is opaque: a C, C++, Dart or Rust consumer reads a tree, a schema or
a descriptor with no call into any library.

*Guatiao* is the Taíno pact in which two people exchange names and become
kin. An ABI is the same agreement between two sides of a boundary.

> **Status: alpha.** The API can still change between releases; the
> [changelog](https://github.com/Boriken-Dev/guatiao/blob/main/CHANGELOG.md)
> says what did and what to do about it.

## Features

- **Values as C structs** — null, bool, number (kept as text), string, bytes,
  list and map; one node type; insertion-ordered.
- **Views and owned containers** — `{ptr, len}` views for every parameter and
  read; `{ptr, len, cap}` owned containers that grow through the allocator
  that travels with the tree.
- **One allocator, carried** — every owned container records the allocator
  that made it, so a tree built in a library frees correctly in the host.
- **A schema is a JSON Schema** — fields, kinds, defaults and validity,
  written as an ordinary value in JSON Schema 2020-12's own keys, so a
  schema written out as text is a document existing tools already read.
  There is no serialisation here: how a schema is written down is the
  consumer's decision.
- **Rust ergonomics on top** — `#[derive(ToValue, FromValue, Schema)]`,
  `try_into` for reading, and a generated C header for everyone else.

## Installation

```toml
[dependencies]
guatiao = { version = "0.0.0-alpha.0", features = ["derive"] }
```

Optional features:

| Flag | Adds | Needed for |
| --- | --- | --- |
| `derive` | `guatiao-derive` | `#[derive(ToValue, FromValue, Schema)]` |
| `provider` | `derive`, with `syn`'s full parser | `#[guatiao::kind]`, `#[derive(Provider)]`, `guatiao::providers!`: a kind crate and every library need it |
| `load` | `libloading`, `object` | the `Registry` and loading libraries from disk into it (`load_file`, `scan_dir`, `scan_path`): a host needs it |
| `regex` | `regex` | enforcing a schema's `pattern` when validating |
| `c-header` | nothing | regenerating the committed C header |

A default build pulls in nothing.

## Quick start

```rust
use guatiao::Map;

let mut options = Map::new();
options.set("compression", 6)?;

let mut map = Map::new();
map.set("host", "10.0.0.1")?;
map.set("port", 5900)?;
map.set("tls", true)?;
map.set("options", options)?;

// Get the value, then convert it: no per-kind getters on a map, and no
// per-kind readers on a value. `required` is the step from a lookup to a
// value and it names the key it did not find; the rest is `TryInto`.
let host: &str = map.required("host")?.try_into()?;
let port: u16 = map.required("port")?.try_into()?;
let options: &Map = map.required("options")?.try_into()?;
let compression: i64 = options.required("compression")?.try_into()?;
assert_eq!((host, port, compression), ("10.0.0.1", 5900, 6));

// A key that is absent and a value of another kind each say which it was.
assert!(map.required("missing").is_err());
assert!(TryInto::<i64>::try_into(map.required("host")?).is_err());
```

A struct can do all of that for you:

```rust
use guatiao::{Alloc, FromValue, Schema, ToValue};

#[derive(ToValue, FromValue, Schema, PartialEq, Debug)]
struct Connection {
    /// Where to connect.
    host: String,
    port: u16,
    #[schema(sensitive)]
    password: Option<String>,
}
```

`Connection::schema(alloc)` is what a consumer reads to know it must ask for
a host, may leave out a password, and should not log it. `to_value` and
`from_value` move the struct across the boundary, and the three derives read
one declaration, so the schema cannot describe a value the type refuses.

## API overview

| Module | Purpose |
| --- | --- |
| `guatiao::value` | the value model: `Value`, `Map`, `List`, `Text`, `Buffer`, `Number`, the allocator, conversion, merging, and the `wire` byte format |
| `guatiao::schema` | JSON Schema as a value: `SchemaBuilder`/`FieldBuilder`/`KindBuilder`, the `SchemaRef` reader, `validate_*` |
| `guatiao::library` | the envelope: one entry symbol, provider descriptors, a `Registry` a host owns, and (with `load`) scanning and loading |
| `guatiao::exports` | the `extern "C"` surface declared in `include/guatiao.h` |

Two sibling crates build on this one:
[`guatiao-serde`](https://crates.io/crates/guatiao-serde) writes and reads
a value in any serde format, and
[`guatiao-intake`](https://crates.io/crates/guatiao-intake) describes how a
schema is shown to a person.

## Development

The four crates, the examples and the Python and Dart bindings share one
workspace; its [README](https://github.com/Boriken-Dev/guatiao#development)
has the commands.

```bash
cargo test --workspace --all-features
```

## License

**Mozilla Public License 2.0** — see
[LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
Per-file copyleft: using this crate imposes nothing on your code, whether
you link it statically or dynamically and whatever licence your own work
carries. Modifying this crate's own files is what carries the obligation to
publish those files under the MPL 2.0.

A combined work may be distributed under terms of your choosing (MPL 2.0
section 3.3), and the licence stays GPL-compatible.
