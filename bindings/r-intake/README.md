# guatiao.intake

[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

Taking a value in from a person, for
[guatiao](https://github.com/Boriken-Dev/guatiao/) values: the path
grammar (`agent[1].name`), whether a form fits a schema and how it is laid
out, which fields are shown, and the flat key-to-text store a front end
keeps, through the `guatiao_intake` library. A source package whose small C glue
is built on `guatiao`'s C interface.

> **Status: alpha.** It carries no guatiao library; one is opened at run
> time.

## Features

- **Over its own library** — `guatiao_intake`, found through `GUATIAO_INTAKE_LIBRARY`, then
  `GUATIAO_LIBRARY`.
- **Any guatiao value** — every argument is a value of `guatiao`, a view
  or R data, and every answer an ordinary value.
- **Nothing but a C compiler** — no libffi, no dependency beyond
  `guatiao`.

## Installation

```r
# install.packages("remotes")
remotes::install_github("Boriken-Dev/guatiao", subdir = "bindings/r")
remotes::install_github("Boriken-Dev/guatiao", subdir = "bindings/r-intake")
```

Installing compiles the package's C glue (Rtools on Windows). Say where
the `guatiao_intake` library is:

```bash
export GUATIAO_INTAKE_LIBRARY=/opt/myapp/lib          # a directory holding guatiao_intake.dll / libguatiao_intake.so / .dylib
export GUATIAO_INTAKE_LIBRARY=/opt/myapp/myapp.dll    # or the file itself, whatever it is called
```

Without it, `GUATIAO_LIBRARY` is tried: a directory holding the library,
or a file that exports its functions (an application's own library
carrying several guatiao surfaces), or a file it sits beside. Then the
platform's search. `guatiao::use_library(path, library = "guatiao_intake")` names
it from code instead. Nothing is loaded until the first call, which
signals `guatiao_library_not_found` naming every place it looked.

R 4.2 or later, on a 64-bit platform, and a library built for the R
process's architecture.

## Quick start

```r
library(guatiao.intake)     # attaches guatiao too

intake_check(schema, form)                       # NULL when the form fits
intake_layout(schema, form)                      # list(list(section, fields), ...)
intake_is_visible(schema, form, "tls.verify", values)
intake_at(value, "agent[1].name")                # a view, or NULL
intake_flatten(schema, "auth", value)            # named character vector
```

## Known limits

- What `intake_at` and `intake_resolve` answer borrows from a value it
  was found in, and is refused once that value is closed.
- 64-bit platforms only.

## API overview

| Name | Purpose |
| --- | --- |
| `intake_check`, `intake_layout`, `intake_is_visible`, `intake_for_schema`, `intake_form_for` | a form over a schema |
| `intake_at` | a path's place in a value |
| `intake_resolve`, `intake_flat_keys`, `intake_flatten`, `intake_unflatten` | the flat key-to-text store |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/r-intake/inst/AGENTS.md)
is the full API reference and ships in the package. The C ABI is
described by
[`guatiao_intake.h`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao-intake/include/guatiao_intake.h).

## Development

```bash
R CMD build bindings/r && R CMD INSTALL guatiao_*.tar.gz
R CMD build bindings/r-intake
GUATIAO_LIBRARY=target/debug R CMD check guatiao.intake_*.tar.gz
```

The `guatiao` package is installed first: this one compiles against its
C interface. The repository's own `AGENTS.md` says how to produce the
build.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
