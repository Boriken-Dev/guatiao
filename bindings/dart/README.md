# guatiao for Dart

[![pub package](https://img.shields.io/pub/v/guatiao.svg)](https://pub.dev/packages/guatiao)
[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

Dart bindings for the [guatiao](https://github.com/Boriken-Dev/guatiao/)
ABI: **one value model for passing data between languages**, a JSON
Schema that describes a value, and a registry that loads plugin
libraries. Plain `dart:ffi` over `package:ffi`, with nothing to compile.
It works with any shared library that exports the ABI, whether that is
`guatiao` itself or an application that carries it.

> **Status: alpha.** A plain Dart package — no Flutter dependency, so it
> works both inside a Flutter app and in a command-line program. It
> carries no native library; you point it at one.

## Features

- **Value trees as Dart objects** — build, read and change a tree in
  place, or convert it to and from plain `Map`/`List` data.
- **Numbers keep their exact text** — `1.10` stays `1.10` and an integer
  of any size survives; you choose `int`/`double`, `BigInt` or `String`
  when reading.
- **Be a plugin host** — scan a directory, filter on what a library
  declares before loading it, list providers best first, build an
  instance from a configuration, call a provider through its table.
- **JSON, TOML and YAML**, and **form checks against a schema**, through
  the `guatiao_serde` and `guatiao_form` libraries when they are present.
- **Importing never fails** — a native library is looked for on first
  use, and a missing one says every place that was searched.

## Installation

```bash
dart pub add guatiao
```

The package drives a shared library that exports the guatiao ABI. Say
where it is, either with an environment variable:

```bash
export GUATIAO_LIBRARY=/opt/myapp/lib         # a directory holding guatiao.dll / libguatiao.so / libguatiao.dylib
export GUATIAO_LIBRARY=/opt/myapp/myapp.dll   # or the file itself, whatever it is called
```

or, since a process cannot set its own environment, from the code that
knows where its libraries landed:

```dart
import 'package:guatiao/guatiao.dart' as guatiao;

guatiao.libraryDirectory = await resolveBundledLibraryDir();
```

Without either, the platform's own library search is tried. Nothing is
loaded at import time: importing always succeeds, and the first native
call throws `LibraryNotFound` naming every place it looked. A library
that exports only part of the ABI is fine: reaching for a function it
lacks throws `MissingSymbol` naming it, and everything else works.

| Exports from | Needed for |
| --- | --- |
| `guatiao` | values, the registry, provider tables |
| `guatiao_serde` | `package:guatiao/serde.dart`: JSON, and TOML and YAML when the library has them |
| `guatiao_form` | `package:guatiao/form.dart` |

The two optional libraries are looked for the same way, in the same
directory, only when `serde` or `form` is first used.

## Quick start

### Values

```dart
import 'package:guatiao/guatiao.dart';

final v = Value.fromDart({'host': '10.0.0.1', 'port': 5900, 'tags': ['a', 'b']});
try {
  final m = v.asMap();
  m['port'] = 5901;
  m['tags']!.asList().add('c');

  m['port']!.toDart();      // 5901
  m.keys;                   // ['host', 'port', 'tags'], insertion order
  m['missing'];             // null
  m['port']!.intOr(22);     // 5901, or 22 if it were not an integer
  v.toDart();               // back to a plain Map
} finally {
  v.close();
}
```

A `Value` owns its tree and frees it on `close()`, which is required: a
`NativeFinalizer` is attached as a backstop, but it runs at the garbage
collector's convenience. `m['tags']` is a `Ref`: a path into the tree
that is walked again on every access, so it stays valid while the tree
grows around it.

**Numbers keep their exact text.** `1.10` stays `1.10`, and an integer of
any size survives. Choose what comes back when reading:

```dart
final v = Value.number('1.10');
v.toDart();                          // 1.1   (Numbers.auto)
v.toDart(numbers: Numbers.text);     // '1.10'
v.toDart(numbers: Numbers.bigInt);   // 1.1 here; a BigInt for integral text
```

`Value.fromDart` takes what you would expect: `null` becomes null, a
`bool` a boolean, an `int`, `double` or `BigInt` a number, a `String` a
string, a `TypedData` the bytes kind, a `List` a list, and a `Map` with
`String` keys a map. A `Value` or `Ref` is deep-copied in. A missing
value reads back as `absent`, which is not `null`. A `double` that is
`nan` or infinite is an `ArgumentError`.

### Hosting plugins

```dart
import 'package:guatiao/guatiao.dart';

final reg = Registry('my-host', '1.0');
try {
  reg.scanDir('/opt/myapp/plugins', rules: 'kind=greeter');
  for (final p in reg.providers('greeter')) {      // best first
    print('${p['key']} ${p['version']} ${p['from']}');
  }

  reg.whyNot('codec');                             // why nothing serves a kind

  final schema = reg.providerConfig('acme_shouter');   // borrowed; read only
  final instance = reg.create('acme_shouter', {'prefix': 'hey'});
  instance.close();

  reg.retire('acme');                              // out of the registry, still mapped
  reg.unload('acme');                              // and unmapped, on your word
} finally {
  reg.close();
}
```

Every answer that is data comes back as a plain `Map` or `List`, already
freed. `rules` is one `[!]KEY=VALUE` per line and filters on what a
library declares about itself, before it is loaded.

To call a provider, describe its table with the `Struct` the kind's C
header declares and let the package check it:

```dart
final found = reg.providerTable('acme_greeter', kind: 'greeter');
final greeter = kindTable<GreeterVtable>(
  found,
  floorHash: greeterVtableFloorHash,
  floorSize: sizeOf<GreeterVtable>(),
);
greeter.ref.greet.asFunction<GreetDart>()(found.ctx, name, outMap, err);
```

[`test/greeter_test.dart`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/dart/test/greeter_test.dart)
is a complete worked example.

### JSON, TOML, YAML and forms

```dart
import 'package:guatiao/guatiao.dart';
import 'package:guatiao/serde.dart' as serde;
import 'package:guatiao/form.dart' as form;

final value = serde.loads('{"port": 5900}');            // Format.json by default
final text = serde.dumps(value, format: Format.yaml);
final pretty = serde.dumps(value, pretty: true);

final problem = form.check(schema, formDoc);            // null when the form fits
final sections = form.layout(schema, formDoc);
final shown = form.isVisible(schema, formDoc, 'tls.verify', values);
```

A format the loaded library does not export throws `UnsupportedError`
naming the symbol and the feature it belongs to.

### Errors

A failed native call throws `GuatiaoException`, as one of `BadValue`,
`AllocFailed`, `WrongKind`, `NotFound`, `NullArgument`, `Gone`,
`Unsupported`, `Busy` or `InternalError`. `code` is the raw C status,
`status` the `Status` it names, and `message` carries a provider's own
words when it gave any.

## Known limits

- `close()` is required on a `Value`, a `Registry` and an `Instance`. The
  `NativeFinalizer` on a dropped `Value` releases its tree but not the
  40-byte root box: a finalizer runs one native function, and those two
  frees have to happen in order.
- `ListRef.insert` and `list[i] = x` rebuild the list, because the C ABI
  only appends and removes. `add` and `removeAt` are direct.
- A `Value` and a `Registry` are used from the isolate that made them.
  Nothing here crosses an isolate boundary, and a provider cannot call
  back into Dart.
- `Registry.unload` unmaps a library, and nothing can check that you have
  released what you took from it first — every value it built through its
  own allocator, every table, `ctx` and `Instance`. `retire` has no such
  condition and leaves the library mapped.
- 64-bit platforms only: every pointer and `size_t` in the ABI is eight
  bytes.

## API overview

| Library | Purpose |
| --- | --- |
| `package:guatiao/guatiao.dart` | `Value`, `Ref`, `MapRef`, `ListRef`, `absent`, `Registry`, `Instance`, `kindTable`, the error classes |
| `package:guatiao/serde.dart` | `loads` / `dumps` for JSON, TOML, YAML |
| `package:guatiao/form.dart` | `check`, `layout`, `isVisible` |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/dart/AGENTS.md)
is the full API reference and ships with the package. The C ABI is
described by
[`guatiao.h`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao/include/guatiao.h).

## Development

```bash
dart pub get
dart analyze --fatal-infos
dart format --output=none --set-exit-if-changed .
dart test
```

The tests need the three libraries and the repository's example plugins
in one directory. `test/support.dart` points the suite at the
repository's build output when `GUATIAO_LIBRARY` is unset; the
repository's own `AGENTS.md` says how to produce it. Nothing skips.

`lib/src/bindings.g.dart` is generated and never edited by hand:

```bash
dart run ffigen --config ffigen.yaml
```

**Check that the result is not empty.** ffigen does not fail on an entry
point it cannot find — it reports `Input Headers: []` and writes a file
with no bindings, deleting the whole surface silently. The file is over
two thousand lines.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
