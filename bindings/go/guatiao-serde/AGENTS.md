# guatiao-serde (Go) — API header

JSON, TOML and YAML for guatiao values, over the `guatiao_serde` C
library: https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships in the module, so it names no
repository path.

Module `github.com/Boriken-Dev/guatiao/bindings/go/guatiao-serde`,
package `serde`; it requires the module
`github.com/Boriken-Dev/guatiao/bindings/go/guatiao` (package `guatiao`)
at the same version, for the value model and its errors. Go 1.26 or
later, 64-bit platforms, cgo (on Windows a MinGW-style C compiler, such
as llvm-mingw or MinGW-w64 GCC). No other dependencies, and no native
library of its own: `guatiao_serde` is loaded at run time.

```go
import serde "github.com/Boriken-Dev/guatiao/bindings/go/guatiao-serde"
```

The last element of the import path is not the package's name, so an
import names it, as above.

## Finding the native library

Nothing native is touched until the first call. `guatiao_serde` is then
found in this order; the core library is found as the `guatiao` package
finds it.

1. `serde.UseLibrary(path)`, called before that first call: a file
   exporting `guatiao_serde`'s functions, whatever it is called (an
   application's own library carrying several surfaces), or a directory.
   Naming another file once one is loaded is an error; naming the same
   one is not.
2. `GUATIAO_SERDE_LIBRARY`: a file, taken as it is, or a directory holding
   `guatiao_serde.dll` / `libguatiao_serde.so` / `libguatiao_serde.dylib`.
3. `GUATIAO_LIBRARY`: a directory holding it, or a file: taken when it
   exports `guatiao_serde`'s functions, else the library beside it.
4. The platform's own library search, by file name.

Not found is a `*guatiao.LibraryNotFound` (`Basename`, `Looked`); a file
lacking an export every build has is a `*guatiao.MissingSymbol`. The
library stays loaded for the process.

## Reading and writing documents

```go
v, err := serde.Parse(`{"port": 5900}`, serde.JSON)
defer v.Close()
text, err := serde.Emit(v, serde.YAML, serde.Pretty())
```

- `Parse(text string, format Format, opts ...Option) (*guatiao.Value, error)`:
  a new Value to close.
- `Emit(x any, format Format, opts ...Option) (string, error)`: `x` is a
  `*guatiao.Value`, a view (`Ref`, `MapRef`, `ListRef`) or plain data
  `guatiao.NewValue` takes.
- `Format`: `JSON`, `TOML`, `YAML`.
- Options: `Pretty()` indents what `Emit` writes; `How(flags uint32)` takes
  one of `BytesDataURI`, `BytesBase64`, `BytesArray`, `BytesRefuse`, or'd
  with `ReadDataURIs`; `InAllocator(alloc unsafe.Pointer)` builds what
  `Parse` reads in the `guatiao_alloc` at `alloc`, which the caller keeps
  alive while the tree lives.
- `UseLibrary(where string) error`: as above.

A document that does not parse is a `*serde.ParseError` (`Format`,
`Line`, `Column` from 1, the column in characters, 0 when the parser did
not say; `Reason`, which never quotes the document);
`errors.Is(err, guatiao.ErrBadValue)` matches it. A TOML document must be
a map (`guatiao.ErrWrongKind`). TOML and YAML need a `guatiao_serde`
built with them: otherwise the call answers a `*guatiao.MissingSymbol`
naming the feature. A number keeps its exact text, except that an
exponent is read back with its sign (`1e400` is `1e+400`).

A tree `Parse` built is an ordinary `guatiao.Value`: read, changed and
freed through the `guatiao` package, and any guatiao library takes it.
