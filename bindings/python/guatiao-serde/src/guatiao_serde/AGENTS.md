# guatiao-serde (Python) — API header

JSON, TOML and YAML for guatiao values, over the `guatiao_serde` C ABI:
https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships inside the wheel, so it names no repo path.
Built on the `guatiao` package (`guatiao.Value`, its errors, `guatiao.ffi`);
the same version of both.

## Finding the native library

Importing `guatiao_serde` touches no native library. The first `loads`
or `dumps` finds it, in this order:

1. `guatiao_serde.use_library(path)`, before that first call: a file
   whatever it is called, or a directory holding `guatiao_serde.dll` /
   `libguatiao_serde.so` / `libguatiao_serde.dylib`. `RuntimeError` when
   the library is already loaded from elsewhere.
2. `GUATIAO_SERDE_LIBRARY`: a file, or such a directory.
3. `GUATIAO_LIBRARY`: such a directory; or a file, used when it exports
   `guatiao_json_parse_ex` (an application's own library carrying several
   surfaces), else the library beside it.
4. `ctypes.util.find_library("guatiao_serde")`.

Not found raises `guatiao.LibraryNotFound` naming every place tried.
`library()` answers the loaded `guatiao.ffi.Library`. The core `guatiao`
library is found by `guatiao`'s own rules (`GUATIAO_LIBRARY`); the
`guatiao_serde` library carries the core too, so one file can serve both.

## Reading and writing

```python
import guatiao_serde as serde

value = serde.loads(text, format="json")                # or "toml", "yaml"
text = serde.dumps(value, format="json", pretty=True)
```

- `loads(text, *, format="json", how=0, alloc=None) -> guatiao.Value`: a
  new tree, built in `alloc` (a `guatiao.ffi.Alloc`; default the
  library's own allocator); close it.
- `dumps(value, *, format="json", pretty=False, how=0, alloc=None) ->
  str`: `value` is a `guatiao.Value` or `Ref`, built by any library. A
  TOML document must be a map (`guatiao.WrongKind` otherwise). `pretty`
  indents JSON; other formats ignore it.
- `how`: one of `BYTES_DATA_URI` (0), `BYTES_BASE64` (1), `BYTES_ARRAY`
  (2), `BYTES_REFUSE` (3) for how bytes are written where the format has
  none, or'd with `READ_DATA_URIS` (turn `data:;base64,...` strings back
  into bytes when reading) and `PRETTY`.
- `toml`/`yaml` raise `NotImplementedError` naming the feature when the
  loaded library was not built with it. An unknown format is
  `ValueError`.

A number keeps its exact text through JSON, with one exception the parser
owns: an exponent is read back with its sign written (`1e400` parses as
`1e+400`).

## Errors

A document that does not parse raises `guatiao_serde.ParseError`: a
`guatiao.BadValue` and a `ValueError` with `.format` (`"json"`, `"toml"`
or `"yaml"`), `.line` and `.column` (from 1, the column in characters; 0
when the parser did not say where; for text that is not UTF-8, its first
bad byte) and `.message`, which never quotes the document. It pickles.
Any other refusal raises the status's `guatiao.GuatiaoError` subclass
carrying the library's words.

## Threading

Calls take no lock of their own; a `guatiao.Value` is not shared between
threads without the caller's own lock.
