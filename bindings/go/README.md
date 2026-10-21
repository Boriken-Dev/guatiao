# guatiao for Go

[![Go Reference](https://pkg.go.dev/badge/github.com/Boriken-Dev/guatiao/bindings/go.svg)](https://pkg.go.dev/github.com/Boriken-Dev/guatiao/bindings/go)
[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![Docs](https://img.shields.io/badge/docs-latest-blue.svg)](https://boriken-dev.github.io/guatiao/go/)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

Go bindings for the [guatiao](https://github.com/Boriken-Dev/guatiao/)
ABI: **one value model for passing data between languages**, a JSON
Schema that describes a value, and a registry that loads plugin
libraries. cgo over the ABI's C header, loading the library at run time:
a build needs a C compiler and nothing else. It works with any shared
library that exports the ABI, whether that is `guatiao` itself or an
application that carries it.

> **Status: alpha.** The module carries no native library of its own:
> you point it at one.

## Features

- **Value trees as Go data** — build, read and change a tree in place,
  or convert it to and from Go values the way `encoding/json` does:
  structs by their tags, maps, slices, `time.Time`, `time.Duration`.
- **Numbers keep their exact text** — `1.10` stays `1.10`, and an integer
  of any size survives as a `guatiao.Number`.
- **A configuration is a struct** — `guatiao.SchemaOf[T]()` writes the
  JSON Schema its fields and tags describe.
- **Dates, times, spans and exact lengths** — `civil.Date`, `civil.Time`,
  `civil.DateTime` and `civil.Span` to the nanosecond, instants as
  `time.Time`, read and written by the library itself.
- **Redaction** — a copy of a value with every field its schema marks
  secret hidden.
- **JSON, TOML and YAML**, and **form checks against a schema**, through
  the `guatiao-serde` and `guatiao-intake` libraries when they are present.
- **Nothing native until it is needed** — the library is looked for on
  first use, and a missing one names every place that was searched.

## Installation

```bash
go get github.com/Boriken-Dev/guatiao/bindings/go
```

cgo must be on (`CGO_ENABLED=1`, the default where a C compiler is
found): GCC or clang, and on Windows a MinGW-style compiler such as
[llvm-mingw](https://github.com/mstorsjo/llvm-mingw) or MinGW-w64 GCC.
The module drives a shared library that exports the guatiao ABI. Say
where it is:

```bash
export GUATIAO_LIBRARY=/opt/myapp/lib          # a directory holding guatiao.dll / libguatiao.so / libguatiao.dylib
export GUATIAO_LIBRARY=/opt/myapp/myapp.dll    # or the file itself, whatever it is called
```

or call `guatiao.UseLibrary(path)` before the first call. Without either,
the platform's library search is tried by file name.

| Exports from | Needed for |
| --- | --- |
| `guatiao` | values, schemas, time, redaction |
| `guatiao_serde` | `serde`: JSON, and TOML and YAML when the library has them |
| `guatiao_intake` | `intake` |

Go 1.26 or later, on a 64-bit platform.

## Quick start

### Values

```go
v, err := guatiao.NewValue(map[string]any{"host": "10.0.0.1", "port": 5900, "tags": []string{"a", "b"}})
if err != nil {
	return err
}
defer v.Close()

m, _ := v.Map()
m.Set("port", 5901)
tags, _ := v.Key("tags").List()
tags.Push("c")

v.Key("port").IntOr(22) // 5901
m.Keys()                // [host port tags], insertion order

var config struct {
	Host string   `guatiao:"host"`
	Port uint16   `guatiao:"port"`
	Tags []string `guatiao:"tags"`
}
err = v.Decode(&config)
```

A `*Value` owns its tree and frees it on `Close`. `v.Key("tags")` is a
`Ref`: a path into the tree that is walked again on every access, so it
stays valid while the tree grows around it.

**Numbers keep their exact text**:

```go
n, _ := guatiao.NewValue(guatiao.Number("1.10"))
n.NumberText() // "1.10"
n.FloatOr(0)   // 1.1
```

### A configuration's schema

```go
type Listen struct {
	Host string `guatiao:"host" schema:"minLength=1"`
	Port uint16 `guatiao:"port" schema:"default=5900"`
}

schema, err := guatiao.SchemaOf[Listen]()
defer schema.Close()
```

### JSON, TOML and YAML

```go
doc, err := serde.Parse(`{"name": "ana", "n": 1.10}`, serde.JSON)
defer doc.Close()
text, err := serde.Emit(doc, serde.YAML)
```

### Dates and spans

```go
d, _ := civil.ParseDate("2026-01-31")
later, _ := d.Add(civil.Span{Months: 1})
later.String() // "2026-02-28"
```

## Known limits

- A Go map has no order, so one is written with its keys sorted; a tree
  keeps its own order, which `MapRef.Keys` and `All` read.
- `ListRef.Set` and `Insert` rebuild the list, because the C ABI only
  appends and removes.
- `time.Duration` holds about 292 years; `civil.Length` holds every exact
  length.
- 64-bit platforms only, and cgo is required.

## API overview

| Package | Purpose |
| --- | --- |
| `guatiao` | `Value`, `Ref`, `MapRef`, `ListRef`, `Number`, `NewValue`, `SchemaOf`, `Redact`, `UseLibrary`, the errors |
| `guatiao/serde` | `Parse` / `Emit` for JSON, TOML, YAML |
| `guatiao/intake` | `Check`, `Layout`, `IsVisible`, `At`, `ForSchema`, `FormFor`, `Resolve`, `FlatKeys`, `Flatten`, `Unflatten` |
| `guatiao/civil` | `Date`, `Time`, `DateTime`, `Span`, `Length`, instants, `Normalize` |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/go/AGENTS.md)
is the full API reference and ships in the module. The C ABI is described
by [`guatiao.h`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao/include/guatiao.h).

## Development

```bash
cd bindings/go
go vet ./...
go test ./...
```

The tests need the three libraries in one directory; they point
`GUATIAO_LIBRARY` at the repository's build output when the variable is
unset, and the repository's own `AGENTS.md` says how to produce it.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
