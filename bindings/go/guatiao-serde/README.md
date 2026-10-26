# guatiao-serde for Go

[![Go Reference](https://pkg.go.dev/badge/github.com/Boriken-Dev/guatiao/bindings/go/guatiao-serde.svg)](https://pkg.go.dev/github.com/Boriken-Dev/guatiao/bindings/go/guatiao-serde)
[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![Docs](https://img.shields.io/badge/docs-latest-blue.svg)](https://boriken-dev.github.io/guatiao/go/)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

JSON, TOML and YAML for [guatiao](https://github.com/Boriken-Dev/guatiao/)
values: read a document into a `guatiao.Value`, write one back out. cgo
over the `guatiao_serde` library, loaded at run time, and built on the
[`guatiao`](https://pkg.go.dev/github.com/Boriken-Dev/guatiao/bindings/go/guatiao)
module for the value model and its errors.

> **Status: alpha.** The module carries no native library of its own:
> you point it at one.

## Features

- **Three formats, one call each** — `Parse` and `Emit` for JSON, TOML
  and YAML, parsed and written by the library itself, so every language
  bound to guatiao reads a document the same way.
- **Numbers keep their exact text** — `1.10` stays `1.10`, and an integer
  of any size survives the trip.
- **Bytes have a spelling** — a data URI, base64 or a list of numbers
  where the format has no bytes, and data URIs read back as bytes when
  asked.
- **A document that does not parse says where** — `*serde.ParseError`
  carries the format, line and column, and never quotes the document.
- **Any guatiao value** — what `Parse` returns is an ordinary
  `*guatiao.Value`, and `Emit` takes any Value, view or plain data,
  whichever library built it.
- **Or Go data, with no value in between** — `Unmarshal` reads a document
  straight into a struct, a map or `any`, and `Emit` writes Go data the
  same way; the document crosses once.

## Installation

```bash
go get github.com/Boriken-Dev/guatiao/bindings/go/guatiao-serde
```

It requires the `guatiao` module at the same version, which `go get`
adds. cgo must be on, as for `guatiao`. The module drives the
`guatiao_serde` shared library. Say where it is:

```bash
export GUATIAO_SERDE_LIBRARY=/opt/myapp/lib       # a directory holding guatiao_serde.dll / libguatiao_serde.so / .dylib
export GUATIAO_SERDE_LIBRARY=/opt/myapp/myapp.dll # or the file itself, whatever it is called
```

Without it, `GUATIAO_LIBRARY` is tried: a directory holding the library,
or a file that exports its functions (an application's own library
carrying several guatiao surfaces), or a file it sits beside. Then the
platform's library search. Nothing is loaded until the first call, which
then answers a `*guatiao.LibraryNotFound` naming every place it looked.
`serde.UseLibrary(path)` names the library from code instead.

Go 1.26 or later, on a 64-bit platform or 32-bit ARM Linux.

## Quick start

```go
import serde "github.com/Boriken-Dev/guatiao/bindings/go/guatiao-serde"

v, err := serde.Parse(`{"port": 5900, "tags": ["a"]}`, serde.JSON)
if err != nil {
	return err
}
defer v.Close()
v.Key("port").IntOr(0) // 5900

text, err := serde.Emit(v, serde.YAML, serde.Pretty())

var cfg struct {
	Port uint16   `guatiao:"port"`
	Tags []string `guatiao:"tags"`
}
err = serde.Unmarshal(`{"port": 5900, "tags": ["a"]}`, serde.JSON, &cfg)
```

## Known limits

- Reading JSON writes an exponent's sign: `1e400` parses as `1e+400`.
  Everything else about a number's text survives a round trip.
- The last element of the import path, `guatiao-serde`, is not the
  package's name, `serde`: an import names it.
- 64-bit platforms and 32-bit ARM Linux, and cgo is required, as for
  `guatiao`.

## API overview

| Name | Purpose |
| --- | --- |
| `Parse(text, format, opts...)` | a document as a `*guatiao.Value` |
| `Unmarshal(text, format, dst, opts...)` | a document into Go data, no Value built |
| `Emit(x, format, opts...)` | a Value, view or plain data as text |
| `JSON`, `TOML`, `YAML` | the formats |
| `Pretty()`, `How(flags)`, `InAllocator(alloc)` | the options |
| `ParseError` | a document that does not parse: `Format`, `Line`, `Column`, `Reason` |
| `BytesDataURI`, `BytesBase64`, `BytesArray`, `BytesRefuse`, `ReadDataURIs` | the `How` flags |
| `UseLibrary(path)` | naming `guatiao_serde` |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/go/guatiao-serde/AGENTS.md)
is the full API reference and ships in the module. The C ABI is described
by [`guatiao_serde.h`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao-serde/include/guatiao_serde.h).

## Development

```bash
cd bindings/go/guatiao-serde
go vet ./...
go test ./...
```

`bindings/go/go.work` builds this module against the `guatiao` module
of the same checkout. The tests point `GUATIAO_LIBRARY` at the
repository's build output when the variable is unset; the repository's
own `AGENTS.md` says how to produce it.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
