# guatiao-libffi

[![Crate](https://img.shields.io/crates/v/guatiao-libffi.svg)](https://crates.io/crates/guatiao-libffi)
[![docs.rs](https://img.shields.io/docsrs/guatiao-libffi)](https://docs.rs/guatiao-libffi)
[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

libffi as a [guatiao](https://crates.io/crates/guatiao) mechanism. A
kind's table built while the program runs, from the kind's description,
needs a C function pointer for each slot's signature, and calling such a
table needs a call through one. Rust can do neither for a signature it
learns at run time; `Libffi` does both, with libffi's closures and
`ffi_call`. Everything else about those tables — reading the arguments,
writing the answer, laying out a call — is in `guatiao`.

What uses it: a bridge to a language with no FFI of its own (Lua,
JavaScript, Rhai, WebAssembly) serving the providers its libraries
declare, and anything that calls a kind it knows only by its description.

> **Status: alpha.** Released in lockstep with `guatiao`.

## Features

- **One type, `Libffi`**, guatiao's `Mechanism` over libffi: a slot's
  signature prepared once, a closure per served slot, `ffi_call` per call.
- **No features.** libffi is the whole crate.

## Installation

```toml
[dependencies]
guatiao-libffi = "=0.0.0-alpha.1"
```

libffi builds from its C source: MSVC's tools on Windows, `configure` and
`make` elsewhere. On a Windows MSVC target a `CC` naming a GNU-style
compiler (`clang`) breaks that build; leave `CC` unset for cargo. libffi
does not support `wasm32`.

## Usage

```rust,ignore
use std::sync::Arc;
use guatiao::library::kind::{Caller, ServedTable};
use guatiao_libffi::Libffi;

// A table whose slots call `serve`, from the kind's description.
let table = ServedTable::new(Arc::clone(&kind), |_| true, serve, &Libffi)?;
// Any table of the kind, called from its description.
let caller = Caller::new(kind, &Libffi)?;
```

A bridge hands it to its source, so the providers an engine declares are
served:

```rust,ignore
let source = BridgeSource::pinned(config, "lua")?.with_mechanism(Arc::new(Libffi));
```

## License

[MPL-2.0](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
