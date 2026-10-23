# guatiao for Lua

[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

`require "guatiao"` in a stock Lua interpreter: load libraries written in
any language with a guatiao binding, and call their providers through a
kind's description. The guatiao library is inside the module, so nothing
else needs to be found.

> **Status: alpha, not published.** Built from this repository, against
> the Lua that loads it.

## Building

```bash
luarocks make --local                                   # in this directory: Lua 5.4, 5.5 or LuaJIT, as LuaRocks runs
cargo build --release --no-default-features --features lua54   # or lua55, luajit
```

Cargo writes `libguatiao_lua_module.so` (`.dylib`, or
`guatiao_lua_module.dll`); `require` finds it as `guatiao.so`
(`guatiao.dll`) on `package.cpath`. On Windows LuaRocks' Rust backend
looks for another file name: build with cargo there and copy it. The
interpreter must be the Lua the module was built for; on Windows its
library is `lua54.dll` (`lua55.dll`, `lua51.dll`), as Lua's own builds
name it.

## Quick start

```lua
local guatiao = require "guatiao"

local kinds = guatiao.kinds("greeter_kind.kinds.json")
local registry = guatiao.registry("my-host", "1.0")
registry:load_file("libderived_greeter.so")
registry:refresh()

local greeter = registry:bind("derived_greeter_hello", kinds.greeter)
local answer = assert(greeter:greet("ana"))
print(answer.greeting)                       -- hello, ana
local nothing, err = greeter:greet("")
print(err.status, err.message)               -- bad_value  nobody to greet
```

[`examples/lua_host/host.lua`](https://github.com/Boriken-Dev/guatiao/blob/main/examples/lua_host/host.lua)
calls every slot of every greeter, written in Rust, C, Python or Lua.

## What it offers

- **A registry**: `load_file`, `add_source_file` (Python through
  guatiao-py, Lua through guatiao-lua, any source), `refresh`,
  `providers`, `bind`, `retire`, `collect`, `close`.
- **Kinds from their descriptions**: `guatiao.kinds(file)`; a bound
  provider's methods are its kind's slots.
- **Values that keep their shape**: `guatiao.null`, `guatiao.absent`,
  `guatiao.list`, `guatiao.map`, `guatiao.bytes`, `guatiao.number`
  (exact text), `guatiao.buffer`, and refusals as `guatiao.error`.

## Not offered

- Instances built from a configuration (`create`).
- Serving providers written in Lua from this module: that is the
  guatiao-lua source, which a host adds.
- A Lua table handed to a provider as an object is called from this
  interpreter's thread only; a provider calling it from another thread
  is refused.

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/lua/guatiao/AGENTS.md)
is the full reference.

## Development

```bash
cargo build --workspace --all-features                          # the Rust greeter, the guatiao library
cargo build --manifest-path bindings/lua/guatiao/Cargo.toml     # the module, which its tests copy as guatiao.so
GUATIAO_LIBRARY=target/debug GUATIAO_LUA=lua cargo test --manifest-path bindings/lua/guatiao/Cargo.toml
```

`GUATIAO_PY_BRIDGE` and `GUATIAO_LUA_SOURCE` add the Python and Lua
greeters through their sources.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
