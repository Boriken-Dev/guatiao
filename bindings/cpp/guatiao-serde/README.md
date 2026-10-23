# guatiao-serde for C++

[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![Docs](https://img.shields.io/badge/docs-latest-blue.svg)](https://boriken-dev.github.io/guatiao/cpp/)

JSON, TOML and YAML for [guatiao](https://github.com/Boriken-Dev/guatiao/)
values from C++: read a document into a `guatiao::value`, write one back
out. Header-only over `guatiao_serde.h` and the `guatiao_serde` library,
built on the [`guatiao`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/cpp/guatiao/README.md)
C++ package.

> **Status: alpha.** No prebuilt library is distributed.

## Features

- **Three formats, one call each** — `parse` and `emit`, read and written
  by the library itself, as every language bound to guatiao reads them.
- **Numbers keep their exact text**, at any size.
- **Bytes have a spelling** where a format has none: a data URI, base64
  or a list of numbers.
- **A document that does not parse says where** — format, line and
  column, never quoting the document.

## Installation

```cmake
find_package(guatiao-serde CONFIG REQUIRED)   # -Dguatiao-serde_DIR=<repo>/bindings/cpp/guatiao-serde
target_link_libraries(app PRIVATE guatiao::serde)
```

`GUATIAO_SERDE_LIBRARY` or `GUATIAO_LIBRARY` names a build of the
`guatiao_serde` library (`cargo build -p guatiao-serde --features
toml,yaml`); without either, configuring builds one with cargo.
`GUATIAO_SERDE_FEATURES` says which formats it carries (default
`toml;yaml`).

## Quick start

```cpp
#include <guatiao/serde.hpp>
namespace serde = guatiao::serde;

guatiao::value v = serde::parse(serde::format::json, R"({"port": 5900, "tags": ["a"]})");
std::string text = serde::emit(serde::format::yaml, v);

auto bad = serde::try_parse(serde::format::json, "{\"a\":\n 1,,}");
bad.error().detail()["line"].int_or(0);   // 2
```

## API overview

| Name | Purpose |
| --- | --- |
| `parse`, `try_parse` | a document as a `guatiao::value` |
| `emit`, `try_emit` | a value as a document |
| `to_wire`, `from_wire` | a document to and from wire bytes, no tree built |
| `format::{json, toml, yaml}` | the formats |
| `how::{data_uri, base64, array, refuse, read_data_uris, pretty}` | the `how` flags |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/cpp/guatiao-serde/AGENTS.md)
is the full API reference.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
