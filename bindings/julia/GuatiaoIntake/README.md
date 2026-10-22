# GuatiaoIntake

[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

Taking a value in from a person, for
[guatiao](https://github.com/Boriken-Dev/guatiao/) values: the path
grammar (`agent[1].name`), whether a form fits a schema and how it is
laid out, which fields are shown, and the flat `key => text` store a
front end keeps. Pure Julia over `ccall`, calling the `guatiao_intake` library,
and built on the `Guatiao` package for the value model and its errors.

> **Status: alpha.** No native library of its own; you point it at one.

## Features

- **Forms over a schema** — check a form document against the schema it
  shows, lay its fields out in sections, decide which are visible given
  what was entered, or take the form a schema implies.
- **A path names a place in a value** — `agent[1].name`, `env[PATH]`,
  read as a borrowed view of the tree you passed.
- **The flat store** — a value as `key => text` and back, the way a front
  end keeps what a person typed.
- **Any guatiao value** — every argument is a `Guatiao.Value`, a view or
  plain data, and every answer an ordinary `Value`.

## Installation

```julia
using Pkg
Pkg.add(url = "https://github.com/Boriken-Dev/guatiao", subdir = "bindings/julia/Guatiao")
Pkg.add(url = "https://github.com/Boriken-Dev/guatiao", subdir = "bindings/julia/GuatiaoIntake")
```

The package drives the `guatiao_intake` shared library. Say where it is:

```bash
export GUATIAO_INTAKE_LIBRARY=/opt/myapp/lib          # a directory holding guatiao_intake.dll / libguatiao_intake.so / .dylib
export GUATIAO_INTAKE_LIBRARY=/opt/myapp/myapp.dll    # or the file itself, whatever it is called
```

Without it, `GUATIAO_LIBRARY` is tried: a directory holding the library,
or a file that exports its functions (an application's own library
carrying several guatiao surfaces), or a file it sits beside. Then the
platform's search. `GuatiaoIntake.use_library(path)` names it from code instead.
Nothing is loaded at `using`; the first call throws
`Guatiao.LibraryNotFound`, naming every place it looked.

Julia 1.10 or later.

## Quick start

```julia
using Guatiao, GuatiaoIntake

GuatiaoIntake.check(schema, form)                  # nothing when the form fits
GuatiaoIntake.layout(schema, form)                 # [Dict("section" => ..., "fields" => [...]), ...]
GuatiaoIntake.isvisible(schema, form, "tls.verify", values)
GuatiaoIntake.at(value, "agent[1].name")           # a borrowed ValueRef, or nothing
GuatiaoIntake.flatten(schema, "auth", value)       # Dict{String,String}
```

## Known limits

- What `at` and `resolve` answer borrows from the tree it was found in:
  read-only, and refused once that tree is closed. `copy` keeps it.
- 64-bit platforms only, and a library built for the Julia process's
  architecture.

## API overview

| Name | Purpose |
| --- | --- |
| `check`, `layout`, `isvisible`, `for_schema`, `form_for` | a form over a schema |
| `at` | a path's place in a value |
| `resolve`, `flat_keys`, `flatten`, `unflatten` | the flat `key => text` store |
| `GuatiaoIntake.use_library(path)` | naming `guatiao_intake` |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/julia/GuatiaoIntake/AGENTS.md)
is the full API reference and ships in the package. The C ABI is
described by
[`guatiao_intake.h`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao-intake/include/guatiao_intake.h).

## Development

```bash
julia -e 'using Pkg; Pkg.activate(temp = true);
          Pkg.develop([PackageSpec(path = "bindings/julia/Guatiao"), PackageSpec(path = "bindings/julia/GuatiaoIntake")]);
          Pkg.test("GuatiaoIntake")'
```

From the repository root. The tests point `GUATIAO_LIBRARY` at the
repository's build output when the variable is unset; the repository's
own `AGENTS.md` says how to produce it.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
