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
- **A configuration is a struct** — `Guatiao.schema(T)` writes the JSON
  Schema its fields describe, and `decode(T, value)` builds it back;
  `Guatiao.@config` is `@kwdef` that also puts defaults and docstrings in
  the schema.
- **Dates, times, spans and exact lengths** — `Dates.Date`,
  `Dates.Time` (to the nanosecond), `Dates.DateTime`, instants as
  `Timestamp`, `Span` and `Length`, read and written by the library itself.
- **Redaction** — a copy of a value with every field its schema marks
  secret hidden.
- **JSON, TOML and YAML**, and **form checks against a schema**, through
  the `guatiao-serde` and `guatiao-intake` libraries when they are present.
- **Nothing native until it is needed** — the library is looked for on
  first use, and a missing one names every place that was searched.

## Installation

```julia
using Pkg
Pkg.add(url = "https://github.com/Boriken-Dev/guatiao", subdir = "bindings/julia")
```

The package drives a shared library that exports the guatiao ABI. Say
where it is:

```bash
export GUATIAO_LIBRARY=/opt/myapp/lib          # a directory holding guatiao.dll / libguatiao.so / libguatiao.dylib
export GUATIAO_LIBRARY=/opt/myapp/myapp.dll    # or the file itself, whatever it is called
```

or call `Guatiao.use_library(path)` before the first call. Without
either, the platform's library search is tried by file name.

| Exports from | Needed for |
| --- | --- |
| `guatiao` | values, the registry, kinds and providers, schemas, time, redaction |
| `guatiao_serde` | `Serde`: JSON, and TOML and YAML when the library has them |
| `guatiao_intake` | `Intake` |

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

### JSON, TOML and YAML

```julia
doc = Serde.parse("""{"name": "ana", "n": 1.10}""")
Serde.emit(doc; format = :yaml)
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
- 64-bit platforms only. There is no Julia for Windows on ARM64; an x64
  Julia there needs an x64 build of the library.

## API overview

| Name | Purpose |
| --- | --- |
| `Value`, `ValueRef`, `MapRef`, `ListRef`, `NumberText`, `absent` | the value model |
| `decode`, `Guatiao.schema`, `Guatiao.@config`, `Guatiao.lower` | Julia types in and out, and their schemas |
| `Timestamp`, `Span`, `Length`, `Guatiao.fromtext`, `Guatiao.totext`, `Guatiao.normalize` | dates, times, spans, exact lengths |
| `redact` | secrets hidden |
| `Serde.parse`, `Serde.emit` | JSON, TOML, YAML |
| `Intake` | `check`, `layout`, `isvisible`, `at`, `for_schema`, `form_for`, `resolve`, `flat_keys`, `flatten`, `unflatten` |
| `GuatiaoError`, `ParseError`, `Unparsed`, `LibraryNotFound`, `MissingSymbol` | errors |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/julia/AGENTS.md)
is the full API reference and ships in the package. The C ABI is
described by
[`guatiao.h`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao/include/guatiao.h).

## Development

```bash
cd bindings/julia
julia --project=. -e 'using Pkg; Pkg.test()'
```

The tests need the three libraries in one directory; they point
`GUATIAO_LIBRARY` at the repository's build output when the variable is
unset, and the repository's own `AGENTS.md` says how to produce it.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
