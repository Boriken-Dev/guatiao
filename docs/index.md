# guatiao

**One contract for passing configuration and providers between languages.**
A value model whose C form is plain structs, a JSON Schema through which a
provider says what it needs to be initialised, and an envelope through which
one shared library offers any number of providers. Nothing is opaque: a C,
C++, Dart, Python or Rust consumer reads a tree, a schema or a descriptor
without calling into any library.

*Guatiao* is the Taíno pact in which two people exchange names and become
kin. An ABI is the same agreement between two sides of a boundary.

## Installation

The crate is not on a registry yet, so a Rust consumer takes it from git:

```toml
[dependencies]
guatiao = { git = "https://github.com/Boriken-Dev/guatiao" }
```

Every dependency sits behind a default-off feature, so a default build
pulls in nothing.

| Feature | Adds |
| --- | --- |
| `derive` | `#[derive(ToValue, FromValue, Schema)]` |
| `provider` | `#[guatiao::kind]`, `#[derive(Provider)]`, `guatiao::providers!` |
| `load` | `Registry::load_file`, `scan_dir`: mapping a library and calling it |
| `c-header` | regenerating the committed `include/guatiao.h` |

A Python consumer installs the bindings and points them at a library that
exports the ABI:

```bash
pip install guatiao
export GUATIAO_LIBRARY=/path/to/the/library
```

A C consumer includes `guatiao.h` and links nothing to read a value.

## 30-second tour

Build a value in Rust, and read it back by converting rather than by a
getter per kind:

```rust
use guatiao::{Map, ReadValue};

let mut map = Map::new();
map.set("host", "10.0.0.1")?;
map.set("port", 5900)?;

let host: &str = map.required("host")?.try_into()?;
let port: u16 = map.required("port")?.try_into()?;
```

The same tree from Python, over ctypes:

```python
from guatiao import Value

with Value(host="10.0.0.1", port=5900) as v:
    v.as_map()["port"].to_python()     # 5900
```

Load the plugins in a directory and call one through the table its kind
declares:

```rust
let mut registry = guatiao::library::Registry::new("my-host", "1.0");
registry.scan_dir("/opt/myapp/plugins")?;
let greeter = registry.best("greeter").expect("something serves it");
```

## Learn more

- [Rust API reference](https://boriken-dev.github.io/guatiao/rust/guatiao/index.html), built by rustdoc
  from the crates in this workspace.
- [Python API reference](api/python.md), for the ctypes bindings.
- [Changelog](changelog.md).
