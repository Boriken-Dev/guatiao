# guatiao-intake for Go

[![Go Reference](https://pkg.go.dev/badge/github.com/Boriken-Dev/guatiao/bindings/go/guatiao-intake.svg)](https://pkg.go.dev/github.com/Boriken-Dev/guatiao/bindings/go/guatiao-intake)
[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![Docs](https://img.shields.io/badge/docs-latest-blue.svg)](https://boriken-dev.github.io/guatiao/go/)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

Taking a value in from a person, for
[guatiao](https://github.com/Boriken-Dev/guatiao/) values: the path
grammar (`agent[1].name`), whether a form fits a schema and how it is
laid out, which fields are shown, and the flat `key -> text` store a front
end keeps. cgo over the `guatiao_intake` library, loaded at run time, and
built on the
[`guatiao`](https://pkg.go.dev/github.com/Boriken-Dev/guatiao/bindings/go/guatiao)
module for the value model and its errors.

> **Status: alpha.** The module carries no native library of its own:
> you point it at one.

## Features

- **Forms over a schema** — check a form document against the schema it
  shows, lay its fields out in sections, decide which are visible given
  what was entered, or take the form a schema implies when nobody wrote
  one.
- **A path names a place in a value** — `agent[1].name`, `env[PATH]`,
  read as a borrowed view of the tree you passed.
- **The flat store** — a value as `key -> text` and back, the way a front
  end keeps what a person typed, with tagged fields, lists and open maps.
- **Any guatiao value** — every argument is a `*guatiao.Value`, a view or
  plain data, whichever library built it, and every answer an ordinary
  Value.

## Installation

```bash
go get github.com/Boriken-Dev/guatiao/bindings/go/guatiao-intake
```

It requires the `guatiao` module at the same version, which `go get`
adds. cgo must be on, as for `guatiao`. The module drives the
`guatiao_intake` shared library. Say where it is:

```bash
export GUATIAO_INTAKE_LIBRARY=/opt/myapp/lib       # a directory holding guatiao_intake.dll / libguatiao_intake.so / .dylib
export GUATIAO_INTAKE_LIBRARY=/opt/myapp/myapp.dll # or the file itself, whatever it is called
```

Without it, `GUATIAO_LIBRARY` is tried: a directory holding the library,
or a file that exports its functions (an application's own library
carrying several guatiao surfaces), or a file it sits beside. Then the
platform's library search. Nothing is loaded until the first call, which
then answers a `*guatiao.LibraryNotFound` naming every place it looked.
`intake.UseLibrary(path)` names the library from code instead.

Go 1.26 or later, on a 64-bit platform or 32-bit ARM Linux.

## Quick start

```go
import intake "github.com/Boriken-Dev/guatiao/bindings/go/guatiao-intake"

problem, err := intake.Check(schema, form)                // nil when the form fits the schema
sections, err := intake.Layout(schema, form)              // []intake.Section{Section, Fields}
shown, err := intake.IsVisible(schema, form, "tls.verify", values)
name, ok, err := intake.At(value, "agent[1].name")        // a borrowed guatiao.Ref

// A front end that keeps text under flat keys (`agent[0].name`, `env[PATH]`):
field, ok, err := intake.Resolve(schema, "auth.password") // the schema field
keys, err := intake.FlatKeys(schema, "auth")              // [auth auth.username ...]
store, err := intake.Flatten(schema, "auth", authValue)   // map[auth:userpass ...]
back, err := intake.Unflatten(schema, "auth", store)      // a Value to close, or nil
```

## Known limits

- What `At` and `Resolve` answer borrows from the tree it was found in:
  read-only, and refused once that tree is closed. `Clone()` copies it
  out.
- The last element of the import path, `guatiao-intake`, is not the
  package's name, `intake`: an import names it.
- 64-bit platforms and 32-bit ARM Linux, and cgo is required, as for
  `guatiao`.

## API overview

| Name | Purpose |
| --- | --- |
| `Check`, `Layout`, `IsVisible`, `ForSchema`, `FormFor` | a form over a schema |
| `At` | a path's place in a value |
| `Resolve`, `FlatKeys`, `Flatten`, `Unflatten` | the flat `key -> text` store |
| `Section` | one group of `Layout` |
| `UseLibrary(path)` | naming `guatiao_intake` |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/go/guatiao-intake/AGENTS.md)
is the full API reference and ships in the module. The C ABI is described
by [`guatiao_intake.h`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao-intake/include/guatiao_intake.h).

## Development

```bash
cd bindings/go/guatiao-intake
go vet ./...
go test ./...
```

`bindings/go/go.work` builds this module against the `guatiao` module
of the same checkout. The tests point `GUATIAO_LIBRARY` at the
repository's build output when the variable is unset; the repository's
own `AGENTS.md` says how to produce it.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
