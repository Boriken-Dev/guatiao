# guatiao for Fortran

[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![Docs](https://img.shields.io/badge/docs-latest-blue.svg)](https://boriken-dev.github.io/guatiao/fortran/)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

Fortran bindings for the [guatiao](https://github.com/Boriken-Dev/guatiao/)
ABI: **one value model for passing data between languages**, a JSON
Schema that describes a value, and a registry that loads plugin
libraries. Pure Fortran over `iso_c_binding`, opening
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
- **Be a plugin host** — scan a directory, filter on what a library
  declares before loading it, list providers best first, build an
  instance from a configuration, call a provider through its table.
- **A kind is an abstract type** — a deferred procedure per required
  slot, so the compiler checks a provider implements them, and a remote
  type calling another side's table with the same methods.
- **Write a provider in Fortran** — a type extending a kind, in a library
  a host calls the way it calls one written in Rust, C, Python or Go; a
  library file any host's scan finds, its entry and declaration written
  in Fortran.
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
guatiao = { git = "https://github.com/Boriken-Dev/guatiao", tag = "..." }
```

With CMake:

```cmake
include(FetchContent)
FetchContent_Declare(guatiao GIT_REPOSITORY https://github.com/Boriken-Dev/guatiao
  GIT_TAG <release tag> SOURCE_SUBDIR bindings/fortran)
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

| Library | Module | Its own variable, before `GUATIAO_LIBRARY` |
| --- | --- | --- |
| `guatiao` | `guatiao`: values, schemas, configurations, dates and times, redaction | |
| `guatiao_serde` | `guatiao_serde`: JSON, and TOML and YAML when the library has them | `GUATIAO_SERDE_LIBRARY` |
| `guatiao_intake` | `guatiao_intake` | `GUATIAO_INTAKE_LIBRARY` |

A file `GUATIAO_LIBRARY` names serves every module whose library's
functions it exports (an application's own library carrying several
surfaces), and the others find theirs beside it. Each module has its own
`use_library`.

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

### Hosting plugins

```fortran
use greeter_kinds                      ! a kind's module, written once by its author
type(guatiao_registry) :: reg
type(greeter_remote) :: g
type(guatiao_value) :: answer
type(guatiao_error), allocatable :: error

reg = guatiao_open_registry('my-host', '1.0')
call reg%scan_dir('/path/to/plugins', rules=['kind=greeter'])
call greeter_bind(reg, 'acme_hello', g)
call g%greet('ana', answer, error)     ! {"greeting": "hello, ana"}
```

### A kind, and a provider of it

```fortran
type, extends(greeter) :: hello
contains
  procedure :: greet => hello_greet    ! deferred in greeter: the compiler checks it
end type

subroutine hello_greet(self, name, answer, error)
  class(hello), intent(inout) :: self
  character(*), intent(in) :: name
  type(guatiao_value), intent(inout) :: answer   ! an empty map, in the caller's allocator
  type(guatiao_error), allocatable, intent(out) :: error
  if (len(name) == 0) then
    call guatiao_fail(error, GUATIAO_ERR_BAD_VALUE, 'nobody to greet')
    return
  end if
  call answer%set('greeting', 'hello, ' // name)
end subroutine

lib = guatiao_new_library('my_greeters', '1.0.0')      ! as its fpm.toml says
call lib%single('hello', [greeter_serving(h)])
```

Built as a shared library exporting `guatiao_library_entry` (answering
`lib%answer(host)`) and `guatiao_declares`, both written in Fortran, it is
a file any host's scan finds; or `guatiao_register_library(reg, lib)`
registers it in the host's own process.

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
- Fortran cannot ask whether a type overrides a procedure: a provider
  declares the optional slots it serves where it is registered
  (`greeter_serving(h, shout=.true.)`).
- Single inheritance: a provider serving two kinds is one object per
  kind, sharing state through a pointer.
- A remote cannot run a kind's default body for a slot an older table
  lacks, as a Rust caller does: it answers `GUATIAO_ERR_UNSUPPORTED`.
- An object handed to another side is moved, never copied; keep state
  that must be seen from outside behind a pointer component.
- A process holds at most 64 providers that build instances and
  registers at most 16 libraries in process: the descriptor slots that
  carry no `ctx` come from fixed pools.
- A Fortran library is never unmapped: its `unload` refuses.
- flang 22 gets a 24-byte struct passed by value wrong in a `bind(c)`
  procedure on AArch64 and Windows; a kind module takes a
  `guatiao_object` by address there (its API header says how). On
  x86-64 it passes a 16-byte struct wrongly through a procedure pointer
  when the struct lands in the last two argument registers; the API
  header says how a kind avoids it.
- Intel ifx keeps the derived-type and pointer locals of a procedure
  that is not `recursive` in static storage: mark a provider's
  procedures that a slot reaches `recursive` (the binding's all are).
- fpm versions are three numbers: a pre-release `0.0.0-alpha.N` is
  `0.0.0` in `fpm.toml`, and the tag names the build.
- 64-bit platforms only.

## API overview

| Module | Purpose |
| --- | --- |
| `guatiao` | `guatiao_value` and its constructors, statuses, `guatiao_error`, `guatiao_allocator`, `guatiao_use_library`, `guatiao_validate`, `guatiao_redact`, `guatiao_schema_builder`, `guatiao_config`; `guatiao_registry`, `guatiao_instance`; `guatiao_impl`, `guatiao_kind`, `guatiao_serving`, `guatiao_remote`; `guatiao_library`, `guatiao_register_library` |
| `guatiao_kinds` | the helpers a kind's module writes its glue with |
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
unset, and the repository's own `AGENTS.md` says how to produce it. The
examples hold the greeter kinds (`examples/greeter_kind/fortran`), a
library of greeters written in Fortran (`examples/fortran_greeter`), and
a host (`examples/fortran_host`) whose test calls the greeters written in
Rust, C, Python, Go and Fortran both ways.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
