# guatiao-bridge

[![Crate](https://img.shields.io/crates/v/guatiao-bridge.svg)](https://crates.io/crates/guatiao-bridge)
[![docs.rs](https://img.shields.io/docsrs/guatiao-bridge)](https://docs.rs/guatiao-bridge)
[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

What every [guatiao](https://crates.io/crates/guatiao) bridge to a
language runtime shares. A **bridge** makes libraries written in another
language — Python, Lua, JavaScript, Rhai, WebAssembly — providers that
any guatiao host loads, in any language: a source file the host adds,
whose libraries are that language's modules. This crate is the machinery
around the runtime, so a bridge for a new language is its engine and its
value conversion, and nothing else.

> **Status: alpha.** Released in lockstep with `guatiao`.

## Features

- **An engine trait** — start from the host's configuration, open the
  library a manifest describes, answer calls into it, close it — with a
  **thread model**: called from any thread, behind the engine's own lock,
  or pinned to one engine thread the bridge starts and queues calls to.
- **Manifests and stamps** — a library is described by a small TOML file
  read without running any code; its stamp follows the manifest and the
  newest code file, so a refresh serves an edit and an old instance keeps
  working on the old build.
- **The source, done** — `BridgeSource` is guatiao's `Source`; a language
  crate exports it with `guatiao::source!` and any host adds the file.
- **Kinds from their descriptions** — a runtime with no FFI declares its
  providers and the slots they implement; the kind tables are built at run
  time from the kinds' descriptions (`<crate>.kinds.json`), every slot a
  call into the engine, objects included (`dynamic-kinds`).
- **Limits and refusals** — operations, memory, time and depth from the
  configuration, and one mapping from every refusal to a status.
- **Conversion, checked** — a trait for an engine's values both ways, and
  the shared vectors every language's implementation is checked against.

## Installation

```toml
[dependencies]
guatiao-bridge = { version = "=0.0.0-alpha.1", features = ["dynamic-kinds"] }
```

| Flag | Adds | Needed for |
| --- | --- | --- |
| `dynamic-kinds` | `guatiao/dynamic-kinds` (libffi, built from C) | serving providers an engine declares, through tables built from their kinds' descriptions |

A runtime that builds its own kind tables (CPython does, through ctypes)
needs no flag.

## Usage

```rust,ignore
use guatiao::library::{Host, ProviderError};
use guatiao::Map;
use guatiao_bridge::{BridgeError, BridgeSource, Call, Config, Engine, Manifest,
                     ManifestFormat, Opened, Provided, Reply, ThreadModel};

struct Lua { /* the runtime */ }

impl Engine for Lua {
    type Library = u64;
    const THREADS: ThreadModel = ThreadModel::Pinned;
    const MANIFESTS: ManifestFormat = ManifestFormat {
        suffix: ".guatiao-lua.toml",
        id_key: "module",
        code_extensions: &["lua"],
        skip_dirs: &[],
    };
    fn start(config: &Config) -> Result<Lua, BridgeError> { /* .. */ }
    fn open(&self, m: &Manifest, version: &str) -> Result<Opened<u64>, BridgeError> {
        // run the module, then:
        Ok(Opened::Described {
            library: 1,
            providers: vec![Provided::new("hello").serves("greeter", ["greet"])],
        })
    }
    fn call(&self, library: &u64, call: &mut Call) -> Result<Reply, ProviderError> { /* .. */ }
    fn close(&self, library: u64) { /* .. */ }
}

fn start(_host: Host, config: &Map) -> Result<BridgeSource<Lua>, ProviderError> {
    Ok(BridgeSource::pinned(config, "lua")?)
}
guatiao::source!(serves = "lua", make = start);
```

A host adds the file and refreshes:

```rust,ignore
registry.add_source_file(Path::new("guatiao_lua.dll"), &config)?;
let changed = registry.refresh();
```

## License

[MPL-2.0](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
