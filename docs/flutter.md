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

## The example

`examples/flutter_app` in the repository is a Flutter app using the three
packages from the checkout. It builds a value, writes it as JSON, reads it
back, finds a field by its path, and prints where each library was loaded
from:

```bash
cd examples/flutter_app
flutter build windows    # or apk, linux, macos
GUATIAO_EXAMPLE_EXIT=1 build/windows/x64/runner/Release/flutter_app.exe
```

```text
guatiao example: json {"host":"10.0.0.1","port":5900,"agent":[{"name":"first"},{"name":"second"}]}, agent[1].name second
guatiao example: libraries: guatiao ...\guatiao.dll; guatiao_serde ...\guatiao_serde.dll; guatiao_intake ...\guatiao_intake.dll
```

On Android the same two lines are in the log under the `flutter` tag.
