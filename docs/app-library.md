# One library for your application

Each guatiao crate builds a C library of its own: `guatiao`, `guatiao_serde`
and `guatiao_intake`. An application that ships native code may want one file
instead, often a library it already builds for its own C surface. Linking the
crates into that library is enough: rustc exports a dependency's
`#[no_mangle]` functions from a `cdylib`, so the file carries guatiao's C
surfaces beside the application's own functions, and every binding can be
told to use it.

## The crate

A small crate of the application's, here called `myapp`
(`examples/app_library` in the repository is this crate, and the test suites
run against it):

```toml
[package]
name = "myapp"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib", "staticlib"]

[dependencies]
guatiao = "0.0.0-alpha.1"
guatiao-intake = "0.0.0-alpha.1"
guatiao-serde = { version = "0.0.0-alpha.1", features = ["exact-numbers", "toml", "yaml"] }

# The registry maps files, which a browser has none of.
[target.'cfg(not(target_family = "wasm"))'.dependencies]
guatiao = { version = "0.0.0-alpha.1", features = ["load"] }
```

```rust
use guatiao as _;
use guatiao_intake as _;
use guatiao_serde as _;

/// A function of the application's own, exported beside guatiao's.
#[unsafe(no_mangle)]
pub extern "C" fn myapp_version() -> u32 {
    1
}
```

Depend on the crates the application calls, with the features it needs
(here every format, and numbers read from JSON kept as their exact text):
a crate left out is a surface the file does not carry, and features cost
size (in a WebAssembly module, about 900 KB for TOML and YAML together,
1.2 MB for `guatiao`'s `regex`, which enforces a schema's `pattern`).
The `use ... as _` lines link each crate even where the application's own
code names nothing in it. Each surface is exported once, and the headers
(`guatiao.h`, `guatiao_serde.h`, `guatiao_intake.h`) describe the file as
they describe the crates' own libraries.

## Building it

One command per target, from the application's own checkout:

```bash
cargo build -p myapp --release                                        # myapp.dll, libmyapp.so, libmyapp.dylib
cargo build -p myapp --release --target aarch64-linux-android         # an Android ABI (with the NDK's linker)
cargo build -p myapp --release --target wasm32-unknown-unknown        # myapp.wasm, for a page
```

A 32-bit target builds the same way (`armv7-linux-androideabi`,
`armv7-unknown-linux-gnueabihf`): the layout is written in pointer widths.

## Using it from a binding

Every binding finds a surface's library in the same order: what it was told
in code, the surface's own variable (`GUATIAO_SERDE_LIBRARY`,
`GUATIAO_INTAKE_LIBRARY`), then `GUATIAO_LIBRARY`, then the platform's
search. Name the application's file once:

```bash
GUATIAO_LIBRARY=/opt/myapp/lib/libmyapp.so
```

The core surface takes a file `GUATIAO_LIBRARY` names as it is. The serde
and intake surfaces take it when it exports their functions, and otherwise
look for their own file beside it, so the same setting serves an
application file carrying all three and a core file with the others next to
it. In code, the setting is each binding's `use_library` in its own
spelling (Python `guatiao.use_library(path)`, Dart
`useLibrary(DynamicLibrary.open(path))`, Go `guatiao.UseLibrary(path)`, and
so on on each binding's page).

On the web there is one module for every surface, since each module has a
memory of its own: build `myapp.wasm` and load it once (see
[In a browser](wasm.md)).

## Traps

**A Rust program linking guatiao and the application's library.** On
Windows, a program that depends on the `guatiao` crate and links
`myapp.dll` through its import library gets `LNK2005`: the import library
defines every `guatiao_*` the crate already does. Link the DLL with
`raw-dylib`, importing only the application's own functions:

```rust
#[link(name = "myapp", kind = "raw-dylib")]
unsafe extern "C" {
    fn myapp_version() -> u32;
}
```

`examples/app_host` is such a program. On Linux and macOS a shared
library's symbols do not collide with a program's, and nothing is needed.

**Plugins beside it.** A plugin library linking guatiao itself exports
`guatiao_*` too. Loaded in one process with the application's file, both
keep working, whether the application's file is mapped `RTLD_LOCAL` or
`RTLD_GLOBAL`: a value records its own allocator, so a tree built by either
file is read and freed by the other.
