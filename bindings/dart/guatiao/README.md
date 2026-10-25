# guatiao for Dart

[![pub package](https://img.shields.io/pub/v/guatiao.svg)](https://pub.dev/packages/guatiao)
[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

Dart bindings for the [guatiao](https://github.com/Boriken-Dev/guatiao/)
ABI: **one value model for passing data between languages**, a JSON
Schema that describes a value, and a registry that loads plugin
libraries. Plain `dart:ffi` over `package:ffi`. It works with any shared
library that exports the ABI, whether that is `guatiao` itself or an
application that carries it.

> **Status: alpha.** A plain Dart package — no Flutter dependency, so it
> works both inside a Flutter app and in a command-line program. It
> carries no native library: its build hook builds one from source when
> an app builds, or you point it at one.

## Features

- **Value trees as Dart objects** — build, read and change a tree in
  place, or convert it to and from plain `Map`/`List` data.
- **Numbers keep their exact text** — `1.10` stays `1.10` and an integer
  of any size survives; you choose `int`/`double`, `BigInt` or `String`
  when reading.
- **Be a plugin host** — scan a directory, filter on what a library
  declares before loading it, list providers best first, build an
  instance from a configuration, call a provider through its table.
- **Kinds and providers as classes** — `dart run guatiao:kind` writes,
  from the description a kind crate commits beside its C header, an
  interface per kind and per appended slot and a remote class to call a
  table with; a provider is a class extending `Provider` that implements
  them, so the analyzer checks every method, and a `Library` serves it to
  any host in the process, Rust and C callers included. Nothing is
  declared by hand, and it runs under AOT and in Flutter.
- **Dates, times, instants and spans** — `PlainDate`, `PlainTime`,
  `PlainDateTime` and `Span`, a UTC `DateTime` for an instant and a
  `Duration` for an exact length, read and written by the library itself.
- **Values on the wire** — `value.toWire()` and `Value.fromWire(bytes)`
  carry a tree exactly as bytes, and `package:guatiao/wire.dart`
  announces a schema on a channel before its values, with a `Receiver`
  that checks each one.
- **Redaction** — a copy of a value with every field its schema marks
  secret hidden.
- **JSON, TOML and YAML**, and **forms checked against a schema**, are
  the packages [`guatiao_serde`](https://pub.dev/packages/guatiao_serde)
  and [`guatiao_intake`](https://pub.dev/packages/guatiao_intake), each
  over its own library and built on this one.
- **Importing never fails** — a native library is looked for on first
  use, and a missing one says every place that was searched.

## Platforms

Android, iOS, Linux, macOS and Windows through `dart:ffi`, 64-bit and
32-bit (`armeabi-v7a`, armhf Linux): the ABI's layout is written in
pointer widths. And **the web**, under dart2js and dart2wasm, through the
WebAssembly module the page loads, with the same API (see "On the web").
The package asks for Dart 3.10 or newer (Flutter 3.38).

## On the web

Build the module from guatiao's sources, serve it with the page, and load
it once before the first call:

```bash
cargo build -p guatiao-intake --target wasm32-unknown-unknown --release
```

```dart
await loadModule(Uri.parse('guatiao_intake.wasm'));
```

Every package uses that one module (`guatiao_serde` needs one built with
its surface: an application's own crate, see the
[docs](https://boriken-dev.github.io/guatiao/app-library/)). Values live
in its memory and are closed as on the VM. Under dart2js an `int` is
exact only to 2^53, so a larger integer reads back as a `BigInt`.

## Installation

```bash
flutter pub add guatiao        # dart pub add guatiao outside Flutter
```

The package drives a shared library that exports the guatiao ABI, and
**its build hook builds it**: `flutter build` (and `flutter run`, `dart
run`, `dart test`, `dart build`) runs cargo on guatiao's sources for the
target being built and bundles the library with the app. Nothing binary
is downloaded. `guatiao_serde` and `guatiao_intake` do the same for their
own libraries.

### One-time setup

[rustup](https://rustup.rs), and the Rust targets of the platforms you
build for. Nothing else beyond the platform's usual Flutter setup:

| building for | once, besides rustup |
| --- | --- |
| Windows, Linux | nothing: the machine's own target comes with rustup |
| Android | `rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android`; the NDK is the one Flutter's Android setup installs |
| macOS | `rustup target add aarch64-apple-darwin x86_64-apple-darwin` (a release app is universal); Xcode |
| iOS | `rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios`; Xcode |

A missing piece fails the build with one sentence naming it, for example
``The Rust target x86_64-linux-android is not installed: run `rustup
target add x86_64-linux-android`.`` The first build compiles the crates;
later ones reuse cargo's output, and a build with nothing changed skips
the hook. The hook finds cargo on `PATH` or in `~/.cargo/bin`.

### Your own library, or none

In the application's `pubspec.yaml`:

```yaml
hooks:
  user_defines:
    guatiao:
      crate: native/myapp   # your own cdylib crate, built instead: one file carrying every surface it links
      # build: false        # or build nothing, and say where the library is (below)
```

With `crate`, `guatiao_serde` and `guatiao_intake` build nothing either:
every surface comes from your file, which carries your own functions
too (`openSurface('guatiao').library` from `package:guatiao/ffi.dart`).
The [docs](https://boriken-dev.github.io/guatiao/app-library/) give the
recipe for the crate.

### Saying where the library is

Without a hook's library — `build: false`, or a program not built by
`dart`/`flutter` — say where it is, either with an environment variable:

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

`GUATIAO_LIBRARY` wins over the library a build hook bundled; without
either, the platform's own library search is tried. Nothing is loaded at
import time: importing always succeeds, and the first native
call throws `LibraryNotFound` naming every place it looked. A library
that exports only part of the ABI is fine: reaching for a function it
lacks throws `MissingSymbol` naming it, and everything else works.


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
  reg.collect();                                   // ['acme']: closed, once nothing of it is alive
  reg.unload('acme2');                             // out and unmapped at once, when it agrees

  reg.addSourceFile('/opt/myapp/guatiao_source_py.so', {'dirs': ['/opt/myapp/py']});  // a source
  reg.watchDir('/opt/myapp/plugins', rules: 'kind=greeter');
  final changed = reg.refresh();                   // loaded, retired, skipped, refused
} finally {
  reg.close();
}
```

Every answer that is data comes back as a plain `Map` or `List`, already
freed. `rules` is one `[!]KEY=VALUE` per line and filters on what a
library declares about itself, before it is loaded.

### Kinds and providers as classes

A kind crate commits its kinds as data beside its C header
(`greeter_kind.kinds.json`). Write the Dart for them once, and commit it:

```bash
dart run guatiao:kind greeter_kind.kinds.json -o lib/greeter_kind.dart
```

Each kind is then an interface of its required slots (`Greeter`), one per
appended slot (`GreeterShout`), a remote class a caller holds
(`GreeterRemote`), and the kind itself as the interface's static `kind`:

```dart
import 'package:guatiao/kinds.dart';
import 'greeter_kind.dart';

final greeter = reg.bind('acme_hello', Greeter.kind);   // a GreeterRemote
final answer = greeter.greet('ana');                    // a Value
if (greeter.has('shout')) print(greeter.shout('ana'));

final class Hello extends Provider implements Greeter, GreeterShout {
  @override
  Map<String, Object?> greet(String name) => {'greeting': 'hello, $name'};
  @override
  String shout(String name) => 'HELLO, ${name.toUpperCase()}';
}

final library = Library('my_greeters', '1.0.0', offers: [
  Offer.single(Hello(), named: 'hello', kinds: [Greeter.kind]),
]);
reg.registerLibrary(library);   // my_greeters_hello, for any host in the process
```

The analyzer refuses a provider that lacks a required method or types
one wrongly. `dart run guatiao:kind identity` writes the library's id and
version from `pubspec.yaml`, for `Library.of(packageLibrary, ...)`. In a
Flutter app, import the generated file with a prefix: Flutter has a
`Listener` of its own.

Under that, `kindTable<T>(found, floorHash:, floorSize:)` casts a
`providerTable` answer to a `Struct` a kind's C header declares, checked.

### Reading a value you were handed

When an engine hands you a value, as in a callback, read it in place
rather than copying it:

```dart
final view = Ref.borrowed(pointer);   // read-only; you keep the memory alive
view.toDart();
```

It touches no library and no global state, so it is safe inside an
`isolateGroupBound` callback that a native thread calls.

## Embedding it, and sharing its types

**One library may carry several surfaces.** An application that links the
ABI into its own shared library exports `guatiao_*` from that one file,
under its own name. Register it, rather than trying to describe it with a
path:

```dart
import 'dart:ffi';
import 'package:guatiao/guatiao.dart';

useLibrary(DynamicLibrary.open('/opt/myapp/myapp.dll'));
```

With no `forSurfaces`, it is registered for each surface it actually
exports and nothing else. It is also the only way in where the symbols
are already in the process and there is no file to open, as in a
statically linked build: `useLibrary(DynamicLibrary.process())`.

**Another package's bindings can reuse these types.** Dart's FFI structs
are nominal, so a `guatiao_value` generated by a second `ffigen` run is a
different type from this one and the two cannot be passed between
packages. This package ships a symbol file that says otherwise:

```yaml
# the other package's ffigen.yaml
import:
  symbol-files:
    - 'package:guatiao/ffi_symbols.yaml'
```

Its generated bindings then import these types from
`package:guatiao/ffi.dart` instead of minting their own, and a pointer
crosses between the two packages freely. `package:guatiao/ffi.dart` is
also what such a package builds on: `openSurface` finds and opens its
library, `Value.adopt` takes over a tree a call wrote, and `strOf`,
`takeValue`, `freeNode` and `checkStatus` do the rest. `guatiao_serde`
and `guatiao_intake` are built that way.

### Dates, times and secrets

```dart
import 'package:guatiao/guatiao.dart';
import 'package:guatiao/time.dart' as time;

time.parseTimestamp('2026-10-21T19:15:00+05:30');      // a UTC DateTime
time.PlainDate.parse('2026-01-31').add(time.Span(months: 1));   // 2026-02-28
time.toDuration([90, 500000000]);                      // 90.5 seconds

final shown = redact(schema, value);                   // secrets hidden
```

Text a form's grammar refuses throws `Unparsed`, a `FormatException`
carrying the byte offset and what was expected there.

## Errors

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
  A provider or object written in Dart runs on the thread of the isolate
  that built it: a library calling one from another thread ends the
  process, as Dart does for any isolate-local callback.
- `Registry.unload` unmaps a library once its own `unload` function
  agrees, and `collect` a retired one; neither can see a `ProviderTable`
  you took. Let it go first: it is valid until the library is closed,
  by `unload`, or once retired by `collect` or the registry's next load,
  retire or unload.
- `DateTime` and `Duration` count microseconds: reading an instant or
  an exact length into one drops what is below. The `Plain` classes and
  `Span` keep nanoseconds.
- On the web there is no registry: `Registry` and `kindTable` throw
  `Unsupported`, since a browser maps no libraries, and
  `package:guatiao/kinds.dart` is native only.

## API overview

| Library | Purpose |
| --- | --- |
| `package:guatiao/guatiao.dart` | `Value`, `Ref`, `MapRef`, `ListRef`, `absent`, `Registry`, `Instance`, `kindTable`, the error classes |
| `package:guatiao/kinds.dart` | kinds and providers as classes: `Kind`, `KindRemote`, `Provider`, `Offer`, `Library`, `LibraryIdentity`, `Disposable`, `liveObjects` |
| `dart run guatiao:kind` | writes a kind description's Dart, and a library identity from `pubspec.yaml` |
| `package:guatiao/kind_glue.dart` | what the generated code calls: `CallScope`, `ServeScope` |
| `package:guatiao/ffi.dart` | for a package over another guatiao library: the generated bindings, `openSurface`, `NativeLib`, `strOf`, `takeValue`, `freeNode`, `checkStatus` |
| `package:guatiao/wasm.dart` | the same on the web: `WasmModule`, `Address`, `withScratch`, `openSurface`, `takeValue`, `freeNode`, `checkStatus` |
| `package:guatiao/time.dart` | `PlainDate`, `PlainTime`, `PlainDateTime`, `Span`, `parseTimestamp`, `formatTimestamp`, `toDuration`, `fromDuration`, `normalize` |
| `package:guatiao/wire.dart` | channel frames (`announce`, `send`, `close`), `Receiver`, `Event`, the refusals |
| `package:guatiao/hook.dart` | for a package's build hook: `buildHook`, `buildLibrary`, `rustTarget`, `HookFailure` |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/dart/guatiao/AGENTS.md)
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

The tests need the library and the repository's example plugins in one
directory. `test/support.dart` points the suite at the
repository's build output when `GUATIAO_LIBRARY` is unset; the
repository's own `AGENTS.md` says how to produce it. The package's own
`pubspec.yaml` tells its hook to build nothing (`build: false`), so a
suite runs against that build; `test/hook_test.dart` runs the hook
itself, cargo builds included. Nothing skips.

`lib/src/bindings.g.dart` is generated and never edited by hand:

```bash
dart run ffigen --config ffigen.yaml
```

**Check that the result is not empty.** ffigen does not fail on an entry
point it cannot find — it reports `Input Headers: []` and writes a file
with no bindings, deleting the whole surface silently. The file is over
two thousand lines.

It is generated from `guatiao.h` alone, and also writes
`lib/ffi_symbols.yaml`, the symbol file other packages import.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
