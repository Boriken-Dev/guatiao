# guatiao

[![Version](https://img.shields.io/crates/v/guatiao.svg)](https://crates.io/crates/guatiao)
[![Rust version](https://img.shields.io/crates/msrv/guatiao.svg)](https://crates.io/crates/guatiao)
[![PyPI](https://img.shields.io/pypi/v/guatiao.svg)](https://pypi.org/project/guatiao/)
[![pub package](https://img.shields.io/pub/v/guatiao.svg)](https://pub.dev/packages/guatiao)
[![License: MPL-2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![Docs](https://img.shields.io/badge/docs-online-blue.svg)](https://boriken-dev.github.io/guatiao/)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

**One contract for passing configuration and providers between languages.**
A value model whose C form is plain structs with pointer-and-length strings,
a schema through which a provider says what it needs to be initialised, and a
library envelope through which one library registers any number of providers.
The design rule that makes it trustworthy: nothing is opaque, so a C, C++,
Dart or Rust consumer reads a tree, a schema or a descriptor with no call
into any library. The [documentation](https://boriken-dev.github.io/guatiao/)
has the tour and the API references.

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

A pre-release is only selected when it is named exactly, so the version is
spelled out:

```toml
[dependencies]
guatiao = "=0.0.0-alpha.1"
```

Moving from `0.0.0-alpha.0`? The
[migration page](https://boriken-dev.github.io/guatiao/migrating/) lists
every change since then that asks something of your code, and what to
write now.

Optional features:

| Flag | Adds | Needed for |
| --- | --- | --- |
| `derive` | `guatiao-derive` | `#[derive(ToValue, FromValue, Schema)]` |
| `provider` | `derive`, with `syn`'s full parser | `#[guatiao::kind]`, `#[derive(Provider)]`, `guatiao::providers!`: a kind crate and every library need it |
| `load` | `libloading`, `object` | the `Registry` and loading libraries from disk into it (`load_file`, `scan_dir`, `scan_path`): a host needs it |
| `regex` | `regex` | enforcing a schema's `pattern` when validating |
| `c-header` | nothing | regenerating `include/guatiao.h` (it is committed, so most builds do not) |

A default build pulls in nothing.

Bindings over the same C ABI, for a consumer that is not Rust:

| Language | Package | Install |
| --- | --- | --- |
| Python | [`bindings/python/`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/python/README.md) | `pip install guatiao` |
| Dart | [`bindings/dart/`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/dart/README.md) | `dart pub add guatiao` |
| Node.js, TypeScript | [`bindings/node/`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/node/README.md) | `npm install guatiao` |
| Go | [`bindings/go/`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/go/README.md) | `go get github.com/Boriken-Dev/guatiao/bindings/go` |
| Julia | [`bindings/julia/`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/julia/README.md) | `Pkg.add(url = "https://github.com/Boriken-Dev/guatiao", subdir = "bindings/julia")` |
| R | [`bindings/r/`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/r/README.md) | `remotes::install_github("Boriken-Dev/guatiao", subdir = "bindings/r")` |
| Fortran | [`bindings/fortran/`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/fortran/README.md) | fpm: `guatiao = { git = "https://github.com/Boriken-Dev/guatiao", tag = "..." }`; CMake: `FetchContent` with `SOURCE_SUBDIR bindings/fortran` |
| MATLAB, GNU Octave | [`bindings/matlab/`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/matlab/README.md) | Octave: `pkg install` a release's `guatiao-<version>.tar.gz`; MATLAB: a release's `guatiao.mltbx`, then `guatiao.build()` |

Each drives a shared library that exports the ABI — `guatiao` itself, or
an application that carries it — named by `GUATIAO_LIBRARY`.

JSON, TOML and YAML, and forms, are a package of their own beside each
core one, over the `guatiao_serde` and `guatiao_intake` libraries: PyPI
`guatiao-serde` and `guatiao-intake`, pub.dev `guatiao_serde` and
`guatiao_intake`, npm `guatiao-serde` and `guatiao-intake`, the Go packages
`.../bindings/go/serde` and `.../bindings/go/intake`, the Julia packages
`GuatiaoSerde` and `GuatiaoIntake` (`bindings/julia-serde`,
`bindings/julia-intake`), the R packages `guatiao.serde` and
`guatiao.intake` (`bindings/r-serde`, `bindings/r-intake`), and in MATLAB,
Octave and Fortran the sub-packages `guatiao.serde` and `guatiao.intake` and
the modules `guatiao_serde` and `guatiao_intake` of the one package. Each
finds its library through `GUATIAO_SERDE_LIBRARY` or
`GUATIAO_INTAKE_LIBRARY`, then `GUATIAO_LIBRARY` (a directory, or a file
exporting its functions, else the file beside it).

## Quick start

Build a value, then read it back. Building in Rust names no allocator —
the crate's own is the default, and a library building into a host's arena
says so with the `_in` constructors. Get the value and convert it: there
are no per-kind getters on a map.

```rust
use guatiao::Map;

let mut options = Map::new();
options.set("compression", 6);

let mut map = Map::new();
map.set("host", "10.0.0.1");
map.set("port", 5900);
map.set("options", options);

let host: &str = map.required("host")?.try_into()?;
let port: u16 = map.required("port")?.try_into()?;
assert_eq!((host, port), ("10.0.0.1", 5900));
# Ok::<(), Box<dyn core::error::Error>>(())
```

The same tree is what a C, C++ or Dart consumer reads directly, through
the generated header in `crates/guatiao/include/`. The full quick start,
including the derives, is in
[that crate's README](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao/README.md).
Both snippets are compiled by a test, and checked against the text shown
here.

## API overview

| Crate | Purpose |
| --- | --- |
| [`guatiao`](https://crates.io/crates/guatiao) | the value model, the C type vocabulary, the schema, the library envelope and the loader; the `guatiao.h` header |
| [`guatiao-derive`](https://crates.io/crates/guatiao-derive) | `#[derive(ToValue, FromValue, Schema)]` and the provider derives, reached through `guatiao`'s features |
| [`guatiao-serde`](https://crates.io/crates/guatiao-serde) | a value in and out of JSON, TOML and YAML, and the same through a C surface |
| [`guatiao-intake`](https://crates.io/crates/guatiao-intake) | taking a value in from a person: the path grammar, flat key-to-text storage, and how a schema is shown as a form |

## Development

```bash
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all --check
```

The C header at `crates/guatiao/include/guatiao.h` is rendered by `build.rs`
and committed, so a consumer reads it out of the repository without a Rust
toolchain. A test compares the committed file against what the build just
rendered, so it cannot go stale. To refresh it:

```bash
GUATIAO_WRITE_HEADER=1 cargo build -p guatiao --features c-header
```

The remaining gates (Miri, the public-API snapshots, the bindings' tests) are
in [`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/AGENTS.md).

### Releasing

This project follows [Semantic Versioning](https://semver.org/) and keeps a
[`CHANGELOG.md`](https://github.com/Boriken-Dev/guatiao/blob/main/CHANGELOG.md).
Pushing a tag matching `v*` triggers the release workflow: test gate, a strict
docs build as a gate, packaging of the crates, the Python package and the Dart
package, the GitHub release, then publishing each. The release workflow never
deploys the docs site itself: for a final release its last job dispatches the
docs workflow at the tag, which owns every Pages deploy.

## License

**Mozilla Public License 2.0** — see
[`LICENSE`](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE). Every
file in this repository is Covered Software under that licence.

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

Contributions need a one-time
[contributor licence agreement](https://github.com/Boriken-Dev/guatiao/blob/main/CLA.md);
see
[`CONTRIBUTING.md`](https://github.com/Boriken-Dev/guatiao/blob/main/CONTRIBUTING.md).
