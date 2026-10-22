# guatiao for R

[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![Docs](https://img.shields.io/badge/docs-latest-blue.svg)](https://boriken-dev.github.io/guatiao/r/)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

R bindings for the [guatiao](https://github.com/Boriken-Dev/guatiao/)
ABI: **one value model for passing data between languages**, a JSON
Schema that describes a value, and a registry that loads plugin
libraries. A source package whose small C glue opens the library at run
time, with no dependency outside base R. It works with any shared library
that exports the ABI, whether that is `guatiao` itself or an application
that carries it.

> **Status: alpha.** The package carries no native library of its own:
> you point it at one.

## Features

- **Value trees read the way R reads lists** — `v$host`, `v[["port"]]`,
  `v[[1]]`, `names`, `length`, `as.list`; build a tree from named and
  unnamed lists and vectors, change it in place, read it back as plain R
  data.
- **Numbers keep their exact text** — `1.10` stays `1.10`, and an integer
  of any size survives as `number("18446744073709551615")` where a double
  would round it.
- **Be a plugin host** — scan a directory, filter on what a library
  declares before loading it, list providers best first, build an
  instance from a configuration, call a provider through its table with R
  values in and out, and keep sources of libraries that come and go
  (`reg$refresh()`).
- **A kind is a virtual S4 class and its generics** — `kind()` declares
  one from its C header; its remote extends the class, so code written
  against the kind takes a local provider or a remote alike.
- **Write a provider in R** — an S4 or Reference class with methods of
  the kind's generics, declared with `provider()`, in a library a host
  calls the way it calls one written in Rust, C, Python, Go or Julia; the
  package is the library, named and versioned by its `DESCRIPTION`.
- **Hand a library an R object** — any S4 or Reference Class object with
  the right methods becomes an object of a kind (a listener, a callback)
  that a library calls, or holds until it is done with it.
- **A configuration is an S4 class** — `schema_of()` writes the JSON
  Schema its slots describe, and `from_value()` builds it back;
  `config_class()` is `setClass` that also puts defaults and docs in the
  schema.
- **Dates, times, spans and exact lengths** — `Date`, times of day and
  civil dates and times to the nanosecond, instants, spans and exact
  lengths, read and written by the library itself.
- **Redaction** — a copy of a value with every field its schema marks
  secret hidden.
- **JSON, TOML and YAML**, and **form checks against a schema**, through
  the `guatiao-serde` and `guatiao-intake` libraries when they are present.

## Installation

```r
# install.packages("remotes")
remotes::install_github("Boriken-Dev/guatiao", subdir = "bindings/r")
```

Installing compiles the package's C glue, which needs R's compiler setup
(Rtools on Windows) and libffi (`libffi-dev` on Debian and Ubuntu,
`libffi-devel` on Fedora; macOS and Rtools carry their own). The package
drives a shared library that exports the guatiao ABI. Say where it is:

```bash
export GUATIAO_LIBRARY=/opt/myapp/lib          # a directory holding guatiao.dll / libguatiao.so / libguatiao.dylib
export GUATIAO_LIBRARY=/opt/myapp/myapp.dll    # or the file itself, whatever it is called
```

or call `use_library(path)` before the first call. Without either, the
platform's library search is tried by file name.

| Exports from | Needed for |
| --- | --- |
| `guatiao` | values, the registry, kinds and providers, schemas, time, redaction |
| `guatiao_serde` | `serde_parse`, `serde_emit`: JSON, and TOML and YAML when the library has them |
| `guatiao_intake` | the `intake_*` functions |

R 4.2 or later, on a 64-bit platform, and a library built for the same
architecture as the R process.

## Quick start

### Values

```r
library(guatiao)

v <- value(list(host = "10.0.0.1", port = 5900L, tags = c("a", "b")))
v$port <- 5901L
push(v$tags, "c")

value_data(v$port)        # 5901
names(v)                  # "host" "port" "tags", insertion order
as.list(v$tags)           # list("a", "b", "c")
close(v)
```

A value owns its tree and frees it on `close` (or when the collector
finds it). `v$tags` is a view: a path into the tree that is walked again
on every access, so it stays valid while the tree grows around it.

**Numbers keep their exact text**:

```r
n <- value(number("1.10"))
value_data(n, numbers = "text")   # <number 1.10>
value_data(n)                     # 1.1
```

### Hosting plugins

```r
library(guatiao)
library(greeterkind)   # the kind, declared with kind()

reg <- registry("my-host", "1.0")
reg$scan_dir("/opt/plugins", rules = "kind=greeter")
for (p in reg$providers("greeter")) print(p$key)

greeter <- reg$bind("acme_hello", Greeter)
answer <- greet(greeter, "ana")   # a value: list(greeting = "hello, ana")
close(answer)
reg$close()
```

### A kind, and a provider of it

A kind is declared once, from its C header:

```r
Greeter <- kind("greeter", floor_hash = 2695856491,
  greet = method(name = "string", returns = "map", required = TRUE, fallible = TRUE),
  shout = method(name = "string", returns = "string"))
```

A provider is a class with methods of its generics:

```r
Hello <- setRefClass("Hello", contains = "Greeter")
setMethod("greet", "Hello", function(self, name) {
  if (!nzchar(name)) stop(guatiao_error("BadValue", "nobody to greet"))
  list(greeting = paste0("hello, ", name))
})
provider(Hello, display_name = "Hello, in R")

reg$register_library(library_of("mypackage"))   # the package declaring it is the library
```

A host in this session, written in any language, calls `Hello` through
the same table it would call a Rust or C greeter through.

### A configuration's schema

```r
config_class("Listen",
  host = field("character", default = "0.0.0.0", doc = "Where to listen."),
  port = field("integer"))

schema_of("Listen")                              # the JSON Schema, as a list
from_value(list(port = 5900L), "Listen")@host    # "0.0.0.0"
```

### JSON, TOML and YAML

```r
doc <- serde_parse('{"name": "ana", "n": 1.10}')
serde_emit(doc, format = "yaml")
```

### Dates and spans

```r
add_span(as.Date("2026-01-31"), span(months = 1))         # 2026-02-28
time_parse("timestamp", "2026-10-21T19:15:00.5+05:30")    # <instant 2026-10-21T13:45:00.5Z>
```

## Known limits

- **R code runs only on R's main thread.** A library that calls a
  provider written in R from a thread of its own is answered
  `Unsupported` at once, the provider never running; a destroy it asks
  for there waits for R's thread. Hosts that call providers from worker
  threads need providers written in another language.
- R has no 64-bit integer: integers past 2^53 cross as doubles that may
  round (a slot's `int64` answer, a span's units). Inside a value a
  number keeps its exact text, which `number()` and
  `value_data(numbers = "text")` reach.
- A length-one vector is a scalar; wrap it in `I()` for a list of one,
  as with jsonlite. A data frame is refused: give its rows or columns.
- `v[[i]] <- x` replaces an element in place; there is no slice
  assignment.
- A remote cannot run a kind's default body for a slot an older table
  lacks, as a Rust caller does: `has_slot(r, "slot")` asks, and the call
  answers `Unsupported`.
- An R library is registered in a host in the same session; there is no
  library file a foreign host's scan loads, since that would need R
  embedded in it.
- 64-bit platforms only. CRAN's R for Windows is x64; on Windows ARM64
  it needs an x64 build of the library.

## API overview

| Name | Purpose |
| --- | --- |
| `value`, `absent`, `number`, `value_data`, `push`, `insert`, `delete`, `clear`, `merge_from`, `lower_value` | the value model |
| `schema_of`, `config_class`, `field`, `from_value`, `validate` | configurations and their schemas |
| `time_parse`, `time_format`, `time_normalize`, `civil_time`, `civil_datetime`, `instant`, `span`, `exact_length`, `add_span`, `read_length` | dates, times, spans, exact lengths |
| `redact` | secrets hidden |
| `serde_parse`, `serde_emit` | JSON, TOML, YAML |
| `intake_check`, `intake_layout`, `intake_is_visible`, `intake_at`, `intake_for_schema`, `intake_form_for`, `intake_resolve`, `intake_flat_keys`, `intake_flatten`, `intake_unflatten` | forms and the flat projection |
| `registry` | hosting plugins: `$scan_dir`, `$providers`, `$bind`, `$create`, `$refresh`, ... |
| `kind`, `method`, `lent`, `taken`, `has_slot`, `byte_buffer`, `bind_table` | a kind, declared from its C header |
| `provider`, `guatiao_library`, `library_of`, `live_instances` | providers and libraries written in R |
| `guatiao_error`, `statuses` | errors |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/r/inst/AGENTS.md)
is the full API reference and ships in the package. The C ABI is
described by
[`guatiao.h`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao/include/guatiao.h).

## Development

```bash
R CMD build bindings/r
GUATIAO_LIBRARY=target/debug GUATIAO_REPO=. R CMD check guatiao_*.tar.gz
```

The tests need the three libraries in one directory, named by
`GUATIAO_LIBRARY`, and a checkout (`GUATIAO_REPO`) for the shared vectors
and the example packages; the repository's own `AGENTS.md` says how to
produce the build. The examples (`examples/greeter_kind/r/greeterkind`,
`examples/r_provider/rgreeter`, `examples/r_host`) hold the greeter kinds,
a library of providers and a host written in R; the tests call greeters
written in Rust, C, Python, Go and R both ways.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
