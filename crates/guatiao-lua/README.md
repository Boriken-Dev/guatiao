# guatiao-lua

[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

A [guatiao](https://crates.io/crates/guatiao) **source of libraries
written in Lua**: any guatiao host, in Rust, C, Python or any language
with a binding, adds this one file and calls providers written in Lua as
it calls the ones written in Rust or C. Lua is built into the file from
its C sources; the host needs nothing else.

> **Status: alpha, not published.** Built from this repository with the
> Lua it should run.

## Features

- **A library is a Lua module.** It returns a table; its `providers` are
  tables of methods named for the kinds' slots. The kinds come from their
  descriptions (`<crate>.kinds.json`), so nothing is declared twice.
- **Libraries come and go while the host runs.** A refresh opens a new
  manifest, runs an edited module afresh while instances of the old build
  still answer, and retires a deleted one. Nothing runs on its own.
- **Limits.** Operations, time, call depth and memory, from the source's
  configuration: runaway code is stopped and the host carries on, even
  code that catches the stop with `pcall`.
- **One Lua state on a thread of its own**, which every call reaches.

| feature | what it is | default |
| --- | --- | --- |
| `lua54` | Lua 5.4 | yes |
| `lua55` | Lua 5.5 | |
| `luau` | Luau, its sandbox on: libraries cannot change the standard ones | |
| `source` | the source: Lua built in, `guatiao_source` exported | yes |
| `module` | the Lua side for the loadable module (`bindings/lua/guatiao`), built against the interpreter that loads it | |

Exactly one Lua; `source` and `module` exclude each other.

## Building

```bash
cargo build --release --manifest-path crates/guatiao-lua/Cargo.toml
cargo build --release --manifest-path crates/guatiao-lua/Cargo.toml --no-default-features --features lua55,source
```

The output is `libguatiao_lua.so`, `libguatiao_lua.dylib` or
`guatiao_lua.dll`. Building needs a C compiler (Lua, and libffi for the
kind tables).

## Quick start

A library, `greeters.lua`:

```lua
local M = { _VERSION = "1.0.0" }

local Hello = { name = "Hello, in Lua", kinds = { "greeter" } }

function Hello:greet(name)
  if name == "" then return nil, "nobody to greet" end
  return { greeting = "hello, " .. name }
end

M.providers = { hello = Hello }      -- served as `greeters_hello`
return M
```

its manifest beside it, `greeters.guatiao-lua.toml`:

```toml
module = "greeters"
kinds = ["greeter"]
paths = ["."]
```

and a host adds the source with where the manifests and the kinds are:

```rust,ignore
let mut config = Map::new();
config.set("dirs", List::from_iter(["/opt/myapp/lua"]));
config.set("kinds", List::from_iter(["/opt/myapp/greeter_kind.kinds.json"]));
config.set("limits", limits);        // optional: {"operations": 1000000, "memory": 67108864}
registry.add_source_file(Path::new("/opt/myapp/libguatiao_lua.so"), &config)?;
let changed = registry.refresh();
```

[`examples/lua_provider`](https://github.com/Boriken-Dev/guatiao/tree/main/examples/lua_provider)
holds a greeter serving every slot of the example kinds.

## What Lua cannot do here

- **Build instances from a configuration.** A provider is one table; a
  kind's `create` is not served.
- **Be called from another thread while it waits on one.** Calls are
  queued to the Lua thread; a library that calls the host, which calls
  back into Lua from another thread, waits on itself.
- **Stop a library function mid-call.** A limit is checked between
  instructions: one long `table.sort` or `string.rep` runs to its end (the
  memory limit bounds what it can build).
- **Keep libraries apart in the globals.** Each library has its own
  environment and its own `require`, but a library can still change the
  standard libraries every library sees, except under Luau's sandbox.

## API overview

| Item | What it is |
| --- | --- |
| `guatiao_source` | the entry point a host's `add_source_file` calls; declares `source=lua` |
| configuration | `{"dirs": [..], "manifests": [..], "kinds": [..], "limits": {..}}` |
| `*.guatiao-lua.toml` | one Lua library: `module`, `kinds`, `paths`, optional `version` |
| `require "guatiao"` | in a library: `null`, `absent`, `list`, `map`, `bytes`, `number`, `buffer`, `error`, `status`, `kinds`, `kind` |
| `LuaEngine`, `LuaValues`, `helpers`, `module` | the Rust side: the engine, the value conversion, the `guatiao` tables |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao-lua/AGENTS.md)
is the full reference.

## Development

```bash
cargo build --workspace --all-features                       # the Rust greeter, the guatiao library
cargo build --manifest-path crates/guatiao-lua/Cargo.toml    # the source, which its tests add as a file
GUATIAO_LIBRARY=target/debug cargo test --manifest-path crates/guatiao-lua/Cargo.toml
```

`GUATIAO_PY_BRIDGE` naming a guatiao-py build adds the Python greeters to
the Lua host test. The crate is a workspace of its own: its Lua flavours
are exclusive features.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
