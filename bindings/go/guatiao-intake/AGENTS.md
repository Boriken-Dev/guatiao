# guatiao-intake (Go) — API header

Taking a value in from a person, over the `guatiao_intake` C library:
https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships in the module, so it names no
repository path.

Module `github.com/Boriken-Dev/guatiao/bindings/go/guatiao-intake`,
package `intake`; it requires the module
`github.com/Boriken-Dev/guatiao/bindings/go/guatiao` (package `guatiao`)
at the same version, for the value model and its errors. Go 1.26 or
later, 64-bit platforms, cgo (on Windows a MinGW-style C compiler, such
as llvm-mingw or MinGW-w64 GCC). No other dependencies, and no native
library of its own: `guatiao_intake` is loaded at run time.

```go
import intake "github.com/Boriken-Dev/guatiao/bindings/go/guatiao-intake"
```

The last element of the import path is not the package's name, so an
import names it, as above.

## Finding the native library

Nothing native is touched until the first call. `guatiao_intake` is then
found in this order; the core library is found as the `guatiao` package
finds it.

1. `intake.UseLibrary(path)`, called before that first call: a file
   exporting `guatiao_intake`'s functions, whatever it is called (an
   application's own library carrying several surfaces), or a directory.
   Naming another file once one is loaded is an error; naming the same
   one is not.
2. `GUATIAO_INTAKE_LIBRARY`: a file, taken as it is, or a directory
   holding `guatiao_intake.dll` / `libguatiao_intake.so` /
   `libguatiao_intake.dylib`.
3. `GUATIAO_LIBRARY`: a directory holding it, or a file: taken when it
   exports `guatiao_intake`'s functions, else the library beside it.
4. The platform's own library search, by file name.

Not found is a `*guatiao.LibraryNotFound` (`Basename`, `Looked`); a file
lacking an export every build has is a `*guatiao.MissingSymbol`. The
library stays loaded for the process.

## Forms, paths and the flat projection

```go
why, err := intake.Check(schema, form)              // nil, or the error map
sections, err := intake.Layout(schema, form)         // []intake.Section{Section, Fields}
shown, err := intake.IsVisible(schema, form, "key", values)
ref, ok, err := intake.At(value, "agent[1].name")    // borrowed
form, err := intake.ForSchema(schema)                // a Value to close
form, err := intake.FormFor(form, schema, "path")    // a Value to close, or nil
ref, ok, err := intake.Resolve(schema, "auth.password")
keys, err := intake.FlatKeys(schema, "auth")         // ErrWrongKind for an unknown key
store, err := intake.Flatten(schema, "auth", value)  // map[string]string
value, err := intake.Unflatten(schema, "auth", store) // a Value to close, or nil
```

- `Check(schema, form any) (map[string]any, error)`: nil when the form
  fits the schema, else the reason: `kind` (`malformed`, `unknown_field`,
  `duplicate_section`, `condition_refused` or `cyclic_condition`), `at`,
  whichever of `path`, `id`, `field` and `expected` apply, and a
  `message`.
- `Layout(schema, form any) ([]Section, error)`: `Section{Section
  map[string]any; Fields []string}`, the default section's `Section` nil.
- `IsVisible(schema, form any, key string, values any) (bool, error)`:
  whether the field under `key` is shown, given the values entered so
  far; `guatiao.ErrNotFound` when `key`, or a key a condition reads, names
  no field.
- `At(value any, path string) (guatiao.Ref, bool, error)` and
  `Resolve(schema any, key string) (guatiao.Ref, bool, error)`: false when
  nothing is there.
- `ForSchema(schema any) (*guatiao.Value, error)`,
  `FormFor(form, schema any, path string) (*guatiao.Value, error)` (nil
  for none).
- `FlatKeys(schema any, key string) ([]string, error)`,
  `Flatten(schema any, key string, value any) (map[string]string, error)`,
  `Unflatten(schema any, key string, store any) (*guatiao.Value, error)`.
- `UseLibrary(where string) error`: as above.

Every schema, form, value and values argument is a `*guatiao.Value`, a
view or plain data `guatiao.NewValue` takes. `At` and `Resolve` take a
Value or a view and answer a `Ref` borrowed from it: read-only, found
again on every access, `guatiao.ErrClosed` once the source is closed;
`Clone()` copies it out. `Unflatten` answers nil only for "nothing
stored"; a store that is not strings to strings is
`guatiao.ErrWrongKind`.

A Value this package answers is an ordinary `guatiao.Value`: read,
changed and freed through the `guatiao` package.
