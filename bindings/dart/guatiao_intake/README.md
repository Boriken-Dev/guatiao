# guatiao_intake for Dart

[![pub package](https://img.shields.io/pub/v/guatiao_intake.svg)](https://pub.dev/packages/guatiao_intake)
[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

Taking a value in from a person, for
[guatiao](https://github.com/Boriken-Dev/guatiao/) values: the path
grammar (`agent[1].name`), whether a form fits a schema and how it is laid
out, which fields are shown, and the flat `key -> text` store a front end
keeps. Plain `dart:ffi` over the `guatiao_intake` library, built on the
[`guatiao`](https://pub.dev/packages/guatiao) package for the value model
and its errors.

> **Status: alpha.** A plain Dart package — no Flutter dependency. It
> carries no native library; you point it at one.

## Features

- **Forms over a schema** — check a form document against the schema it
  shows, lay its fields out in sections, decide which are visible given
  what was entered, or take the form a schema implies when nobody wrote
  one.
- **A path names a place in a value** — `agent[1].name`, `env[PATH]`,
  read as a borrowed view of the tree you passed.
- **The flat store** — a value as `key -> text` and back, the way a front
  end keeps what a person typed, with tagged fields, lists and open maps.
- **A provider's form** — read from its `meta` under `providerFormKey`
  and checked against its configuration schema.
- **Any guatiao value** — every argument is a `Ref` of package:guatiao,
  a borrowed one included, and every answer an ordinary `Value`.

## Platforms

Android, iOS, Linux, macOS and Windows, 64-bit and 32-bit, as
package:guatiao; and **the web**, through the one WebAssembly module the
page loads with package:guatiao's `loadModule`, which must carry this
package's surface. The package asks for Dart 3.10 or newer (Flutter
3.38).

## Installation

```bash
dart pub add guatiao_intake
```

The package drives the `guatiao_intake` shared library, and **its build hook
builds it**: `flutter build` (or `dart run`, `dart test`, `dart build`)
runs cargo for the target being built and bundles the library with the
app. Nothing binary is downloaded. The one-time setup is
[package:guatiao's](https://pub.dev/packages/guatiao#installation):
rustup and the Rust targets. An application whose own crate carries this
surface names that crate to package:guatiao's hook, and this package then
builds nothing.

Without a hook's library (package:guatiao's `build: false`), say where it
is:

```bash
export GUATIAO_INTAKE_LIBRARY=/opt/myapp/lib         # a directory holding guatiao_intake.dll / libguatiao_intake.so / .dylib
export GUATIAO_INTAKE_LIBRARY=/opt/myapp/myapp.dll   # or the file itself, whatever it is called
```

Without it, `GUATIAO_LIBRARY` is tried: a directory holding the library,
or a file that exports its functions (an application's own library
carrying several guatiao surfaces), or a file it sits beside. Then the
library a build hook bundled, then package:guatiao's `libraryDirectory`,
then the platform's search. A
library already open is registered with package:guatiao's
`useLibrary(DynamicLibrary.open(path))`. Nothing is loaded at import
time; the first call throws `LibraryNotFound`, naming every place it
looked.

## Quick start

```dart
import 'package:guatiao/guatiao.dart';
import 'package:guatiao_intake/guatiao_intake.dart' as intake;

final problem = intake.check(schema, formDoc);          // null when the form fits
final sections = intake.layout(schema, formDoc);
final shown = intake.isVisible(schema, formDoc, 'tls.verify', values);
final name = intake.at(value, 'agent[1].name');         // a borrowed Ref, or null

// A front end that keeps text under flat keys (`agent[0].name`, `env[PATH]`):
final field = intake.resolve(schema, 'auth.password');  // the schema field, or null
final keys = intake.flatKeys(schema, 'auth');           // ['auth', 'auth.username', ...]
final store = intake.flatten(schema, 'auth', authValue);// {'auth': 'userpass', ...}
final back = intake.unflatten(schema, 'auth', store);   // a Value to close, or null

final meta = registry.providerMeta('acme_dialer')!;     // a provider's form
final form = meta.asMap()[intake.providerFormKey]!;
intake.check(registry.providerConfig('acme_dialer')!, form);   // null: it fits
meta.close();
```

## Known limits

- What `at` and `resolve` answer borrows from the tree it was found in:
  read-only, and refused once that tree is closed. `.clone()` copies it
  out.
- On the web, the page's one module, which must carry this surface.

## API overview

| Name | Purpose |
| --- | --- |
| `check`, `layout`, `isVisible`, `forSchema`, `formFor` | a form over a schema |
| `at` | a path's place in a value |
| `resolve`, `flatKeys`, `flatten`, `unflatten` | the flat `key -> text` store |
| `providerFormKey` | where a provider's `meta` carries its form |
| `library()` | the loaded `guatiao_intake` |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/dart/guatiao_intake/AGENTS.md)
is the full API reference and ships with the package. The C ABI is
described by
[`guatiao_intake.h`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao-intake/include/guatiao_intake.h).

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

`lib/src/bindings.g.dart` is generated from `guatiao_intake.h` and never
edited by hand: `dart run ffigen --config ffigen.yaml`, after `dart pub
get` (it reads package:guatiao's symbol file). Check the result is not
empty: ffigen writes an empty file for an entry point it cannot find.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
