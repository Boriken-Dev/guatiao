# guatiao for Fortran

[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![Docs](https://img.shields.io/badge/docs-latest-blue.svg)](https://boriken-dev.github.io/guatiao/fortran/)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

Fortran bindings for the [guatiao](https://github.com/Boriken-Dev/guatiao/)
ABI: **one value model for passing data between languages**, a JSON
Schema that describes a value, and documents, forms and dates read and
written by the library itself. Pure Fortran over `iso_c_binding`, opening
the library at run time: a build needs a Fortran compiler and nothing
else. It works with any shared library that exports the ABI, whether
that is `guatiao` itself or an application that carries it.

> **Status: alpha.** The package carries no native library of its own:
> you point it at one.

## Features

- **Value trees as Fortran data** — build, read and change a tree in
  place through a handle, with `set`, `push` and `get` generic over
  Fortran's own types.
- **Numbers keep their exact text** — `1.10` stays `1.10`, an integer of
  any size survives, a real is written as the shortest text that reads
  back the same.
- **Handles that cannot double free** — copies share a counted tree; the
  last one going, or `close()`, frees it.
- **A configuration is a derived type** — it describes its schema field
  by field and decodes the checked value into its components.
- **Dates, times, spans and exact lengths** — `guatiao_date`,
  `guatiao_time`, `guatiao_datetime`, `guatiao_timestamp`, `guatiao_span`
  and `guatiao_length`, to the nanosecond.
- **Redaction** — a copy of a value with every field its schema marks
  secret hidden.
- **JSON, TOML and YAML**, and **form checks against a schema**, through
  the `guatiao-serde` and `guatiao-intake` libraries when they are present.
- **Nothing native until it is needed** — the library is looked for on
  first use, and a missing one names where it was looked for.

## Installation

With [fpm](https://fpm.fortran-lang.org/), as a git dependency:

```toml
[dependencies]
guatiao = { git = "https://github.com/Boriken-Dev/guatiao", tag = "v0.0.0-alpha.2" }
```

With CMake:

```cmake
include(FetchContent)
FetchContent_Declare(guatiao GIT_REPOSITORY https://github.com/Boriken-Dev/guatiao
  GIT_TAG v0.0.0-alpha.2 SOURCE_SUBDIR bindings/fortran)
FetchContent_MakeAvailable(guatiao)
target_link_libraries(app PRIVATE guatiao::fortran)
```

The package drives a shared library that exports the guatiao ABI. Say
where it is:

```bash
export GUATIAO_LIBRARY=/opt/myapp/lib          # a directory holding guatiao.dll / libguatiao.so / libguatiao.dylib
export GUATIAO_LIBRARY=/opt/myapp/myapp.dll    # or the file itself, whatever it is called
```

or `call guatiao_use_library(path)` before the first call. Without
either, the platform's library search is tried by file name.

| Exports from | Needed for |
| --- | --- |
| `guatiao` | values, schemas, configurations, dates and times, redaction |
| `guatiao_serde` | `guatiao_serde`: JSON, and TOML and YAML when the library has them |
| `guatiao_intake` | `guatiao_intake` |

## Quick start

### Values

```fortran
use, intrinsic :: iso_fortran_env, only: int64
use guatiao
type(guatiao_value) :: v, tags, port

v = guatiao_map()
call v%set('host', '10.0.0.1')
call v%set('port', 5900)
call v%set('tags', guatiao_list())
tags = v%key('tags')
call tags%push('a')

port = v%key('port')
print *, port%int_or(22_int64)   ! 5900
call v%close()                   ! or let the last handle go
```

### A configuration's schema

```fortran
type, extends(guatiao_config) :: shout_config
  character(:), allocatable :: prefix
contains
  procedure :: describe => shout_describe
  procedure :: decode => shout_decode
end type

subroutine shout_describe(self, schema)
  class(shout_config), intent(in) :: self
  type(guatiao_schema_builder), intent(inout) :: schema
  call schema%string_field('prefix', description='What every shout starts with', min_length=1)
end subroutine

subroutine shout_decode(self, value, error)
  class(shout_config), intent(inout) :: self
  type(guatiao_value), intent(in) :: value
  type(guatiao_error), allocatable, intent(out) :: error
  call value%get_key('prefix', self%prefix)
end subroutine
```

`guatiao_schema_of(c)` is the schema; `guatiao_decode_config(c, value)`
checks a value against it and decodes it.

### JSON, TOML and YAML

```fortran
use guatiao_serde
call guatiao_parse('{"port": 5900}', GUATIAO_FORMAT_JSON, v)
call guatiao_emit(v, GUATIAO_FORMAT_YAML, text)
```

### Dates and spans

```fortran
use guatiao_civil
type(guatiao_date) :: d, later
call guatiao_from_text('2026-01-31', d)
call guatiao_add_span(d, guatiao_span(months=1), later)
call guatiao_to_text(later, text)   ! "2026-02-28"
```

## Known limits

- Fortran allows no `%` after a function reference: assign a handle
  (`port = v%key('port')`) or use `associate` before calling on it.
- A call without `stat` stops the program when it fails, the standard
  library's convention; pass `stat` wherever a failure is expected.
- A value is not safe to change from two threads at once.
- gfortran 14 and older keep the length of a function's deferred-length
  character result in static storage: functions such as `string_or` are
  not safe from two threads at once there; the subroutine forms are.
- fpm versions are three numbers: a pre-release `0.0.0-alpha.N` is
  `0.0.0` in `fpm.toml`, and the tag names the build.
- 64-bit platforms only.

## API overview

| Module | Purpose |
| --- | --- |
| `guatiao` | `guatiao_value` and its constructors, statuses, `guatiao_error`, `guatiao_allocator`, `guatiao_use_library`, `guatiao_validate`, `guatiao_redact`, `guatiao_schema_builder`, `guatiao_config` |
| `guatiao_serde` | `guatiao_parse` / `guatiao_emit` for JSON, TOML, YAML |
| `guatiao_intake` | `check`, `layout`, `is_visible`, `at`, `for_schema`, `form_for`, `resolve`, `flat_keys`, `flatten`, `unflatten` |
| `guatiao_civil` | dates, times, instants, spans, exact lengths, `guatiao_normalize` |
| `guatiao_c`, `guatiao_serde_c`, `guatiao_intake_c` | the C headers as `bind(c)` mirrors and procedure pointers |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/fortran/AGENTS.md)
is the full API reference and ships with the package. The C ABI is
described by [`guatiao.h`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao/include/guatiao.h).

## Development

```bash
cd bindings/fortran
fpm test --compiler gfortran            # or flang, ifx
cmake -S . -B build && cmake --build build && ctest --test-dir build
```

The tests read the headers and the shared vectors from the repository,
and need the three libraries in one directory; they point
`GUATIAO_LIBRARY` at the repository's build output when the variable is
unset, and the repository's own `AGENTS.md` says how to produce it.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
