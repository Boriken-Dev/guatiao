# Providers on another process, machine or network

`guatiao-remote` lets a host call providers that live somewhere else: in
a child process, on another machine, on a device across a network. The
device runs a **server** over its own registry; the host adds a
**remote source** and, after `refresh()`, sees the device's providers as
ordinary ones, called through the same kind traits, tables and C
functions as a library it loaded itself.

Either side may call the other while a call is open. A listener lent to
a remote provider is called back over the same link during the call; an
object a remote provider answers is a handle that calls back across; a
taken argument and an answered object live until the side holding them
lets them go.

## A host

In Rust, add the source to a registry:

```rust
use guatiao_remote::transport::tcp::TcpConnect;
use guatiao_remote::{LinkOptions, RemoteSource, Trust};

let source = RemoteSource::new(
    Box::new(TcpConnect::new("192.0.2.21:7000")),
    LinkOptions::new(Trust::Trusted).key(&key),
);
registry.add_source("device", source)?;
registry.refresh();
for greeter in registry.offers::<dyn Greeter>() { /* … */ }
```

In any language, add the crate's library file as a source, with a
configuration map:

| key | holds |
| --- | --- |
| `connect` | `tcp://host:port`; `local://name` (a Unix socket's path, or a named pipe's name on Windows); `shm://name` (shared memory beside that) |
| `child` | instead of `connect`: a list, the server program and its arguments, run with the link on its stdin and stdout |
| `trust` | `local`, `trusted` or `secure`; `local` for a child |
| `key`, `key_file` | the trusted level's pre-shared key: text, or a file's bytes |
| `token` | what a secure client presents |
| `timeout_ms` | how long a call waits for its answer; `0` waits as long as the link lives (default 30 000) |
| `max_frame`, `max_calls` | the link's bounds (defaults 64 MiB and 64) |

```c
/* {"child": ["guatiao-remote-server", "--stdio", "--kinds", "greeter_kind.kinds.json",
              "--load", "libderived_greeter.so"]} */
guatiao_registry_add_source_file(reg, guatiao_cstr("libguatiao_remote.so"), &config, &err);
guatiao_registry_refresh(reg, &alloc, &answer);
```

A call that gets no answer within its timeout is cancelled and answers
`GUATIAO_ERR_GONE`, as does every call open when the link is lost; the
host carries on. The next `refresh()` connects again (a child is started
again) and lists every library anew, so the registry retires the old
ones and opens the new.

## A server

```text
guatiao-remote-server --listen tcp://192.0.2.21:7000 --trust trusted \
    --range 192.0.2.0/24 --key-file link.key \
    --kinds greeter_kind.kinds.json --scan /opt/providers
```

It serves the providers of the libraries it loads (`--load`, `--scan`)
and of the sources it adds (`--source path=json`, refreshed every
`--refresh-ms`), for the kinds it has descriptions of (`--kinds`, a kind
crate's `<crate>.kinds.json`). A provider of a kind it has no
description of is not served. `--stdio` serves one link on stdin and
stdout, for a host that starts it as its child. In Rust, `Server::new`
over a `Registry` does the same.

## Transports

| transport | reach | a call (local, provisional) |
| --- | --- | --- |
| `shm://name` | the same machine: two rings in a file both processes map, a local socket beside them for wake-ups | 3–5 µs |
| `local://name` | the same machine: a Unix socket, or a named pipe on Windows | 17–40 µs |
| `child` | a server the host starts, over its stdin and stdout: a crash takes down the child, not the host | 16–36 µs |
| `tcp://host:port` | another machine, at the trusted level | 21–36 µs on loopback |

A call is a greeter's `count` through the host's table, there and back,
measured on Windows ARM64, WSL and an x86-64 Linux server: local
numbers, provisional. A thread waiting for an answer polls for
50 µs before it sleeps (`LinkOptions::spin`), which keeps a busy link from
paying a wake-up on every hand-off; an idle link's first call pays one.

## Trust

A link's trust level is chosen in its configuration; nothing listens on a
network by default.

- **`local`**: the same machine. The operating system's permissions are
  the gate: a Unix socket and the shared file are created for their owner
  alone (`0600`), a named pipe takes Windows' default access, and a
  child's stdio is the host's own.
- **`trusted`**: a network the owner isolates between known devices. No
  encryption. The listener names its address ranges and **refuses to
  bind outside them**, and to bind every interface (`0.0.0.0`, `::`); a
  peer connecting from outside them is closed unread. Both sides prove a
  pre-shared key in the handshake, so a wrong peer does not link by
  accident. **This is not a defence against anyone on that network**: it
  reads and changes traffic freely. Use it only where the network itself
  is the boundary.
- **`secure`**: any network: TLS 1.3 or QUIC, with a token or client
  certificates. No transport of this build runs it yet, so reach a device
  across an open network only through one you secure yourself.

## The protocol

Every implementation, in any language, speaks the same frames. The
shared vectors are `crates/guatiao-remote/tests/vectors/remote.json`;
`crates/guatiao-remote/tests/c/` holds a codec, a client and a server
written in C over guatiao's C API.

### Frames

On a byte stream (TCP, a pipe, a child's stdio) every frame is its
length, a `u32` little-endian, then that many bytes of body. A
message transport (a WebSocket message, a QUIC datagram) carries the body
alone. A body is a type byte and its fields:

| field | encoding |
| --- | --- |
| uint | unsigned LEB128, in its shortest form |
| int | zigzag, then as uint |
| text, bytes | a uint length, then UTF-8 or raw bytes |
| value | a uint length, then the value in the wire encoding ([Encodings](encodings.md)) |

| type | frame | fields |
| --- | --- | --- |
| `0x01` | HELLO | `"GTRM"`, version `1`, trust (0 local, 1 trusted, 2 secure), nonce: 16 bytes, options: value |
| `0x02` | WELCOME | `"GTRM"`, version `1`, nonce: 16 bytes, proof: bytes, options: value |
| `0x03` | PROOF | proof: bytes |
| `0x04` | LIST | call: uint |
| `0x05` | OPEN | call: uint, library: text, stamp: text |
| `0x06` | CLOSE | library: text, stamp: text |
| `0x10` | CALL | call: uint, parent: uint, target: uint, slot: uint, then the arguments |
| `0x11` | REPLY | call: uint, status: uint, then the answer (status 0) or message: text |
| `0x12` | CANCEL | call: uint |
| `0x13` | RELEASE | object: uint |
| `0x14` | PING | n: uint |
| `0x15` | PONG | n: uint |
| `0x16` | GOODBYE | status: uint, message: text |

Statuses are guatiao's own (`GUATIAO_ERR_*`).

### The handshake

The client sends HELLO; the server answers WELCOME, or GOODBYE with why
it refuses (another trust level, a wrong token). Options are a map:
`max_frame` (the largest body the sender reads), `max_calls` (how many of
the peer's calls it runs at once), and `token` from a secure client. Each
side sends no frame larger than the other's `max_frame`, and keeps no
more of its own calls open than the other's `max_calls`. Before the
handshake ends a body is at most 64 KiB.

At the trusted level the WELCOME's proof is HMAC-SHA256 over the key, of
`"guatiao-remote server"`, the client's nonce and the server's; the
client checks it, then sends PROOF, the same over
`"guatiao-remote client"`. Either side closes the link on a proof that
does not hold. At other levels the proof is empty.

### The catalog

LIST is answered by a REPLY whose body is a value:

```json
{"kinds": [<kind description>, …],
 "libraries": [{"id": "…", "version": "…", "stamp": "…",
                "providers": [{"id": "…", "name": "…", "meta": {…},
                               "tables": [{"kind": "greeter", "table": 1,
                                           "slots": ["greet", "start"]}]}]}]}
```

`kinds` describes every kind a table or an object is of, in the format of
[A kind, described](encodings.md). `table` is the number a CALL names;
`slots` are the slots the table fills, and a caller runs its own default
for the others. `stamp` changes whenever the library does. OPEN holds a
library's providers for the link until CLOSE or the link's end, and is
answered by a REPLY with no body.

### Calls

A CALL's `call` is the caller's own number for it; the REPLY carries it
back. `target` is `table << 1` for a provider's table, or `id << 1 | 1`
for an object the receiver holds. `slot` is the slot's index in its kind.
`parent` is the receiver's call this one is made during, or 0: a
receiver runs a call with a parent on the thread waiting for that call,
so a callback reaches the code that lent the object. Calls are answered
in any order.

Arguments follow, in the slot's order, typed by its description; a reply
with status 0 holds the answer, then what the callee wrote into each lent
tree and each buffer, in the order of the arguments:

| type | encoding |
| --- | --- |
| `bool` | one byte, 0 or 1 |
| `i8` … `i64`, `isize` | int |
| `u8` … `u64`, `usize` | uint |
| `f32`, `f64` | 4 or 8 bytes, little-endian IEEE 754 |
| `str`, `text` | text |
| `bytes` | bytes |
| `bytes_mut` | uint: its length; written back as bytes |
| `value`, `map`, `list` | value; a nullable value that is null: ABSENT |
| `object` | uint: `id << 1`, or `id << 1 \| 1` for an object of the receiver's own |

Numbers meant for computation (samples, a tensor) cross best as one
`bytes` value of packed elements rather than a list of numbers: 100 000
`f64` are 0.8 MB as bytes and 1.96 MB as number texts, which decode about
300 times slower.

`greet("world")` is a 15-byte frame and its answer, `{"greeting":
"hello, world"}`, 33 bytes; the same call as one wire value naming its
provider, kind and method is 88, its answer 56.

### Objects

An object crosses as an id in the numbering of the side that holds it. A
handed or answered object stays held until the side with the handle
sends RELEASE; a lent one only until the call it was lent for returns,
after which a call on it answers `GUATIAO_ERR_GONE`. A side passing back
an object of the receiver's own sets the low bit, and the receiver uses
the object itself.

### Cancelling, failing, bounds

A caller that stops waiting sends CANCEL; the callee finishes, sends
nothing, and drops any object it would have answered. When the link
ends, every call waiting answers `GUATIAO_ERR_GONE` and each side drops
every object it held for the other. A call beyond the receiver's
`max_calls` is refused `GUATIAO_ERR_BUSY`; a body over `max_frame` ends
the link.
