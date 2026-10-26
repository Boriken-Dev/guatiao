# Providers on another process, machine or network

`guatiao-source-remote` lets a host call providers that live somewhere else: in
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
use guatiao_source_remote::transport::tcp::TcpConnect;
use guatiao_source_remote::{LinkOptions, RemoteSource, Trust};

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
| `connect` | `tcp://host:port`; `ws://host:port/path` (WebSocket); `tls://host:port` (the secure level); `local://name` (a Unix socket's path, or a named pipe's name on Windows); `shm://name` (shared memory beside that); `rdma://host:port` (RDMA, experimental, Linux only) |
| `rdma_device` | the RDMA device `rdma://` goes through (default: the first listed) |
| `child` | instead of `connect`: a list, the server program and its arguments, run with the link on its stdin and stdout |
| `trust` | `local`, `trusted` or `secure`; `local` for a child |
| `key`, `key_file` | the trusted level's pre-shared key: text, or a file's bytes |
| `token` | what a secure client presents, when its server requires one |
| `state_dir` | secure: this side's identity (`identity.pem`, made when absent) and trust store (`peers/`) |
| `identity_file` | secure: the identity file instead of `state_dir` (made when absent) |
| `pins` | secure: a list of the fingerprints (`sha256:<hex>`) of the servers trusted; without any, the trust store's accepted peers |
| `policy` | secure: the TLS policy, a map (below, [Trust](#trust)); strict when absent |
| `timeout_ms` | how long a call waits for its answer; `0` waits as long as the link lives (default 30 000) |
| `max_frame`, `max_calls` | the link's bounds (defaults 64 MiB and 64) |
| `dirs`, `manifests` | instead of `connect` or `child`: directories of peer manifests (and their immediate subdirectories), and manifest files; the keys above then apply to every peer whose manifest does not name them |

```c
/* {"child": ["guatiao-remote-server", "--stdio", "--kinds", "greeter_kind.kinds.json",
              "--load", "libderived_greeter.so"]} */
guatiao_registry_add_source_file(reg, guatiao_cstr("libguatiao_source_remote.so"), &config, &err);
guatiao_registry_refresh(reg, &alloc, &answer);
```

Identical devices serve the same library, and a registry files one build
of a library under its id, so a second device's would be skipped. A host
holding several keeps them apart by the source each came through: added
as sources of their own names, its libraries and providers are filed
under `%source/%id` (`libraries_keyed_by` and `keyed_by` in Rust,
`guatiao_registry_libraries_keyed_by` and `guatiao_registry_keyed_by` in
C, the same names in every binding), and each answers `device-0/...`,
`device-1/...`. `examples/remote_pool` fans one call out to three
devices held that way and gathers the answers.

### Peers in a search path

A peer can be a file, as a library is: a manifest of `type = "remote"`
([the format](encodings.md#a-peer)) in a directory the source's `dirs`
names.

```toml
# peers/lab-printer.guatiao.toml
type = "remote"
name = "lab-printer"
connect = "tcp://192.0.2.5:7000"
trust = "trusted"
key_file = "lab-printer.key"
```

```c
/* {"dirs": ["peers"]} */
guatiao_registry_add_source_file(reg, guatiao_cstr("libguatiao_source_remote.so"), &config, &err);
```

Every refresh reads the manifests again. Each is a peer named by its
`name`, and serves its libraries and providers as `<name>/<id>`
(`lab-printer/printer`, `lab-printer/printer_status`), so identical
devices sit in one registry under the default keys. An edited manifest,
or a replaced key file, is a new peer over a new link, and the old link
ends then: should the new peer's library not open, the refusal is
reported and the old one answers `GUATIAO_ERR_GONE` until the next
change. A removed manifest takes its peer's libraries with it. A manifest that describes no peer is
reported in words naming the file, as is a peer that cannot be reached;
the others are served, and an unreachable peer's libraries stay listed,
answering `GUATIAO_ERR_GONE`, until it is back. Peers are listed one
after another, so a peer slow to refuse a connection slows the refresh.
A manifest names its key by file, never inline, since it lies in a
search path; a secure peer's manifest names its `pins` and `policy`, and
the identity this host presents to it by `identity_file` or `state_dir`,
paths beside the manifest.

A call that gets no answer within its timeout is cancelled and answers
`GUATIAO_ERR_GONE`, as does every call open when the link is lost; the
host carries on. The next `refresh()` connects again (a child is started
again) and lists every library anew, so the registry retires the old
ones and opens the new.

## A page

A browser page reaches a server the only way a page can, a WebSocket, and
speaks the frames itself: no native library, no WebAssembly module. The
Dart package `guatiao` carries the client, `package:guatiao/remote.dart`,
in plain Dart, so the same code runs under dart2js, dart2wasm and on the
Dart VM:

```dart
import 'dart:convert';

import 'package:guatiao/remote.dart';

final link = await RemoteLink.connect(
  Uri.parse('ws://192.0.2.21:7000/'),
  key: utf8.encode(key),               // the trusted level's key, as the page gets it
);
final catalog = await link.list();     // the kinds' descriptions type every call
final library = catalog.libraries.singleWhere((l) => l.id == 'derived_greeter');
await link.open(library);
final greeter = library.providers.first.table('greeter')!;
print(await greeter.call('greet', ['web']));          // {greeting: hello, web}

// An object the page serves, called back while a call is open.
final heard = <String>[];
final listener = Served('listener', {'heard': (args) => heard.add(args[0] as String)});
final chat = await greeter.call('start', [listener]) as RemoteObject;
await chat.call('say', ['hi']);                       // heard: [hi]
chat.release();
await link.close();
```

The server listens with `--listen ws://ADDRESS:PORT` at the trusted
level, as `tcp://` does: peers admitted by address range, the key proved
both ways, no encryption. A call is typed by the description the
catalog carries for its kind: a tree crosses as plain Dart (`Map`,
`List`, `String`, numbers, `Uint8List`, `absent`), a 64-bit integer as an
`int` where the platform holds it exactly and a `BigInt` beyond (past
2^53 under dart2js), a lent map, list or `Uint8List` is written back in
place, an object of the server's arrives as a `RemoteObject` and one of
the page's goes as a `Served`. A slot the far table leaves out is
`Unsupported`: a description carries no default body. A refusal is the
status's exception (`BadValue`, `Gone`, ...), a lost link answers `Gone`
for every open call, and a call past its timeout (30 s by default) is
cancelled.

The page holds the key, so whoever reads the page can link: serve the
page only to those who may call the device, and keep the server's ranges
to the network they are on. The server does not read the `Origin` a
browser sends. `wss://` and WebTransport are not in this build.

## A server

```text
guatiao-remote-server --listen tcp://192.0.2.21:7000 --trust trusted \
    --range 192.0.2.0/24 --key-file link.key \
    --kinds greeter_kind.kinds.json --scan /opt/providers
```

At the secure level it presents an identity of its own and admits the
clients it trusts:

```text
guatiao-remote-server --listen tls://0.0.0.0:7000 --trust secure \
    --state-dir /var/lib/guatiao-remote \
    --kinds greeter_kind.kinds.json --scan /opt/providers
guatiao-remote-server --state-dir /var/lib/guatiao-remote --waiting
guatiao-remote-server --state-dir /var/lib/guatiao-remote --accept sha256:...
```

The first line it prints names its address and its fingerprint, what a
client pins. A client it does not know is refused and listed by
`--waiting` with the address it came from; `--accept` admits it from its
next link on. `--pin FINGERPRINT` (repeatable) trusts fixed clients
instead of the store, `--identity PATH` names an identity file instead
of a state directory, `--policy PATH` reads a TLS policy (TOML or JSON,
below), and `--acl PATH` or `--range CIDR` the access control list.

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
| `shm://name` | the same machine: two rings in a file both processes map, a local socket beside them for wake-ups | 1–5 µs |
| `local://name` | the same machine: a Unix socket, or a named pipe on Windows | 16–40 µs |
| `child` | a server the host starts, over its stdin and stdout: a crash takes down the child, not the host | 16–36 µs |
| `tcp://host:port` | another machine, at the trusted level | 22–36 µs on loopback; 114–123 µs from a host to a VM on it |
| `ws://host:port/path` | a browser, or any WebSocket client, at the trusted level (feature `ws`): one binary message per frame body | not measured |
| `tls://host:port` | any network, at the secure level (feature `tls`): mutual TLS 1.3 | 33–37 µs on loopback |
| `rdma://host:port` | another machine with an RDMA device, at the trusted level; experimental, Linux only (below) | 11 µs on Soft-RoCE loopback, which says nothing of RDMA hardware |

A call is a greeter's `count` through the host's table, there and back,
measured on Windows ARM64, WSL and an x86-64 Linux server, and from that
server to a virtual machine on it (virtio, jumbo frames on and off):
local numbers, provisional. `benchmarks/remote` measures each transport,
with calls a second, throughput, bytes and CPU per call
(`benchmarks/README.md`).

A thread waiting for an answer, or an idle thread of the link's pool
waiting for a call, polls before it sleeps, which keeps a busy link from
paying a wake-up on every hand-off; an idle link's first call pays one.
It polls for half again as long as the typical wait it has seen on that
link (an answer's round trip, the gap between the peer's calls), at
least 50 µs and at most `LinkOptions::spin` (200 µs by default), and not
at all when the typical wait is longer than that, or while as many
threads poll as there are cores. A fixed 50 µs poll, as before, made a
thread across a network poll *and* pay the wake-up: from a Windows host
to a Linux VM on it, polling past the round trip brought a call from 160
µs to 96 µs for the same CPU per call, as 200 µs had brought one from an
x86-64 host to its VM from 114 µs to 74 µs. Set `spin` to zero to spend
the least CPU: 180 µs a call there, about 40 % less CPU.

### RDMA (experimental, Linux)

Built with the `rdma` feature (off by default) on Linux, a link can run
over RDMA: InfiniBand, RoCE, or Soft-RoCE (`rdma_rxe`) on any Ethernet
interface. Elsewhere the transport is compiled in and every call answers
`Unsupported`.

```toml
guatiao-source-remote = { version = "…", features = ["rdma"] }
```

```rust
use guatiao_source_remote::transport::rdma::{RdmaConnect, RdmaListen, RdmaPort};

// The device: the first one listed and its first IPv4 address, or one named.
let listener = RdmaListen::bind("0.0.0.0:7001", &ranges)?.port(RdmaPort::named("mlx5_0"));
let source = RemoteSource::new(
    Box::new(RdmaConnect::new("192.0.2.21:7001")),
    LinkOptions::new(Trust::Trusted).key(&key),
);
```

`guatiao-remote-server --listen rdma://0.0.0.0:7001 --trust trusted
--range 192.0.2.0/24 --rdma-device mlx5_0` serves it, and a source file's
configuration names `connect: rdma://host:port`.

- **Setting up.** A link starts as a TCP connection, admitted by address
  range as a `tcp://` one is. Each side sends a 64-byte record over it:
  its queue pair's number and first packet number, its address (GID) and
  MTU, and the address and key of the ring it receives into. Each then
  sends one byte once its queue pair can receive. The handshake and
  every frame after it cross by RDMA; the TCP connection stays open, and
  its end is the link's.
- **Frames.** Each side registers a 4 MiB ring to receive into and a
  4 MiB staging area to write from. A writer copies a frame into staging
  and posts an RDMA write with immediate data into the peer's ring: the
  immediate is the byte count, and it completes a receive on the peer,
  which is how the reader learns bytes arrived and how it sleeps while
  none do. Every quarter ring, the reader writes how far it has read into
  a counter beside the writer's own ring. A frame larger than the ring
  streams through it.
- **What it needs.** libibverbs (rdma-core) at run time; building needs
  none of it. A device: `rdma::devices()` lists them, and with none a
  connect fails saying so. 8 MiB of locked memory per link:
  `RLIMIT_MEMLOCK` must allow it, or the process needs `CAP_IPC_LOCK`.
- **Tested** on Soft-RoCE (one rxe device, both ends on one machine):
  every slot shape, the key and the ranges, frames ten rings long and
  eight at once, a link ended from either side
  (`crates/guatiao-source-remote/rdma-tests`, which fails without a device).

**Measured on Soft-RoCE only.** Soft-RoCE is RDMA done in software by the
kernel's network stack, so these numbers say nothing about an RDMA NIC.
One x86-64 Linux VM, both ends on it, three runs each, medians, local and
provisional (`benchmarks/results/remote-vm-rdma.jsonl`):

| link, same VM | p50 µs | p99 µs | calls/s, 32 threads | MB/s, 4 MiB round trip | CPU µs/call, client / server |
| --- | --- | --- | --- | --- | --- |
| RDMA, Soft-RoCE loopback | 11.2 | 18.6 | 17 536 | 388 | 22.9 / 22.9 |
| Unix socket | 19.4 | 44.5 | 108 172 | 540 | 28.9 / 27.9 |
| trusted TCP, loopback | 41.2 | 132.4 | 9 539 | 561 | 48.2 / 46.6 |

perftest on the same device moves 4 MiB RDMA writes at about 1.8 GB/s,
and a 64-byte write takes about 3 µs one way. So:

- **A small call is the quickest of the three** (11 µs, about 6 µs of it
  Soft-RoCE's round trip); the rest is the link's own work per call.
- **A large one is not faster.** A 4 MiB round trip moves 388 MB/s,
  where Soft-RoCE alone moves 1.8 GB/s: most of the time is the link's
  own copies (encoding the argument into a frame, decoding it, building
  and encoding the answer), which RDMA does not remove. It adds a copy
  into staging, as a socket adds one into the kernel.
- **Calls from 32 threads reach 17 500 a second**, near TCP's ceiling and
  far below the Unix socket's; the cause is not measured.
- CPU per call counts the two processes alone: Soft-RoCE's work done in
  the kernel's own threads is not in it.

Not measured: RDMA hardware, and two machines with a link between them.

## Trust

A link's trust level is chosen in its configuration; nothing listens on a
network by default.

- **`local`**: the same machine. The operating system's permissions are
  the gate: a Unix socket and the shared file are created for their owner
  alone (`0600`), a named pipe takes Windows' default access, and a
  child's stdio is the host's own.
- **`trusted`**: a network the owner isolates between known devices. No
  encryption. The listener binds whatever address it is given, a
  wildcard (`0.0.0.0`, `::`) or one peers route to included: where to
  listen is the deployment's choice. It admits peers by an access
  control list (below), and **a peer the list denies is closed unread**.
  Both sides prove a pre-shared key in the handshake, so a wrong peer
  does not link by accident. **This is not a defence against anyone on that network**: it
  reads and changes traffic freely. Use it only where the network itself
  is the boundary.
- **`secure`**: any network: mutual TLS 1.3 over TCP (`tls://`, feature
  `tls`), through [coqui-peer-tls](https://github.com/Boriken-Dev/coqui-peer-tls).
  Each side presents a certificate of its own, self-issued, and knows the
  other by its **fingerprint**, `sha256:<hex>` (the SHA-256 of the
  certificate's DER encoding: `openssl x509 -outform DER | sha256sum`).
  What a peer proves is its fingerprint, not its address: the link's
  `Link::peer_id()` answers it, and the access control list may name it.
  A `token` may be required on top.

### Admission: the access control list

A network listener admits peers by an ordered list of `permit` and
`deny` rules ([coqui-net-acl](https://github.com/Boriken-Dev/coqui-net-acl)):
the first rule matching a connection decides, and `default` (`deny`
unless set) decides one no rule matches. A rule matches the connecting
address (`source`, `any` by default), and may narrow to the local
address and port connected to (`destination`, `port`) and, at the secure
level, to a peer's fingerprint. Nothing is refused about where a
listener binds: an empty list binds and denies every peer.

```toml
default = "deny"

[[rule]]
action = "deny"
source = "192.0.2.66"

[[rule]]
action = "permit"
source = "192.0.2.0/24"
port = "7000"

[[rule]]                  # secure level only
action = "permit"
fingerprint = "sha256:3f5a..."
```

A listener decides by address as it accepts, before anything is read: a
denied peer is closed unread. A rule naming a fingerprint is decided
after the TLS handshake has proved one, and never matches a peer at the
trusted level, which proves none. A fingerprint in the list is
canonicalised (any spelling `PeerId` reads) and must be a SHA-256 one. In
Rust the listeners take a `coqui_net_acl::Acl` (re-exported as
`guatiao_source_remote::acl`); `guatiao-remote-server` reads one with
`--acl PATH` (this form, in TOML or JSON) or builds one of permit rules
from `--range CIDR`, and at the secure level takes it from the policy's
`[acl]` table. A secure listener with no list admits every source to the
handshake, where the certificate decides.

### The secure level

```rust
use guatiao_source_remote::transport::tls::{TlsConfig, TlsConnect, TlsListen};

// A device: its identity and trust store in one directory.
let device = TlsConfig::open(Path::new("/var/lib/guatiao-remote"))?;
let listener = TlsListen::bind("0.0.0.0:7000", &device)?;
println!("pin {}", device.fingerprint());

// A host: its own identity, pinning the device.
let host = TlsConfig::open(Path::new("state"))?.pins([device_fingerprint]);
let source = RemoteSource::new(
    Box::new(TlsConnect::new("192.0.2.21:7000", &host)?),
    LinkOptions::new(Trust::Secure),
);
```

- **Identity.** A PEM file holding one certificate and its PKCS#8 key
  (`identity.pem` under a state directory, made on first use with an
  ECDSA P-256 key, readable by its owner alone on Unix). It is a secret:
  a manifest names it by path.
- **Whom a side trusts.** Fixed pins (`pins`, `--pin`), or, when none are
  given, the peers a trust store has accepted. Every certificate must
  prove its key in the handshake, be inside its validity dates and come
  without a chain (under the strict policy).
- **Pairing.** A server with a store records a client it does not know
  as *waiting*, with the address it came from, once the client has proved
  it holds its key (a stolen certificate leaves no entry), and refuses
  it. An operator lists the waiting clients and accepts one by
  fingerprint; its next link goes through. Waiting entries are capped per
  source address and in all. Removing a peer from the store refuses its
  next link: TLS session resumption is off at both ends.
- **A refused client** is told so in words naming its own fingerprint:
  TLS 1.3 lets a client finish its handshake before the server has
  checked its certificate, so the refusal arrives as the link ending
  before the server's first answer.
- **The handshake** runs on the server's serving thread, bounded as a
  whole by the link's handshake timeout, so a client that connects and
  stalls holds only that thread for that long. Both ends require the
  application protocol `guatiao-remote`.
- **The policy** is coqui-peer-tls's, in its text form (version 1),
  in a configuration's `policy` map, a manifest's `[policy]` table or the
  server's `--policy` file. It starts from a `profile` and overrides any
  field; unknown keys are refused:

  | key | strict (the default) |
  | --- | --- |
  | `profile` | `strict`; `compatible` and `legacy` add TLS 1.2 (and SHA-1 pins), which this build lacks: they are refused |
  | `tls_versions` | `["1.3"]` |
  | `cipher_suites`, `kx_groups` | all ring has for those versions; IANA names narrow them |
  | `pin_digests` | `["sha256", "sha384", "sha512"]` |
  | `check_validity` | `true` |
  | `allow_chain_when_pinned` | `false` |
  | `waiting_per_source`, `waiting_total`, `waiting_rerecord_secs` | `16`, `1000`, `60` |
  | `acl` | none (the server's access control list, above) |

  Every setting looser than strict is listed by `TlsConfig::weakened()`,
  and the server prints each when it starts.
- **From C, Python or any language** the secure level is the source
  file's configuration keys above. A peer in another language speaks the
  same protocol inside TLS 1.3 with the same files: OpenSSL presents the
  identity file, pins by comparing the SHA-256 of the peer's DER
  certificate, offers ALPN `guatiao-remote`, and keeps a trust store in
  coqui-peer's format (`peers/{accepted,waiting}/<hex>.pem`).

Measured on loopback, five runs each alternating, medians, local and
provisional (`benchmarks/results/remote-tls-*.jsonl`; the WSL runs on a
busy machine):

| link, loopback | p50 µs | p99 µs | calls/s, 32 threads | MB/s, 4 MiB | CPU µs/call, client / server |
| --- | --- | --- | --- | --- | --- |
| Windows ARM64, trusted TCP | 30.2 | 67.0 | 95 276 | 1 301 | 35.9 / 38.3 |
| Windows ARM64, TLS | 32.8 | 65.9 | 121 606 | 662 | 41.4 / 37.5 |
| WSL (aarch64), trusted TCP | 56.2 | 172.8 | 59 866 | 1 807 | 59.5 / 58.6 |
| WSL (aarch64), TLS | 37.4 | 171.7 | 80 360 | 931 | 49.7 / 50.3 |

A small call costs TLS a few microseconds or nothing; 32 threads calling
at once go faster than over plain TCP, as frames written together share
records and reads; a 4 MiB round trip moves about half as many bytes,
the cost of sealing and opening every byte on top of the link's own
copies. Small calls stay near their idle time while large ones stream
beside them on the same link (`small_calls_stay_quick_while_large_ones_stream`).

Not in this build: QUIC, `wss://`, CA-signed certificates
(coqui-peer-tls's `ca` feature) and TLS 1.2.

## The protocol

Every implementation, in any language, speaks the same frames. The
shared vectors are `crates/guatiao-source-remote/tests/vectors/remote.json`;
`crates/guatiao-source-remote/tests/c/` holds a codec, a client and a server
written in C over guatiao's C API, and the Dart client
(`bindings/dart/guatiao/lib/src/remote/`) passes the same vectors.

### Frames

On a byte stream (TCP, a pipe, a child's stdio) every frame is its
length, a `u32` little-endian, then that many bytes of body. A
message transport (a WebSocket message, a QUIC datagram) carries the body
alone: over `ws://` each binary message is one body, and a text message
ends the link. A body is a type byte and its fields:

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
it refuses (another trust level, a wrong token). At the secure level the
frames cross inside the TLS session, after its handshake, and a HELLO
asking for the secure level over a transport that proves no peer is
refused. Options are a map:
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
every object it held for the other. A peer that takes in nothing of a
frame for as long as a call may wait has stalled, and the side writing
ends the link. A call beyond the receiver's
`max_calls` is refused `GUATIAO_ERR_BUSY`; a body over `max_frame` ends
the link.
