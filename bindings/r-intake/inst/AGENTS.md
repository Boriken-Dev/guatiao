# guatiao.intake (R) — API header

Over the `guatiao_intake` C ABI: https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships in the package, so it names no
repository path.

Package `guatiao.intake`, R 4.2 or later, 64-bit platforms. A source package: its
C glue is compiled when it installs (Rtools on Windows), against the C
interface the `guatiao` package registers (`LinkingTo: guatiao`). It
depends on `guatiao`, which `library(guatiao.intake)` attaches too, at the same
version. It carries no guatiao library: one is opened at run time.

## Finding the native library

Nothing native is touched until the first call that needs it; the
library is then found, by `guatiao`'s lookup, in this order:

1. `guatiao::use_library(path, library = "guatiao_intake")`, called before that
   first call: a file exporting its functions, whatever it is called, or
   a directory.
2. `GUATIAO_INTAKE_LIBRARY`: a file, taken as it is, or a directory holding
   `guatiao_intake.dll` / `libguatiao_intake.so` / `libguatiao_intake.dylib`.
3. `GUATIAO_LIBRARY`: such a directory; or a file, used when it exports
   the library's functions (an application's own library carrying several
   surfaces), else the library beside it.
4. The platform's own search, by file name.

Not found signals `guatiao_library_not_found` (`basename`, `looked`); an
export the loaded build lacks, `guatiao_missing_symbol`. The library
carries the core too, so one file can serve both packages, and it stays
loaded for the session. `guatiao::has_export(symbol, "guatiao_intake")` asks.

## Forms, paths and the flat store

```r
intake_check(schema, form)                  # NULL, or the error as a list
intake_layout(schema, form)                 # list(list(section, fields), ...)
intake_is_visible(schema, form, "key", values)
intake_at(value, "agent[1].name")           # a view, or NULL
intake_for_schema(schema)                   # a value
intake_form_for(form, schema, "path")       # a value, or NULL
intake_resolve(schema, "auth.password")     # a view, or NULL
intake_flat_keys(schema, "auth")            # character vector
intake_flatten(schema, "auth", value)       # named character vector
intake_unflatten(schema, "auth", store)     # a value, or NULL
```

Every schema, form, value and values argument is a value of `guatiao`, a
view or R data, built by any guatiao library. `intake_at` and
`intake_resolve` answer a view of the argument's own node when it is a
value or view (found again on every access, `guatiao_closed` once it is
closed), and a copy when it is R data. `intake_unflatten` answers `NULL`
for "nothing stored" or a key that does not apply; its store is a named
character vector or a named list of strings. A value answered is built in
`guatiao`'s allocator; close it. A refusal signals the status's
`guatiao_error`.
