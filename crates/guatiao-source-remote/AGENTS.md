# guatiao-source-remote — shipped API header

guatiao providers called across a process, a machine or a network: a
`Source` a host adds, a `Server` exposing a registry, and one framed
protocol over every transport. `#![deny(unsafe_code)]`; `unsafe` only in
`src/exports.rs` (the `guatiao::source!` expansion), `src/transport/shm.rs`
(the mapped rings) and `src/transport/rdma/verbs.rs` (libibverbs),
listed by `tests/unsafe_stays_in_exports.rs`. Depends on `guatiao`
(`load`), `guatiao-libffi` (the kind tables' mechanism: libffi built from
C), `guatiao-bridge` (kind descriptions) and `guatiao-serde` (`json`,
`toml`). The network features depend on git crates pinned by commit
(coqui-net-acl, coqui-peer, coqui-peer-tls), which crates.io refuses
until they are published there.

| feature | adds |
| --- | --- |
| `tcp` | TCP with no TLS: `transport::tcp`, the HMAC-SHA256 key proof (`hmac`, `sha2`, `getrandom`), admission by access control list (`coqui-net-acl`, re-exported as `acl`) |
| `tls` | `tcp` and `transport::tls`: TLS 1.3 over TCP, mutual TLS (`coqui-peer`, `coqui-peer-tls`, `rustls` with ring); with `ws`, `wss://` |
| `platform-roots` | `tls` and coqui-peer-tls's `platform-roots` (rustls-platform-verifier): `Roots::platform()`, `roots = "platform"`, a server verified by the operating system's authorities and verifier |
| `quic` | `tls` and `transport::quic`: QUIC, at mutual TLS or no security, with datagrams (`quinn` with rustls on ring, `tokio` inside the transport) |
| `local` | `transport::local`: a Unix socket (std) or a named pipe (`interprocess`, Windows only) |
| `shm` | `local` and `transport::shm`: two rings in a mapped file (`memmap2`) |
| `ws` | `tcp` and `transport::ws`: WebSocket, what a browser opens (`tungstenite`); `ws://` with no TLS, `wss://` over `tls`'s |
| `webtransport` | `quic` and `transport::webtransport`: WebTransport (HTTP/3 over QUIC), what a browser opens with the server's certificate pinned by its hash (`wtransport` over the same quinn) |
| `rdma` | `tcp` and `transport::rdma`: RDMA, experimental, Linux only; libibverbs opened at run time (`libloading`, `libc`, Linux only), so building needs no rdma-core |

## A host

```rust
RemoteSource::new(connect: Box<dyn Connect>, options: LinkOptions) -> RemoteSource   // Clone: one source
RemoteSource::from_config(&Map) -> Result<RemoteSource, ProviderError>  // as the source file reads it
impl Source for RemoteSource                       // registry.add_source(name, source)
source.link() -> Result<Arc<Link>, ProviderError> // connects unless connected; NOT_FOUND for described peers
source.disconnect(); source.objects_out() -> usize  // every peer's
source.weakened() -> Vec<(String, Weakening)>      // `tls`: the configured policy's settings looser than strict, by peer name ("" for one peer); described peers after a refresh
```

- **Peers described by manifests** (`dirs`, `manifests` in the
  configuration): every refresh reads the `type = "remote"` manifests
  there, a peer per manifest, named by its `name`. A peer's libraries and
  providers are `<name>/<id>`, so identical devices sit in one registry
  under the default keys. A manifest that changed (or whose key file did:
  its path and a hash of its bytes) is a new peer over a new link; the
  peer it replaced keeps its link until the registry closed every library
  it opened, so a new peer that cannot be listed or opened leaves the old
  library listed and answering. One that vanished takes its peer's
  libraries with it. The source's own settings are checked once, by
  `from_config`. Peers are listed at once, up to 16 on threads of their
  own; a connection and its handshake run outside the peer's lock, so
  `disconnect()` never waits for one (a `connect` peer's connection under
  way when it is called is ended, its `link()` answering `GONE`).
- A manifest describing no peer, and a peer that cannot be listed, are
  listed under the manifest's path as a candidate whose open refuses in
  words naming the file: a refresh reports a manifest once per content
  and a peer once per outage. A peer that cannot be listed (not reached,
  or its new link failing to list) keeps what its last listing answered,
  its calls answering `GONE`, until it is back.

- **list** connects when there is no live link (a `ChildCommand` starts
  its child again), asks the catalog, and lists each library under
  `<peer stamp>@<link generation>`: after a reconnect every library is new
  to the registry, which retires the old ones.
- **open** sends OPEN (the server holds the library's providers for the
  link) and builds a provider per catalog entry, one `ServedTable` per
  kind, filling only the slots the catalog lists: a slot the far table
  leaves out runs the kind's default here. Each provider carries its
  kinds' document, written from the catalog's descriptions (the kinds its
  tables serve and every object kind they reach), so
  `Provider::kind_descriptions()` answers on the host as it would for the
  peer's own provider.
- **close** sends CLOSE when the link that opened it still lives.
- A provider whose link has ended answers `GUATIAO_ERR_GONE` until a
  refresh opens it anew.

## A server

```rust
Described::new(); d.add_value(&Value)?; d.add_json(&str)?; d.load(&Path)?; d.kinds()
Server::new(registry: Registry, described: Described, options: LinkOptions) -> Server   // Clone
server.serve(conn: Connection) -> Result<Arc<Link>, LinkError>   // handshake here, then threads; kept until it ends
server.refresh() -> Refreshed        // a library that loads again gets a new stamp
server.registry() -> MutexGuard<Registry>
server.end_links(); server.links(); server.links_holding(); server.objects_out()   // end_links: every connection in its handshake too, at once
```

Serves every provider of a kind described either by `described` or by a
provider's own document (`Provider::kind_descriptions()`, which derived
and bridged providers carry); a provider of a kind neither describes is
left out of the catalog. A kind `described` names is served through that
description; otherwise the first provider describing a kind decides it
for the server's life. A provider whose own description of a kind is
called otherwise (a slot's arguments, answer or flags, the floor hash;
docs and slots past the shorter description do not count) is left out.
The catalog's `kinds` lists each description once.

Binary `guatiao-remote-server` (`--stdio` | `--listen tcp://ADDR | tls://ADDR | quic://ADDR | ws://ADDR | wss://ADDR | https://ADDR | local://NAME | shm://NAME | rdma://ADDR`,
`--security none|server-tls|mutual-tls`, `--range CIDR`..., `--acl PATH`, `--key-file`,
`--token-file PATH`, `--state-dir DIR`, `--identity PATH`, `--hashes-file PATH`,
`--pin FP`..., `--policy PATH`, `--rdma-device NAME`, `--kinds PATH`... (optional), `--load`,
`--scan`, `--source PATH[=JSON]`, `--refresh-ms`, `--max-calls`,
`--max-frame`, `--max-handshakes N`; `--state-dir DIR --waiting | --accept FP` pair clients and exit).
At most `--max-handshakes` connections (default 64) are in their handshake
at once; past it a new one is closed unread. A refused source (denied by
the access control list, or failing its handshake) is reported at most
once a minute per address, with how often it was held back (past 1024
addresses in a minute, a new one goes unreported).
`--security` is none on `--stdio`, `local://` and `shm://` (the only one
they run) and mutual-tls on the network transports unless told; `ws://`
and `rdma://` run none alone, `wss://` and `tls://` TLS alone,
`quic://` and `https://` any. `https://` (WebTransport) presents
certificates it makes, a new one every `ROTATE_EVERY`, unless
`--identity` (at any security) or `--state-dir` names one; its second stderr line is
`guatiao-remote-server: certificate hashes sha256:<hex> sha256:<hex>`
(the one presented, the next), said again at every change, and
`--hashes-file` writes them as a JSON list, written before the line.
`--trust` is refused with what to write now. No security across a
network needs `--range` or `--acl`; mutual TLS needs `--state-dir` or
`--identity`; server TLS needs those and `--key-file` or `--token-file`,
and refuses `--pin`. `--token-file` (the token every client presents: the
file's text, a byte order mark and one last line end left out; empty or
white space refused) runs at server TLS and mutual TLS only. The first stderr line is
`guatiao-remote-server: serving SCHEME://ADDR ...` (over TLS, `as
sha256:<hex>`, the fingerprint a client pins).

## Links

```rust
LinkOptions::new(Trust::{Local, Trusted, Secure, ServerTls})
    .key(&[u8]) .token(&str) .max_frame(bytes) .max_calls(n)
    .timeout(Option<Duration>) .handshake_timeout(Duration)
    .spin(Duration)    // the most a waiting thread polls before it sleeps; zero never polls
// defaults: 64 MiB frames, 64 calls, 30 s per call, 10 s handshake, 200 µs spin
link.alive() link.why() link.peer() link.peer_id() link.datagrams() link.trust() link.list() link.open(id, stamp)
link.close(id, stamp) link.kinds() link.ping() link.end()
LinkError::{Io, Protocol, Refused { status, why }, Trust}   // -> ProviderError
```

- **`peer_id()`** is the peer's verified fingerprint, `sha256:<hex>`, at
  the secure level (mutual TLS) and a client's at server TLS; `None`
  otherwise. The secure level over a transport that proves no peer
  (`FrameRead::peer_id` answers `None`) is refused at both ends.
- **Server TLS** (`Trust::ServerTls`, HELLO trust 3): a client needs a
  key or a token and a transport that proved the server (`peer_id()`);
  a server needs a key or a token and a transport that encrypts
  (`FrameRead::encrypted()`), else it refuses `UNSUPPORTED`. A server
  with a key sends its proof in WELCOME, which asks the client for its
  PROOF; a WELCOME with no proof asks none. A server's token is checked
  on HELLO.
- **Dropping a `Link` ends it.** Calls waiting answer `GONE`; objects held
  for the peer are dropped.
- **Polling adapts.** A thread waiting for an answer, or an idle pool
  thread, polls for 1.5x the typical wait it has seen on the link (an
  answer's round trip, the gap between the peer's calls), at least 50 µs,
  not at all when that is past `spin`, and not while as many threads poll
  as there are cores.
- **A call past its timeout** is cancelled (CANCEL) and answers `GONE`; an
  object its late answer carries is released. A call waits for the link's
  writer at most until its deadline, and CANCEL never waits: it is queued
  for the thread writing. A write the peer takes nothing of for the
  timeout ends the link (`FrameWrite::set_timeout`, set from it).
- **Nested calls** (a callback during a call) run on the thread waiting
  for the outer call; a call with no parent runs on the link's pool (at
  most `max_calls` threads; past it, or when a thread will not start,
  `GUATIAO_ERR_BUSY`).
- **A call an object makes on itself from inside one of its calls** is
  refused `BUSY` rather than deadlocked.
- The tables behind handles on a peer's objects are shared by a
  source's or a server's links; when their owner goes with a handle still
  out, they are kept for the process's life, never freed under it.

## Transports (`transport`)

```rust
trait FrameRead { fn read_frame(&mut self, &mut Vec<u8>, max) -> io::Result<bool>; fn set_timeout(..); fn peer_id(&self) -> Option<String>; fn encrypted(&self) -> bool; fn datagrams(&self) -> Option<Arc<dyn Datagrams>> }
trait Datagrams { fn send(&self, &[u8]) -> io::Result<()>; fn recv(&self, Option<Duration>) -> io::Result<Option<Vec<u8>>>; fn max_size(&self) -> Option<usize> }
trait FrameWrite { fn write_frame(&mut self, &[u8]); fn write_frames(&mut self, &[&[u8]]); fn write_frame_parts(&mut self, head, tail); fn set_timeout(..); fn shutdown(&mut self) }
struct Connection { reader, writer, peer: String, closer: Option<Box<dyn Fn()>> }
trait Connect { fn connect(&self) -> io::Result<Connection>; fn describe(&self) -> String }
memory::pair() -> (Connection, Connection)        // tests
child::ChildCommand::new(program).arg(..).args(..).env(..)   // Connect: the child's stdio
child::stdio() -> Connection                       // a server started as a child
stream::{StreamReader, StreamWriter}               // u32 LE length + body over any Read/Write
StreamWriter::with_shutdown(w, f).bounded_by(|w, timeout| ..)   // how the stream bounds a write
acl::{Acl { rules, default }, Rule::new(Action::Permit, "10.0.0.0/8".parse()?), Action, IpNet}   // tcp: coqui-net-acl
tcp::{TcpConnect::new("host:port").timeout(d), TcpListen::bind(addr, Acl)?, listen.acl()}   // tcp
tcp::TcpListen::accept() -> io::Result<Accepted::{Connection, Refused(SocketAddr)}>
tcp::{TcpListen::from_listener(TcpListener, Acl), from_stream(TcpStream)?}   // sockets the caller set up
tls::TlsConfig::{new(Identity), open(dir)?}.pins(ids).store(TrustStore).roots(Roots).policy(Policy)   // tls
tls::{TlsConfig::fingerprint(), weakened(), identity(), trust_store()}
tls::TlsConfig::{server_tls(self), is_mutual(), without_identity()?}   // server TLS: the client presents no certificate
tls::{TlsConnect::new("host:port", &TlsConfig)?.timeout(d), TlsListen::bind(addr, &TlsConfig)?, TlsListen::from_listener(TcpListener, &TlsConfig)?}
tls::TlsListen::accept() -> io::Result<tcp::Accepted>      // the handshake runs on the first read
tls::{Identity, PeerId, Policy, Roots, TrustStore, WaitingEntry, Weakening, IDENTITY_FILE}   // coqui-peer-tls's
tls::Roots::{bundle_file(path), bundle_pem(text), platform()}   // platform() with `platform-roots`
quic::{QuicConnect::new("host:port", &TlsConfig)?.timeout(d), QuicListen::bind(addr, &TlsConfig)?, QuicListen::from_socket(UdpSocket, &TlsConfig)?}   // quic
quic::{QuicConnect::trusted("host:port")?, QuicListen::bind_trusted(addr, Acl)?, QuicListen::from_socket_trusted(UdpSocket, Acl)?}   // quic, no security
quic::QuicListen::accept() -> io::Result<tcp::Accepted>    // the handshake runs on the first read
webtransport::{WtConnect::new("https://host:port/path", &TlsConfig)?.timeout(d), WtConnect::trusted(url)?}   // webtransport
webtransport::{WtListen::bind(addr, &TlsConfig)?, WtListen::bind_trusted(addr, Acl)?, from_socket(UdpSocket, &TlsConfig)?, from_socket_trusted(UdpSocket, Acl)?}
webtransport::WtListen::{rotating(self, Duration)?, presenting(self, Identity)?, certificate_hashes()? -> Vec<PeerId>, accept() -> io::Result<tcp::Accepted>}
webtransport::{ROTATE_EVERY, ROTATE_MAX}                   // six days; six days and 22 hours
local::{LocalConnect::new(name), LocalListen::bind(name)?.accept()}   // local: a path on Unix, a pipe name on Windows
shm::{ShmConnect::new(name), ShmListen::bind(name)?.accept(), RING}   // shm: the same names; the rings are mapped on the first read
ws::{WsConnect::new("ws://host:port/path").timeout(d), WsListen::bind(addr, Acl)?}   // ws
ws::{WsListen::from_listener(TcpListener, Acl), WsListen::accept() -> io::Result<tcp::Accepted>}
ws::{WsConnect::new("wss://host:port/path").tls(&TlsConfig)?, WsListen::bind_tls(addr, &TlsConfig)?, WsListen::from_listener_tls(TcpListener, &TlsConfig)?}   // ws + tls
rdma::{RdmaConnect::new("host:port").port(RdmaPort).timeout(d), RdmaListen::bind(addr, Acl)?.port(RdmaPort)}   // rdma
rdma::{RdmaListen::from_listener(TcpListener, Acl)?, RdmaListen::accept() -> io::Result<tcp::Accepted>}   // set up on the first read
rdma::{RdmaPort::first(), RdmaPort::named("rxe0").port(1).gid_index(1), devices() -> io::Result<Vec<String>>, RING, SETUP_TIMEOUT}
```

- **Admission.** `TcpListen`, `WsListen`, `RdmaListen`, `TlsListen`, `QuicListen` and `WtListen`
  bind any address, a wildcard included; nothing about the address or
  the list is refused (an empty `Acl` denies every peer). The first rule
  matching the source (and the local address and port) decides, else
  `default` (`Deny` unless set); `accept` closes a denied peer unread
  (`Accepted::Refused`). A rule naming a fingerprint never matches at the
  trusted level; a TLS listener decides it after the handshake, refusing
  a list naming a fingerprint of a digest other than SHA-256.
- **Server TLS** (`TlsConfig::server_tls`): a server presents its
  certificate and asks for none (coqui-peer-tls's configuration without
  client authentication, listed by `weakened()` as `client_auth`); a
  client presents none and verifies the server by its pins, store or roots; on
  `tls://`, `quic://` and `wss://`. A server's link then has no
  `peer_id()`, and its access control list decides by address alone. A
  mutual-TLS server refuses a client with no certificate.
- **TLS** (`tls`): `TlsConfig` trusts its pins when any are given, else
  its store's accepted peers; a server with a store records an unknown
  client as waiting (with its source address) once it proved its key, and
  refuses it, unless the access control list would deny that fingerprint
  from that source (then it is refused as it presents its certificate and
  not recorded); the waiting caps count an IPv6 source by its /64
  (`TrustStore::with_ipv6_prefix`). Session resumption is off at both
  ends unless the policy sets `allow_resumption` (a weakening), so a peer
  removed from the store is refused at its next link. A refusal names its
  reason (not pinned, not accepted, a chain where a pinned peer presents
  one certificate), and a refused link is `BAD_VALUE`, never `GONE`.
  A client may also trust a server by the web PKI (`roots(Roots)`,
  coqui-peer-tls's `Trust::Roots`, or `PinnedOrRoots` beside pins or a
  store): the chain to one of the roots, the dates and the name dialled
  are checked, a pinned certificate is taken under any name (served
  alone, or with its chain under `allow_chain_when_pinned`). Every
  client dials by the host it was given (`tls://`, `wss://` and
  `quic://` too, not coqui-peer-tls's placeholder name); under roots a
  host no certificate can name is refused when the connect is built.
  The roots' refusals name what to change: the name dialled
  (`NameMismatch`), an authority the roots do not hold (`NotTrusted`).
  A mutual-TLS listener given roots and no pins or store is refused:
  roots verify only a server this side dials.
  Strict policy by default (TLS 1.3); this build has no TLS 1.2, so
  `compatible` and `legacy` policies are refused when the configuration
  is built; a client's policy may hold no `acl`. ALPN `guatiao-remote` is required at both
  ends. `TlsConnect::connect` runs the handshake within its timeout; a
  listener's runs on the serving thread's first read, bounded as a whole
  by the read timeout the link sets (its handshake timeout). The reader
  and writers share one rustls state: no lock is held across a socket
  read, and after the handshake the reader never writes. A client the
  server refuses hears `certificate_unknown` and is told the server does
  not trust it, with its own fingerprint; one whose link the server ends
  before its first answer otherwise is told the server most likely
  refused it.
- **QUIC** (`quic`): the same `TlsConfig`, trust, policy, ALPN, waiting
  entries (with the address the client came from), refusals and words
  as TLS; resumption follows the policy (off). One bidirectional stream
  per link carries the frames. `accept` proves a client's address with a
  Retry, then refuses a denied source before any handshake; connection
  migration is off, so a link keeps the address it was admitted from.
  At no security (`trusted`, `bind_trusted`) each side presents a
  certificate it generated and nobody checks either: the list decides
  by address alone, the link's key proof decides the link, and
  `peer_id()` answers `None`.
- **WebTransport** (`webtransport`): QUIC's TLS, admission, Retry and
  words, under ALPN `h3`, every session taken whatever its path; one
  bidirectional stream per session carries the frames as over QUIC, and
  `datagrams()` are HTTP/3 datagrams. At no security the client
  presents no certificate and a native one checks none. A browser takes
  a certificate pinned by hash only when it is valid for at most 14
  days: `rotating(every)` presents certificates it makes (each valid
  two periods and two hours, the next made a period ahead, a fresh one
  after an idle period), `certificate_hashes()` names the one presented
  and the next. A server's handshake, HTTP/3 session and stream all run
  on the first read; a client's in `connect`. Chrome 155 refuses a
  pinned certificate with no X.509v3 extension: the ones `rotating`
  makes carry a subjectAltName, a subject key identifier and
  basicConstraints, and one given to `presenting` needs at least one
  extension.
  Datagrams (`FrameRead::datagrams`, `Link::datagrams`) are
  the payload alone, one QUIC DATAGRAM frame each, lost or whole; before
  the server's handshake `send` is `NotConnected` and `max_size` `None`;
  past `max_size` `InvalidInput`. quinn runs on a 2-thread tokio runtime
  per `QuicConnect` and per `QuicListen`; a thread waits on it by
  polling under its handle, so a caller inside another tokio runtime
  does not panic. A clean end (`shutdown`, or `closer` when no write is
  under way) finishes the stream, waits up to 2 s for the peer to
  acknowledge it, then closes with code 1; code 0 (a `closer` during a
  write, or a connection dropped unclosed) the peer reads as an error. A
  server refusing after the handshake closes with code 2; any TLS alert
  is a refusal (`PermissionDenied`, so `BAD_VALUE`), never `GONE`. The
  `closer` (or `shutdown`) before a server's handshake is through ends it
  at once (`ConnectionAborted`). A name's addresses are tried at once,
  each 250 ms after the one before (the first link wins, a refusal is
  the answer); connects through one `QuicConnect` run side by side
  unless the policy keeps resumption, when they take turns and try one
  address at a time.
- A frame a writer gets already carries its 4-byte length; a message
  transport drops it. `write_frames` gets frames queued while another
  thread wrote: by default one `write_frame` each, a stream writes them
  in one vectored write; a wrapper around a writer forwards it, and so
  does a wrapper around a stream's `Write` (`write_vectored`, else each
  frame is a system call of its own). `write_frame_parts` (a large last
  argument or answer as the frame's tail, from where it lies) is one
  vectored write on a stream; shared memory publishes every part it has
  room for at once.
- **`FrameWrite::set_timeout`**: shared memory and RDMA fail a write
  that finds no room for that long; TCP and Unix sockets set the send
  timeout, which on Windows bounds a whole `send` once earlier bytes are
  still unsent; a named pipe, a child's stdio and memory ignore it.
- **A Unix socket** is created `0600`; `LocalListen::bind` replaces a
  socket left by a listener that is gone and refuses anything else at the
  path. **A named pipe** has Windows' default access. It takes no read
  timeout, so while a read bound is set (the handshake) a thread watches
  each read and cancels one that outlives it or the connection's
  `closer`; past the handshake nothing ends a read blocked on it (the
  closer stops later reads): ending a link sends GOODBYE, and the peer
  closing its end ends that read. **Shared memory** gives the bound to
  the local connection it waits on for the peer's bytes, and its closer
  is that connection's too.
- **WebSocket** (`ws`): a frame's body is one binary message, with no
  length in front. `WsListen` admits peers by its access control list as `TcpListen` does;
  `accept` reads nothing, and a connection answers the HTTP upgrade on
  its first read, so a client that connects and stalls holds only the
  thread serving it; a write before the upgrade is refused
  (`NotConnected`). Any path is accepted; no subprotocol, no extension,
  the `Origin` header unread (the key proof is the gate). A text
  message, an unmasked client frame or a message over `max` is an error;
  a close or a dropped connection ends the reads as a stream's end of
  file does (tungstenite does not tell a drop inside a message from one
  between).
  The reader answers a ping or a close only when the writer is not
  writing, else on a later read. A frame is copied into a message on its
  way out. **`wss://`** (with `tls`): `WsConnect::tls` and
  `WsListen::bind_tls` run the TLS of `tls://` (identity, pins or store,
  policy and its `acl`, refusals as `BAD_VALUE`) under the upgrade,
  naming ALPN `http/1.1` and requiring none (a browser offers it or
  nothing); a server's TLS handshake and upgrade run on its first read.
  A `wss://` URL without TLS given is `Unsupported`; `ws://` with it,
  `InvalidInput`.
- **Shared memory**: the client creates the file (`/dev/shm`, else the
  temporary directory; `0600`; removed once mapped on Unix, deleted with
  its last handle on Windows), sends its path once over the local
  connection, and the server maps it. Each direction is a 4 MiB byte
  ring; frames of any size stream through it. A reader polls 50 µs, then
  sleeps on the local connection until rung; the peer's end there ends
  the link. Ring counters are checked on every read: a ring claiming more
  than 4 MiB ends the link.
- **RDMA** (experimental, Linux; elsewhere every call answers
  `Unsupported`): a TCP connection admitted by its access control list as `TcpListen`
  admits one carries a 64-byte record each way (queue pair number, PSN,
  GID, MTU, the inbox's address and key), then a ready byte; after that
  every frame crosses as RDMA writes with immediate data (the byte
  count) into the peer's 4 MiB inbox, from a 4 MiB staging area, both
  registered per link (8 MiB locked: `RLIMIT_MEMLOCK` or `CAP_IPC_LOCK`).
  The reader writes how far it has read into a counter after the writer's
  inbox every quarter ring; the TCP connection stays open as the peer's
  liveness and its end is the link's. Device: the first listed, port 1,
  the first IPv4 GID (RoCE v2), else GID 0; `RdmaPort` picks another.
  No device: `devices()` and connect fail saying so. The tests needing a
  device are the package `rdma-tests/` (a workspace of its own, not run by
  `cargo test --workspace`): `cargo test --manifest-path
  crates/guatiao-source-remote/rdma-tests/Cargo.toml` where a device exists
  (`GUATIAO_RDMA_DEVICE` names one); without one every test fails.
- **Windows:** one TCP socket is shared by `Arc` between reader, writer
  and closer, never `try_clone`d: a duplicated handle's `shutdown` and
  read timeout do not reliably reach a read blocked on another. A
  socket's `shutdown` does not wake a `recv` already blocked there, so a
  closer (TCP, TLS, WebSocket) shuts the socket down and then cancels its
  I/O in flight (`CancelIoEx`); a read started between the two ends at
  once on the shut socket.

## Configuration of the source file (`config::client_of`)

| key | holds |
| --- | --- |
| `connect` | `tcp://host:port`, `tls://host:port`, `quic://host:port`, `ws://host:port/path`, `wss://host:port/path`, `https://host:port/path` (WebTransport), `local://name`, `shm://name`, `rdma://host:port` |
| `rdma_device` | the RDMA device `rdma://` goes through (default: the first) |
| `child` | list: the server program and its arguments |
| `security` | `none`, `server-tls`, `mutual-tls` (`config::Security`); none on a child, `local://`, `shm://`, mutual-tls elsewhere unless named |
| `key`, `key_file` | the pre-shared key (none; server TLS when asked) |
| `token` | a client's token, for a server that requires one |
| `state_dir`, `identity_file` | TLS: this side's identity (and trust store), made when absent; one of them |
| `pins` | TLS: list of the server fingerprints trusted |
| `roots` | TLS: also trust a server an authority issued a certificate for the host dialled: `"platform"` (feature `platform-roots`, else `UNSUPPORTED`) or a PEM CA bundle's path, read when the source is built (`config::roots_of`) |
| `policy` | TLS: map, coqui-peer-tls's `Policy` text form v1 (`config::policy_of`) |
| `timeout_ms` | per call; `0` none |
| `max_frame`, `max_calls` | the link's bounds |
| `dirs`, `manifests` | instead of `connect`/`child` (both is `BAD_VALUE`): peer manifests, the keys above every peer's unless its manifest names them |

A scheme this build lacks is `GUATIAO_ERR_UNSUPPORTED`, naming it, as is
a TLS security in a build without `tls`. At server TLS a client names
`pins`, a store or `roots`, and needs no identity.
`ws://`, `rdma://`, `local://`, `shm://` and a child run `none` alone,
`wss://` and `tls://` TLS alone; another is `BAD_VALUE` naming the fix,
as is the former `trust` key. `tcp://`, `quic://` and `https://` run any.
`config::{security_of, Security::default_for, Security::trust, acl_of,
policy_of, roots_of, tls_config_of}` read those values.

A peer manifest (`docs/encodings.md`, "A peer"): `type = "remote"`,
`name` (no `/`), `connect`, `security`, `key_file` (beside the manifest),
`timeout_ms`, `max_frame`, `max_calls`, `rdma_device`, `pins`, `roots`
(a bundle beside the manifest, or `platform`), `policy`,
`identity_file` and `state_dir` (beside the manifest); any other key is
refused, `key` and `token` (secrets) and `child` with words of their own.

## The protocol (`protocol`)

`Frame::{decode(&[u8], Alloc), encode(&mut Vec<u8>, max)}`, `start`,
`finish`, `start_call`, `start_reply`, `refusal`; `args::{encode_args,
decode_args, encode_answer, decode_answer, encode_writebacks, Decoded,
ObjectRef}`; `catalog::Catalog`. No I/O. The frames, fields and argument
forms are in `docs/remote.md`; `tests/vectors/remote.json` has them as
bytes.

## Gotchas

- **The library file exports `guatiao_source`**: a binary linking this
  crate's rlib beside another crate that exports it does not link.
- **A server serves a kind only with its description**: a hand-written
  C provider carries none, so pass the kind crate's `<crate>.kinds.json`
  (or serve a provider that describes the kind beside it).
- **Object proxies fill every slot**: a slot the far object leaves out
  answers `UNSUPPORTED`, not the kind's default.
