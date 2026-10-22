# guatiao-serde (Node.js) — API header

JSON, TOML and YAML for guatiao values, over the `guatiao_serde` C ABI:
https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships inside the npm package, so it names no
repository path.

ESM only, typed, Node 22 or later, 64-bit platforms. One dependency,
`guatiao`, at the same version: its `Value`, errors and koffi
(`guatiao/ffi`). This package ships no native code and loads no koffi of
its own.

## Finding the native library

Importing touches nothing native. The first call that needs it opens the
library, in this order:

1. `useLibrary(pathOrUrl)`, when called before that first call: a file
   exporting its functions, whatever it is called, or a directory.
   Naming another file once one is loaded throws; naming the same one
   does nothing.
2. `GUATIAO_SERDE_LIBRARY`: a file, or a directory holding
   `guatiao_serde.dll` / `libguatiao_serde.so` / `libguatiao_serde.dylib`.
3. `GUATIAO_LIBRARY`: such a directory; or a file, used when it exports
   `guatiao_json_parse_ex` (an application's own library carrying several
   surfaces), else the library beside it.
4. The platform's own library search, by file name.

Not found throws `LibraryNotFound` (of `guatiao`) naming each route.
`library()` answers the loaded `Surface`. The library carries the core
too, so one file can serve both packages.

## Reading and writing

```ts
import * as serde from 'guatiao-serde';

const text = serde.dumps(value, { format: 'json', pretty: true });  // or 'toml', 'yaml'
using parsed = serde.loads(text, { format: 'json' });
```

- `loads(text, { format = 'json', how = 0, alloc? }) -> Value`: a new
  tree, built in `alloc` (the address of a `guatiao_alloc`; default the
  library's own), that the caller closes.
- `dumps(value, { format = 'json', pretty = false, how = 0 }) -> string`:
  `value` is a `Ref` of `guatiao` (built by any library) or plain data. A
  TOML document must be a map (`WrongKind`).
- `how`: `BYTES_DATA_URI`, `BYTES_BASE64`, `BYTES_ARRAY` or
  `BYTES_REFUSE` for how bytes are written where the format has none,
  or'd with `READ_DATA_URIS` (turn `data:;base64,...` strings back into
  bytes when reading) and `PRETTY`.
- `toml`/`yaml` throw `MissingSymbol` naming the feature when the loaded
  library lacks it; an unknown format is a `TypeError`.

A number keeps its exact text, except that an exponent is read back with
its sign (`1e400` is `1e+400`).

## Errors

A document that does not parse throws `ParseError`, a `BadValue` of
`guatiao`, with `.format`, `.line` and `.column` (from 1, the column in
characters; 0 when the parser did not say) and `.reason`, which never
quotes the document. Any other refusal throws the status's
`GuatiaoError` with the library's words.

## Threads

Everything runs on Node's main thread, as in `guatiao`.
