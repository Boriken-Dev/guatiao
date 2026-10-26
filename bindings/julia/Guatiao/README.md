# guatiao for Julia

[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![Docs](https://img.shields.io/badge/docs-latest-blue.svg)](https://boriken-dev.github.io/guatiao/julia/)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

Julia bindings for the [guatiao](https://github.com/Boriken-Dev/guatiao/)
ABI: **one value model for passing data between languages**, a JSON
Schema that describes a value, and a registry that loads plugin
libraries. `ccall` over the ABI, with nothing to compile and no
dependency outside Julia's standard library. It works with any shared
library that exports the ABI, whether that is `guatiao` itself or an
application that carries it.

> **Status: alpha.** The package carries no native library of its own:
> you point it at one.

## Features

- **Value trees as Julia collections** — a map is an `AbstractDict`, a
  list an `AbstractVector`; build, read and change a tree in place, or
  convert it to and from Julia data: `Dict`s, vectors, `NamedTuple`s,
  structs.
- **Numbers keep their exact text** — `1.10` stays `1.10`, and an
  integer of any size survives (`NumberText`, `BigInt`).
- **Be a plugin host** — scan a directory, filter on what a library
  declares before loading it, list providers best first, build an
  instance from a configuration, call a provider through its table with
  Julia values in and out, and keep sources of libraries that come and go
  (`refresh!`).
- **A kind is an abstract type and its functions** — `@kind` declares
  one from its C header; its remote is a subtype, so code written against
  the kind takes a local provider or a remote alike.
- **Write a provider in Julia** — a struct with methods for the kind's
  functions, declared with `@provider`, in a library a host calls the way
  it calls one written in Rust, C, Python or Go; the package is the
  library, named and versioned by its `Project.toml`.
- **Hand a library a Julia value** — any value with the right methods
  becomes an object of a kind (a listener, a callback) that a library
  calls, or holds until it is done with it, from any of its threads.
- **A configuration is a struct** — `Guatiao.schema(T)` writes the JSON
  Schema its fields describe, and `decode(T, value)` builds it back;
  `Guatiao.@config` is `@kwdef` that also puts defaults and docstrings in
  the schema.
- **Dates, times, spans and exact lengths** — `Dates.Date`,
  `Dates.Time` (to the nanosecond), `Dates.DateTime`, instants as
  `Timestamp`, `Span` and `Length`, read and written by the library itself.
- **Values on the wire** — `to_wire(v)` and `from_wire(bytes)` carry a
  tree exactly as bytes, and `Guatiao.Wire` announces a schema on a
  channel before its values, with a `Receiver` that checks each one.
- **Redaction** — a copy of a value with every field its schema marks
  secret hidden.
- **JSON, TOML and YAML**, and **forms checked against a schema**, are
  the packages `GuatiaoSerde` and `GuatiaoIntake` beside this one, each
  over its own library and built on `Guatiao.FFI`.
- **Nothing native until it is needed** — the library is looked for on
  first use, and a missing one names every place that was searched.

## Installation

```julia
using Pkg
Pkg.add(url = "https://github.com/Boriken-Dev/guatiao", subdir = "bindings/julia/Guatiao")
```

The package drives a shared library that exports the guatiao ABI. Say
where it is:

```bash
export GUATIAO_LIBRARY=/opt/myapp/lib          # a directory holding guatiao.dll / libguatiao.so / libguatiao.dylib
export GUATIAO_LIBRARY=/opt/myapp/myapp.dll    # or the file itself, whatever it is called
```

or call `Guatiao.use_library(path)` before the first call. Without
either, the platform's library search is tried by file name.

Julia 1.10 or later, on a 64-bit platform, and a library built for the
same architecture as the Julia process.

## Quick start

### Values

```julia
using Guatiao

v = Value(Dict("host" => "10.0.0.1", "port" => 5900, "tags" => ["a", "b"]))
v["port"] = 5901
push!(v["tags"], "c")

v["port"][]                         # 5901
keys(MapRef(v))                     # ["host", "port", "tags"], insertion order
decode(Vector{String}, v["tags"])   # ["a", "b", "c"]
close(v)
```

A `Value` owns its tree and frees it on `close` (or when the collector
finds it). `v["tags"]` is a `ValueRef`: a path into the tree that is
walked again on every access, so it stays valid while the tree grows
around it. `Value(x) do v ... end` closes it for you.

**Numbers keep their exact text**:

```julia
n = Value(NumberText("1.10"))
decode(NumberText, n)   # NumberText("1.10")
decode(Float64, n)      # 1.1
```

### Hosting plugins

```julia
using Guatiao, GreeterKind   # the kind, declared with @kind

reg = Registry("my-host", "1.0")
Guatiao.scan_dir(reg, "/opt/plugins"; rules = "kind=greeter")
for p in Guatiao.providers(reg, "greeter")
    println(p["key"])
end

greeter = Guatiao.bind(reg, "acme_hello", Greeter)
answer = GreeterKind.greet(greeter, "ana")   # a Value: Dict("greeting" => "hello, ana")
close(answer)
close(reg)
```

### A kind, and a provider of it

A kind is declared once, from its C header:

```julia
@kind Greeter floor_hash = 2695856491 begin
    @required @fallible greet(name::String)::MapRef
    shout(name::String)::String
end
```

A provider is a struct with methods for its functions:

```julia
struct Hello <: Greeter end
GreeterKind.greet(::Hello, name::String) = isempty(name) ?
    throw(GuatiaoError(Guatiao.BadValue, "nobody to greet")) :
    Dict("greeting" => "hello, $name")
@provider Hello display_name = "Hello, in Julia"

Guatiao.register_library(reg, MyPackage)   # the package declaring it is the library
```

A host in this process, written in any language, calls `Hello` through
the same table it would call a Rust or C greeter through.

### A configuration's schema

```julia
Guatiao.@config struct Listen
    "Where to listen."
    host::String = "0.0.0.0"
    port::UInt16
end

schema = Guatiao.schema(Listen)            # a Value: the JSON Schema
config = decode(Listen, Value((port = 5900,)))
```

### Dates and spans

```julia
using Dates
Date(2026, 1, 31) + Span(months = 1)          # 2026-02-28
Guatiao.fromtext(Timestamp, "2026-10-21T19:15:00.5+05:30")   # Timestamp("2026-10-21T13:45:00.5Z")
```

## Known limits

- A `Dict` has no order, so converting a map to one loses it; a tree
  keeps its own order, which `keys(MapRef(v))` reads.
- `l[i] = x` and `insert!` on a `ListRef` rebuild the list, because the
  C ABI only appends and removes.
- `Dates.DateTime` counts milliseconds: a civil date and time read into
  one drops what is below a millisecond. `Dates.Time`, `Timestamp`,
  `Span` and `Length` keep every nanosecond.
- A `@kwdef` struct's defaults cannot be read back from it: `@config`
  records them for the schema.
- A remote cannot run a kind's default body for a slot a provider leaves
  out, as a Rust caller does: `Guatiao.has(r, :slot)` asks, and the
  call answers `Unsupported`.
- A process builds at most 64 libraries and 256 providers in Julia: a
  descriptor's own slots come from fixed pools of C function pointers.
- A Julia library is registered in a host in the same process; there is
  no library file a foreign host's scan loads, since that would need
  Julia embedded in it.
- A C call that may call back into Julia from another thread must be made
  in a GC-safe region, as every call of this package is; a `ccall` of
  your own that does so wraps itself (see `AGENTS.md`, "Threads").
- 64-bit platforms only. The struct mirrors follow the pointer width, but
  no Julia 1.10 or later is built for 32-bit ARM, and the library does not
  build for 32-bit x86. There is no Julia for Windows on ARM64; an x64
  Julia there needs an x64 build of the library.

## API overview

| Name | Purpose |
| --- | --- |
| `Value`, `ValueRef`, `MapRef`, `ListRef`, `NumberText`, `absent` | the value model |
| `decode`, `Guatiao.schema`, `Guatiao.@config`, `Guatiao.lower` | Julia types in and out, and their schemas |
| `Timestamp`, `Span`, `Length`, `Guatiao.fromtext`, `Guatiao.totext`, `Guatiao.normalize` | dates, times, spans, exact lengths |
| `redact` | secrets hidden |
| `from_wire`, `to_wire`, `Guatiao.Wire` | values as bytes; channel frames and a receiver checking each value against its channel's schema |
| `Guatiao.FFI` | for a package over another guatiao library |
| `Registry`, `Instance`, `Guatiao.bind`, `Guatiao.create`, `Guatiao.refresh!` | hosting plugins |
| `@kind`, `Lent`, `Taken`, `@required`, `@fallible` | a kind, declared from its C header |
| `@provider`, `Guatiao.Provider`, `Guatiao.Library`, `Guatiao.library_of`, `Guatiao.register_library` | providers and libraries written in Julia |
| `GuatiaoError`, `Unparsed`, `LibraryNotFound`, `MissingSymbol`, `FloorMismatch` | errors |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/julia/Guatiao/AGENTS.md)
is the full API reference and ships in the package. The C ABI is
described by
[`guatiao.h`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao/include/guatiao.h).

## Development

```bash
cd bindings/julia/Guatiao
julia --project=. -e 'using Pkg; Pkg.test()'
```

The tests need the library in one directory with the example plugins; they point
`GUATIAO_LIBRARY` at the repository's build output when the variable is
unset, and the repository's own `AGENTS.md` says how to produce it. The
examples (`examples/greeter_kind/julia`, `examples/julia_provider`,
`examples/julia_host`) hold the greeter kinds, a library of providers and
a host written in Julia; the tests call greeters written in Rust, C,
Python, Go and Julia both ways.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
