# GuatiaoIntake (Julia) — API header

Over the `guatiao_intake` C ABI: https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships in the package, so it names no repository
path. Built on `Guatiao` (its `Value`, views, errors and `Guatiao.FFI`),
at the same version. No native library of its own and nothing compiled.

## Finding the native library

Nothing native is touched until the first call that needs it; the
library is then found in this order:

1. `GuatiaoIntake.use_library(path)`, called before that first call: a file
   exporting its functions, whatever it is called, or a directory.
2. `GUATIAO_INTAKE_LIBRARY`: a file, taken as it is, or a directory holding
   `guatiao_intake.dll` / `libguatiao_intake.so` / `libguatiao_intake.dylib`.
3. `GUATIAO_LIBRARY`: such a directory; or a file, used when it exports
   the surface (an application's own library carrying several surfaces),
   else the library beside it.
4. `Libdl.find_library`, by base name.

Not found is a `Guatiao.LibraryNotFound` (`basename`, `looked`). An export
the loaded build lacks is a `Guatiao.MissingSymbol` when it is used. The
library carries the core too, so one file can serve both packages, and it
stays loaded for the process.

## Forms, paths and the flat store

```julia
using GuatiaoIntake

GuatiaoIntake.check(schema, form)                  # nothing, or the error map
GuatiaoIntake.layout(schema, form)                 # [Dict("section" => ..., "fields" => [...]), ...]
GuatiaoIntake.isvisible(schema, form, "key", values)
GuatiaoIntake.at(value, "agent[1].name")           # a borrowed ValueRef, or nothing
GuatiaoIntake.for_schema(schema)                   # a Value
GuatiaoIntake.form_for(form, schema, "path")       # a Value, or nothing
GuatiaoIntake.resolve(schema, "auth.password")     # a borrowed ValueRef, or nothing
GuatiaoIntake.flat_keys(schema, "auth")            # Vector{String}; WrongKind for an unknown key
GuatiaoIntake.flatten(schema, "auth", value)       # Dict{String,String}
GuatiaoIntake.unflatten(schema, "auth", store)     # a Value, or nothing
GuatiaoIntake.PROVIDER_FORM_KEY                    # "form": a provider's form in Guatiao.provider_meta
```

Nothing is exported: write `GuatiaoIntake.check` (or `const Intake =
GuatiaoIntake`). Every schema, form, value and values argument is a
`Guatiao.Value`, a view or plain data, built by any guatiao library.
`at` and `resolve` answer a view borrowed from their argument: read-only,
found again on every access, `InvalidStateException` once the source is
closed; `copy` it to keep it. `unflatten` answers `nothing` for "nothing
stored" or a key that does not apply; a store that is not text to text is
an `ArgumentError`. A `Value` answered is built in the core library's
default allocator; `close` it. A refusal is the status's `GuatiaoError`.
