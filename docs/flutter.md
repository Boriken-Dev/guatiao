# Dart and Flutter

The Dart packages (`guatiao`, `guatiao_serde`, `guatiao_intake`) build the
libraries they call from guatiao's own sources. An application runs its
usual build and nothing else:

```bash
flutter build apk        # or appbundle, ios, linux, macos, windows; flutter run; dart run
```

Each package's build hook runs cargo for the target being built (on
Android, once per ABI) and the build bundles the library with the app:
beside the executable on Windows, in `lib/` of the Linux bundle, in the
APK's `lib/<abi>/`, as a framework on Apple's platforms. Nothing binary
is downloaded. The packages find what their hook bundled on their own; no
path is configured.

## One-time setup

[rustup](https://rustup.rs) and the Rust targets of the platforms you build
for, beside the platform's usual Flutter setup:

| building for | once, besides rustup |
| --- | --- |
| Windows, Linux | nothing: the machine's own target comes with rustup |
| Android | `rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android`; the NDK is the one Flutter's Android setup installs |
| macOS | `rustup target add aarch64-apple-darwin x86_64-apple-darwin` (a release app is universal); Xcode |
| iOS | `rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios`; Xcode |

A missing piece stops the build with one sentence naming it:

```text
The Rust target x86_64-linux-android is not installed: run `rustup target add x86_64-linux-android`.
```

and likewise for cargo itself (looked for on `PATH`, then in
`~/.cargo/bin`), the Android NDK and Xcode. 32-bit x86 is not a target:
the ABI's structs are not laid out for its 4-byte alignment of a 64-bit
integer.

## What a build does

- **Source.** Inside a checkout of this repository (a `path` or `git`
  dependency), the crates in its `crates/`; from pub.dev, the crates on
  crates.io at the package's own version.
- **Cache.** Cargo's output stays in the app's `.dart_tool/`, so the first
  build compiles the crates and later ones reuse them; a build with
  nothing changed does not run the hooks at all. An edited crate is
  rebuilt.
- **Features.** The core is built with the loader and pattern checks,
  `guatiao_serde` with JSON (numbers kept as their text), TOML and YAML,
  `guatiao_intake` with pattern checks.

## Your own library

An application with a library of its own (see
[One library for your application](app-library.md)) names its crate in
its `pubspec.yaml`, and that one file is built and bundled instead of
guatiao's three:

```yaml
hooks:
  user_defines:
    guatiao:
      crate: native/myapp   # a directory holding Cargo.toml, relative to this pubspec
```

Every package then finds its surface in that file, and so does the
application for its own functions:

```dart
import 'package:guatiao/ffi.dart' show openSurface;

final mine = openSurface('guatiao').library;   // the application's own file
```

## Without the hook

`build: false` builds nothing, for the three packages:

```yaml
hooks:
  user_defines:
    guatiao:
      build: false
```

The library then comes from where it came before: `GUATIAO_LIBRARY`,
`useLibrary`, `libraryDirectory` or the platform's search. A hook runs
with only a few of the environment's variables, so `GUATIAO_LIBRARY`
cannot switch the build off; at run time it still wins over the library a
hook bundled.

## Kinds and providers as classes

A kind crate commits its kinds' [description](encodings.md#a-kind-described)
beside their C header. Dart never declares them again: the `guatiao`
package writes the Dart for a description, which you commit.

```bash
dart run guatiao:kind greeter_kind.kinds.json -o lib/greeter_kind.dart
```

For each kind it writes an abstract interface class of its required
slots (`Greeter`), one per slot appended after them (`GreeterShout`), a
remote class a caller holds (`GreeterRemote`), and the kind as the
interface's static `kind`.

**As a host**, a provider's table is called through the remote:

```dart
import 'package:guatiao/guatiao.dart';
import 'greeter_kind.dart';

final reg = Registry('my-host', '1.0')..scanDir(dir, rules: 'kind=greeter');
final greeter = reg.bind('acme_hello', Greeter.kind);
final answer = greeter.greet('ana');            // a Value: close it
if (greeter.has('shout')) print(greeter.shout('ana'));
```

**As a provider**, a class extends `Provider` and implements the
interfaces of what it serves, which is what the analyzer checks: a
missing required method, or a method taking or answering the wrong type,
does not compile. A `Library` offers it to any host in the process, a
Rust or C caller included:

```dart
import 'package:guatiao/kinds.dart';
import 'greeter_kind.dart';
import 'src/library_identity.g.dart';   // dart run guatiao:kind identity

final class Hello extends Provider implements Greeter, GreeterShout {
  @override
  Map<String, Object?> greet(String name) => {'greeting': 'hello, $name'};

  @override
  String shout(String name) => 'HELLO, ${name.toUpperCase()}';
}

final library = Library.of(packageLibrary, offers: [
  Offer.single(Hello(), named: 'hello', kinds: [Greeter.kind]),
]);
reg.registerLibrary(library);           // my_package_hello
```

`dart run guatiao:kind identity` writes the library's id and version from
`pubspec.yaml`, which an AOT program cannot read. `Offer.configured`
builds an instance per configuration the host checked against a schema.
Dart objects cross as objects of a kind both ways: a listener a host
lends, a conversation a provider hands back.

A table served from Dart runs on the thread of the isolate that built
it; a library calling it from another thread ends the process, as Dart
does for every isolate-local callback. In a Flutter app, import the
generated file with a prefix: Flutter has a `Listener` of its own.

## The example

`examples/flutter_app` in the repository is a Flutter app using the three
packages from the checkout. It builds a value, writes it as JSON, reads it
back, finds a field by its path, serves a greeter written in Dart and
calls it back through its table, and prints where each library was loaded
from:

```bash
cd examples/flutter_app
flutter build windows    # or apk, linux, macos
GUATIAO_EXAMPLE_EXIT=1 build/windows/x64/runner/Release/flutter_app.exe
```

```text
guatiao example: json {"host":"10.0.0.1","port":5900,"agent":[{"name":"first"},{"name":"second"}]}, agent[1].name second
guatiao example: kinds: greeter {greeting: hello, ana}, heard [hello, bo]
guatiao example: libraries: guatiao ...\guatiao.dll; guatiao_serde ...\guatiao_serde.dll; guatiao_intake ...\guatiao_intake.dll
```

On Android the same three lines are in the log under the `flutter` tag.
`examples/dart_host` and `examples/dart_provider` are a host and a
library in plain Dart: the host calls greeters written in Rust, C, Python
and Dart through the classes, and has a Rust caller call the Dart one;
`dart build cli` makes it an AOT executable.
