# Changelog

All notable changes to this package will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- **A kind born in Dart** (`package:guatiao/describe.dart`): abstract
  interface classes spelled with its marks (`U32`, `Lent<MapRef>`,
  `Taken<Listener>`, `MapAnswer`, `implements ObjectKind`, `@fallible`,
  `@appended`) are read by `dart run guatiao:kind describe FILE.dart -o
  DOCUMENT` (or `describeDeclarations`) into the description document,
  with the floor hash `#[guatiao::kind]` gives the same required slots;
  `dart run guatiao:kind DOCUMENT -o FILE` then writes the kind's classes.

- **A server's providers from plain Dart** (`package:guatiao/remote.dart`):
  `RemoteLink.connect(Uri.parse('ws://host:port/'), security:
  RemoteSecurity.none, key: ...)` speaks guatiao's remote protocol over a
  WebSocket with no native library and no module, so a browser page calls
  the providers `guatiao-remote-server --listen ws://...` serves, under
  dart2js and dart2wasm as on the VM. A link's security is one choice
  (`RemoteSecurity.none`, `serverTls`, `mutualTls`, the default): over
  `wss://` a page's browser does the TLS, and on the VM `RemoteTls` pins
  the server and presents an identity.
  Calls are typed by the kinds' descriptions the catalog carries; a page
  serves objects (`Served`) the server calls back while a call is open.
  The package depends on `package:crypto` for the key proof.

### Changed

- **Breaking: Dart 3.10 or newer** (Flutter 3.38), the first with build
  hooks; the package asked for 3.6. *Migration:* raise the application's
  `sdk:` constraint to `^3.10.0`.
- **Breaking: JSON, TOML and YAML, and forms, are packages of their own**:
  `guatiao_serde` and `guatiao_intake`, each over its own library and
  built on this one. `package:guatiao/serde.dart` and
  `package:guatiao/intake.dart` are gone, and `Format` and `ParseError`
  are exported by `package:guatiao_serde/guatiao_serde.dart`.
  *Migration:* `dart pub add guatiao_serde guatiao_intake`, then import
  `package:guatiao_serde/guatiao_serde.dart` and
  `package:guatiao_intake/guatiao_intake.dart` with the same prefixes.
- The ffigen symbol file is `package:guatiao/ffi_symbols.yaml`, its types
  imported from `package:guatiao/ffi.dart`; it was
  `package:guatiao/src/symbols.yaml`.

### Added

- **Kinds and providers as classes** (`package:guatiao/kinds.dart`, native
  platforms). `dart run guatiao:kind DOCUMENT -o FILE` writes, from the
  description a kind crate commits beside its C header, an abstract
  interface class per kind (its required slots) and per appended slot, a
  `<Kind>Remote` a caller holds, and the `Kind` as the interface's static
  `kind`; `--check` fails on a stale file. `Registry.bind(key, kind)` and
  `Instance.bind(kind)` answer a remote; a class extending `Provider` and
  implementing the interfaces is served through `Offer.single` or
  `Offer.configured` in a `Library`, which `Registry.registerLibrary`
  files for any host in the process (Rust and C callers included) and
  `dart run guatiao:kind identity` names from `pubspec.yaml`. Dart objects
  cross as objects of a kind both ways, lent or handed over. The analyzer
  refuses a missing or mistyped slot; a missing required slot is also an
  `ArgumentError` naming it when the library is built. Also
  `Registry.registerEntry` and `Registry.host`.
- `wireToDart` and `dartToWire` (`package:guatiao/ffi.dart`,
  `package:guatiao/wasm.dart`): plain Dart read from and written as wire
  bytes in Dart, answering what `Value.fromWire(...).toDart()` and
  `Value.fromDart(...).toWire()` do with no tree built, for a package
  whose library answers a whole document as wire bytes.
- **A build hook that builds the library from source**: `flutter build`
  (or `dart run`, `dart test`, `dart build`) runs cargo for the target
  being built and bundles the library with the app. One-time setup:
  rustup and the Rust targets. A missing piece stops the build with one
  sentence naming it. In the application's `pubspec.yaml`, `hooks:
  user_defines: guatiao: crate: <path>` builds its own crate instead (one
  file for every surface), and `build: false` builds nothing. The library
  is found through its asset id after `GUATIAO_LIBRARY`
  (`bundledLibraryPath` in `package:guatiao/ffi.dart`).
  `package:guatiao/hook.dart` is the build, for other packages' hooks.
- **32-bit platforms**: `armeabi-v7a` and armhf Linux, whose 4-byte
  pointers the package refused; the suites run on 32-bit ARM.
- **The web**: the same API under dart2js and dart2wasm, over the
  WebAssembly module a page loads with `loadModule(url)` or
  `loadModuleBytes(bytes)`; one module per page, every package using it.
  Nodes and allocators are an `Address` there; `Registry` and `kindTable`
  throw `Unsupported`. `package:guatiao/wasm.dart` is the web's
  counterpart of `ffi.dart`. `pubspec.yaml` declares `web`.
- **Values on the wire and channels**: `ref.toWire()` and
  `Value.fromWire(bytes)`; `package:guatiao/wire.dart` with `announce`,
  `send` and `close` frames and a `Receiver` whose `accept(frame)`
  answers an `Event` or throws the refusal (`Malformed`, `NoSchema`,
  `NotASchema`, `Invalid`, each also the status's exception).
- `package:guatiao/ffi.dart`: what a package over another guatiao library
  builds on: the generated bindings, `openSurface` (a surface's library,
  found through its own variable, then `GUATIAO_LIBRARY`: a directory, or
  a file exporting the surface, else the file beside it), `NativeLib`,
  `strOf`, `takeValue`, `freeNode`, `checkStatus`.

- `Registry.providerMeta(key)`: a copy of the provider's `meta` as a
  `Value` to close, `null` when it declares none, `NotFound` for an unknown
  key; and `providerFormKey` in `package:guatiao/intake.dart`, the key its
  form is under, so a host checks a provider's form against
  `providerConfig` with `intake.check`.
- `package:guatiao/time.dart`: dates, times of day, civil date-times,
  instants, spans and exact lengths, each read and written by the
  library's own functions so every language accepts the same text.
  `PlainDate`, `PlainTime` and `PlainDateTime` keep a civil date or time
  to the nanosecond (a `DateTime` would carry a zone); an instant is a
  UTC `DateTime`, an exact length a `Duration`, both to the microsecond;
  `Span` is a calendar length, applied with `PlainDate.add` and
  `PlainDateTime.add`. Text a grammar refuses throws `Unparsed`, with
  the byte offset.
- `redact(schema, value)`: a copy with every field the schema marks
  secret replaced by a placeholder.
- Sources: `Registry.addSourceFile(path, [config])` adds a source file
  (one exporting `guatiao_source`, such as a bridge to another
  language), `watchDir(directory, rules: ...)` keeps a directory in step
  with disk, and `refresh()` makes the registry agree with what every
  source serves now, answering what loaded, retired, was skipped or
  refused.

### Fixed

- **A tree a kind's slot answers outlives its library**, whatever
  allocator the slot built it in: one that ignored the carrier built it in
  its own, freed after the unload through code that was gone. It is moved
  into this binding's on arrival (`guatiao_value_make_local`; a library
  older than that export is left as it answered). What a slot grew a lent
  tree with is moved, once the call returns, into the allocator the tree
  recorded when it was lent, so a slot that wrote a fresh tree over it in
  its own allocator is moved home too.

## [0.0.0-alpha.1] - 2026-10-19

### Changed

- A retired library is closed once nothing uses it: by the new
  `Registry.collect()`, which returns the keys it closed, or by the
  registry's next load, retire or unload. A `ProviderTable` taken from
  it is valid until then. Migration: close an `Instance` before its
  library is retired, and drop a `ProviderTable` before the next call.
- `unload` also answers `Busy` while another library holds a pinned
  answer from the library through the host's services.
- The generated bindings follow the C header: the status enum's tag is
  `guatiao_status_code`, and `guatiao_status` is the plain integer
  typedef. No name this package exports changes.
- `pubspec.yaml` declares the platforms the package runs on (Android, iOS,
  Linux, macOS and Windows). `dart:ffi` is not available on the web, so
  pub.dev no longer lists it there.

### Added

- `package:guatiao/intake.dart` wraps the flat `key -> text` projection:
  `resolve`, `flatKeys`, `flatten` and `unflatten`.
- `loads` reads through the `_parse_ex` entry points and throws
  `ParseError` (a `BadValue` and a `FormatException`) carrying `format`,
  `line` and `column`.
- `Ref.borrowedFrom(source, find)`: a read-only view of a node inside
  another `Ref`, which holds `source` and finds the node again on every
  access.

### Fixed

- `guatiao_value` and `guatiao_alloc` are exported from
  `package:guatiao/guatiao.dart`, which the public API names in its
  signatures; a consumer can write `Pointer<guatiao_value>` without
  importing a `src/` file.
- What `intake.resolve` and `intake.at` return no longer outlives the
  schema or value it borrows from: it holds its owner, and reading it after
  the owner is closed throws a `StateError` instead of reading freed memory.
- Every write through a borrowed `Ref` throws, including `remove`,
  `removeAt` and `clear`, which used to change the tree they viewed.
- `intake.unflatten` throws an `ArgumentError` for a `Ref` store that is not
  a map of strings, as it already did for a `Map`; `null` stays the answer
  for nothing stored.
- A parse refusal that is not a syntax error keeps the library's message in
  the thrown exception.
- `ParseError`'s message is located from the struct's size, not a pinned
  offset, and a process with pointers that are not eight bytes wide gets an
  `UnsupportedError` when the first library loads.
- The 3.6.0 test leg no longer fails to compile on a suite that needs
  `NativeCallable.isolateGroupBound`; that suite runs on the stable leg only.

## [0.0.0-alpha.0] - 2026-09-27

### Added

- First release: `dart:ffi` bindings over the guatiao C ABI.
  - `Value`, `Ref`, `MapRef` and `ListRef`: a value tree built, read and
    changed in place, or converted to and from plain `Map`/`List` data.
    A number keeps its exact text, and `toDart` reads it back as an
    `int`, a `BigInt`, a `double` or the text itself.
  - `Registry` and `Instance`: load a library by file or by scan, list
    providers best first, rank them, read a provider's configuration
    schema, build an instance from a configuration, and take a library
    back out with `retire` or `unload`.
  - `kindTable`: a provider's function table, checked against the size
    and the floor hash the kind's own C header declares.
  - `package:guatiao/serde.dart`: JSON, TOML and YAML.
  - `package:guatiao/intake.dart`: `check`, `layout`, `isVisible`.
  - `libraryDirectory`, for an app that ships its own copies of the
    libraries and cannot set its own environment.
