# guatiao.serde (R) — API header

Over the `guatiao_serde` C ABI: https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships in the package, so it names no
repository path.

Package `guatiao.serde`, R 4.2 or later, 64-bit platforms and 32-bit ARM. A
source package: its C glue is compiled when it installs (Rtools on Windows), against the C
interface the `guatiao` package registers (`LinkingTo: guatiao`). It
depends on `guatiao`, which `library(guatiao.serde)` attaches too, at the same
version. It carries no guatiao library: one is opened at run time.

## Finding the native library

Nothing native is touched until the first call that needs it; the
library is then found, by `guatiao`'s lookup, in this order:

1. `guatiao::use_library(path, library = "guatiao_serde")`, called before that
   first call: a file exporting its functions, whatever it is called, or
   a directory.
2. `GUATIAO_SERDE_LIBRARY`: a file, taken as it is, or a directory holding
   `guatiao_serde.dll` / `libguatiao_serde.so` / `libguatiao_serde.dylib`.
3. `GUATIAO_LIBRARY`: such a directory; or a file, used when it exports
   the library's functions (an application's own library carrying several
   surfaces), else the library beside it.
4. The platform's own search, by file name.

Not found signals `guatiao_library_not_found` (`basename`, `looked`); an
export the loaded build lacks, `guatiao_missing_symbol`. The library
carries the core too, so one file can serve both packages, and it stays
loaded for the session. `guatiao::has_export(symbol, "guatiao_serde")` asks.

## Reading and writing

`serde_parse(text, format = "json", read_data_uris = FALSE, alloc =
NULL)` answers a value of `guatiao` (close it); `serde_emit(x, format =
"json", pretty = FALSE, bytes = c("data_uri", "base64", "array",
"refuse"))` a string, from a value, a view or R data, built by any guatiao
library. `format` is `"json"`, `"toml"` or `"yaml"`. A document that does
not parse signals `guatiao_parse_error` (a `guatiao_error` with status
`BadValue`: `format`, `line` and `column` from 1, the column in
characters, 0 when the parser did not say; `reason`, which never quotes
the document). TOML writes only a map, and no null. TOML and YAML need a
`guatiao_serde` built with them (`guatiao_missing_symbol` naming the
feature otherwise). A number keeps its exact text, except that an
exponent is read back with its sign.

`serde_data(text, format = "json", read_data_uris = FALSE, numbers =
c("auto", "text", "double", "integer64"))` answers the R data
`guatiao::value_data(serde_parse(text), numbers = numbers)` answers, with
no value built: the document crosses once, as wire bytes read in C, and
the same `guatiao_parse_error` is signalled. `"integer64"` needs the
`bit64` package. R data given to `serde_emit` is written the same way,
as wire bytes in C with no value built, and refused as `guatiao::value`
refuses it. Either is faster than the value for a document of many small
items; a single string of megabytes is faster through the value.
