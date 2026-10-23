# Lua: host and provider

Lua plays both parts. As a **provider** it writes a library, a Lua module
returning a table, that the `guatiao-lua` source serves to a host in any
language. As a **host** it loads libraries written in any language and
calls their providers, through the `guatiao` module a stock Lua
interpreter requires. Both read kinds from their descriptions
(`<crate>.kinds.json`, [Encodings](encodings.md)), so a kind is never
written out again in Lua.

The source is a [bridge to another language](language-bridges.md): Lua
is its engine, on a thread of its own, and its libraries come and go
with the host's `refresh()`.

## Building

Nothing is downloaded prebuilt: both are built from the repository with
cargo and a C compiler.

```bash
# The source: Lua 5.4 built in (or --no-default-features --features lua55,source / luau,source)
cargo build --release --manifest-path crates/guatiao-lua/Cargo.toml

# The module, for the Lua that will load it (lua54, lua55 or luajit)
cd bindings/lua/guatiao && luarocks make --local
cargo build --release --manifest-path bindings/lua/guatiao/Cargo.toml --no-default-features --features lua55
```

`luarocks make` picks the feature for the Lua LuaRocks runs for. Built
with cargo, the module is `libguatiao_lua_module.so`
(`guatiao_lua_module.dll`, `libguatiao_lua_module.dylib`): copy it as
`guatiao.so` (`guatiao.dll`) somewhere on `package.cpath`.

## As a provider

A library is a module returning a table. Its `providers` name its
providers: each a table of methods named for the slots of the kinds it
serves, called with `:` so `self` is the provider.

```lua
-- greeters.lua
local guatiao = require "guatiao"

local M = { _VERSION = "1.0.0" }

local Hello = { name = "Hello, in Lua", kinds = { "greeter", "counter" }, asked = 0 }

function Hello:greet(name)
  if name == "" then return nil, "nobody to greet" end
  return { greeting = "hello, " .. name }
end

function Hello:check(config)          -- a map lent for the call: changed in place
  config.name = config.name or "ana"
  return true
end

function Hello:start(listener)        -- an object handed over; answers one of `conversation`
  return setmetatable({ listener = listener, said = {} }, Chat)
end

function Hello:count()
  self.asked = self.asked + 1
  return self.asked
end

M.providers = { hello = Hello }       -- served as `greeters_hello`
return M
```

and its manifest beside it:

```toml
# greeters.guatiao-lua.toml
module = "greeters"
kinds = ["greeter", "counter"]
paths = ["."]
```

- **Identity.** The id is the manifest's `module`; the version its
  `version`, else the module's `_VERSION = "..."`, read as a literal
  without running it. A provider's id is `<module>_<key>`.
- **Slots.** A provider's functions under the slots' names, through its
  metatable, so a class serves as well as a table. A slot left out is
  null in the table, and a caller in Rust runs the kind's default; a
  required one left out refuses the library, naming it.
- **Refusals.** `return nil, "why"` is `GUATIAO_ERR_BAD_VALUE` in those
  words; `guatiao.error("not_found", "why")`, returned or raised, carries
  its status; any other error is `GUATIAO_ERR_INTERNAL` with its message.
- **Objects.** A slot answering an object returns a table of that kind's
  methods; an object handed in is a handle whose methods are its kind's
  slots (`listener:heard("one")`). A buffer to write into is a
  `guatiao.buffer`: `dst:write(text)` answers how many bytes fit.
- **Isolation.** Each library runs in an environment of its own, with its
  own `require` over its `paths`. An edit is run afresh at the next
  refresh, while the old build answers the instances taken from it.

A host adds the source with where the manifests and the kinds are, and
any limits:

```python
registry.add_source_file("libguatiao_lua.so", {
    "dirs": ["/opt/myapp/lua"],
    "kinds": ["/opt/myapp/greeter_kind.kinds.json"],
    "limits": {"operations": 1_000_000, "time_ms": 100, "memory": 64 << 20},
})
registry.refresh()
```

### Limits

| limit | how Lua keeps it |
| --- | --- |
| `operations` | an instruction count per call; past it the call is stopped at its next instruction, a `pcall` around the loop included |
| `time_ms` | checked at the same count |
| `depth` | the Lua call stack's levels |
| `memory` | the state's allocator, in bytes |

A stop is `GUATIAO_ERR_INTERNAL` (`GUATIAO_ERR_ALLOC` for memory) in words
naming the limit, `stopped at its operations limit (1000000)`, and the
next call runs. Under Luau (`luau` feature) the libraries cannot change
the standard ones, and operations count its interrupts.

## As a host

```lua
local guatiao = require "guatiao"

local kinds = guatiao.kinds("greeter_kind.kinds.json")
local registry = guatiao.registry("my-host", "1.0")
registry:load_file("libderived_greeter.so")
registry:add_source_file("libguatiao_lua.so", { dirs = { "lua" }, kinds = { "greeter_kind.kinds.json" } })
registry:refresh()

for _, p in ipairs(registry:providers("greeter")) do
  local greeter = registry:bind(p.key, kinds.greeter)
  local answer, err = greeter:greet("ana")        -- nil, guatiao.error for a refusal
  print(p.key, answer and answer.greeting or tostring(err))
  greeter:greet_into("bo", { heard = function(self, what) print("heard", what) end })
end
```

- A slot that can fail answers `nil, err` (`err.status`, `err.message`),
  one that cannot raises; a slot left out answers `unsupported`.
- An object argument is a handle, or a Lua table of the kind's methods.
- [`examples/lua_host/host.lua`](https://github.com/Boriken-Dev/guatiao/blob/main/examples/lua_host/host.lua)
  calls every slot of every greeter, in Rust, C, Python and Lua.

## Values

| guatiao | Lua |
| --- | --- |
| absent | `nil`; `guatiao.absent` inside a table |
| null | `guatiao.null` (the same light userdata as `cjson.null`) |
| bool | boolean |
| number | an integer or a float when one holds its text exactly; else a `guatiao.number` keeping the text (`tostring` gives it, arithmetic goes through its float) |
| text | a string; a string that is not UTF-8 goes back as bytes |
| bytes | a `guatiao.bytes` (`tostring`, `#`) |
| list | a table `1..n` (`guatiao.list{...}` marks one, so an empty one stays a list) |
| map | a table keyed by text, its order kept; a plain one goes in byte order |

Every value of the shared wire vectors comes back as it went.

## What Lua cannot do

- **Build provider instances from a configuration.** A provider is one
  table; a kind's `create` is not served.
- **Call a Lua table back from another thread.** A table handed to a slot
  as an object is called on its interpreter's thread only, which is
  waiting for the call: a provider that calls it from another thread,
  such as a Lua library served by the source, is refused.
- **Stop inside one library call.** Limits are checked between
  instructions: one long `table.sort` or `string.rep` runs to its end
  (the memory limit bounds what it builds).
- **Register a provider in its own process.** A Lua host loads libraries
  and sources; serving Lua takes the source.
- **Load the module into Luau**, which has no C modules, or **run the
  source on LuaJIT**, whose instruction count is not kept in compiled
  code.
