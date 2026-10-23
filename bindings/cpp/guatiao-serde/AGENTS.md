# guatiao-serde (C++) — API header

JSON, TOML and YAML for guatiao values, over the `guatiao_serde` C
library: https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships with the package, so it names no
repository path.

Header-only, C++17 or later, over `<guatiao_serde.h>` and the
`guatiao_serde` shared library, built on the `guatiao` C++ package for
the value model and its errors. `#include <guatiao/serde.hpp>`; namespace
`guatiao::serde`.

## The package and the library

`find_package(guatiao-serde CONFIG)` (`guatiao-serde_DIR` naming this
package's directory), `add_subdirectory` or `FetchContent`, then
`target_link_libraries(app PRIVATE guatiao::serde)`, which brings
`guatiao::guatiao` with it. The library is, in order:

1. `GUATIAO_SERDE_LIBRARY` (a CMake variable, else the environment): a
   file, taken as it is, or a directory holding `guatiao_serde.dll` /
   `libguatiao_serde.so` / `libguatiao_serde.dylib`;
2. `GUATIAO_LIBRARY`: a directory holding it; or a file: the library
   beside it when that file is the core's own, else the file itself (an
   application's own library carrying every surface);
3. otherwise built by cargo from the Rust sources beside the package.

`GUATIAO_SERDE_FEATURES` (default `toml;yaml`; JSON is always there)
names the formats: what a cargo build turns on, and which of
`GUATIAO_SERDE_TOML` and `GUATIAO_SERDE_YAML` the target defines for the
headers. Name what the library was built with when you point at one.

## Reading and writing documents

```cpp
namespace serde = guatiao::serde;
guatiao::value v = serde::parse(serde::format::json, R"({"port": 5900})");
std::string yaml = serde::emit(serde::format::yaml, v);
```

- `parse(format, text, how = 0, alloc = default_alloc())` → `value`;
  `try_parse` → `result<value>`.
- `emit(format, view, how = 0)` → `std::string`; JSON is compact unless
  `how` carries `how::pretty`. A TOML document is a map: anything else is
  WRONG_KIND.
- `to_wire(format, text, how)` → wire bytes, and `from_wire(format,
  bytes, how)` → text, with no tree built.
- `format::{json, toml, yaml}`, `format_name(f)`. A format this build was
  not told about answers UNSUPPORTED.
- `how::` — the low byte says how bytes are written where the format has
  none: `data_uri` (default), `base64`, `array`, `refuse` (BAD_VALUE);
  `read_data_uris` turns a `data:;base64,...` string back into bytes;
  `pretty` indents JSON.

A document that does not parse is BAD_VALUE: `message()` the parser's
words (never quoting the document), `detail()` `{"format", "line",
"column"}`, line and column from 1 (the column in characters), 0 when the
parser did not say. A number keeps its exact text; JSON writes an
exponent's sign (`1e400` is read as `1e+400`).

A tree `parse` built is an ordinary `guatiao::value`, read, changed and
freed through the core package; any guatiao library takes it.
