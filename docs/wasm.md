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

## Providers compiled to WebAssembly

The other way round: a **native host** calling providers that run as
WebAssembly. The `guatiao-wasm` source (`crates/guatiao-wasm`, built from
this repository, not published) is a file any host adds, in Rust, C,
Python or any language with a binding, the way it adds any
[source](language-bridges.md): its libraries are WebAssembly modules,
each run by wasmtime in a sandbox of its own. A module sees its own
memory and what a call hands it, and nothing else: no files, no network,
no environment, no host memory. The same `.wasm` file serves every
platform, and a refresh serves a rebuilt one while the old build answers
the instances taken from it.

```bash
cargo build --release --manifest-path crates/guatiao-wasm/Cargo.toml
```

A library is a module and a manifest beside it, `greeters.guatiao-wasm.toml`:

```toml
module = "greeters"               # greeters.wasm, in this directory
kinds = ["greeter", "counter"]    # every kind its providers serve
version = "1.0.0"                 # optional when the module states its own
```

and the host gives the source where the manifests and the kinds'
descriptions are, and its limits:

```python
from guatiao import Registry

reg = Registry("myapp", "1.0")
reg.add_source_file("libguatiao_wasm.so", {
    "dirs": ["/opt/myapp/wasm"],
    "kinds": ["/opt/myapp/greeter_kind.kinds.json"],
    "limits": {"operations": 10_000_000, "time_ms": 100, "memory": 64 * 1024 * 1024},
})
reg.refresh()
hello = reg.bind("greeters_hello", Greeter)
```

**Writing one.** In Rust, a `cdylib` built for `wasm32-unknown-unknown`
with the guest crate, `guatiao-wasm-guest`
([`examples/wasm_greeter_rust`](https://github.com/Boriken-Dev/guatiao/tree/main/examples/wasm_greeter_rust)):
each provider is a handler called with the kind and slot named and
guatiao's own values. In Go 1.24 or later, a reactor built by Go's own
toolchain (`GOOS=wasip1 GOARCH=wasm go build -buildmode=c-shared`) with the
`guest` package
([`examples/wasm_greeter_go`](https://github.com/Boriken-Dev/guatiao/tree/main/examples/wasm_greeter_go)).
Both serve every slot of the example kinds and pass the same host tests.

**Any other language** that emits a core WebAssembly module speaks
**guatiao ABI 1**: five exports (`guatiao_guest_abi`, `_alloc`, `_declare`,
`_call`, and optionally `_release`) and five imports from `guatiao`
(`answer`, `write_back`, `object_call`, `result`, `drop`), every value as
its [wire encoding](encodings.md#values-on-the-wire) in the module's
memory. A call is the wire list `[kind, method, arg...]`; its answer a
wire value, or a status and its words. The whole interface, every refusal
and its status are in the crate's
[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao-wasm/AGENTS.md).
A module that imports anything else, beyond fifteen WASI preview-1
functions that grant stdio, a clock and randomness, is refused before it
runs.

**Limits** are per call, opening included: `operations` (wasmtime's fuel),
`time_ms` and `memory`. A limit stops the call (`GUATIAO_ERR_INTERNAL`,
`GUATIAO_ERR_ALLOC` for memory, "stopped at its operations limit
(10000000)"); so does a trap, named with what the module last said. Either
way the host carries on, and the library is made afresh for its next
call. `time_ms` is measured by the clock, and a call stops within about a
millisecond (a system timer period on Windows) past it, at its next loop
or call. What the host does for a module counts too: the bytes it copies,
randomness and decoding are charged to `operations` and checked against
`time_ms`, bulk memory instructions are charged to `operations` by their
length, and what it takes from a module at once stays within `memory`
(64 MiB without it). Calls run on
the caller's thread, each library behind a lock of its own: calls into
different libraries run at once, and a call that would wait for a library
while its thread is inside another call answers `GUATIAO_ERR_BUSY` rather
than risk a deadlock.
