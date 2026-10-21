# guatiao-py

[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

A [guatiao](https://crates.io/crates/guatiao) **source of libraries
written in Python**: any guatiao host, in Rust, C, Python or Dart, adds
this one file and calls providers written in Python as it calls the ones
written in Rust or C.

> **Status: alpha, not published.** Built from this repository against
> the Python it will run with.

## Features

- **One module, two ways in.** A Python library is a module written with
  the Python binding's `guatiao.provide`: a Python host registers it
  directly, and this bridge serves the same module to a host in any
  language.
- **Libraries come and go while the host runs.** The host calls
  `refresh()`: a new manifest is opened, an edited module is imported
  afresh while instances of the old build still answer, a deleted
  manifest retires its library. Nothing runs on its own: no thread, no
  watcher.
- **Checked before served.** Listing reads manifests and imports nothing;
  opening refuses a module whose name, version or kinds differ from its
  manifest, or whose top-level package is already imported from
  elsewhere, and says which.
- **One interpreter for the program**, started when the source is added,
  or the host's own when the host is Python, and never finalised.

## Building

```bash
PYO3_PYTHON=python3 cargo build --release --manifest-path crates/guatiao-py/Cargo.toml
```

The output is `libguatiao_py.so`, `libguatiao_py.dylib` or
`guatiao_py.dll`. It builds for CPython's stable ABI from 3.9, which
decides where one build runs:

| OS | the build links | so one build serves |
| --- | --- | --- |
| Windows | `python3.dll`, nothing versioned | every CPython from 3.9 on, found on the DLL search path |
| Linux | `libpython3.X.so` of the Python it was built with | that minor version |
| macOS | that Python's framework or `libpython3.X.dylib` | that minor version |

On Windows a host that is not Python finds `python3.dll` through `PATH`:
put the directory holding it (`sys.base_prefix`) there.

## Quick start

A manifest beside each library, `py_greeter.guatiao-py.toml`:

```toml
module = "py_greeter"               # the library: its id, imported by this name
kinds = ["greeter", "counter"]      # checked against what the module serves
paths = ["."]                       # put on sys.path, relative to this file
```

and a host adds the bridge with where the manifests are:

```rust,ignore
let mut config = Map::new();
config.set("dirs", List::from_iter(["/opt/myapp/python"]));
let source = registry.add_source_file(Path::new("/opt/myapp/libguatiao_py.so"), &config)?;
let changed = registry.refresh();     // what loaded, retired or was refused
```

From C it is `guatiao_registry_add_source_file` and
`guatiao_registry_refresh`; from Python `Registry.add_source_file` and
`refresh`. [`examples/python_provider`](https://github.com/Boriken-Dev/guatiao/tree/main/examples/python_provider)
holds two libraries and the commands that serve them to the example hosts.

## What Python cannot do here

- **Keep modules apart.** Every library imports into one interpreter: two
  libraries needing different versions of one package need separate
  processes.
- **Reload a compiled extension.** Closing drops a library's modules, so a
  refresh imports edited `.py` files afresh; an extension module (`.so`,
  `.pyd`) stays loaded until the process exits.
- **Run calls in parallel.** Every call into Python takes the GIL.
- **Run a kind's default body.** A method the class leaves out is a null
  slot, for which a Rust caller runs the kind's default; a caller written
  in Python has no default to run and raises `Unsupported`.

## API overview

| Item | What it is |
| --- | --- |
| `guatiao_source` | the entry point a host's `add_source_file` calls; answers a `guatiao_source_info` |
| configuration | `{"dirs": [..], "manifests": [..], "paths": [..]}` |
| `*.guatiao-py.toml` | one Python library: `module`, `kinds`, `paths`, optional `version` |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao-py/AGENTS.md)
is the full reference.

## Development

```bash
cargo build --workspace --all-features                 # the guatiao library the tests' C host links
PYO3_PYTHON=python3 cargo build --manifest-path crates/guatiao-py/Cargo.toml
GUATIAO_LIBRARY=target/debug cargo test --manifest-path crates/guatiao-py/Cargo.toml
```

The crate is a workspace of its own, so no other build in the repository
needs a Python to link against. Its tests add the bridge from a Rust host
and a C host; the Python suite (`bindings/python/tests/test_bridge.py`)
adds it from Python and edits a library under a refresh.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
