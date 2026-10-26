# guatiao.serde

[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

JSON, TOML and YAML for [guatiao](https://github.com/Boriken-Dev/guatiao/)
values: read a document into a value of the `guatiao` package, write one
back out, through the `guatiao_serde` library. A source package whose small C
glue is built on `guatiao`'s C interface.

> **Status: alpha.** It carries no guatiao library; one is opened at run
> time.

## Features

- **Over its own library** — `guatiao_serde`, found through `GUATIAO_SERDE_LIBRARY`, then
  `GUATIAO_LIBRARY`.
- **Any guatiao value** — every argument is a value of `guatiao`, a view
  or R data, and every answer an ordinary value.
- **Nothing but a C compiler** — no libffi, no dependency beyond
  `guatiao`.

## Installation

```r
# install.packages("remotes")
remotes::install_github("Boriken-Dev/guatiao", subdir = "bindings/r/guatiao")
remotes::install_github("Boriken-Dev/guatiao", subdir = "bindings/r/guatiao.serde")
```

Installing compiles the package's C glue (Rtools on Windows). Say where
the `guatiao_serde` library is:

```bash
export GUATIAO_SERDE_LIBRARY=/opt/myapp/lib          # a directory holding guatiao_serde.dll / libguatiao_serde.so / .dylib
export GUATIAO_SERDE_LIBRARY=/opt/myapp/myapp.dll    # or the file itself, whatever it is called
```

Without it, `GUATIAO_LIBRARY` is tried: a directory holding the library,
or a file that exports its functions (an application's own library
carrying several guatiao surfaces), or a file it sits beside. Then the
platform's search. `guatiao::use_library(path, library = "guatiao_serde")` names
it from code instead. Nothing is loaded until the first call, which
signals `guatiao_library_not_found` naming every place it looked.

R 4.2 or later, on a 64-bit platform or 32-bit ARM Linux, and a library
built for the R
process's architecture.

## Quick start

```r
library(guatiao.serde)      # attaches guatiao too

doc <- serde_parse('{"name": "ana", "n": 1.10}')
value_data(doc)                                   # list(name = "ana", n = 1.1)
serde_emit(doc, format = "yaml")
serde_emit(list(port = 5900L), pretty = TRUE)
close(doc)
serde_data('{"name": "ana", "n": 1.10}')          # the same data, no value built

e <- tryCatch(serde_parse("{"), guatiao_parse_error = function(e) e)
c(e$format, e$line, e$column)
```

## Known limits

- Reading JSON writes an exponent's sign: `1e400` parses as `1e+400`.
- 64-bit platforms and 32-bit ARM Linux, as `guatiao`.

## API overview

| Name | Purpose |
| --- | --- |
| `serde_parse(text, format, read_data_uris, alloc)` | a document as a value |
| `serde_data(text, format, read_data_uris, numbers)` | a document as R data, no value built |
| `serde_emit(x, format, pretty, bytes)` | a value, view or R data as text |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/r/guatiao.serde/inst/AGENTS.md)
is the full API reference and ships in the package. The C ABI is
described by
[`guatiao_serde.h`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao-serde/include/guatiao_serde.h).

## Development

```bash
R CMD build bindings/r/guatiao && R CMD INSTALL guatiao_*.tar.gz
R CMD build bindings/r/guatiao.serde
GUATIAO_LIBRARY=target/debug R CMD check guatiao.serde_*.tar.gz
```

The `guatiao` package is installed first: this one compiles against its
C interface. The repository's own `AGENTS.md` says how to produce the
build.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
