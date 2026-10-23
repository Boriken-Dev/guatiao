# guatiao (Dart) — API header

`dart:ffi` bindings over the `guatiao` C ABI: https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships with the package, so it names no repo path.

A plain Dart package, Dart 3.10 or newer. Its dependencies are
`package:ffi`, and `package:hooks` and `package:code_assets` for its build
hook (below). It does **not** depend on Flutter: a plain Dart package
works inside a Flutter app, and the reverse is not true. 32- and 64-bit
native platforms (`armeabi-v7a` included), and the web: there the same
API runs over the WebAssembly module a page loads (below).

## Finding the native library

Importing any of this package's libraries touches nothing native. A
native call is resolved lazily, on first use, in this order:

1. A library registered with `useLibrary` for the surface.
2. `GUATIAO_LIBRARY` — a directory holding `guatiao.dll` /
   `libguatiao.so` / `libguatiao.dylib`, or a file, taken as it is.
3. The library the package's build hook bundled (below), wherever the
   build put it and whatever it is called: found through its asset id
   `package:guatiao/guatiao`, by the address of one of its symbols
   (`dladdr`, `GetModuleFileNameW`). `bundledLibraryPath` in
   `package:guatiao/ffi.dart` answers it, or `null`.
4. `libraryDirectory`, a settable top-level in
   `package:guatiao/guatiao.dart`. A process cannot set its own
   environment, so this is how an app that ships its own copies says
   where they landed.
5. The platform's own library search, by name.

Not found throws `LibraryNotFound` naming every route. A symbol a build
does not export throws `MissingSymbol` naming it. JSON, TOML and YAML are
the package `guatiao_serde`, forms `guatiao_intake`: each opens its own
library through `openSurface` (below), consulting its own variable
(`GUATIAO_SERDE_LIBRARY`, `GUATIAO_INTAKE_LIBRARY`) before
`GUATIAO_LIBRARY`.

## The build hook

`hook/build.dart` builds the crate `guatiao` (features `load`, `regex`)
from source with cargo for the target a Dart or Flutter build names, and
bundles it as the code asset `package:guatiao/guatiao`. `guatiao_serde`
and `guatiao_intake` do the same with their crates, through the same
code. It never downloads a binary.

- **Source**: the repository checkout the package sits in (a path or git
  dependency: `crates/` three directories up), else the crate on
  crates.io at the package's own version (`=x.y.z`).
- **Cache**: cargo's target directory is in the hook's shared output
  directory (`.dart_tool/hooks_runner/shared/<package>/`); the hook
  declares every source in cargo's dep-info and each path crate's
  manifest, so an unchanged build is skipped (`Skipping build for
  guatiao`) and an edited crate rebuilds.
- **Log**: the cargo command, in the hook's `stdout.txt` beside its
  `output.json`; cargo's own output on its stderr.
- **Targets**: Android `aarch64-linux-android`, `armv7-linux-androideabi`
  (clang target `armv7a-linux-androideabi<api>`), `x86_64-linux-android`,
  `riscv64-linux-android`; Linux `aarch64`, `armv7` (gnueabihf),
  `x86_64`, `riscv64gc` `-unknown-linux-gnu`; Windows `aarch64`/`x86_64`
  `-pc-windows-msvc`; macOS `aarch64`/`x86_64` `-apple-darwin`; iOS
  `aarch64-apple-ios`, `aarch64-apple-ios-sim`, `x86_64-apple-ios`. No
  32-bit x86: a `u64` there is aligned to four bytes, which the ABI's
  structs are not written for.
- **Linking**: Android links with the NDK clang the build hands the hook
  (Flutter passes the NDK's), else the newest NDK under
  `ANDROID_NDK_HOME`, `ANDROID_NDK_ROOT`, `ANDROID_NDK`,
  `ANDROID_NDK_LATEST_HOME` or `ANDROID_HOME/ndk`, with
  `--target=<clang triple><api>` and 16 KB pages on 64-bit. macOS and iOS
  set `MACOSX_DEPLOYMENT_TARGET` / `IPHONEOS_DEPLOYMENT_TARGET` from the
  build. Linux links with the C compiler the build hands the hook
  (Flutter passes clang), else `cc`; Windows with the MSVC rustc finds.
- **Failures**, one sentence each, in this order: cargo not on `PATH` or
  in `~/.cargo/bin`; the Android NDK not found; Xcode not found (`xcrun`
  has no SDK); the Rust target not installed (``run `rustup target add
  <target>` ``). A failed cargo build quotes its last lines.
- **The hook's environment** is the one the Dart SDK passes hooks: `PATH`,
  `HOME`, `USERPROFILE`, the Android variables, the temp and proxy ones.
  **`GUATIAO_LIBRARY` does not reach it**, nor `CARGO_TARGET_*`.

User defines, in the root `pubspec.yaml` (the application's, or a
workspace's), read by the core package's hook:

```yaml
hooks:
  user_defines:
    guatiao:
      crate: native/myapp   # a path, relative to that pubspec
      build: false
```

- `crate`: the application's own crate (a directory holding
  `Cargo.toml`, or the file), whose `[lib]` has `crate-type =
  ["cdylib"]`, built instead of `guatiao` as the asset
  `package:guatiao/guatiao`; cargo runs in its directory, so its own
  toolchain file applies. `guatiao_serde` and `guatiao_intake` then build
  nothing, and each surface resolves to that file when it exports the
  surface's probe. A crate missing a surface the app calls answers
  `LibraryNotFound` for it.
- `build: false`: nothing is built or bundled, for the three packages;
  the library comes from `GUATIAO_LIBRARY`, `useLibrary`,
  `libraryDirectory` or the platform search. This package's own
  `pubspec.yaml` says so for its own suites.

`package:guatiao/hook.dart`, for a package's own `hook/build.dart`:

- `buildHook(List<String> arguments, {required String crate,
  List<String> features = const []})`: a hook's `main`, running
  `buildLibrary` and reporting a `HookFailure` as the hook's `BuildError`.
- `buildLibrary(BuildInput input, BuildOutputBuilder output, {required
  String crate, List<String> features = const [], Map<String, String>?
  environment})`: the build itself. A feature `dep/feat` turns on `feat`
  of the crate's dependency `dep`. `environment` is where cargo and the
  NDK are looked for and what cargo runs with (the process's own by
  default). The core package publishes `library` metadata (`guatiao`,
  `app` or `none`) for the hooks of the packages over it.
- `rustTarget(OS, Architecture, {IOSSdk? iOSSdk}) -> String?`: the
  target table above; `null` where there is none.
- `missingTarget(String triple) -> String`: the sentence a missing target
  fails with.
- `HookFailure` (`message`): a missing prerequisite or a failed build.

## On the web

The same libraries and API, over one WebAssembly module instead of a
native library: a conditional import picks it under dart2js and dart2wasm.
The module is built from guatiao's sources by one command, the core and
forms in one file:

```bash
cargo build -p guatiao-intake --target wasm32-unknown-unknown --release
# target/wasm32-unknown-unknown/release/guatiao_intake.wasm
```

or an application's own crate carrying the surfaces it wants, JSON, TOML
and YAML included. Serve it with the page and load it before the first
call:

```dart
import 'package:guatiao/guatiao.dart';

await loadModule(Uri.parse('guatiao_intake.wasm'));   // relative to the page
final v = Value.fromDart({'host': '10.0.0.1'});
```

- `loadModule(Uri)` fetches and instantiates it; `loadModuleBytes(bytes)`
  takes the file's bytes. **One module per page**: a second load throws
  `StateError`, since every value lives in the loaded module's memory.
  `forgetLibraries()` drops it; a value built in it then throws
  `StateError`. A call before any load throws `LibraryNotFound`.
- `guatiao_serde` and `guatiao_intake` call the same module, so it must
  carry their surfaces: a missing one is `LibraryNotFound`, a format the
  build left out `UnsupportedError` naming its feature.
- Values live in the module's memory; text and bytes are copied in and
  out. `close()` is required as on the VM; a `Finalizer` backstop frees
  both the tree and its root. The memory may grow on any call, and every
  read goes through the module again, so a value stays readable.
- Where the VM names `Pointer<guatiao_value>` or `Pointer<guatiao_alloc>`
  the web has `Address`, an offset into the module's memory:
  `Value.adopt(Address, {Address? alloc})`, `Ref.borrowed(Address)`,
  `value.pointer`, `ref.requireNode()`, `ValueOwner.rootNode` and
  `allocator`, and every `alloc:` parameter.
- **Not on the web**, each throwing `Unsupported` that says so: `Registry`
  (its constructor) and `kindTable`, since a browser maps no libraries.
  `useLibrary` and `package:guatiao/ffi.dart` do not exist there;
  `libraryDirectory` is settable and ignored. `loadModule` and
  `loadModuleBytes` on the VM throw `Unsupported`.
- **dart2js integers** are JavaScript numbers, exact to 2^53: `toDart()`
  answers a `BigInt` for an integer past that, `intOr` falls back, a
  channel or a span field past it is not exact, and a `double` with no
  fraction is an `int` and is written as one (`1`, not `1.0`). dart2wasm
  has 64-bit integers, as the VM does.
- `package:guatiao/wasm.dart` is the web's counterpart of `ffi.dart`, for
  a package over another surface: `WasmModule` (`call`, `provides`,
  memory readers and writers, `alloc`/`free` over
  `guatiao_memory_alloc`, `defaultAlloc`), `Address`, `I64`,
  `withScratch`/`Scratch` (a call's arguments, released together),
  `openSurface`, `takeValue`, `freeNode`, `freeRoot`, `strOf`, and the
  wasm32 layout constants. It imports `dart:js_interop` and exists on the
  web only.

## Values

`Value` owns a root `guatiao_value`; free it with `close()`, which is
**required**. A `NativeFinalizer` is a backstop that releases the tree on
a dropped value, but not the 40-byte root box, and it runs at the
collector's convenience.

`Ref` addresses a node inside a `Value`'s tree by a path of keys and
indices, walked fresh on every access — never a cached pointer, since a
sibling write can reallocate the array it points into. `MapRef` and
`ListRef` are `Ref`s with the map and list protocols. `Value` is itself a
`Ref` onto its own root.

```dart
import 'package:guatiao/guatiao.dart';

final v = Value.fromDart({'host': '10.0.0.1', 'port': 5900, 'tags': ['a']});
final m = v.asMap();
m['port']!.toDart();          // 5900
m['port'] = 5901;
m.keys;                       // ['host', 'port', 'tags']
m['tags']!.asList().add('b');
v.toDart();                   // back to a plain Map/List tree
v.close();
```

**Construction**: `Value(obj)` converts as `Value.fromDart(obj)` does;
`Value()` is absent. Also `Value.nullValue()`, `.absent()`,
`.boolean(b)`, `.string(s)`, `.bytes(data)`, `.number(text)` (the exact
text, no `double` detour), `.map()`, `.list()`, and `.adopt(pointer)` for
a root a native call filled into `calloc`-allocated storage.

Conversion, for `fromDart` and for anything written into a tree: `null` →
null, `absent` → absent, `bool`, `int` / `double` / `BigInt` → number (a
non-finite `double` is an `ArgumentError`), `String` → string,
`TypedData` → bytes, `List` → list, `Map` with `String` keys → map, a
`Value` or `Ref` → a deep copy.

**Reading**: `ref.toDart({numbers})` walks the whole node.
`Numbers.auto` gives an `int` when the number's text has no `.`/`e`/`E`
and fits 64 bits, a `BigInt` when it does not, a `double` otherwise;
`Numbers.text` gives the exact text, always; `Numbers.bigInt` gives a
`BigInt` for integral text and a `double` for the rest. A node that is
not there reads back as `absent`, which is not `null`.
`ref.boolOr/intOr/doubleOr/stringOr(fallback)` read one field with the
header's own rules (`intOr` never truncates: a wrong kind, a fractional
or exponent spelling, and a value outside a signed 64-bit range all fall
back). Also `ref.tag`, `.isAbsent`, `.isNull`, `.clone()` (a deep,
independent `Value`), `.asMap()`, `.asList()`.

**`MapRef`**: `[]` (a `Ref?` — `null` for no such entry), `[]=`,
`remove`, `keys`, `entries`, `length`, `isEmpty`, `containsKey`,
`clear()`, `copyFrom(other)` (every entry of `other`, replacing
collisions).

**`ListRef`**: `[]` (throws out of range), `[]=`, `add`, `insert`,
`removeAt`, `clear`, `length`, `isEmpty`, `refs`. The C surface exports
only append, discard-by-index and clear — no native "set" or "insert" at
a position — so `[]=` and `insert` are built from what exists (clone
every element, clear, push the new order back) and are O(n); `add` and
`removeAt` use their own direct export.

A key is UTF-8 with no terminator and may hold a NUL, so lookups compare
bytes.

## Registry

```dart
import 'package:guatiao/guatiao.dart';

final reg = Registry('my-host', '1.0');
reg.scanDir('/path/to/plugins', rules: 'kind=greeter');   // rules: newline-separated
reg.providers('greeter');        // List<Map>, best first
reg.best('greeter');             // key, id, version, library, display_name,
                                 // from, kinds, has_config, vtable_size
reg.whyNot('codec');             // {'available': false, 'why': ...}
final found = reg.providerTable('acme_hello', kind: 'greeter');
final instance = reg.create('acme_shouter', {'prefix': 'hey'});
instance.close();
reg.close();
```

Also `loadFile`, `scanPath`, `libraries()`, `provider(key)`,
`available(kind)`, `keyedBy` / `librariesKeyedBy`, `setPriority` /
`priority`, `providerConfig(key)` (a borrowed, read-only `Ref` onto the
schema — writing through it throws), `providerMeta(key)` (a copy of the
provider's `meta` as a `Value` to close, `null` when it declares none,
`NotFound` for an unknown key; its form is under `'form'`,
`providerFormKey` of package:guatiao_intake),
`retire`, `collect`, `unload`. Every answer that is data comes back as a
plain `Map`/`List`, already freed.

`providerTable(key, kind: 'greeter')` answers the table the provider
speaks that kind through; `providerTable(key)` answers its single table
only. `ProviderTable.isEmpty` means there is none.

`retire(libraryKey)` takes a library out of the registry and leaves it
mapped until nothing uses it: `collect()` closes every retired library
whose own `unload` function agrees and returns their keys, as does the
registry's next load, retire or unload; one that refuses is asked again
next time, one with no such function stays mapped. An `Instance` holds
its library open until it is closed, and `close()` reaches the provider
that built it even after a retire, or a refresh that loaded a new build
under the same key. `unload(libraryKey)` unmaps it
**when the library agrees**: `Busy` while another library holds a
pinned answer from it, then its own `unload` function is asked, its
promise that what it can account for is released: `Busy` while an
instance or object it handed out is alive; `Unsupported` when it has no such function; `WrongKind` when it
is linked into the host rather than mapped; `NotFound` for an unknown
key. A refusal leaves it loaded. `unload(key, unchecked: true)` is the
caller insisting on a library with no `unload` function. **Both take the
LIBRARY key** — `librariesKeyedBy`'s template, `%id` by default — not a
provider key. Either way no table, `ctx` or borrowed schema taken from
the library is used again, after an unload or a collect that closed it.

**Sources** — libraries that come and go while the host runs:
`addSourceFile(path, [config]) -> String` maps a file exporting
`guatiao_source` (a bridge to another language, say) and adds the source
it answers for `config` (a map or `Value`); the file is never unmapped
and is not a library. Throws with the source's words: `NotFound` (cannot
map), `WrongKind` (no source), `BadValue` (refused table, or already
added), or the source's own status. `watchDir(directory, rules: ...)`:
each refresh loads a library file that appeared, retires one that
vanished, reloads one that changed. `refresh() -> Map`: `loaded` and
`retired` (`{source, id, stamp, key}`), `skipped` (`{source, id,
skipped, ...}`), `refused` (`{source, id?, why}`). A changed candidate
loads before its old build is retired, and an `Instance` of the old keeps
working; refusals are part of the answer. Nothing refreshes on its own.

`kindTable<T>(found, floorHash:, floorSize:)` turns a `providerTable`
answer into the `Struct` a kind's own C header declares (`greeter_vtable`
and friends): checks `found.size` against `floorSize` and the header's
`floor_hash` against `floorHash`, then casts. Throws `FloorMismatch` on
either. `floorSize` is the caller's `sizeOf<T>()`, because `sizeOf` needs
a type known where it is written.

## time: dates, times, instants, spans, exact lengths

The forms are text in a value (an exact length is `[seconds,
nanoseconds]`), fixed at <https://boriken-dev.github.io/guatiao/encodings/>.
Every text is read and written by the library's own functions.

```dart
import 'package:guatiao/time.dart' as time;

time.PlainDate(2026, 10, 21)            // checked; '2026-10-21'
time.PlainDate.parse('2026-10-21'); time.PlainDate.fromDateTime(dt)
date.year / month / day; date.add(span) // years+months (clamped), weeks, days
time.PlainTime(13, 45, 0, 500000000)    // hour, minute, second, [nanosecond]
time.PlainTime.parse('13:45:00.5'); time.PlainTime.fromDateTime(dt)
time.PlainDateTime(date, t); time.PlainDateTime.parse('2026-10-21T13:45:00.5')
at.date / at.time; at.add(span)         // then time units, across midnight
time.parseTimestamp('2026-10-21T19:15:00+05:30')  // DateTime, isUtc
time.formatTimestamp(moment)            // '...Z', any DateTime
time.normalize(time.TextForm.span, '1h 30m')       // 'PT1H30M', every ns kept
time.toDuration(v)                      // [s, ns], seconds, {secs, nanos}; Ref, Value or data
time.fromDuration(d)                    // [seconds, nanoseconds]; negative: ArgumentError
time.Span(days: 1, hours: 2)            // named units, one sign; '-P1D' is Span(days: -1)
time.Span.parse('P1DT2H'); span.years .. span.nanoseconds; -span
span.isZero; span.isNegative            // == unit by unit; hashable
```

- **The `Plain` classes** (named after JavaScript's Temporal) exist
  because `DateTime` always carries a time and a zone: a local one moves
  a time a daylight-saving change skips. They keep nanoseconds, are
  `Comparable`, and `toString()` is the canonical text. A constructor is
  checked by the library and throws `BadValue` for a day the month does
  not have, a time off the clock, or a field past its C width.
- **Text the grammar refuses throws `Unparsed`** (a `BadValue` and a
  `FormatException`; exported from `guatiao.dart` too) with `form`, `at`
  (the byte offset in the UTF-8 text) and `expected`; it never quotes the
  text.
- **`DateTime` and `Duration` count microseconds**: reading into one
  drops the nanoseconds below, toward the past. `toDuration` throws
  `RangeError` past what a `Duration` holds (2^63 - 1 microseconds),
  `BadValue` for anything that is no exact length.
- **`add`** throws `BadValue` for a span with time units on a date and a
  result outside the years 0000 to 9999. `Span(...)` throws `BadValue`
  for fields of both signs or nanoseconds of a second or more.

## wire: values as bytes, and channels

`ref.toWire() -> Uint8List` (any `Ref`) and `Value.fromWire(List<int>
data, {alloc})` are the wire encoding: a tag byte per node, LEB128
lengths, a number as its own text, bytes as bytes, a map in its order,
the same bytes in every language. `fromWire` throws `BadValue` for bytes
that are not exactly one value.

A channel announces a schema before its values; a receiver checks every
value against it. A frame is `kind:u8 channel:LEB128 payload`; the
transport says where one ends.

```dart
import 'package:guatiao/wire.dart' as wire;

final frames = [wire.announce(1, schema), wire.send(1, {'greeting': 'hi'}), wire.close(1)];
final receiver = wire.Receiver();
for (final frame in frames) {
  final event = receiver.accept(frame);   // Event(kind, channel, value)
  event.value?.close();                   // the schema's copy, or the checked value
}
receiver.close();
```

- `announce(int channel, Object? schema)`, `send(int channel, Object?
  value)` (a `Value`, a `Ref` or what `Value.fromDart` takes),
  `close(int channel)`, each a `Uint8List`. A channel is 64 bits: Dart's
  `int` carries the ones past 2^63 as negative numbers.
- `Receiver()`; `accept(List<int> frame) -> Event`; `close()`
  (idempotent; a `NativeFinalizer` is the backstop). `Event`: `kind` a
  `Frame` (`schema`, `value`, `close`, with `.code` the
  `GUATIAO_FRAME_*` byte), `channel`, `value` a `Value?` the caller
  closes.
- A refusal throws a `Refused` (`channel`, `kind`, `message`) that is
  also the status's exception: `Malformed` (a `BadValue`, `at` the byte
  of the frame), `NoSchema` (`NotFound`), `NotASchema` (`WrongKind`),
  `Invalid` (`BadValue`; `key` the field, `expected` or `known`). It
  changes nothing the receiver keeps and quotes nothing it was given.

## Secrets

```dart
final shown = redact(schema, value, placeholder: '[redacted]');  // close it
```

`schema` and `value` are `Ref`s, `Value`s or plain data. Every field a
schema marks (`x-sensitive` or `writeOnly`) is replaced at any depth, and
whatever the schema does not describe is hidden too. Local `$ref`s are
followed. `WrongKind` for a schema that is not a map.

## Reading memory you do not own

`Ref.borrowed(Pointer<guatiao_value>)` is a read-only view of a tree
someone else keeps alive: a value an engine hands a callback, or one a
registry lends. Writing through it throws and nothing frees it. It does not
keep that memory alive: that is the caller's (`Ref.borrowedFrom(source,
find)` is the form that does, for a view inside another `Ref`). Reading
touches no library and no top-level state, so it works inside an
`isolateGroupBound` callback that a native thread calls. Any function
taking a `Ref` takes a borrowed one, with no copy.

## Embedding, and sharing the types

`useLibrary(DynamicLibrary, {Iterable<String>? forSurfaces, String path})`
registers an open library ahead of every other route. With no
`forSurfaces` it is registered for each surface it exports and no other,
so a host that carries the value and intake surfaces in one file registers
that file once. It is also the way in when the symbols are already in
the process and there is no file to open. `forgetLibraries()` undoes it.
`surfaces` lists the three base names.

## ffi: for a package over another guatiao library

`package:guatiao/ffi.dart` is what `guatiao_serde` and `guatiao_intake`
build on, and what any package binding a C library that takes and
answers guatiao values may use. Supported API, versioned with this
package.

- **The generated bindings** of `guatiao.h`: `GuatiaoBindings`, every
  struct (`guatiao_value`, `guatiao_alloc`, `guatiao_str`, ...) and
  constant. `package:guatiao/ffi_symbols.yaml` is an ffigen symbol file: a
  package generating its own bindings over a header that includes
  `guatiao.h` lists it under `import: symbol-files:`, and its bindings
  then import these types from `package:guatiao/ffi.dart` rather than
  defining their own, which is the only way a pointer crosses between the
  two packages: Dart's FFI structs are nominal. Its generated file aliases
  them (`typedef guatiao_value = imp$1.guatiao_value`): import it with
  `hide guatiao_alloc, guatiao_str, guatiao_value` beside `ffi.dart`.
- `openSurface(basename, {String? variable, String? probe}) ->
  NativeLib`: the library carrying a surface, opened once: the one
  registered with `useLibrary` for it, else `variable` (a file taken as
  it is, a directory holding the platform filename), else
  `GUATIAO_LIBRARY` (a directory; or a file, taken when it exports
  `probe`, else its sibling), else the library a build hook bundled
  (`bundledLibraryPath(basename, probe:)`: the surface's own, else the
  core's when it exports `probe`), else `libraryDirectory`, else the
  platform search. `'guatiao_serde'` and `'guatiao_intake'` know their own
  variable and probe. `resolveLibraryPath` is the path half alone.
- `NativeLib`: `.path`, `.library` (the `DynamicLibrary`, for a
  package's own generated bindings), `.bindings` (the core's, over the
  same file), `.alloc` (its `guatiao_alloc_default`, filled once),
  `.provides(name)`, `.require(name, {feature})`.
- `strOf(arena, text)`: a `guatiao_str` over a copy of `text`, valid until
  `arena` is released. `takeValue(node)`: reads a `calloc`ed root a call
  filled as plain Dart, then frees its tree and the box.
  `freeNode(node)`: frees what a node owns through the core library.
  `checkStatus(code, [message])`: throws the status's
  `GuatiaoException`. `Status`, `Tag`, `Value`, `Ref` as in
  `guatiao.dart`.

**A tree crosses libraries as its root node**: `Value.adopt(root,
{alloc})` takes over a `calloc`ed root a call wrote (in `alloc`, default
the core library's), `value.pointer` / `ref.requireNode()` lend one to a
call, `Ref.borrowedFrom(source, find)` views a node another call found.
Built by one library, a tree is read and freed by any other, since a
value records its own allocator; the library that built it stays loaded
while the tree lives.

## Errors

Every failing `guatiao_status` throws `GuatiaoException` (`code` the raw
number, `status` the `Status` it names or `null` for one this binding
does not know, `message` a provider's own words when there were any), as
one of `BadValue`, `AllocFailed`, `WrongKind`, `NotFound`,
`NullArgument`, `Gone`, `Unsupported`, `Busy`, `InternalError`.

`LibraryNotFound`, `MissingSymbol` and `FloorMismatch` are this
binding's own, and are not status failures. A refusal the library words
itself carries those words in `message`.
A process whose pointers are not four or eight bytes wide gets
`UnsupportedError` when the first library loads.

## Not offered

A kind's borrowed object arguments (an object lent to a call for its
duration, `const guatiao_object *` in a kind table) are not wrapped: this
binding drives a kind's table through the pointer `providerTable` answers
and builds no object for a callee to borrow.

## Isolates

A `Value` and a `Registry` are used from the isolate that made them, like
the C handles they wrap. Nothing here crosses an isolate boundary, and a
provider cannot call back into Dart.

## Generated bindings

`lib/src/bindings.g.dart` (and `lib/ffi_symbols.yaml`) is written by
ffigen from the committed `guatiao.h` and is never edited by hand. Regenerate with `dart run ffigen
--config ffigen.yaml`, then **check the file is still over two thousand
lines**: ffigen does not fail on an entry point it cannot find, it writes
an empty file. The header's `static inline` helpers have no symbol in any
library and are skipped by design; `value.dart` reimplements them over
the generated structs, which is why reading a value needs no library at
all.
