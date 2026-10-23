# guatiao for C++

[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![Docs](https://img.shields.io/badge/docs-latest-blue.svg)](https://boriken-dev.github.io/guatiao/cpp/)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

[guatiao](https://github.com/Boriken-Dev/guatiao/) values, schemas and
plugins from C++: a header-only library over the C header `guatiao.h` and
the `guatiao` shared library, in C++'s own idiom — owners that free what
they hold, `std::string_view` and iterators, exceptions or `try_` results,
kinds served by ordinary classes.

> **Status: alpha.** No prebuilt library is distributed: the CMake
> package finds one you built, or builds it with cargo.

## Features

- **Values that own themselves** — `guatiao::value` frees its tree when it
  goes out of scope and moves rather than copies; `view`, `node` and the
  map and list views borrow, like iterators.
- **Numbers keep their exact text** — `as<std::int64_t>()` answers only
  what fits, never a truncation.
- **Exceptions or not** — every call that can fail has a throwing form and
  a `try_` form answering `result<T>`, so code built with
  `-fno-exceptions` uses the same library.
- **The encodings every language shares** — dates, times, spans and exact
  lengths, the wire encoding and channels, schema checks and redaction,
  each read and written by the library itself.
- **Plugins both ways** — a `registry` loads libraries written in any
  language, watches directories and sources and calls `refresh()`; a C++
  class serves a kind by having the slots as methods, checked by the
  compiler, and `GUATIAO_LIBRARY(...)` makes a shared library of them that
  any host's scan finds.

## Installation

With CMake, from a checkout of the repository:

```cmake
find_package(guatiao CONFIG REQUIRED)   # cmake -Dguatiao_DIR=<repo>/bindings/cpp/guatiao
target_link_libraries(app PRIVATE guatiao::guatiao)
guatiao_copy_runtime(app)               # Windows: guatiao.dll beside the program
```

or `FetchContent_Declare(guatiao GIT_REPOSITORY https://github.com/Boriken-Dev/guatiao
GIT_TAG <tag> SOURCE_SUBDIR bindings/cpp/guatiao)` and
`FetchContent_MakeAvailable(guatiao)`.

The package links the `guatiao` library. Set `GUATIAO_LIBRARY` to a
directory holding a build of it (`cargo build -p guatiao --features
load,regex`), or to an application's own library carrying it; without it,
configuring builds one with cargo from the repository's sources, so Rust
(https://rustup.rs) is the only prerequisite. C++17 or later: MSVC, clang
or g++.

## Quick start

A host calling a greeter, whatever language wrote it:

```cpp
#include "greeter_kind.hpp"   // the kind, declared once with GUATIAO_KIND

guatiao::registry reg("my-host", "1.0");
reg.watch_dir("/opt/plugins", guatiao::rules({"kind=greeter"}));
reg.refresh();
auto g = greeter_kind::greeter::bind(reg, "derived_greeter_hello");
guatiao::map answer = g.greet("ana");             // {"greeting": "hello, ana"}
```

A provider written in C++, as a library file:

```cpp
class hello {
 public:
  guatiao::map greet(std::string_view name) { return guatiao::map{{"greeting", "hello, " + std::string(name)}}; }
};

guatiao::library acme() {
  guatiao::library lib("acme", "1.0.0");
  lib.single<hello, greeter_kind::greeter>("hello");
  return lib;
}

GUATIAO_LIBRARY(acme, greeter_kind::greeter)
```

Values:

```cpp
#include <guatiao/guatiao.hpp>

guatiao::value v = guatiao::make(guatiao::item::map({{"port", 5900}, {"tags", guatiao::item::list({"a"})}}));
v["tags"].push("b");
std::int64_t port = v["port"].int_or(22);

auto when = guatiao::date::parse("2026-01-31").add(guatiao::span::parse("P1M"));   // 2026-02-28
std::vector<std::uint8_t> bytes = guatiao::to_wire(v);

auto r = guatiao::try_from_wire(bytes);      // no exceptions: a result
if (!r) std::printf("%s\n", r.error().what());
```

## API overview

| Name | Purpose |
| --- | --- |
| `value`, `map`, `list` | owned trees |
| `view`, `map_view`, `list_view` | borrowed, read-only |
| `node`, `map_ref`, `list_ref` | borrowed, writable |
| `item`, `make`, `try_make` | describing and building a tree |
| `error`, `result<T>`, `status` | failures |
| `date`, `time_of_day`, `date_time`, `timestamp`, `span`, `length`, `normalize` | the time forms |
| `to_wire`, `from_wire`, `wire_announce`, `wire_send`, `wire_close`, `receiver` | values as bytes, and channels |
| `validate`, `redact`, `merge` | schemas, secrets, layering |
| `registry`, `instance`, `rules` | a host: loading, sources, `refresh()`, ranking, unloading |
| `GUATIAO_KIND`, `GUATIAO_OBJECT_KIND`, `remote<K>`, `object<K>`, `lent<K>` | kinds: declared over their C tables, called through any table |
| `library`, `GUATIAO_LIBRARY`, `guatiao_library_target` | providers written in C++, as a library file a scan finds |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/cpp/guatiao/AGENTS.md)
is the full API reference. JSON, TOML and YAML are
[`guatiao-serde`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/cpp/guatiao-serde/README.md);
taking a value in from a person is
[`guatiao-intake`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/cpp/guatiao-intake/README.md).

## Development

```bash
cargo build --workspace --all-features
cd bindings/cpp/guatiao
GUATIAO_LIBRARY=../../../target/debug cmake -B build
cmake --build build && ctest --test-dir build
```

Each suite is built twice, with exceptions on and off, with warnings as
errors; `-DCMAKE_CXX_STANDARD=20` builds them as C++20.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
