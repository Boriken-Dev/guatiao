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

// No TLS: the key proved both ways (security "none", the trusted level).
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
| `connect` | `tcp://host:port` (`tls://host:port` the same, TLS required); `quic://host:port` (QUIC); `ws://host:port/path` and `wss://host:port/path` (WebSocket, without and with TLS); `local://name` (a Unix socket's path, or a named pipe's name on Windows); `shm://name` (shared memory beside that); `rdma://host:port` (RDMA, experimental, Linux only) |
| `rdma_device` | the RDMA device `rdma://` goes through (default: the first listed) |
| `child` | instead of `connect`: a list, the server program and its arguments, run with the link on its stdin and stdout |
| `security` | `none`, `server-tls` or `mutual-tls` ([Security](#security)): `mutual-tls` on a network unless named; a child, `local://` and `shm://` run `none` alone |
| `key`, `key_file` | the pre-shared key: text, or a file's bytes; proved both ways with `none`, and at server TLS when the server asks for it |
| `token` | what a client presents when its server requires a token |
| `state_dir` | TLS: this side's identity (`identity.pem`, made when absent) and trust store (`peers/`) |
| `identity_file` | TLS: the identity file instead of `state_dir` (made when absent) |
| `pins` | TLS: a list of the fingerprints (`sha256:<hex>`) of the servers trusted; without any, the trust store's accepted peers |
| `roots` | TLS: also trust a server whose certificate an authority issued for the host dialled ([A server by its authority](#a-server-by-its-authority)): `"platform"`, the operating system's, or a CA bundle's path |
| `policy` | TLS: the TLS policy, a map (below, [Mutual TLS](#mutual-tls)); strict when absent |
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
security = "none"
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
or a replaced key file, is a new peer over a new link; the old peer keeps
its link until the registry lets go of every library it opened, so
should the new peer not be reached or its library not open, the refusal
is reported and the old library goes on answering. A removed manifest
takes its peer's libraries with it. A manifest that describes no peer is
reported in words naming the file, as is a peer that cannot be reached;
the others are served, and an unreachable peer's libraries stay listed,
answering `GUATIAO_ERR_GONE`, until it is back. Peers are listed at once
(up to 16 connecting together), so a peer slow to refuse a connection
costs a refresh one wait, not one per peer.
A manifest names its key by file, never inline, since it lies in a
search path; a TLS peer's manifest names its `pins`, `roots` and
`policy`, and
the identity this host presents to it by `identity_file` or `state_dir`,
paths beside the manifest. The source's own `security` (like its other
keys) is every manifest's that names none: a host whose peers all sit on
an isolated network says `none` once there.

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
  security: RemoteSecurity.none,       // ws:// carries no TLS: say so
  key: utf8.encode(key),               // the key, as the page gets it
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

The server listens with `--listen ws://ADDRESS:PORT --security none`,
as `tcp://` does with no TLS: peers admitted by address range, the key
proved both ways, no encryption. A link's security is one choice, the
same as everywhere ([Security](#security)), mutual TLS unless named, and
the URL must agree with it: `ws://` runs `none` alone, `wss://` the two
TLS modes. A call is typed by the description the
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

A page written in TypeScript or JavaScript uses the npm package
`guatiao-remote`: the same client in plain TypeScript, which runs in any
browser and in Node 22 and later, and needs nothing of the `guatiao`
package (no koffi, no native library):

```ts
import { RemoteLink, RemoteObject, Served } from 'guatiao-remote';

const link = await RemoteLink.connect('ws://192.0.2.21:7000/', { security: 'none', key });
const catalog = await link.list();
const library = catalog.libraries.find((l) => l.id === 'derived_greeter')!;
await link.open(library);
const greeter = library.providers[0]!.table('greeter')!;
console.log(await greeter.call('greet', ['web']));     // { greeting: 'hello, web' }

const heard: string[] = [];
const listener = new Served('listener', { heard: ([text]) => void heard.push(text as string) });
const chat = (await greeter.call('start', [listener])) as RemoteObject;
await chat.call('say', ['hi']);                        // heard: ['hi']
chat.release();
link.close();
```

A tree crosses as plain data as the `guatiao` package maps it
(`undefined` absent, objects, arrays, strings, numbers, `Uint8Array`), a
64-bit integer as a `number` while it is a safe integer and a `bigint`
beyond, and a lent object, array or buffer is written back in place. The
key proof is HMAC-SHA256 from `@noble/hashes`: WebCrypto exists only in a
secure context, and a page that may open `ws://` is a plain `http://`
page, secure only on localhost. A browser has no context that follows a
call across `await`, so a handler of a `Served` that calls back into the
server while the server's call is open names it:
`table.call(slot, args, { during: call })`, `call` being the handler's
second argument; the server then runs it as part of that call.

The page holds the key, so whoever reads the page can link: serve the
page only to those who may call the device, and keep the server's ranges
to the network they are on. The server does not read the `Origin` a
browser sends.

**Over TLS (`wss://`).** A page's TLS is its browser's: it trusts a
server whose certificate a certificate authority it knows has issued for
the server's name, and it presents a client certificate only one the
user installed in the browser, chosen by the user. So from a page:

- **mutual TLS** (`security: RemoteSecurity.mutualTls`, `'mutual-tls'`)
  works with a server certificate the browser trusts and a client
  certificate the user installed; the server pins or accepts that
  client's fingerprint as any other. Nothing in the page names either.
- **server TLS** (`security: RemoteSecurity.serverTls`, `'server-tls'`)
  needs a server presenting a certificate the browser trusts; the page
  proves itself with the key or a token inside the link. The server:
  `guatiao-remote-server --listen wss://ADDRESS:PORT --security
  server-tls --identity server.pem --key-file link.key`, the identity a
  certificate for the server's name a certificate authority issued, with
  its chain in the same file, where a browser is to trust it. A page that
  should not hold the key presents a token instead: `--token-file
  link.token` in place of `--key-file`, every client then presenting the
  file's text (a last line end left out).

**Over WebTransport (`https://`).** A page opens WebTransport to a
server's `https://` URL and pins its certificate by hash: no
certificate authority, no name, no browser flag, which suits a device on
a network. The page passes the hashes the server says (`certificate
hashes`, or its `--hashes-file`):

```ts
const link = await RemoteLink.connect('https://192.0.2.21:7443/', {
  security: 'server-tls', key, pins: ['sha256:9f86d0...', 'sha256:60303a...'],
});
```

```dart
final link = await RemoteLink.connect(Uri.parse('https://192.0.2.21:7443/'),
    security: RemoteSecurity.serverTls, key: keyBytes,
    tls: RemoteTls(pins: hashes));
```

Server TLS and none work from a page; mutual TLS needs the browser to
present a client certificate, which Chrome did not to a test profile
holding none (with one a user installed: not measured). Without pins the
browser trusts the server as it trusts any site. The Dart VM and Node have
no WebTransport: there `https://` is `Unsupported`.

On the Dart VM, which owns its TLS, `RemoteTls(pins: [...], identity:
pem)` pins the server by fingerprint and presents an identity file's
certificate, so a Dart program reaches a `wss://` server at mutual TLS
as a Rust host does. In Node the TypeScript client uses Node's own
WebSocket, which trusts Node's certificate authorities
(`NODE_EXTRA_CA_CERTS`) and presents no client certificate.

## A server

```text
guatiao-remote-server --listen tcp://192.0.2.21:7000 --security none \
    --range 192.0.2.0/24 --key-file link.key \
    --kinds greeter_kind.kinds.json --scan /opt/providers
```

At mutual TLS, what a network listener runs unless `--security` says
otherwise, it presents an identity of its own and admits the clients it
trusts:

```text
guatiao-remote-server --listen tcp://0.0.0.0:7000 \
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
`--refresh-ms`), for the kinds it has descriptions of: those a provider
carries of its own (every derived provider and every provider a bridge
serves does), and those `--kinds` names (a kind crate's
`<crate>.kinds.json`), which a hand-written C provider needs. A kind
`--kinds` describes is served through that description; otherwise the
first provider describing a kind decides it. A provider whose own
description calls a kind otherwise (its documentation, and slots past
the shorter description, aside) is left out, as is a provider of a kind
it has no description of. `--stdio` serves one link on stdin and
stdout, for a host that starts it as its child. In Rust, `Server::new`
over a `Registry` does the same.

Each connection's link handshake runs on a thread of its own, at most
`--max-handshakes` at once (64 unless told); past it, a new connection is
closed unread. A transport's own set-up (TLS, QUIC, the WebSocket
upgrade, shared memory's rings, RDMA's queue pairs) runs there too, never
on the listener's thread, and within the same handshake time limit (10 s
unless told): a client that connects and stays silent, on any transport,
gives up its place then, or as soon as the server ends its links
(`Server::end_links`). A source the access control list denies, or whose
handshake fails, is reported at most once a minute per address, the next
report saying how many were held back; past 1024 addresses in one minute,
a new one goes unreported. A listener whose accepts keep failing (out of
file descriptors, say) says so once a minute and pauses between attempts,
from 5 ms up to a second while the failures last.

## Transports

| transport | reach | a call (local, provisional) |
| --- | --- | --- |
| `shm://name` | the same machine: two rings in a file both processes map, a local socket beside them for wake-ups | 1–5 µs |
| `local://name` | the same machine: a Unix socket, or a named pipe on Windows | 16–40 µs |
| `child` | a server the host starts, over its stdin and stdout: a crash takes down the child, not the host | 16–36 µs |
| `tcp://host:port` | another machine: with no TLS (feature `tcp`), or TLS 1.3 (feature `tls`; `tls://` names the same) | no TLS 22–36 µs on loopback, 114–123 µs from a host to a VM on it; mutual TLS 33–37 µs on loopback |
| `quic://host:port` | any network (feature `quic`): TLS 1.3 inside QUIC, with datagrams beside the calls | 58–83 µs on loopback (Windows), about 220 µs in WSL, at mutual TLS |
| `ws://host:port/path`, `wss://host:port/path` | a browser, or any WebSocket client (feature `ws`; `wss://` with `tls`): one binary message per frame body; `ws://` with no TLS, `wss://` with it | not measured |
| `https://host:port/path` | WebTransport, HTTP/3 over QUIC (feature `webtransport`): what a browser opens with the server's certificate pinned by its hash; one stream per link, datagrams beside it | not measured |
| `rdma://host:port` | another machine with an RDMA device, with no TLS; experimental, Linux only (below) | 11 µs on Soft-RoCE loopback, which says nothing of RDMA hardware |

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

A link keeps the large buffers it reads frames into and writes replies
from (64 KiB and up, at most 16 MiB a link besides the one its reader
reads into, let go when the link ends), so a large call's buffers stay
mapped from one call to the next. Freed instead, they went back to the
operating system (glibc trims a thread's arena; on Windows the fault
counts say every free of one did), and every page was faulted in again
on each call. A link that made a 4 MiB call holds about 16 MiB from then
on, idle or not; a server with many such links holds that much for each.
Kept, a 4 MiB round trip moved 1.3 to 1.6 times as many bytes a second
over a local socket and shared memory (TCP 1.5 times in WSL, 1.1 to 1.3
on Windows), and the server spent a third to a half less CPU on it
(`benchmarks/README.md`).

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

`guatiao-remote-server --listen rdma://0.0.0.0:7001 --security none
--range 192.0.2.0/24 --rdma-device mlx5_0` serves it, and a source file's
configuration names `connect: rdma://host:port` and `security: none`:
RDMA runs no TLS, so the network default (mutual TLS) is refused there.

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

## Security

A link's security is one choice, made in its configuration (`security`,
`--security`, a page's `security`), the same on every transport and in
every client. Nothing listens on a network unless told where.

- **`none`**: no TLS.
  - On the same machine (a child, `local://`, `shm://`, the only security
    they run) the operating system's permissions are the gate: a Unix
    socket and the shared file are created for their owner alone
    (`0600`), a named pipe takes Windows' default access, and a child's
    stdio is the host's own. The HELLO names trust `local`.
  - Across a network (`tcp://`, `ws://`, `quic://`, `https://`, `rdma://`), for one
    the owner isolates between known devices: the listener admits peers
    by an access control list (below), and **a peer the list denies is
    closed unread**; both sides prove a pre-shared key in the handshake,
    so a wrong peer does not link by accident. The HELLO names trust
    `trusted`. **This is not a defence against anyone on that network**:
    it reads and changes traffic freely. Use it only where the network
    itself is the boundary. QUIC always runs TLS, so over `quic://` the
    bytes are encrypted, but no certificate is checked (each side
    presents one it generated) and the key alone decides: still no
    defence against a peer in the middle. Over WebTransport (`https://`)
    likewise, except that no client presents a certificate and a page's
    browser still checks the server's.
- **`server-tls`**: TLS 1.3 proves the server by its certificate, and the
  client proves itself inside the encrypted channel: by the pre-shared
  key, when the server holds one (its WELCOME then carries a proof), or
  by a `token` the server requires. No client certificate, so a browser
  page and a host with no identity of its own can use it. The HELLO
  names trust `server-tls`. A client needs the server's `pins` (or a
  trust store, or `roots`) and a `key` or `token`, and no identity; a server needs
  its identity and a key or token (`--key-file`, `--token-file`), and
  lists "client_auth = none" among the settings weaker than strict. A
  server's token is required at mutual TLS as well; with no TLS the
  server refuses `--token-file`, a token adding nothing to the key's
  proof there. In Rust,
  `TlsConfig::server_tls()` on either side (`TlsConfig::without_identity()`
  for a client with none) over `tls://`, `quic://`, `wss://` or
  `https://` (WebTransport), and
  `LinkOptions::new(Trust::ServerTls)`.
- **`mutual-tls`**: TLS 1.3 proves both sides by certificate, through
  [coqui-peer-tls](https://github.com/Boriken-Dev/coqui-peer-tls) (below,
  [Mutual TLS](#mutual-tls)). Each side presents a certificate of its
  own, self-issued, and knows the other by its **fingerprint**,
  `sha256:<hex>` (the SHA-256 of the certificate's DER encoding: `openssl
  x509 -outform DER | sha256sum`). What a peer proves is its
  fingerprint, not its address: the link's `Link::peer_id()` answers it,
  and the access control list may name it. A `token` may be required on
  top. The HELLO names trust `secure`.

**Defaults**: `none` on a child, `local://` and `shm://`; `mutual-tls` on
every network transport. A network link with no TLS is one somebody
named. A host sets its own default for the peers its manifests describe
by naming `security` in the source's configuration; a manifest's own
`security` wins.

Where a URL says whether it runs TLS, it must agree with the security:

| scheme | `none` | `server-tls` | `mutual-tls` |
| --- | --- | --- | --- |
| `tcp://` | plain TCP | TLS over TCP | TLS over TCP (default) |
| `tls://` | refused | TLS over TCP | TLS over TCP (default) |
| `quic://` | QUIC, no certificate checked | QUIC | QUIC (default) |
| `ws://` | WebSocket | refused: use `wss://` | refused: use `wss://` |
| `wss://` | refused: use `ws://` | WebSocket over TLS | WebSocket over TLS (default) |
| `https://` | WebTransport, no certificate asked of the client | WebTransport | WebTransport (default) |
| `rdma://` | RDMA | refused | refused |
| `local://`, `shm://`, a child | the default | refused | refused |

A refusal is `GUATIAO_ERR_BAD_VALUE` naming what to write. In Rust the
choice is `config::Security` (`security_of`, `Security::default_for`,
`Security::trust`), and a link's trust level `LinkOptions::new(Trust)`.

### Admission: the access control list

A network listener admits peers by an ordered list of `permit` and
`deny` rules ([coqui-net-acl](https://github.com/Boriken-Dev/coqui-net-acl)):
the first rule matching a connection decides, and `default` (`deny`
unless set) decides one no rule matches. A rule matches the connecting
address (`source`, `any` by default), and may narrow to the local
address and port connected to (`destination`, `port`) and, at mutual
TLS, to a peer's fingerprint. Nothing is refused about where a
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

[[rule]]                  # over TLS only
action = "permit"
fingerprint = "sha256:3f5a..."
```

A listener decides by address as it accepts, before anything is read: a
denied peer is closed unread. A rule naming a fingerprint is decided
after the TLS handshake has proved one, and never matches a peer with no
security, which proves none. A fingerprint in the list is
canonicalised (any spelling `PeerId` reads) and must be a SHA-256 one. In
Rust the listeners take a `coqui_net_acl::Acl` (re-exported as
`guatiao_source_remote::acl`); `guatiao-remote-server` reads one with
`--acl PATH` (this form, in TOML or JSON) or builds one of permit rules
from `--range CIDR`, and over TLS takes it from the policy's `[acl]`
table. A TLS listener with no list admits every source to the
handshake, where the certificate decides.

### Mutual TLS

```rust
use guatiao_source_remote::transport::tls::{TlsConfig, TlsConnect, TlsListen};
use guatiao_source_remote::transport::ws::{WsConnect, WsListen};

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

// The same over a WebSocket (`wss://`), as a browser holding a client
// certificate connects.
let listener = WsListen::bind_tls("0.0.0.0:7443", &device)?;
let connect = WsConnect::new("wss://192.0.2.21:7443/").tls(&host)?;
```

- **Identity.** A PEM file holding a certificate and its PKCS#8 key
  (`identity.pem` under a state directory, made on first use with an
  ECDSA P-256 key, readable by its owner alone on Unix), and the
  certificate's chain after it when one was issued (the fingerprint is
  the certificate's; a peer pinning it takes no chain unless its policy
  says `allow_chain_when_pinned`). It is a secret: a manifest names it by
  path.
- **Whom a side trusts.** Fixed pins (`pins`, `--pin`), or, when none are
  given, the peers a trust store has accepted. Every certificate must
  prove its key in the handshake, be inside its validity dates and come
  without a chain (under the strict policy).
- **Pairing.** A server with a store records a client it does not know
  as *waiting*, with the address it came from, once the client has proved
  it holds its key (a stolen certificate leaves no entry), and refuses
  it. An operator lists the waiting clients and accepts one by
  fingerprint; its next link goes through. Waiting entries are capped per
  source and in all; a source is an IPv4 address, or an IPv6 /64, so the
  holder of one subnet cannot fill the queue from many addresses (in
  Rust, `TrustStore::with_ipv6_prefix(bits)` on the store handed to
  `TlsConfig::store` groups by another prefix). Removing a peer from the
  store refuses its next link: TLS session resumption is off at both
  ends unless the policy's `allow_resumption` keeps it.
- **A refusal says why.** The side that refuses a certificate names the
  reason: not pinned, not accepted by the trust store (a server's
  waiting client), or a chain where a pinned peer presents one
  certificate. The refused side hears the TLS alert
  `certificate_unknown`: a client is told the server does not trust it,
  in words naming its own fingerprint. TLS 1.3 lets a client finish its
  handshake before the server has checked its certificate, so that
  refusal arrives with the link's first read. A refused link answers
  `GUATIAO_ERR_BAD_VALUE`, as every failed trust check does, never
  `GUATIAO_ERR_GONE`.
- **The handshake** runs on the server's serving thread, bounded as a
  whole by the link's handshake timeout, so a client that connects and
  stalls holds only that thread for that long. Both ends require the
  application protocol `guatiao-remote`, except under a WebSocket
  (`wss://`), whose TLS names HTTP's (`http/1.1`), as a browser offers
  it, and requires none: the HELLO's `"GTRM"` names the protocol there.
  The server's TLS handshake and then the WebSocket's upgrade run on its
  first read.
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
  | `allow_resumption` | `false`: a resumed session skips the certificate check, so a removed peer could come back on one |
  | `waiting_per_source`, `waiting_total`, `waiting_rerecord_secs` | `16`, `1000`, `60` |
  | `acl` | none (the server's access control list, above) |

  Every setting looser than strict is listed by `TlsConfig::weakened()`,
  and the server prints each when it starts.
- **From C, Python or any language** mutual TLS is the source file's
  configuration keys above. A peer in another language speaks the
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

### A server by its authority

A client may also trust a server the way a browser does, by the
web PKI: one of the authorities it names issued the server's
certificate for the host the client dials, and the certificate is
within its dates. Nothing is pinned, so a server can renew its
certificate without telling its clients, and one server serves pages
and hosts alike.

```toml
connect = "tls://device.example.net:7443"
security = "server-tls"
key_file = "device.key"
roots = "platform"          # or a CA bundle: roots = "lab-ca.pem"
```

- `roots = "platform"` is the operating system's authorities and
  verifier (Windows, macOS, iOS, Android; elsewhere the system's CA
  bundle), in a build with the `platform-roots` feature; without it,
  `GUATIAO_ERR_UNSUPPORTED`. On Android the host initialises
  rustls-platform-verifier from its JVM before the first link.
- Any other text is a PEM file of authorities' certificates, read when
  the source is built (beside the manifest, in a peer manifest); a
  replaced bundle is a new peer, as a replaced key is.
- With `pins` (or a trust store) as well, a pinned certificate is taken
  whatever it names, and any other is checked against the roots. A
  pinned certificate the server presents with its chain is refused
  under the strict policy, as on every pinned link, unless the policy
  says `allow_chain_when_pinned`.
- The client dials by the host in `connect`, on every transport: a
  certificate for `device.example.net` is refused when the client dials
  `192.0.2.21`, in words naming the name dialled. A certificate no
  root issued, or one past its dates, is refused naming that. Each is
  `GUATIAO_ERR_BAD_VALUE`.
- Only a client verifies by roots; a listener trusts clients by pins or
  its store, and a server's identity is the certificate and chain the
  authority issued (`--identity`).

In Rust, `TlsConfig::roots(Roots::bundle_file(path))` (or
`Roots::bundle_pem(text)`, `Roots::platform()`), beside `pins` or
alone, on `TlsConnect`, `QuicConnect`, `WsConnect::tls` and
`WtConnect`. The tests run a test authority's certificates over every
transport; the platform's roots are tested refusing a self-issued
certificate only, since no publicly trusted server is reachable
offline.

### Over QUIC

`quic://host:port` (feature `quic`) is the same mutual TLS inside QUIC
(RFC 9000), over UDP: the same identity, pins or trust store, policy,
access control list, application protocol `guatiao-remote`, waiting
entries and refusals as `tls://`, and the same configuration keys
(`connect = "quic://..."` with `state_dir` or `identity_file`, `pins`,
`policy`). The server listens with `--listen quic://ADDRESS:PORT`; in
Rust, `transport::quic::{QuicConnect, QuicListen}` take the `TlsConfig`
above. At no security (`security = "none"`, `--security none`;
`QuicConnect::trusted`, `QuicListen::bind_trusted(address, acl)` in Rust)
each side presents a certificate it generated, nobody checks either, the
access control list decides by address alone and the key proof decides
the link; `Link::peer_id()` answers `None`. A listener first proves that a client receives at
the address it names (a QUIC Retry, as TCP's handshake does), then
decides by that address, refusing a denied source before any handshake;
a connection cannot move to another address afterwards.

A link is one bidirectional QUIC stream carrying the frames exactly as
over TCP. A link ends cleanly when the stream is finished, or the
connection is closed with code 1; code 0, also what a connection dropped
unclosed sends, is an end without it, read as an error.

**Datagrams.** Beside the calls a QUIC link carries datagrams
(RFC 9221): each is delivered whole or not at all, in any order, for
what a caller accepts losing (a sensor's readings, samples). In Rust,
`Link::datagrams()` answers `send(&[u8])`, `recv(timeout)` and
`max_size()` (the largest payload the path takes now; past it a send is
refused), and `None` over every other transport. A datagram is the
payload alone, nothing added, so a peer with any QUIC stack reads it.
For values the payload is a wire channel's frame (`kind:u8 channel:LEB128
payload`, "Values on the wire" in [encodings](encodings.md)): announce
the channel's schema, then send values, and read them with the channel's
receiver: `guatiao_wire_announce`, `guatiao_wire_send` and
`guatiao_wire_receiver_accept` in C, `wire` in every binding. An
announcement may be lost like any datagram: the sender repeats it until
the receiver knows the channel (or announces it over the calls). The
protocol's frames are unchanged: datagrams carry no calls.

quinn runs the connection on a small tokio runtime inside the
transport; the link stays on threads, which wait on it without needing
to be outside another runtime.

Measured on loopback, five runs each alternating with `tls://`, medians,
local and provisional (`benchmarks/results/remote-quic-*.jsonl`):

| link, loopback | p50 µs | p99 µs | calls/s, 32 threads | MB/s, 4 MiB | CPU µs/call, client / server |
| --- | --- | --- | --- | --- | --- |
| Windows ARM64, TLS | 32.0 | 85.6 | 108 965 | 617 | 37.5 / 36.7 |
| Windows ARM64, QUIC | 70.3 | 261.8 | 137 089 | 384 | 120.3 / 120.3 |
| WSL (aarch64), TLS | 60.1 | 188.6 | 59 149 | 1 223 | 67.9 / 67.4 |
| WSL (aarch64), QUIC | 217.3 | 2 321.5 | 33 764 | 226 | 129.9 / 132.0 |

On Windows a small call over QUIC costs about twice its time over TLS
and three times its CPU, and 32 threads calling at once go faster. In
WSL it costs four times its time and twice its CPU, most likely
because quinn runs on threads of its own and hands each datagram to the
link's: a call wakes threads several times over, and waking one is slow
in a virtual machine. Larger socket buffers stop the datagrams WSL drops but not the
gap (`benchmarks/README.md`). Prefer `tls://` unless datagrams are
wanted. Not measured: across a real network.

A name with several addresses (`localhost` is `::1` and `127.0.0.1`)
has them tried at once, each a quarter of a second after the one before
or as soon as that one failed: QUIC hears nothing back from an address
nobody listens at, so trying one at a time would wait out the timeout on
each. The first to link wins, and a refusal from whichever answers first
is the answer. Connects through one
`QuicConnect` run side by side, unless the policy keeps resumption: then
one verifier serves them all, so they take turns and try one address at
a time.

### Over WebTransport

`https://host:port/path` (feature `webtransport`) is WebTransport: an
HTTP/3 session over QUIC, what a browser opens with `new
WebTransport(url)`. It runs `quic://`'s TLS (the same identity, pins or
trust store, policy, access control list, Retry, no migration) under
HTTP/3's application protocol `h3`, and any of the three securities. A
page uses server TLS (the key or a token inside the link) or none (the
key alone); mutual TLS needs its browser to present a client
certificate, which Chrome did not to a profile holding none (whether it
presents one a user installed is not measured). A session carries one bidirectional stream with the frames
exactly as over TCP; the server takes every session, whatever its path,
and does not read the `Origin` a browser sends. Datagrams are the
session's (HTTP/3 datagrams): `Link::datagrams()` as over QUIC.

A browser trusts a certificate no authority issued when the page names
its SHA-256 (`serverCertificateHashes`), and only one valid for at most
14 days. So unless `--identity` or `--state-dir` names a certificate,
`guatiao-remote-server --listen https://ADDRESS:PORT` presents
certificates it makes itself: a
new one every six days, each valid for twelve days and two hours, the
next one made a period ahead. Its second line says the hashes a page
pins, the one presented and the next, and again whenever they change:

```text
guatiao-remote-server: serving https://192.0.2.21:7443/ (WebTransport), trusting clients by the key (server TLS), admitting every source
guatiao-remote-server: certificate hashes sha256:9f86d0... sha256:60303a...
```

`--hashes-file PATH` writes them as a JSON list (`["sha256:...",
"sha256:..."]`) at every change, for whatever serves the page to hand
on. A page that pinned both before a rotation still links after it; one
loaded more than a period before must fetch them again. A fingerprint is
the SHA-256 of the certificate's DER encoding, so a page passes its 32
bytes as `{ algorithm: 'sha-256', value }`. With `--identity` the server
presents that certificate instead, at any security: one an authority
issued for the server's name, which pages trust as they trust any site.
With `--state-dir` (TLS) it presents the identity there, which native
peers pin; the trust store decides clients as on every TLS transport.

Chrome refuses a certificate pinned by hash that carries no X.509v3
extension. The certificates the server makes carry a subjectAltName, a
subject key identifier and basicConstraints; one given with
`--identity` for a page to pin needs at least one extension and a
validity of at most 14 days.

In Rust, `transport::webtransport::{WtConnect, WtListen}`:
`WtListen::bind(address, &config)` presents the configuration's
identity, `.rotating(every)` (at most `ROTATE_MAX`, six days and 22
hours) makes and rotates its own, `.presenting(identity)` presents
another, and `certificate_hashes()` answers what a page pins;
`WtListen::bind_trusted(address, acl)` and `WtConnect::trusted(url)` run
no security. A source file's configuration names `connect =
"https://host:port/path"` with the keys of `quic://`. Not measured.

Not in this build: CA-signed client certificates (coqui-peer-tls's `ca`
feature) and TLS 1.2.

## The protocol

Every implementation, in any language, speaks the same frames. The
shared vectors are `crates/guatiao-source-remote/tests/vectors/remote.json`;
`crates/guatiao-source-remote/tests/c/` holds a codec, a client and a server
written in C over guatiao's C API, and the Dart client
(`bindings/dart/guatiao/lib/src/remote/`) and the TypeScript one
(`bindings/node/guatiao-remote/`) pass the same vectors.

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
| `0x01` | HELLO | `"GTRM"`, version `1`, trust (0 local, 1 trusted, 2 secure, 3 server TLS), nonce: 16 bytes, options: value |
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

The client sends HELLO naming the trust level its security runs at (0
`local` and 1 `trusted` for `none` on the same machine and across a
network, 2 `secure` for mutual TLS, 3 `server-tls`); the server answers
WELCOME, or GOODBYE with why it refuses (another trust level, a wrong
token). Over TLS the frames cross inside the TLS session, after its
handshake; a HELLO asking for the secure level over a transport that
proves no peer, or for server TLS over one that does not encrypt, is
refused. A peer that knows no trust 3 refuses it as a malformed HELLO
(GOODBYE `GUATIAO_ERR_BAD_VALUE`). Options are a map: `max_frame` (the
largest body the sender reads), `max_calls` (how many of the peer's
calls it runs at once), and `token` from a client whose server requires
one. Each
side sends no frame larger than the other's `max_frame`, and keeps no
more of its own calls open than the other's `max_calls`. Before the
handshake ends a body is at most 64 KiB.

At the trusted level the WELCOME's proof is HMAC-SHA256 over the key, of
`"guatiao-remote server"`, the client's nonce and the server's; the
client checks it, then sends PROOF, the same over
`"guatiao-remote client"`. Either side closes the link on a proof that
does not hold. At server TLS the same holds when the server has a key:
its WELCOME carries the proof, which asks the client for its own; a
WELCOME with an empty proof asks for none, and a client whose server
requires a token sent it in the HELLO. A server at server TLS holds a
key, a token or both, and a client asked for a key it does not hold
ends the link. At the other levels the proof is empty.

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
`max_calls`, or one the receiver cannot start a thread for, is refused
`GUATIAO_ERR_BUSY`; a body over `max_frame` ends the link. A call whose
serving fails without an answer of its own (a Rust panic on the receiver)
is answered `GUATIAO_ERR_INTERNAL`.
