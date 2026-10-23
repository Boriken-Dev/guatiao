# `guatiao` for Lua — API header

Header-file-style reference for the Lua module `guatiao`: every function
with its arguments, answers and gotchas, so it can be used without
reading its source. It ships beside the module's sources and is
self-contained. Development documentation lives with the source at
<https://github.com/Boriken-Dev/guatiao>.

A loadable module (`luaopen_guatiao`), built with cargo against the Lua
that loads it: features `lua54` (default), `lua55`, `luajit`, exactly
one. The guatiao library is linked in: no `GUATIAO_LIBRARY` is read. The
rockspec `guatiao-dev-1.rockspec` builds it with `luarocks make`.

## The module table

| name | is |
| --- | --- |
| `registry(id, version)` | a registry, for a host named `id` |
| `kinds(source)` | loads kind descriptions: a file, or JSON text starting with `{`; answers every kind known, by name |
| `kind(name)` | one kind known: `.name`, `.object`, `.slots` (names, in table order) |
| `null` | guatiao's null: the NULL light userdata, the same value as `cjson.null` |
| `absent` | absent inside a table; `nil` stands for it elsewhere |
| `list(t)`, `map(t)` | `t`, or a new table, marked a list or a map |
| `bytes(s)` | `s` as bytes rather than text: `tostring`, `#` |
| `number(x)` | an exact number from text: a Lua number when one holds it exactly, else a `guatiao.number` (`tostring` gives the text; `+ - * /`, `<`, `<=` through its float; `:float()`) |
| `buffer(n)` | `n` zeroed bytes for a slot that writes into a buffer: `#b`, `b:write(s [, at])` answers the bytes written, `b:sub(i [, j])`, `tostring(b)` |
| `error(status, words)` | a refusal: `.status` (its name), `.code`, `.message`, `tostring` |
| `status` | every status by name: `ok` 0, `bad_value` 7, `alloc` 8, `wrong_kind` 9, `not_found` 10, `null` 11, `gone` 12, `unsupported` 13, `busy` 14, `internal` 100 |
| `_VERSION` | the module's version |

## A registry

```lua
reg:load_file(path)              --> {id, version, key, providers} | nil, why
reg:add_source_file(path, config) --> the source's name; raises on refusal
reg:refresh()                    --> {loaded = {...}, retired = {...}, skipped = {...}, refused = {...}}
reg:providers([kind])            --> {{key, id, name, library, version, kinds, configured}, ...}
reg:bind(key, kind)              --> a handle; raises when the provider has no table of the kind
reg:retire(key)                  --> true; raises on refusal
reg:collect()                    --> the keys of the libraries closed
reg:close()
```

- `refresh` entries: `loaded` and `retired` are `{source, id, stamp,
  key}`, `skipped` `{source, id, why}`, `refused` `{source, id, reason}`.
- `config` is a table, converted as a map (lists of paths as plain
  sequences).
- `kind` is a `guatiao.kind` or a kind's name.
- `configured` is true for a provider built from a configuration, which
  this module does not call.

## A handle

A bound provider, or an object a slot answered: its methods are its
kind's slots, `h:greet("ana")`.

- **Arguments**, by the slot's description: numbers (an integer argument
  takes an integral float or a `guatiao.number` too, refused when it does
  not fit), booleans, text, bytes (a string or `guatiao.bytes`), trees (a
  table; one lent for the call is changed in place), a buffer
  (`guatiao.buffer`, or a size), an object (a handle, handed over or
  lent, or a Lua table of the kind's methods).
- **Answers**: a slot that can fail answers `nil, guatiao.error` for a
  refusal, and `true` for success when it answers nothing; one that
  cannot fail raises. A slot the provider left out answers
  `nil, error("unsupported")`. An object answered is a handle, destroyed
  when collected.
- A handle handed over to a slot is spent: calling it raises `gone`.

## Values

| guatiao | Lua | back |
| --- | --- | --- |
| absent | `nil`; `guatiao.absent` in a table | either |
| null | `guatiao.null` | |
| bool | boolean | |
| number | an integer or float holding its text exactly (integers to 2^53 under LuaJIT); else `guatiao.number` | a Lua number as its text; NaN and the infinities refused |
| string | string | a string that is not UTF-8 goes as bytes |
| bytes | `guatiao.bytes` | |
| list | a table `1..n`, marked a list | a plain table keyed exactly `1..n` |
| map | a table, marked a map, its order kept | a plain table keyed by text (byte order), or empty |

Refused: a table with mixed or sparse keys, one containing itself, a
function, a coroutine, another userdata.

## Errors

A refusal crossing back is a `guatiao.error`. A Lua table served as an
object answers a call from another thread with `internal`. Wrong
arguments raise, naming the slot and the argument.

## Gotchas

- **One interpreter thread.** A Lua table handed to a provider as an
  object is called only while this thread is in a call and only from it:
  a provider that calls it from a thread of its own (a Lua library
  through the guatiao-lua source, which runs on its own thread) gets
  `internal`. Its table is kept for the process.
- **The module carries the guatiao library**, exporting its C functions
  too: a C library that links guatiao by name finds them here.
- **Build against the interpreter that loads it**: a module built for
  5.4 does not load into 5.5 or LuaJIT.
