# guatiao_serde for Dart

[![pub package](https://img.shields.io/pub/v/guatiao_serde.svg)](https://pub.dev/packages/guatiao_serde)
[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

JSON, TOML and YAML for [guatiao](https://github.com/Boriken-Dev/guatiao/)
values: read a document into a `Value`, write one back out. Plain
`dart:ffi` over the `guatiao_serde` library, built on the
[`guatiao`](https://pub.dev/packages/guatiao) package for the value model
and its errors.

> **Status: alpha.** A plain Dart package — no Flutter dependency. It
> carries no native library; you point it at one.

## Features

- **Three formats, one call each** — `loads` and `dumps` for JSON, TOML
  and YAML, parsed and written by the library itself, so every language
  bound to guatiao reads a document the same way.
- **Numbers keep their exact text** — `1.10` stays `1.10`, and an integer
  of any size survives the trip.
- **Bytes have a spelling** — a data URI, base64 or a list of numbers
  where the format has none, and data URIs read back as bytes when asked.
- **A document that does not parse says where** — `ParseError` carries
  the format, line and column, and never quotes the document.
- **Any guatiao value** — what `loads` returns is an ordinary `Value` of
  package:guatiao, and `dumps` takes any `Ref`, whichever library built it.

## Platforms

Android, iOS, Linux, macOS and Windows, 64-bit and 32-bit, as
package:guatiao; and **the web**, through the one WebAssembly module the
page loads with package:guatiao's `loadModule`, which must carry this
package's surface. The package asks for Dart 3.6 or newer.

## Installation

```bash
dart pub add guatiao_serde
```

The package drives the `guatiao_serde` shared library. Say where it is:

```bash
export GUATIAO_SERDE_LIBRARY=/opt/myapp/lib         # a directory holding guatiao_serde.dll / libguatiao_serde.so / .dylib
export GUATIAO_SERDE_LIBRARY=/opt/myapp/myapp.dll   # or the file itself, whatever it is called
```

Without it, `GUATIAO_LIBRARY` is tried: a directory holding the library,
or a file that exports its functions (an application's own library
carrying several guatiao surfaces), or a file it sits beside. Then
package:guatiao's `libraryDirectory`, then the platform's search. A
library already open is registered with package:guatiao's
`useLibrary(DynamicLibrary.open(path))`. Nothing is loaded at import
time; the first call throws `LibraryNotFound`, naming every place it
looked.

## Quick start

```dart
import 'package:guatiao/guatiao.dart';
import 'package:guatiao_serde/guatiao_serde.dart' as serde;

final value = serde.loads('{"port": 5900, "tags": ["a"]}');   // Format.json by default
value.asMap()['port']!.toDart();                              // 5900
final text = serde.dumps(value, format: serde.Format.yaml);
final pretty = serde.dumps(value, pretty: true);
value.close();

try {
  serde.loads('{"port": }');
} on serde.ParseError catch (error) {
  print('${error.format.name} ${error.line}:${error.column}');
}
```

A format the loaded library was built without throws `UnsupportedError`
naming the symbol and its feature. A TOML document must be a map.

## Known limits

- Reading JSON writes an exponent's sign: `1e400` parses as `1e+400`.
  Everything else about a number's text survives a round trip.
- On the web, the page's one module, which must carry this surface.

## API overview

| Name | Purpose |
| --- | --- |
| `loads(text, {format, how, alloc})` | a document as a `Value` |
| `dumps(value, {format, pretty, how, alloc})` | a `Value` or `Ref` as text |
| `Format`, `ParseError` | the formats, and a document that does not parse |
| `bytesDataUri`, `bytesBase64`, `bytesArray`, `bytesRefuse`, `readDataUris`, `prettyFlag` | the `how` flags |
| `library()` | the loaded `guatiao_serde` |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/dart/guatiao_serde/AGENTS.md)
is the full API reference and ships with the package. The C ABI is
described by
[`guatiao_serde.h`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao-serde/include/guatiao_serde.h).

## Development

```bash
dart pub get
dart analyze --fatal-infos
dart format --output=none --set-exit-if-changed .
dart test
```

`pubspec_overrides.yaml` takes package:guatiao from the checkout beside
it; it is not published. `test/support.dart` points the suite at the
repository's build output when `GUATIAO_LIBRARY` is unset; the
repository's own `AGENTS.md` says how to produce it.

`lib/src/bindings.g.dart` is generated from `guatiao_serde.h` and never
edited by hand: `dart run ffigen --config ffigen.yaml`, after `dart pub
get` (it reads package:guatiao's symbol file). Check the result is not
empty: ffigen writes an empty file for an entry point it cannot find.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
