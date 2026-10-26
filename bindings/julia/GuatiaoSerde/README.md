# GuatiaoSerde

[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

JSON, TOML and YAML for [guatiao](https://github.com/Boriken-Dev/guatiao/)
values: read a document into a `Guatiao.Value`, write one back out. Pure
Julia over `ccall`, calling the `guatiao_serde` library, and built on the
`Guatiao` package for the value model and its errors.

> **Status: alpha.** No native library of its own; you point it at one.

## Features

- **Three formats, one call each** — `parse` and `emit` for JSON, TOML
  and YAML, parsed and written by the library itself, so every language
  bound to guatiao reads a document the same way.
- **Numbers keep their exact text** — `1.10` stays `1.10`, and an integer
  of any size survives the trip.
- **A document that does not parse says where** — `ParseError` carries
  the format, line and column, and never quotes the document.
- **Any guatiao value** — what `parse` returns is an ordinary
  `Guatiao.Value`, and `emit` takes any `Value`, view or plain data.
- **Or plain Julia, with no value in between** — `parse(Any, text)`
  answers dicts and vectors, and `emit` writes them without building one.

## Installation

```julia
using Pkg
Pkg.add(url = "https://github.com/Boriken-Dev/guatiao", subdir = "bindings/julia/Guatiao")
Pkg.add(url = "https://github.com/Boriken-Dev/guatiao", subdir = "bindings/julia/GuatiaoSerde")
```

The package drives the `guatiao_serde` shared library. Say where it is:

```bash
export GUATIAO_SERDE_LIBRARY=/opt/myapp/lib          # a directory holding guatiao_serde.dll / libguatiao_serde.so / .dylib
export GUATIAO_SERDE_LIBRARY=/opt/myapp/myapp.dll    # or the file itself, whatever it is called
```

Without it, `GUATIAO_LIBRARY` is tried: a directory holding the library,
or a file that exports its functions (an application's own library
carrying several guatiao surfaces), or a file it sits beside. Then the
platform's search. `GuatiaoSerde.use_library(path)` names it from code instead.
Nothing is loaded at `using`; the first call throws
`Guatiao.LibraryNotFound`, naming every place it looked.

Julia 1.10 or later.

## Quick start

```julia
using Guatiao, GuatiaoSerde

doc = GuatiaoSerde.parse("""{"name": "ana", "n": 1.10}""")   # format = :json
GuatiaoSerde.emit(doc; format = :yaml)
GuatiaoSerde.emit(Dict("port" => 5900); pretty = true)
close(doc)

try
    GuatiaoSerde.parse("{")
catch e
    e isa GuatiaoSerde.ParseError && println(e.format, " ", e.line, ":", e.column)
end
```

## Known limits

- Reading JSON writes an exponent's sign: `1e400` parses as `1e+400`.
- 64-bit platforms only, and a library built for the Julia process's
  architecture.

## API overview

| Name | Purpose |
| --- | --- |
| `GuatiaoSerde.parse(text; format, how, alloc)` | a document as a `Guatiao.Value` |
| `GuatiaoSerde.parse(Any, text; format, how)` | a document as plain Julia data, no `Value` built |
| `GuatiaoSerde.emit(x; format, pretty, how, alloc)` | a `Value`, view or plain data as text |
| `ParseError` | a document that does not parse: `format`, `line`, `column`, `message` |
| `BYTES_DATA_URI`, `BYTES_BASE64`, `BYTES_ARRAY`, `BYTES_REFUSE`, `READ_DATA_URIS`, `PRETTY` | the `how` flags |
| `GuatiaoSerde.use_library(path)` | naming `guatiao_serde` |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/julia/GuatiaoSerde/AGENTS.md)
is the full API reference and ships in the package. The C ABI is
described by
[`guatiao_serde.h`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao-serde/include/guatiao_serde.h).

## Development

```bash
julia -e 'using Pkg; Pkg.activate(temp = true);
          Pkg.develop([PackageSpec(path = "bindings/julia/Guatiao"), PackageSpec(path = "bindings/julia/GuatiaoSerde")]);
          Pkg.test("GuatiaoSerde")'
```

From the repository root. The tests point `GUATIAO_LIBRARY` at the
repository's build output when the variable is unset; the repository's
own `AGENTS.md` says how to produce it.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
