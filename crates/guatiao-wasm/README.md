# guatiao-wasm

[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

A [guatiao](https://crates.io/crates/guatiao) **source of libraries
compiled to WebAssembly**: any guatiao host, in Rust, C, Python or any
language with a binding, adds this one file and calls providers written
in Rust, Go or any language that compiles to a WebAssembly module, as it
calls the ones written in Rust or C. wasmtime is built into the file; the
host needs nothing else.

> **Status: alpha, not published.** Built from this repository.

## Features

- **Sandboxed.** A library sees its own memory and what a call hands it:
  no files, no network, no environment, no host memory. A trap stops the
  call, not the host, and the library is made afresh for the next one.
- **Limits.** Operations (fuel), time and memory, from the source's
  configuration: runaway code is stopped and the host carries on.
- **Any language that emits a module.** A library speaks guatiao ABI 1:
  five exports, five imports, every value as its wire encoding. Rust has
  a guest crate (`guest/`), Go a guest package (`guest-go/`).
- **One file on every platform**, reloadable: a refresh serves a rebuilt
  module while the old build answers the instances taken from it.
- **Called on the caller's thread**: each library behind a lock of its
  own, so calls into different libraries run at once.

## Building

```bash
cargo build --release --manifest-path crates/guatiao-wasm/Cargo.toml
```

The output is `libguatiao_wasm.so`, `libguatiao_wasm.dylib` or
`guatiao_wasm.dll`. Building needs a C compiler (libffi, through
`guatiao-libffi`, for the kind tables).

## Quick start

A library in Rust (`examples/wasm_greeter_rust`), a `cdylib` built with
`cargo build --release --target wasm32-unknown-unknown`:

```rust,ignore
use guatiao_wasm_guest::{Answer, Call, Handler, Library, Map, Provider, Refusal};

struct Hello;

impl Handler for Hello {
    fn call(&mut self, call: &mut Call) -> Result<Answer, Refusal> {
        let name = call.text(0)?;
        if name.is_empty() {
            return Err("nobody to greet".into());
        }
        let mut answer = Map::new();
        answer.set("greeting", format!("hello, {name}").as_str());
        Ok(answer.into())
    }
}

fn library() -> Library {
    Library::new().provider(Provider::new("hello", Hello).serves("greeter", ["greet"]))
}

guatiao_wasm_guest::export!(library, version = env!("CARGO_PKG_VERSION"));
```

its manifest beside the module, `greeters.guatiao-wasm.toml`:

```toml
module = "greeters"      # greeters.wasm
kinds = ["greeter"]
```

and a host adds the source with where the manifests and the kinds are:

```rust,ignore
let mut config = Map::new();
config.set("dirs", List::from_iter(["/opt/myapp/wasm"]));
config.set("kinds", List::from_iter(["/opt/myapp/greeter_kind.kinds.json"]));
config.set("limits", limits);        // optional: {"operations": 10000000, "time_ms": 100, "memory": 67108864}
registry.add_source_file(Path::new("/opt/myapp/libguatiao_wasm.so"), &config)?;
let changed = registry.refresh();
```

The interface, the limits and every refusal are in [`AGENTS.md`](AGENTS.md);
writing guests is in [`guest/`](guest/README.md) and
[`guest-go/`](guest-go/README.md).

## Testing

```bash
rustup target add wasm32-unknown-unknown       # the Rust guest
cargo test --manifest-path crates/guatiao-wasm/Cargo.toml
```

The tests build the example guests (Rust, and Go 1.24 or later); a
missing toolchain skips with the reason, and `GUATIAO_REQUIRE_ALL=1` makes
it a failure. The C host's test needs `GUATIAO_LIBRARY` naming a `guatiao`
build with `load`.

## License

MPL-2.0.
