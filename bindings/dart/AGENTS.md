# guatiao (Dart) — API header

`dart:ffi` bindings over the `guatiao` C ABI: https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships with the package, so it names no repo path.

A plain Dart package with one dependency, `package:ffi`. It does **not**
depend on Flutter: a plain Dart package works inside a Flutter app, and
the reverse is not true. 64-bit platforms only.

## Finding the native library

Importing any of this package's libraries touches nothing native. A
native call is resolved lazily, on first use, in this order:

1. `GUATIAO_LIBRARY` — a directory holding `guatiao.dll` /
   `libguatiao.so` / `libguatiao.dylib`, or a file (taken as it is for
   `guatiao` itself; a sibling is looked for beside it for the others).
2. `libraryDirectory`, a settable top-level in
   `package:guatiao/guatiao.dart`. A process cannot set its own
   environment, so this is how an app that ships its own copies says
   where they landed.
3. The platform's own library search, by name.

Not found throws `LibraryNotFound` naming all three. `guatiao_serde` and
`guatiao_form` resolve the same way, independently, only when
`package:guatiao/serde.dart` or `.../form.dart` is first used. A symbol a
build does not export throws `MissingSymbol` naming it.

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
schema — writing through it throws), `retire`, `unload`. Every answer
that is data comes back as a plain `Map`/`List`, already freed.

`providerTable(key, kind: 'greeter')` answers the table the provider
speaks that kind through; `providerTable(key)` answers its single table
only. `ProviderTable.isEmpty` means there is none.

`retire(libraryKey)` takes a library out of the registry and leaves it
mapped. `unload(libraryKey)` unmaps it **when the library agrees**: its
own `unload` function is asked first, and is its promise that what it can
account for is released. `Busy` while an instance or object it handed out
is alive; `Unsupported` when it has no such function; `WrongKind` when it
is linked into the host rather than mapped; `NotFound` for an unknown
key. A refusal leaves it loaded. `unload(key, unchecked: true)` is the
caller insisting on a library with no `unload` function. **Both take the
LIBRARY key** — `librariesKeyedBy`'s template, `%id` by default — not a
provider key. Either way no table, `ctx` or borrowed schema taken from
the library is used again.

`kindTable<T>(found, floorHash:, floorSize:)` turns a `providerTable`
answer into the `Struct` a kind's own C header declares (`greeter_vtable`
and friends): checks `found.size` against `floorSize` and the header's
`floor_hash` against `floorHash`, then casts. Throws `FloorMismatch` on
either. `floorSize` is the caller's `sizeOf<T>()`, because `sizeOf` needs
a type known where it is written.

## serde: JSON, TOML, YAML

```dart
import 'package:guatiao/serde.dart' as serde;

final text = serde.dumps(value, format: Format.json, pretty: true);
final value = serde.loads(text, format: Format.json);
```

`Format.toml` / `Format.yaml` throw `UnsupportedError` naming the symbol
and its feature when the loaded `guatiao_serde` was not built with it. A
TOML document must be a map (`WrongKind` otherwise).

A number keeps its exact text through JSON, with one exception the parser
owns: an exponent is read back with its sign written (`1e400` parses as
`1e+400`).

The `how` flag takes `bytesDataUri`, `bytesBase64`, `bytesArray`,
`bytesRefuse`, `readDataUris` and `prettyFlag`.

## form

```dart
import 'package:guatiao/form.dart' as form;

form.check(schema, formDoc);          // null, or the error map
form.layout(schema, formDoc);         // [{'section': ..., 'fields': [...]}, ...]
form.isVisible(schema, formDoc, 'key', values);   // bool
```

## Reading memory you do not own

`Ref.borrowed(Pointer<guatiao_value>)` is a read-only view of a tree
someone else keeps alive: a value an engine hands a callback, or one a
registry lends. Writing through it throws and nothing frees it. Reading
touches no library and no top-level state, so it works inside an
`isolateGroupBound` callback that a native thread calls. The form
functions take any `Ref`, so a borrowed schema is passed as it is, with
no copy.

## Embedding, and sharing the types

`useLibrary(DynamicLibrary, {Iterable<String>? forSurfaces, String path})`
registers an open library ahead of every other route. With no
`forSurfaces` it is registered for each surface it exports and no other,
so a host that carries the value and form surfaces in one file registers
that file once. It is also the way in when the symbols are already in
the process and there is no file to open. `forgetLibraries()` undoes it.
`surfaces` lists the three base names.

`package:guatiao/src/symbols.yaml` is an ffigen symbol file. A package
generating its own bindings over a header that includes `guatiao.h`
lists it under `import: symbol-files:` and its bindings then reuse these
types rather than defining their own, which is the only way a pointer
can cross between the two packages: Dart's FFI structs are nominal.

## Errors

Every failing `guatiao_status` throws `GuatiaoException` (`code` the raw
number, `status` the `Status` it names or `null` for one this binding
does not know, `message` a provider's own words when there were any), as
one of `BadValue`, `AllocFailed`, `WrongKind`, `NotFound`,
`NullArgument`, `Gone`, `Unsupported`, `Busy`, `InternalError`.

`LibraryNotFound`, `MissingSymbol` and `FloorMismatch` are this
binding's own, and are not status failures.

## Isolates

A `Value` and a `Registry` are used from the isolate that made them, like
the C handles they wrap. Nothing here crosses an isolate boundary, and a
provider cannot call back into Dart.

## Generated bindings

`lib/src/bindings.g.dart` is written by ffigen from the three committed C
headers and is never edited by hand. Regenerate with `dart run ffigen
--config ffigen.yaml`, then **check the file is still over two thousand
lines**: ffigen does not fail on an entry point it cannot find, it writes
an empty file. The header's `static inline` helpers have no symbol in any
library and are skipped by design; `value.dart` reimplements them over
the generated structs, which is why reading a value needs no library at
all.
