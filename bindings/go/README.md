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
- **Be a plugin host** — scan a directory, filter on what a library
  declares before loading it, list providers best first, build an
  instance from a configuration, call a provider through its table with
  Go values in and out.
- **A kind is a Go interface** — `guatiao-kind`, run by `go generate`,
  reads the kind's C header and the interfaces and writes the glue: a
  descriptor, a remote implementing the interfaces, and the cgo calling and
  serving each slot.
- **Write a provider in Go** — any type implementing a kind's interface,
  checked by the compiler, in a library a host calls the way it calls one
  written in Rust, C or Python; its configuration is a struct whose tags
  give the schema, and the package is the library, named and versioned by
  the build.
- **An ordinary library file** — built with `-buildmode=c-shared`, a Go
  library is a file any host's scan finds and loads.
- **Hand a library a Go value** — any value implementing an object kind
  (a listener, a callback) becomes an object a library calls, or holds
  until it is done with it, from any of its threads.
- **A configuration is a struct** — `guatiao.SchemaOf[T]()` writes the
  JSON Schema its fields and tags describe.
- **Dates, times, spans and exact lengths** — `civil.Date`, `civil.Time`,
  `civil.DateTime` and `civil.Span` to the nanosecond, instants as
  `time.Time`, read and written by the library itself.
- **Redaction** — a copy of a value with every field its schema marks
  secret hidden.
- **JSON, TOML and YAML**, and **form checks against a schema**: the
  `serde` and `intake` packages, each over its own library
  (`guatiao_serde`, `guatiao_intake`).
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

| Package | Library | Its own variable, before `GUATIAO_LIBRARY` |
| --- | --- | --- |
| `guatiao`, `civil` | `guatiao` | |
| `serde` | `guatiao_serde` | `GUATIAO_SERDE_LIBRARY` |
| `intake` | `guatiao_intake` | `GUATIAO_INTAKE_LIBRARY` |

A file `GUATIAO_LIBRARY` names serves every package whose functions it
exports (an application's own library carrying several surfaces), and
the others find theirs beside it. Each package has its own `UseLibrary`.

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

### Hosting plugins

```go
reg, err := guatiao.NewRegistry("my-host", "1.0")
defer reg.Close()
reg.ScanDir("/opt/plugins", guatiao.Rules("kind=greeter"))
providers, _ := reg.Providers("greeter")
for _, p := range providers {
	fmt.Println(p["key"])
}

greeter, err := greeterkind.GreeterKind.Bind(reg, "acme_hello")
answer, err := greeter.Greet("ana") // greeterkind.Greeting{Greeting: "hello, ana"}
```

### A kind, and a provider of it

A kind is an interface, its glue written from its C header:

```go
//go:generate go run github.com/Boriken-Dev/guatiao/bindings/go/cmd/guatiao-kind -header ../include/greeter_kind.h

type Greeting struct {
	Greeting string `guatiao:"greeting"`
}

//guatiao:kind greeter greeter_vtable
type Greeter interface {
	Greet(name string) (Greeting, error)
}
```

A provider is a type implementing it:

```go
type Hello struct{}

func (Hello) Greet(name string) (greeterkind.Greeting, error) {
	if name == "" {
		return greeterkind.Greeting{}, guatiao.Errorf(guatiao.BadValue, "nobody to greet")
	}
	return greeterkind.Greeting{Greeting: "hello, " + name}, nil
}

lib, err := guatiao.LibraryOf(guatiao.Single("hello", Hello{}, greeterkind.GreeterKind))
reg.RegisterLibrary(lib)
```

A host in this process, written in any language, calls `Hello` through
the same table it would call a Rust or C greeter through; built with
`-buildmode=c-shared` and exporting `guatiao_library_entry`, the library
is a file any host loads.

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
- A kind's glue is written by `go generate` from its C header: Go cannot
  make a C function pointer at run time, so a kind is not described while
  the program runs, as the Python and Node bindings allow.
- A remote cannot run a kind's default body for a slot an older table
  lacks, as a Rust caller does: it answers `ErrUnsupported`.
- A process builds at most 64 libraries and 256 providers in Go: a
  library's and a provider's descriptor slots come from fixed pools.
- A library built with `-buildmode=c-shared` is never unmapped: the Go
  runtime inside it cannot be unloaded, so its `unload` refuses.
- A `[]byte` a mutable bytes slot receives is the caller's memory, valid
  for the call, as `io.Reader`'s `Read` receives one.
- 64-bit platforms only, and cgo is required.

## API overview

| Package | Purpose |
| --- | --- |
| `guatiao` | `Value`, `Ref`, `MapRef`, `ListRef`, `Number`, `NewValue`, `SchemaOf`, `Redact`, `UseLibrary`, the errors; `Registry`, `Instance`; `Kind`, `Remote`; `Single`, `Configured`, `LibraryOf`, `Library` |
| `guatiao/serde` | `Parse` / `Emit` for JSON, TOML, YAML |
| `guatiao/intake` | `Check`, `Layout`, `IsVisible`, `At`, `ForSchema`, `FormFor`, `Resolve`, `FlatKeys`, `Flatten`, `Unflatten` |
| `guatiao/civil` | `Date`, `Time`, `DateTime`, `Span`, `Length`, instants, `Normalize` |
| `guatiao/cmd/guatiao-kind` | writes a kind package's glue from its C header, run by `go generate` |

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
unset, and the repository's own `AGENTS.md` says how to produce it. The
examples (`examples/`, a module of their own) hold the greeter kinds, a
host and a library of providers written in Go, and the tests that call
greeters written in Rust, C, Python and Go both ways.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
