# guatiao

**One contract for passing configuration and providers between languages.**
A value model whose C form is plain structs, a JSON Schema through which a
provider says what it needs to be initialised, and an envelope through which
one shared library offers any number of providers. Nothing is opaque: a C,
C++, Dart, Node, Python or Rust consumer reads a tree, a schema or a descriptor
without calling into any library.

*Guatiao* is the Taíno pact in which two people exchange names and become
kin. An ABI is the same agreement between two sides of a boundary.

## Installation

The crates are on crates.io. A pre-release is only selected when it is named
exactly, so the version is spelled out:

```toml
[dependencies]
guatiao = "=0.0.0-alpha.1"
```

Every dependency sits behind a default-off feature, so a default build
pulls in nothing.

| Feature | Adds |
| --- | --- |
| `derive` | `#[derive(ToValue, FromValue, Schema)]` |
| `provider` | `#[guatiao::kind]`, `#[derive(Provider)]`, `guatiao::providers!` |
| `load` | `Registry::load_file`, `scan_dir`: mapping a library and calling it |
| `regex` | enforcing a schema's `pattern` when validating |
| `c-header` | regenerating the committed `include/guatiao.h` |

A Python consumer installs the bindings and points them at a library that
exports the ABI:

```bash
pip install guatiao
export GUATIAO_LIBRARY=/path/to/the/library
```

A Dart consumer adds the package and points it at the same library:

```bash
dart pub add guatiao
export GUATIAO_LIBRARY=/path/to/the/library
```

A Node consumer, TypeScript or JavaScript, installs the package the same
way:

```bash
npm install guatiao
export GUATIAO_LIBRARY=/path/to/the/library
```

A C consumer includes `guatiao.h` and links nothing to read a value.

## 30-second tour

Build a value in Rust, and read it back by converting rather than by a
getter per kind:

```rust
use guatiao::Map;

let mut map = Map::new();
map.set("host", "10.0.0.1");
map.set("port", 5900);

let host: &str = map.required("host")?.try_into()?;
let port: u16 = map.required("port")?.try_into()?;
```

The same tree from Python, over ctypes:

```python
from guatiao import Value

with Value(host="10.0.0.1", port=5900) as v:
    v.as_map()["port"].to_python()     # 5900
```

Load the plugins in a directory (the `load` feature) and call one through the
table its kind declares:

```rust
use guatiao::library::{Registry, scan_dir};
use std::path::Path;

let mut registry = Registry::new("my-host", "1.0");
scan_dir(&mut registry, Path::new("/opt/myapp/plugins"))?;
let greeter = registry.best("greeter").expect("something serves it");
```

## Learn more

- [Passing data](ownership.md): who owns a tree, which allocator it is in,
  and how to pass one across a call or a library without copying it.
- [Rust API reference](https://boriken-dev.github.io/guatiao/rust/guatiao/index.html), built by rustdoc
  from the crates in this workspace.
- [Python, host and provider](python.md): calling libraries from Python,
  and writing one in Python that a host in any language loads.
- [Node.js, host and provider](node.md): calling libraries from Node,
  and writing providers in TypeScript that a host in the same process
  calls.
- [Python API reference](api/python.md), for the ctypes bindings.
- [Dart API reference](api/dart.md), for the `dart:ffi` bindings -- a plain
  Dart package, so a Flutter app uses the same one.
- [Changelog](changelog.md).
