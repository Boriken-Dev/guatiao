# guatiao

[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)
[![Crate](https://img.shields.io/crates/v/guatiao.svg)](https://crates.io/crates/guatiao)
[![PyPI](https://img.shields.io/pypi/v/guatiao.svg)](https://pypi.org/project/guatiao/)
[![Docs](https://img.shields.io/badge/docs-online-blue.svg)](https://boriken-dev.github.io/guatiao/)
[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](LICENSE)

**One contract for passing configuration and providers between languages.**
A value model whose C form is plain structs with pointer-and-length strings,
a schema through which a provider says what it needs to be initialised, and a
library envelope through which one library registers any number of providers.
The design rule that makes it trustworthy: nothing is opaque, so a C, C++,
Dart or Rust consumer reads a tree, a schema or a descriptor with no call
into any library.

*Guatiao* is the Taíno pact in which two people exchange names and become
kin. An ABI is the same agreement between two sides of a boundary.

## Features

- **Values as C structs** — null, bool, number (kept as text, so nothing is
  rounded), string, bytes, list and map, one node type, insertion-ordered.
- **Views and owned containers** — `{ptr, len}` views for every parameter and
  read; `{ptr, len, cap}` owned containers that grow through the allocator
  that travels with the tree, so either side of a boundary can add to a map
  it was handed and free it safely.
- **One allocator, carried** — every owned container records the allocator
  that made it, so a tree built inside a library frees correctly in the host.
- **Schema as the init contract** — a provider declares the options it takes,
  their kinds, defaults and validity, written as an ordinary value with a
  documented key vocabulary; a consumer validates before crossing.
- **Configuration layering** — three merge strategies, per-path overrides a
  schema can declare, and provenance recorded per leaf path.
- **Library envelope and loader** — one entry symbol, a descriptor listing
  providers by kind, and an optional loader that scans a directory and hands
  back what it found.
- **Rust ergonomics on top** — `#[derive(ToValue, FromValue, Schema)]`,
  `try_into` for reading, and a generated C header for everyone else.

## Installation

```toml
[dependencies]
guatiao = { git = "https://github.com/Boriken-Dev/guatiao.git" }
```

Optional features:

| Flag | Adds | Needed for |
| --- | --- | --- |
| `derive` | `guatiao-derive` | `#[derive(ToValue, FromValue, Schema)]` |
| `c-header` | nothing | regenerating `include/guatiao.h` (it is committed, so most builds do not) |

A default build pulls in nothing.

Bindings over the same C ABI, for a consumer that is not Rust:

| Language | Package | Install |
| --- | --- | --- |
| Python | [`bindings/python/`](bindings/python/README.md) | `pip install guatiao` |
| Dart | [`bindings/dart/`](bindings/dart/README.md) | `dart pub add guatiao` |

Each drives a shared library that exports the ABI — `guatiao` itself, or
an application that carries it — named by `GUATIAO_LIBRARY`.

## Quick start

Build a value, then read it back. Building in Rust names no allocator —
the crate's own is the default, and a library building into a host's arena
says so with the `_in` constructors. Get the value and convert it: there
are no per-kind getters on a map.

```rust
use guatiao::Map;

let mut options = Map::new();
options.set("compression", 6)?;

let mut map = Map::new();
map.set("host", "10.0.0.1")?;
map.set("port", 5900)?;
map.set("options", options)?;

let host: &str = map.required("host")?.try_into()?;
let port: u16 = map.required("port")?.try_into()?;
assert_eq!((host, port), ("10.0.0.1", 5900));
# Ok::<(), Box<dyn std::error::Error>>(())
```

The same tree is what a C, C++ or Dart consumer reads directly, through
the generated header in `crates/guatiao/include/`. The full quick start,
including the derives, is in [that crate's README](crates/guatiao/README.md)
— which is compiled and checked by a test, unlike this one.

## Development

```bash
cargo test --workspace --all-features
cargo clippy --workspace --all-features -- -D warnings
cargo fmt --all --check
```

The C header at `crates/guatiao/include/guatiao.h` is rendered by `build.rs`
and committed, so a consumer reads it out of the repository without a Rust
toolchain. A test compares the committed file against what the build just
rendered, so it cannot go stale. To refresh it:

```bash
GUATIAO_WRITE_HEADER=1 cargo build -p guatiao --features c-header
```

## Licence

**Mozilla Public License 2.0** — see [`LICENSE`](LICENSE). Every file in this
repository is Covered Software under that licence.

Its copyleft is per file, which is the whole point for a library like this
one:

- **Using guatiao imposes nothing on your code.** Include the header, link
  the crate statically or dynamically, ship a closed-source binary. The one
  obligation is that guatiao's own source stays available, which it is.
- **Changing guatiao's own files does carry an obligation**: a modified file
  of this project is published under the MPL 2.0. Your files beside it stay
  yours, under any licence you choose.
- A combined work may be distributed under terms of your choosing (MPL 2.0
  section 3.3), and the licence stays GPL-compatible, so a GPL consumer can
  use this too.

Contributions need a one-time [contributor licence agreement](CLA.md); see
[`CONTRIBUTING.md`](CONTRIBUTING.md).
