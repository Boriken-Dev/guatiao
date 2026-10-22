# GuatiaoSerde (Julia) — API header

Over the `guatiao_serde` C ABI: https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships in the package, so it names no repository
path. Built on `Guatiao` (its `Value`, views, errors and `Guatiao.FFI`),
at the same version. No native library of its own and nothing compiled.

## Finding the native library

Nothing native is touched until the first call that needs it; the
library is then found in this order:

1. `GuatiaoSerde.use_library(path)`, called before that first call: a file
   exporting its functions, whatever it is called, or a directory.
2. `GUATIAO_SERDE_LIBRARY`: a file, taken as it is, or a directory holding
   `guatiao_serde.dll` / `libguatiao_serde.so` / `libguatiao_serde.dylib`.
3. `GUATIAO_LIBRARY`: such a directory; or a file, used when it exports
   the surface (an application's own library carrying several surfaces),
   else the library beside it.
4. `Libdl.find_library`, by base name.

Not found is a `Guatiao.LibraryNotFound` (`basename`, `looked`). An export
the loaded build lacks is a `Guatiao.MissingSymbol` when it is used. The
library carries the core too, so one file can serve both packages, and it
stays loaded for the process.

## Reading and writing

```julia
using GuatiaoSerde

v = GuatiaoSerde.parse("""{"port": 5900}""")          # format = :json
text = GuatiaoSerde.emit(v; format = :yaml)
```

- `parse(text; format = :json, how = 0, alloc = Guatiao.FFI.binding_alloc())
  -> Guatiao.Value`: `format` is `:json`, `:toml` or `:yaml`; numbers keep
  their exact text. `close` the Value.
- `emit(x; format = :json, pretty = false, how = 0, alloc) -> String`: `x`
  is a `Value`, a view or plain data, built by any guatiao library. A TOML
  document must be a map (`WrongKind`).
- `how`: `GuatiaoSerde.BYTES_DATA_URI`, `BYTES_BASE64`, `BYTES_ARRAY`,
  `BYTES_REFUSE`, or'd with `READ_DATA_URIS` and `PRETTY`.
- TOML and YAML need a `guatiao_serde` built with them (`MissingSymbol`
  naming the feature); an unknown format is an `ArgumentError`.

A number keeps its exact text, except that an exponent is read back with
its sign (`1e400` is `1e+400`).

## Errors

A document that does not parse is a `ParseError` (exported): `format`,
`line` and `column` from 1 (the column in characters, 0 when the parser
did not say), and `message`, which never quotes the document; a
`Guatiao.GuatiaoException` with `Guatiao.status(e) == BadValue`. Any other
refusal is the status's `GuatiaoError` with the library's words.
