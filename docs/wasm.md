# In a browser

guatiao builds for `wasm32-unknown-unknown` with no feature beyond the
defaults. The value model, the schema and the wire encoding all work
there; the loader does not, since a browser has no files to map, and
turning on `load` for a wasm target is a compile error that says so.

```bash
rustup target add wasm32-unknown-unknown
cargo build -p guatiao --target wasm32-unknown-unknown --release
# target/wasm32-unknown-unknown/release/guatiao.wasm
```

## Two ways in

**From Rust compiled to wasm**, use the crate as anywhere else. Nothing
about it changes on a 32-bit target: the layout is written in pointer
widths, and the wire encoding is the same bytes on every width.

**From JavaScript**, load `guatiao.wasm` and call its exports, the same
C surface the Python and Dart bindings call through a native library. A
value lives in the module's linear memory; `guatiao_alloc_default()`
answers the allocator to build it with, and the module exports the
value, schema and merge functions (not the loader's).

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
