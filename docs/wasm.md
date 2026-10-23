# In a browser

guatiao builds for `wasm32-unknown-unknown` with no feature beyond the
defaults. The value model, the schema and the wire encoding all work
there; the loader does not, since a browser has no files to map, and
turning on `load` for a wasm target is a compile error that says so.

## The module

A page loads **one module** for every surface it calls: each module has a
linear memory of its own, and a value built in one cannot be read through
another. Each is built from guatiao's sources by one command:

```bash
rustup target add wasm32-unknown-unknown
cargo build -p guatiao --target wasm32-unknown-unknown --release          # the core: guatiao.wasm
cargo build -p guatiao-intake --target wasm32-unknown-unknown --release   # the core and forms: guatiao_intake.wasm
```

Every module carries the core, so `guatiao_intake.wasm` serves the value
model and forms together. For JSON, TOML or YAML as well, build the
application's own crate carrying the surfaces it wants (see
[One library for your application](app-library.md)): the same recipe
gives a `.wasm` file for this target. Sizes, as a release build and after
`wasm-opt -Oz`, gzipped:

| module | release | `-Oz` | `-Oz`, gzip |
| --- | --- | --- | --- |
| core | 367 KB | 281 KB | 120 KB |
| core and intake | 489 KB | 373 KB | 157 KB |
| core and serde, JSON only | 452 KB | 348 KB | 145 KB |
| core and serde, JSON, TOML and YAML | 1.65 MB | 1.25 MB | 473 KB |
| all three, every format | 1.77 MB | 1.33 MB | 497 KB |

A module imports nothing and exports its functions and its `memory`.

## Two ways in

**From Rust compiled to wasm**, use the crate as anywhere else. Nothing
about it changes on a 32-bit target: the layout is written in pointer
widths, and the wire encoding is the same bytes on every width.

**From JavaScript, Dart or any host of the module**, call its exports, the
same C surface the native bindings call. A value lives in the module's
linear memory, and so does everything passed by address: a `guatiao_str`
view and its bytes, an out-parameter, a root a host keeps.
`guatiao_memory_alloc(size, align)` answers zeroed bytes in the module's
memory for them and `guatiao_memory_free(ptr, size, align)` releases
them; `guatiao_alloc_default(out)` fills the allocator to build values
with. The layout is the header's on a 32-bit target: pointers and
`size_t` are four bytes, a value is 24, and a struct passed by value is
passed by its address. A 64-bit parameter is a JavaScript `BigInt`. Any
call may grow the memory, which detaches the `ArrayBuffer` a view was
made over: take the module's `memory.buffer` again after each call.

## Dart and Flutter

The Dart packages (`guatiao`, `guatiao_serde`, `guatiao_intake`) run in a
browser under dart2js and dart2wasm with the API they have natively. Load
the module once, before the first call:

```dart
import 'package:guatiao/guatiao.dart';

await loadModule(Uri.parse('guatiao_intake.wasm'));    // relative to the page
final form = Value.fromDart({'host': '10.0.0.1'});
```

Every package uses that module, so it must carry their surfaces. What a
browser cannot do throws `Unsupported` saying so: the registry and kind
tables, since there are no libraries to map. Under dart2js an `int` is
exact only to 2^53, so a larger integer in a value reads back as a
`BigInt`. The packages' own pages list the rest.

**In a Flutter web app**, the module is a file the app serves with its
page. Copy the built module into the app's `web/` directory, which
`flutter build web` copies beside `index.html` as it is:

```bash
cargo build -p guatiao-intake --target wasm32-unknown-unknown --release
cp target/wasm32-unknown-unknown/release/guatiao_intake.wasm my_app/web/
```

and load it before `runApp`, on the web only:

```dart
import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:guatiao/guatiao.dart';

Future<void> main() async {
  if (kIsWeb) await loadModule(Uri.parse('guatiao_intake.wasm'));
  runApp(const MyApp());
}
```

The same app on Android, iOS and the desktops loads the native library
instead, which the packages' build hooks build and bundle (see
[Dart and Flutter](flutter.md)).

## Sending values to it

A value crosses a network as the **wire encoding**
(`guatiao::value::wire`, or `guatiao_wire_encode`/`guatiao_wire_decode`
in C): a tag byte per node, LEB128 lengths, a number as its own text,
bytes as bytes, a map in its order. The receiver needs nothing but the
bytes to know what arrived.

To know what it *means*, announce a schema first on a **channel**
(`guatiao::value::wire::channel`):

```text
frame := kind:u8 channel:LEB128 payload     kind: 0 schema, 1 value, 2 close
```

A `Receiver` keeps each channel's schema and checks every value against
it before handing it over; a value on a channel with no schema, or one
its schema refuses, is an error naming why. Frames are bytes in, bytes
out: the transport says where one ends — a length prefix on a
WebTransport or TCP stream, one datagram, one WebSocket message.

From JavaScript, or any caller of the module's C surface, the frames are
`guatiao_wire_announce`, `guatiao_wire_send` and `guatiao_wire_close`,
and the receiver is `guatiao_wire_receiver_new`,
`guatiao_wire_receiver_accept` and `guatiao_wire_receiver_free`: accept
writes a `guatiao_wire_event`, laid out in pointer widths as `guatiao.h`
declares it, saying what the frame was or why it was refused. The grammar,
the refusals and each language's spelling are on the
[encodings page](encodings.md#values-on-the-wire).
