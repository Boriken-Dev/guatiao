# guatiao_serde (Dart) — API header

JSON, TOML and YAML for guatiao values, over the `guatiao_serde` C ABI:
https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships with the package, so it names no repo path.
Built on package:guatiao (`Value`, `Ref`, its errors,
`package:guatiao/ffi.dart`), at the same version. 32- and 64-bit native
platforms, and the web over the module the page loaded (below).

## Finding the native library

Importing touches nothing native. The first `loads` or `dumps` opens the
library, through package:guatiao's `openSurface('guatiao_serde')`:

1. a library registered with package:guatiao's `useLibrary` for
   `'guatiao_serde'` (with no `forSurfaces`, any library exporting
   `guatiao_json_parse` is registered for it);
2. `GUATIAO_SERDE_LIBRARY`: a file taken as it is, or a directory holding
   `guatiao_serde.dll` / `libguatiao_serde.so` / `libguatiao_serde.dylib`;
3. `GUATIAO_LIBRARY`: such a directory; or a file, used when it exports
   `guatiao_json_parse` (an application's own library carrying several
   surfaces), else the library beside it;
4. package:guatiao's `libraryDirectory`;
5. the platform's own search, by name.

Not found throws `LibraryNotFound` naming every route. `library()`
answers the opened `NativeLib`. The library carries the core too, so one
file can serve both packages.

## On the web

The same functions under dart2js and dart2wasm, through the one
WebAssembly module the page loaded with package:guatiao's `loadModule`:
every surface is called in that module, since a value lives in its
memory. The module must carry this surface (`guatiao_json_parse`): an
application's own crate built with it, or for forms
`guatiao_intake.wasm` itself. Without it `loads` and `dumps` throw `LibraryNotFound`.
`library()` answers the module (`WasmModule`), and every `alloc:` is an
`Address` (package:guatiao/wasm.dart) rather than a `Pointer`.

## Reading and writing

```dart
import 'package:guatiao_serde/guatiao_serde.dart' as serde;

final text = serde.dumps(value, format: serde.Format.json, pretty: true);
final value = serde.loads(text, format: serde.Format.json);
```

- `Value loads(String text, {Format format = Format.json, int how = 0,
  Pointer<guatiao_alloc>? alloc})`: a new tree in `alloc` (default the
  library's own allocator); `close()` it.
- `String dumps(Ref value, {Format format = Format.json, bool pretty =
  false, int how = 0, Pointer<guatiao_alloc>? alloc})`: any `Value` or
  `Ref`, built by any library. A TOML document must be a map
  (`WrongKind` otherwise). `pretty` indents JSON; other formats ignore it.
- `Format.json`, `.toml`, `.yaml`; `format.feature` is the build feature
  it needs (`null` for JSON). `Format.toml` / `Format.yaml` throw
  `UnsupportedError` naming the symbol and its feature when the loaded
  library was not built with it.
- `how`: `bytesDataUri`, `bytesBase64`, `bytesArray` or `bytesRefuse` for
  how bytes are written where the format has none, or'd with
  `readDataUris` (turn `data:;base64,...` strings back into bytes when
  reading) and `prettyFlag`.

A number keeps its exact text through JSON, with one exception the parser
owns: an exponent is read back with its sign written (`1e400` parses as
`1e+400`).

## Errors

A document that does not parse throws `ParseError`, which is a
`BadValue` of package:guatiao and a `FormatException`: `format`, `line`
and `column` (from 1, the column in characters; 0 when the parser did not
say where; for text that is not UTF-8, its first bad byte), and `message`,
which never quotes the document (`source` and `offset` are `null`). Any
other refusal throws the status's `GuatiaoException` with the library's
words.

## Isolates

Values are used from the isolate that made them, as in package:guatiao.

## Generated bindings

`lib/src/bindings.g.dart` is written by ffigen from `guatiao_serde.h`,
reusing package:guatiao's types through its symbol file, and is never
edited by hand.
