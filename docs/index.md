# guatiao

**One contract for passing configuration and providers between languages.**
A value model whose C form is plain structs, a JSON Schema through which a
provider says what it needs to be initialised, and an envelope through which
one shared library offers any number of providers. Nothing is opaque: a C,
C++, Dart, Fortran, Go, Julia, MATLAB, Node, Octave, Python, R or Rust consumer reads a tree, a schema or a descriptor
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

JSON, TOML and YAML, and forms, are a package of their own beside each
core one, over the `guatiao_serde` and `guatiao_intake` libraries: PyPI
`guatiao-serde` and `guatiao-intake`, pub.dev `guatiao_serde` and
`guatiao_intake`, npm `guatiao-serde` and `guatiao-intake`, the Go packages
`.../bindings/go/guatiao/serde` and `.../bindings/go/guatiao/intake`, the Julia packages
`GuatiaoSerde` and `GuatiaoIntake` (`bindings/julia/GuatiaoSerde`,
`bindings/julia/GuatiaoIntake`), the R packages `guatiao.serde` and
`guatiao.intake` (`bindings/r/guatiao.serde`, `bindings/r/guatiao.intake`), and in MATLAB,
Octave and Fortran the sub-packages `guatiao.serde` and `guatiao.intake` and
the modules `guatiao_serde` and `guatiao_intake` of the one package. Each
finds its library through `GUATIAO_SERDE_LIBRARY` or
`GUATIAO_INTAKE_LIBRARY`, then `GUATIAO_LIBRARY` (a directory, or a file
exporting its functions, else the file beside it).

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

A Go consumer adds the module, built with cgo, and points it at the same
library:

```bash
go get github.com/Boriken-Dev/guatiao/bindings/go/guatiao
export GUATIAO_LIBRARY=/path/to/the/library
```

A Julia consumer adds the package from its directory of the repository
and points it at the same library:

```bash
julia -e 'using Pkg; Pkg.add(url = "https://github.com/Boriken-Dev/guatiao", subdir = "bindings/julia/Guatiao")'
export GUATIAO_LIBRARY=/path/to/the/library
```

An R consumer installs the source package from its directory of the
repository (it compiles a small C glue, which needs libffi) and points it
at the same library:

```bash
Rscript -e 'remotes::install_github("Boriken-Dev/guatiao", subdir = "bindings/r/guatiao")'
export GUATIAO_LIBRARY=/path/to/the/library
```

A GNU Octave consumer installs the package from a release (it compiles a
small MEX file, which needs libffi); a MATLAB consumer installs the
toolbox and runs `guatiao.build()` once. Both point it at the same
library:

```matlab
pkg install guatiao-VERSION.tar.gz; pkg load guatiao   % Octave
setenv('GUATIAO_LIBRARY', '/path/to/the/library')
```

A Fortran consumer adds the package with fpm (a git dependency on the
repository, whose root manifest builds `bindings/fortran/guatiao`) or CMake, and
points it at the same library:

```toml
[dependencies]
guatiao = { git = "https://github.com/Boriken-Dev/guatiao", tag = "..." }
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
- [Go, host and provider](go.md): calling libraries from Go, and writing
  providers in Go, linked into a host or built as a library file any host
  loads.
- [Julia, host and provider](julia.md): calling libraries from Julia, and
  writing providers in Julia that a host in the same process calls.
- [R, host and provider](r.md): calling libraries from R, and writing
  providers in R that a host in the same session calls on R's thread.
- [MATLAB and Octave, host and provider](matlab.md): calling libraries from
  MATLAB or GNU Octave, and writing providers as classes that a host in
  the same session calls on the interpreter's thread.
- [Fortran, host and provider](fortran.md): calling libraries from
  Fortran, and writing providers in Fortran, linked into a host or built
  as a library file any host loads.
- [Python API reference](api/python.md), for the ctypes bindings.
- [Dart API reference](api/dart.md), for the `dart:ffi` bindings -- a plain
  Dart package, so a Flutter app uses the same one.
- [Changelog](changelog.md).
