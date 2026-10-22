# guatiao-intake (Python) — API header

Taking a value in from a person, over the `guatiao_intake` C ABI:
https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships inside the wheel, so it names no repo path.
Built on the `guatiao` package (`guatiao.Value`, `Ref`, its errors,
`guatiao.ffi`); the same version of both.

## Finding the native library

Importing `guatiao_intake` touches no native library. The first call
that needs it finds it, in this order:

1. `guatiao_intake.use_library(path)`, before that first call: a file
   whatever it is called, or a directory holding `guatiao_intake.dll` /
   `libguatiao_intake.so` / `libguatiao_intake.dylib`. `RuntimeError`
   when the library is already loaded from elsewhere.
2. `GUATIAO_INTAKE_LIBRARY`: a file, or such a directory.
3. `GUATIAO_LIBRARY`: such a directory; or a file, used when it exports
   `guatiao_intake_check` (an application's own library carrying several
   surfaces), else the library beside it.
4. `ctypes.util.find_library("guatiao_intake")`.

Not found raises `guatiao.LibraryNotFound` naming every place tried.
`library()` answers the loaded `guatiao.ffi.Library`. The core `guatiao`
library is found by `guatiao`'s own rules (`GUATIAO_LIBRARY`); the
`guatiao_intake` library carries the core too, so one file can serve both.

## Forms

```python
import guatiao_intake as intake

intake.check(schema, form_doc)             # None, or an error map
intake.layout(schema, form_doc)            # [{"section": ..., "fields": [...]}, ...]
intake.is_visible(schema, form_doc, "key", values)   # bool
intake.at(value, "agent[1].name")          # a borrowed Ref, or None
intake.for_schema(schema)                  # the form a schema implies (a Value)
intake.form_for(form_doc, schema, "path")  # a Value, or None
```

- `check(schema, form, *, alloc=None)`: `None` when `form` fits
  `schema`; otherwise the error map `guatiao_intake_check` writes (`kind`,
  `at`, and whichever of `path`/`id`/`field`/`expected`/`message` apply).
- `layout(schema, form, *, alloc=None)`: the schema's fields grouped into
  sections and put in order.
- `is_visible(schema, form, key, values) -> bool`: whether the field
  under `key` is shown, given `values` entered so far.
- `at(value, path) -> Ref | None`: the place `path` names. A bracket is a
  position or a map key by what it is applied to (`env[PATH]` needs no
  quoting; `env["1"]` is a key). `None` for a path that names nothing or
  is not a path.
- `for_schema(schema, *, alloc=None) -> Value`: one section per distinct
  `x-section`, ids only, plus each member's own form; `{}` when the
  schema groups nothing.
- `form_for(form, schema, path, *, alloc=None) -> Value | None`: the
  form for the field at `path`, assigned or implied; `None` when the path
  names no field, or one with no members and no form.

## The flat store

A value kept as `key -> text`, one entry per scalar leaf (a tagged
field's discriminant under its key and its payload under `key.member`, a
list as `agent[0].name`, an open map as `env[PATH]`):

```python
intake.resolve(schema, "auth.password")    # borrowed Ref onto the field, or None
intake.flat_keys(schema, "auth")           # list[str]; WrongKind for an unknown key
intake.flatten(schema, "auth", value)      # dict[str, str]; WrongKind when it does not apply
intake.unflatten(schema, "auth", store)    # Value (close it), or None; store is a
                                           # Mapping[str, str] or a Value/Ref onto one
```

An empty container flattens to nothing, and an entry the schema does not
declare is dropped by `unflatten`. `unflatten` answers `None` only for
"nothing stored": no such field, no entry under it, or a discriminant
that names no arm. A store that is not text to text raises `TypeError`.

## Arguments and answers

Every `schema`, `form`, `value` and `values` argument is a
`guatiao.Value` or `Ref`, built by any guatiao library. What `resolve`
and `at` return is a read-only view: it keeps the tree it came from
alive, is found again on every access, and raises `ValueError` once that
tree is closed; any write is `TypeError`, and `.clone()` copies it out.
A `Value` answered is built in `alloc` (a `guatiao.ffi.Alloc`; default
the library's own allocator); close it. A refusal raises the status's
`guatiao.GuatiaoError` subclass (`WrongKind`, `BadValue`, ...).

## A provider's form

A provider's form travels in its `meta`, under `PROVIDER_FORM_KEY`
(`"form"`), beside the configuration schema it is shown with:

```python
with reg.provider_meta("acme_dialer") as meta:   # None when it declares no meta
    form = meta.as_map()[intake.PROVIDER_FORM_KEY]
    schema = reg.provider_config("acme_dialer")
    assert intake.check(schema, form) is None
    intake.layout(schema, form)
```
