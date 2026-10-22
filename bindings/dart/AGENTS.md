# guatiao (Dart) — API header

`dart:ffi` bindings over the `guatiao` C ABI: https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships with the package, so it names no repo path.

A plain Dart package with one dependency, `package:ffi`. It does **not**
depend on Flutter: a plain Dart package works inside a Flutter app, and
the reverse is not true. 64-bit platforms only, and **not on the web**:
`dart:ffi` does not exist in a browser, so `pubspec.yaml` declares
Android, iOS, Linux, macOS and Windows and no `web`.

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
`guatiao_intake` resolve the same way, independently, only when
`package:guatiao/serde.dart` or `.../intake.dart` is first used. A symbol a
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
schema — writing through it throws), `providerMeta(key)` (a copy of the
provider's `meta` as a `Value` to close, `null` when it declares none,
`NotFound` for an unknown key; its form is under `intake.providerFormKey`),
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

## intake

```dart
import 'package:guatiao/intake.dart' as intake;

intake.check(schema, formDoc);          // null, or the error map
intake.layout(schema, formDoc);         // [{'section': ..., 'fields': [...]}, ...]
intake.isVisible(schema, formDoc, 'key', values);   // bool
intake.at(value, 'agent[1].name');      // a borrowed Ref, or null
intake.forSchema(schema);               // the form a schema implies (a Value to close)
intake.formFor(formDoc, schema, 'path');   // a Value to close, or null
```

The flat projection keeps a value as `key -> text`, one entry per scalar
leaf (a tagged field's discriminant under its key and its payload under
`key.member`, a list as `agent[0].name`, an open map as `env[PATH]`):

```dart
intake.resolve(schema, 'auth.password');  // borrowed Ref onto the field, or null
intake.flatKeys(schema, 'auth');          // List<String>; WrongKind for an unknown key
intake.flatten(schema, 'auth', value);    // Map<String, String>; WrongKind when it does not apply
intake.unflatten(schema, 'auth', store);  // Value to close, or null; store is a
                                          // Map<String, String> or a Ref onto one
```

What `resolve` and `at` return is a read-only view: it holds the schema or
value it came from, is found again from it on every access, and throws
`StateError` once that is closed. Writing through it throws `StateError`
(every write, not only one that allocates), and `.clone()` copies it out.
An empty container flattens to nothing, and an entry the schema does not
declare is dropped by `unflatten`.

`unflatten` answers `null` only for "nothing stored": no such field, no
entry under it, or a discriminant that names no arm. A `store` that is not
text to text -- a `Map`, or a `Ref` onto anything but a map of strings --
throws `ArgumentError`.

**A provider's form** travels in its `meta`, under `intake.providerFormKey`
(`'form'`), beside the configuration schema it is shown with:

```dart
final meta = reg.providerMeta('acme_dialer');    // null when it declares no meta
try {
  final form = meta!.asMap()[intake.providerFormKey]!;
  final schema = reg.providerConfig('acme_dialer')!;
  intake.check(schema, form);                    // null: it fits
  intake.layout(schema, form);
} finally {
  meta?.close();
}
```

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
`isolateGroupBound` callback that a native thread calls. The intake
functions take any `Ref`, so a borrowed schema is passed as it is, with
no copy.

## Embedding, and sharing the types

`useLibrary(DynamicLibrary, {Iterable<String>? forSurfaces, String path})`
registers an open library ahead of every other route. With no
`forSurfaces` it is registered for each surface it exports and no other,
so a host that carries the value and intake surfaces in one file registers
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
binding's own, and are not status failures. A refusal the library words
itself (a `WrongKind` from `loads`, say) carries those words in `message`.
A process whose pointers are not eight bytes wide gets `UnsupportedError`
when the first library loads.

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

`lib/src/bindings.g.dart` is written by ffigen from the three committed C
headers and is never edited by hand. Regenerate with `dart run ffigen
--config ffigen.yaml`, then **check the file is still over two thousand
lines**: ffigen does not fail on an entry point it cannot find, it writes
an empty file. The header's `static inline` helpers have no symbol in any
library and are skipped by design; `value.dart` reimplements them over
the generated structs, which is why reading a value needs no library at
all.
