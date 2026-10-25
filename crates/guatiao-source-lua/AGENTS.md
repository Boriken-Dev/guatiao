# `guatiao-source-lua` — API header

Header-file-style reference for the `guatiao-source-lua` crate: a guatiao source
serving libraries written in Lua, and the Lua side of guatiao (values,
kinds, a registry) that the loadable module builds on. It ships inside
the package and is self-contained. Development documentation lives with
the source at <https://github.com/Boriken-Dev/guatiao>.

| feature | adds | default |
| --- | --- | --- |
| `lua54` | Lua 5.4 | yes |
| `lua55` | Lua 5.5 | |
| `luau` | Luau, `sandbox(true)` after the kinds and helpers are made | |
| `luajit` | LuaJIT, with `module` only | |
| `source` | Lua built from its C sources (mlua `vendored`), `guatiao_source` exported | yes |
| `module` | built against the interpreter that loads it (mlua `module`) | |

Exactly one Lua. `source` and `module` exclude each other, as do `source`
and `luajit` (its count hook is not called in compiled code) and `module`
and `luau`. MSRV 1.89. Depends on `guatiao` (`load`), `guatiao-bridge`,
`guatiao-libffi` and `mlua` 0.12. Not published; a
workspace of its own. `crate-type = ["cdylib", "rlib"]`.

The root re-exports everything public: `LuaEngine` (with `source`),
`LuaValues`, `helpers`, `module`, `RegistryView`, `Handle`, `KindView`,
`Refusal`, `BufferView`, `Exact`, `Bytes`, `Absent`, and `mlua`. The root
is `deny(unsafe_code)`; `exports.rs` (`guatiao::source!`) alone allows it.

## A library (`*.guatiao-lua.toml` and its module)

```toml
module = "pkg.greeters"           # required: found as require finds it; the library's id
kinds = ["greeter", "counter"]    # required: every kind its providers serve
paths = ["."]                     # where the module and its own modules are, relative to this file
version = "1.0.0"                 # optional
```

The module is `pkg/greeters.lua` or `pkg/greeters/init.lua` on `paths`.
Without `version`, its first `_VERSION = "..."` outside a comment is read
as a literal, without running it; none is a refusal asking for `version`.

```lua
local guatiao = require "guatiao"
local M = { _VERSION = "1.0.0" }

local Hello = { name = "Hello, in Lua", kinds = { "greeter", "counter" }, asked = 0 }
function Hello:greet(name)                    -- a slot: self is the provider
  if name == "" then return nil, "nobody to greet" end
  return { greeting = "hello, " .. name }
end
function Hello:count() self.asked = self.asked + 1; return self.asked end
function Hello:start(listener)                -- answers an object: a table of its kind's methods
  return setmetatable({ listener = listener, said = {} }, Chat)
end

M.providers = { hello = Hello }               -- served as `pkg_greeters_hello`
return M
```

- **Opening** runs the module in an environment of its own (reads fall
  through to the globals) with its own `require`: `"guatiao"` answers the
  helpers; any other name is searched on `paths` only, run in the same
  environment and kept in the library's own loaded table. Text chunks
  only. Opened afresh on every refresh that sees a new stamp.
- **Refused** (`GUATIAO_ERR_BAD_VALUE`, in words naming the module): not
  a table, no `providers` table, a provider with no `kinds`, a kind the
  manifest does not list, a module `_VERSION` other than the manifest's
  `version`, a required slot with no function (naming it), a kind with no
  description (`GUATIAO_ERR_NOT_FOUND`), and any error the module raises.
- **A provider** is any table: its slots are the functions it has under
  the slots' names, through its metatable, so a class works. Its id is
  `<module>_<key>` (dots made `_`); `name` is shown to a person (default
  the key). A slot it leaves out is null: a Rust caller runs the kind's
  default.
- **A slot is called** as `provider:slot(args...)`. Arguments: numbers,
  booleans, text as Lua's; `bytes` as a string; a tree as a table (a lent
  one changed in place and read back); `bytes_mut` as a `guatiao.buffer`;
  an object as a handle whose methods are its kind's slots (a lent one
  answers `gone` after the call).
- **Its answer**: the first value, as the slot's answer (`nil` for none);
  an object kind's answer is a table of that kind's methods, held until
  the caller destroys it, or a handle handed back. **A refusal** is
  `return nil, why` (text: `GUATIAO_ERR_BAD_VALUE`; a `guatiao.error`:
  its status) or `error(guatiao.error(status, words))`; any other raised
  error is `GUATIAO_ERR_INTERNAL` with its first line.

## `require "guatiao"` (`helpers`, `module`)

```rust
pub fn helpers(lua: &Lua) -> LuaResult<LuaTable>   // what a library gets
pub fn module(lua: &Lua) -> LuaResult<LuaTable>    // helpers + `registry`: what a Lua host gets
```

| name | is |
| --- | --- |
| `null` | guatiao's null: the NULL light userdata (`cjson.null` is the same) |
| `absent` | absent inside a table (`nil` at the top) |
| `list(t)`, `map(t)` | `t`, or a new table, marked a list or a map |
| `bytes(s)` | `s` as bytes (`guatiao.bytes`: `tostring`, `#`) |
| `number(x)` | an exact number from text (`guatiao.number` when no Lua number holds it), or `x` |
| `buffer(n)` | `n` zeroed bytes for a `bytes_mut` slot: `#b`, `b:write(s [, at])` (bytes written), `b:sub(i [, j])`, `tostring(b)` |
| `error(status, words)` | a refusal: `.status` (name), `.code`, `.message`, `tostring` |
| `status` | every status by name: `ok`, `bad_value`, `alloc`, `wrong_kind`, `not_found`, `null`, `gone`, `unsupported`, `busy`, `internal` |
| `kinds(source)` | loads kind descriptions (a file, or JSON text starting with `{`); answers every kind known, by name (`guatiao.kind`: `.name`, `.object`, `.slots`) |
| `kind(name)` | one kind known |
| `registry(id, version)` | `module` only: a `RegistryView` |
| `_VERSION` | this crate's version |

### `RegistryView` (a Lua host)

`reg:load_file(path)` → `{id, version, key, providers}` or `nil, why`;
`reg:add_source_file(path [, config])` → the source's name;
`reg:refresh()` → `{loaded, retired, skipped, refused}` (each a list of
`{source, id, stamp, key}`, `{source, id, why}`, `{source, id, reason}`);
`reg:providers([kind])` → `{key, id, name, library, version, kinds,
configured}`; `reg:bind(key, kind)` → a handle (`kind` a `guatiao.kind`
or its name); `reg:retire(key)`; `reg:collect()` → the keys closed;
`reg:close()`.

### A handle (`Handle`)

`h:slot(args...)` calls the slot: arguments as above (an object argument
is a handle, or a Lua table of the kind's methods, served while the Lua
thread is in a call; a `bytes_mut` one a `guatiao.buffer` or a size). A
slot that can fail answers `nil, guatiao.error` for a refusal, and `true`
for success when it answers nothing; one that cannot fail raises.
Calling a slot the provider left out answers `unsupported`.

## Values (`LuaValues`)

```rust
impl Convert for LuaValues { type Native<'a> = mlua::Value; type Error = mlua::Error; }
pub fn LuaValues::new(lua: &Lua) -> LuaValues
```

| guatiao | Lua | back |
| --- | --- | --- |
| absent | `nil` (top), `guatiao.absent` (in a table) | either |
| null | `guatiao.null` | |
| bool | boolean | |
| number | an integer when its text is one Lua holds exactly (64-bit in 5.4 and 5.5, 2^53 in Luau and LuaJIT); a float when the float's shortest text is the text; else `guatiao.number` (its exact text) | an integer or float as its text; NaN and the infinities refused |
| string | string | a string that is not UTF-8 goes as bytes |
| bytes | `guatiao.bytes` | |
| list | a table `1..n`, marked a list | a plain table whose keys are exactly `1..n` |
| map | a table, marked a map, its key order kept | a plain table keyed by text (byte order), or empty; keys added to a marked one go after, in byte order |

Refused: a table with mixed or sparse keys (naming why), one that contains
itself, a function, a coroutine, any other userdata, trees nested deeper
than 512. Every shared wire vector comes back the same.

## The engine (`LuaEngine`, with `source`)

`impl guatiao_bridge::Engine`: `ThreadModel::Pinned` (one `mlua::Lua`,
not `Send`, on the bridge's thread), manifests `*.guatiao-lua.toml` with
`id_key = "module"` and code extension `lua`, `Opened::Described`.

```rust
fn start(_host: Host, config: &Map) -> Result<BridgeSource<LuaEngine>, ProviderError> {
    Ok(BridgeSource::pinned(config, "guatiao-source-lua")?.with_mechanism(Arc::new(Libffi)))
}
guatiao::source!(serves = "lua", make = start);
```

## Limits

From the configuration's `limits` (the bridge's `Limits`), per call,
opening a library included:

| limit | kept by |
| --- | --- |
| `operations` | an instruction hook every `min(limit, 1000)` instructions; once past the limit, every instruction, so a `pcall` around the stopped code cannot keep a loop running. Luau: its interrupt, counted per interrupt |
| `time_ms` | the same hook |
| `depth` | the same hook: a frame at that level of the Lua stack (tail calls take none) |
| `memory` | mlua's allocator, for the whole state, in bytes |

A stop is `GUATIAO_ERR_INTERNAL` ("stopped at its operations limit
(200000)"), or `GUATIAO_ERR_ALLOC` for memory; the state answers the
next call.

## Errors

| Lua | status |
| --- | --- |
| `return nil, "why"` | `GUATIAO_ERR_BAD_VALUE` |
| `return nil, guatiao.error(s, w)` / `error(guatiao.error(s, w))` | `s` |
| any other error | `GUATIAO_ERR_INTERNAL`, its first line |
| Lua's own out-of-memory with a memory limit set | `GUATIAO_ERR_ALLOC` |
| a limit | as above |

## The C surface

```c
guatiao_status guatiao_source(const guatiao_host_info *host,
                              const guatiao_value *config,      /* a map, or null */
                              const guatiao_source_info **out,
                              guatiao_provider_error *err);
```

As `guatiao.h` declares any source's entry. The file declares
`source=lua`, so a scan reports it as a source and never maps it as a
library. A configuration that is not a map is `GUATIAO_ERR_WRONG_KIND`; a
key that is not a list of text, or a misspelled limit,
`GUATIAO_ERR_BAD_VALUE`.

## Gotchas

- **One state, shared globals.** Each library has its own environment,
  but `_G` and the standard libraries are shared: a library that changes
  `string` changes it for every library (not under Luau's sandbox).
- **A Lua table handed to a slot as an object is called on the Lua
  thread only**; a call from another thread answers
  `GUATIAO_ERR_INTERNAL`. Its table is kept for the process (one per kind
  and set of methods), since an object may be destroyed after the thread
  ends.
- **A limit is checked between instructions**: one long library call runs
  to its end.
- **Luau on Windows ARM64**: a release build crashes creating the state
  when Luau's C++ is compiled at `/O2`; build with `CXXFLAGS=/O1`. The
  variable alone does not rebuild Luau: `cargo clean --release -p
  mlua-sys` first.
- **`cargo test` does not build the file**: `cargo build` first, or the
  tests adding it by file state a skip.
