# guatiao-intake for C++

[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![Docs](https://img.shields.io/badge/docs-latest-blue.svg)](https://boriken-dev.github.io/guatiao/cpp/)

Taking a [guatiao](https://github.com/Boriken-Dev/guatiao/) value in from
a person, from C++: the path grammar (`agent[1].name`), a flat
`key -> text` store, and how a schema is shown — sections, hints,
conditions. Header-only over `guatiao_intake.h` and the `guatiao_intake`
library, built on the [`guatiao`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/cpp/guatiao/README.md)
C++ package.

> **Status: alpha.** No prebuilt library is distributed.

## Installation

```cmake
find_package(guatiao-intake CONFIG REQUIRED)   # -Dguatiao-intake_DIR=<repo>/bindings/cpp/guatiao-intake
target_link_libraries(app PRIVATE guatiao::intake)
```

`GUATIAO_INTAKE_LIBRARY` or `GUATIAO_LIBRARY` names a build of the
`guatiao_intake` library; without either, configuring builds one with
cargo.

## Quick start

```cpp
#include <guatiao/intake.hpp>
namespace intake = guatiao::intake;

intake::check(schema, form);                         // throws BAD_VALUE when the form does not fit
guatiao::value sections = intake::layout(schema, form);
guatiao::view name = intake::at(config, "agent[1].name");
guatiao::value flat = intake::flatten(schema, "auth", config["auth"]);
guatiao::value back = intake::unflatten(schema, "auth", flat);
```

## API overview

| Name | Purpose |
| --- | --- |
| `check`, `layout`, `for_schema`, `form_for`, `is_visible` | forms |
| `at`, `resolve` | paths into a value and a schema |
| `flat_keys`, `flatten`, `unflatten` | the flat store |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/cpp/guatiao-intake/AGENTS.md)
is the full API reference.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
