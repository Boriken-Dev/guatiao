# guatiao-source-remote — shipped API header

guatiao providers called across a process, a machine or a network: a
`Source` a host adds, a `Server` exposing a registry, and one framed
protocol over every transport. `#![deny(unsafe_code)]`; `unsafe` only in
`src/exports.rs` (the `guatiao::source!` expansion), `src/transport/shm.rs`
(the mapped rings) and `src/transport/rdma/verbs.rs` (libibverbs),
listed by `tests/unsafe_stays_in_exports.rs`. Depends on `guatiao`
(`load`), `guatiao-libffi` (the kind tables' mechanism: libffi built from
C), `guatiao-bridge` (kind descriptions and configuration files) and `guatiao-serde` (`json`,
`toml`). The network features depend on coqui-net-acl, coqui-peer and
coqui-peer-tls: coqui-net-acl at `=0.0.0-alpha.0`, coqui-peer and coqui-peer-tls at
`=0.0.0-alpha.1`.

**Not publishable yet.** `ws` and `webtransport` take their channels from
[coqui-web](https://github.com/Boriken-Dev/coqui-web), a git dependency
pinned to a revision, and a crate with a git dependency cannot be
published: this crate waits for coqui-web's first release. coqui-web
brings tungstenite and wtransport (and mio, tokio) with it; this crate
names none of them.

| feature | adds |
| --- | --- |
| `tcp` | TCP with no TLS: `transport::tcp`, the HMAC-SHA256 key proof (`hmac`, `sha2`, `getrandom`), admission by access control list (`coqui-net-acl`, re-exported as `acl`) |
| `tls` | `tcp` and `transport::tls`: TLS over TCP (1.3, and 1.2 where the policy allows), mutual TLS or server TLS (`coqui-peer`, `coqui-peer-tls`, `rustls` with ring); with `ws`, `wss://` |
| `platform-roots` | `tls` and coqui-peer-tls's `platform-roots` (rustls-platform-verifier): `Roots::platform()`, `roots = "platform"`, a server verified by the operating system's authorities and verifier |
| `quic` | `tls` and `transport::quic`: QUIC, at mutual TLS or no security, with datagrams (`quinn` with rustls on ring, `tokio` inside the transport) |
| `local` | `transport::local`: a Unix socket (std) or a named pipe (`interprocess`, Windows only) |
| `shm` | `local` and `transport::shm`: two rings in a mapped file (`memmap2`) |
| `ws` | `tcp` and `transport::ws`: WebSocket, what a browser opens (`coqui-web`, `url`, `coqui-peer`); `ws://` with no TLS and on loopback unless named, `wss://` over `tls`'s |
| `webtransport` | `tls` and `transport::webtransport`: WebTransport (HTTP/3 over QUIC), what a browser opens with the server's certificate pinned by its hash (`coqui-web`, `url`); mutual TLS, server TLS or no security, with datagrams |
| `rdma` | `tcp` and `transport::rdma`: RDMA, experimental, Linux only; libibverbs opened at run time (`libloading`, `libc`, Linux only), so building needs no rdma-core |

## A host

```rust
RemoteSource::new(connect: Box<dyn Connect>, options: LinkOptions) -> RemoteSource   // Clone: one source
RemoteSource::from_config(&Map) -> Result<RemoteSource, ProviderError>  // as the source file reads it
impl Source for RemoteSource                       // registry.add_source(name, source)
source.link() -> Result<Arc<Link>, ProviderError> // connects unless connected; NOT_FOUND for described peers
source.disconnect(); source.objects_out() -> usize  // every peer's
source.weakened() -> Vec<(String, Weakening)>      // `tls`: the configured policy's settings looser than modern (TLS 1.3 only), by peer name ("" for one peer); described peers after a refresh
```

- **Notes**: every setting `weakened()` lists is also a note
  (`Source::notes`, so a host in any language reads it in its refresh's
  `notes`): subject the peer's name (empty for the one peer of a `connect`
  or `child` configuration), text `weaker than modern: S = V (modern: M)`,
  detail `{"setting", "value", "modern"}`, each told the first refresh
  that finds it. `guatiao-remote-server` prints its sources' notes as
  `guatiao-remote-server: SOURCE: [SUBJECT: ]TEXT`.

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
Kinds::new(); k.add_value(&Value)?; k.add_json(&str)?; k.load(&Path)?   // guatiao_bridge::Kinds, re-exported
Server::new(registry: Registry, kinds: Kinds, options: LinkOptions) -> Server   // Clone
server.serve(conn: Connection) -> Result<Arc<Link>, LinkError>   // handshake here, then threads; kept until it ends
server.refresh() -> Refreshed        // a library that loads again gets a new stamp
server.registry() -> MutexGuard<Registry>
server.end_links(); server.links(); server.links_holding(); server.objects_out()   // end_links: every connection in its handshake too, at once
```

Serves every provider of a kind described either by `kinds` or by a
provider's own document (`Provider::kind_descriptions()`, which derived
and bridged providers carry); a provider of a kind neither describes is
left out of the catalog. A kind `kinds` names is served through that
description; otherwise the first provider describing a kind decides it
for the server's life. A provider whose own description of a kind is
called otherwise (a slot's arguments, answer or flags, the floor hash;
docs and slots past the shorter description do not count) is left out.
The catalog's `kinds` lists each description once, written from it: the
given ones in name order, then each a provider adds as it is learned.

Binary `guatiao-remote-server` (`--stdio` | `--listen tcp://ADDR | tls://ADDR | quic://ADDR | ws://ADDR | wss://ADDR | https://ADDR | local://NAME | shm://NAME | rdma://ADDR`,
`--security none|server-tls|mutual-tls`, `--range CIDR`..., `--acl PATH`, `--key-file`,
`--token-file PATH`, `--state-dir DIR`, `--identity PATH`, `--hashes-file PATH`,
`--pin FP`..., `--policy PATH`, `--rdma-device NAME`, `--kinds PATH`... (optional), `--load`,
`--scan`, `--source PATH[=JSON]`, `--import-search library|path`, `--refresh-ms`, `--max-calls`,
`--max-frame`, `--max-handshakes N`; `--state-dir DIR --waiting | --accept FP` pair clients and exit).
At most `--max-handshakes` connections (default 64) are in their handshake
at once; past it a new one is closed unread. A refused source (denied by
the access control list, or failing its handshake) is reported at most
once a minute per address, with how often it was held back (past 1024
addresses in a minute, a new one goes unreported).
`--security` is none on `--stdio`, `local://` and `shm://` (the only one
they run) and mutual-tls on the network transports unless told; `ws://`
(loopback unless `--plain-beyond-loopback`) and `rdma://` run none alone,
`wss://` and `tls://` TLS alone, `quic://` and `https://` any.
`--plain-beyond-loopback` (`ws://` only) lets it listen on any address with
nothing encrypted, and it says so at start as a weakening
(`weaker than modern: plain_beyond_loopback = true`). `https://` (WebTransport) presents
certificates it makes, a new one every `ROTATE_EVERY`, unless
`--identity` (at any security) or `--state-dir` names one; its second stderr line is
`guatiao-remote-server: certificate hashes sha256:<hex> sha256:<hex>`
(the one presented, the next), said again at every change, and
`--hashes-file` writes them as a JSON list, written before the line.
No security across a
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
  on HELLO, a client presenting none refused. A token empty or only
  white space is refused at either end: the client's before it sends,
  the server's on every HELLO (`UNSUPPORTED`).
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
  most `max_calls` calls not yet answered, on at most as many threads;
  past it, or when a thread will not start, `GUATIAO_ERR_BUSY`). A call's
  place is free once its answer is sent, so a peer calling again the
  moment it has an answer is not refused.
- **A call an object makes on itself from inside one of its calls** is
  refused `BUSY` rather than deadlocked.
- The tables behind handles on a peer's objects are shared by a
  source's or a server's links; when their owner goes with a handle still
  out, they are kept for the process's life, never freed under it.

## Transports (`transport`)

```rust
trait FrameRead { fn read_frame(&mut self, &mut Vec<u8>, max) -> io::Result<bool>; fn read_frame_in(&mut self, &mut Vec<u8>, max, room: &mut dyn FnMut(usize) -> Vec<u8>) -> io::Result<bool>; fn set_timeout(..); fn peer_id(&self) -> Option<String>; fn encrypted(&self) -> bool; fn datagrams(&self) -> Option<Arc<dyn Datagrams>> }
// read_frame_in: room(n) gives the buffer for a frame of n bytes buf cannot hold (the link's kept one); a wrapper forwards it, as peer_id; default: read_frame
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
webtransport::{WtListen::bind(addr, &TlsConfig)?, WtListen::bind_trusted(addr, Acl)?}
webtransport::WtListen::{rotating(self, Duration)?, presenting(self, Identity)?, certificate_hashes()? -> Vec<PeerId>, accept() -> io::Result<tcp::Accepted>}
webtransport::{ROTATE_EVERY, ROTATE_MAX}                   // six days; six days and 22 hours
local::{LocalConnect::new(name), LocalListen::bind(name)?.accept()}   // local: a path on Unix, a pipe name on Windows
shm::{ShmConnect::new(name), ShmListen::bind(name)?.accept(), RING}   // shm: the same names; the rings are mapped on the first read
ws::{WsConnect::new("ws://host:port/path").timeout(d), WsListen::bind(addr, Acl)?}   // ws, loopback alone
ws::{WsConnect::beyond_loopback(self), WsListen::bind_beyond_loopback(addr, Acl)?, plain_beyond_loopback()}   // a plain ws:// beyond loopback: a weakening
ws::WsConnect::local_addr(self, SocketAddr) -> WsConnect   // binds each connection's socket first (port 0: any); refused: PermissionDenied
webtransport::WtConnect::local_addr(self, SocketAddr) -> WtConnect   // the same; WtConnect is Clone, a clone sharing the TLS session store (so it can resume a session)
transport::FrameRead::set_max_frame_after(&mut self, messages: usize, bytes: usize) -> bool   // stage the read bound by position (default false: cannot); the link calls it with (1, max_frame) before the WELCOME where a PROOF follows
transport::FrameRead::set_read_ahead(&mut self, frames: usize) -> io::Result<()>   // frames a message transport holds ahead of the reader (default Err(Unsupported): none to set); WebSocket and WebTransport channels start at 1, the link sets 64 once the peer is admitted
ws::WsListen::accept() -> io::Result<tcp::Accepted>
ws::{WsConnect::new("wss://host:port/path").tls(&TlsConfig)?, WsListen::bind_tls(addr, &TlsConfig)?}   // ws + tls
rdma::{RdmaConnect::new("host:port").port(RdmaPort).timeout(d), RdmaListen::bind(addr, Acl)?.port(RdmaPort)}   // rdma
rdma::{RdmaListen::from_listener(TcpListener, Acl)?, RdmaListen::accept() -> io::Result<tcp::Accepted>}   // set up on the first read
rdma::{RdmaPort::first(), RdmaPort::named("rxe0").port(1).gid_index(1), devices() -> io::Result<Vec<String>>, RING, SETUP_TIMEOUT}
```

- **Admission.** `TcpListen`, `WsListen`, `RdmaListen`, `TlsListen`, `QuicListen` and `WtListen`
  bind any address, a wildcard included; nothing about the address or
  the list is refused (an empty `Acl` denies every peer). The first rule
  matching the source (and the local address and port) decides, else
  `default` (`Deny` unless set); `accept` closes a denied peer unread
  (`Accepted::Refused`). A rule naming a fingerprint never matches a peer
  presenting no certificate (no security, server TLS: `decide_anonymous`,
  `Policy::admits_anonymous`); at mutual TLS coqui-peer-tls decides it as
  the client presents its certificate, before pins or the store (TLS and
  `wss://` through the handshake's addresses, QUIC through a
  configuration built per connection by coqui-peer-tls's
  `ServerConfigs::for_connection`, which shares the listener's sessions
  and keeps the refusal for the server's words). `WsListen` and `WtListen`
  decide a source's address in coqui-web's admission, before any handshake,
  and report the sources it turned away as `Accepted::Refused`. A listener whose list
  names something that is not a SHA-256 fingerprint is refused as it is
  made.
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
  removed from the store is refused at its next link; a resumed session
  (a mutual `wss://` too) runs no verifier, so the store and the list's
  fingerprint rules are not asked again for it. A WebTransport listener
  keeps a post-handshake check of the list's fingerprint rules
  (`web::list_permits`) where resumption is on, so resumption does not
  skip them there. A refusal names its
  reason, one wording for each of coqui-peer-tls's `Refusal`s (the
  list by address or by fingerprint, `rule N` counted in tens or `its
  default`; not pinned; not accepted; a chain where a pinned peer
  presents one certificate; not trusted or the wrong name by the roots)
  and for an unusable CA bundle (`PeerError::CaFile`, "the roots
  (`roots`) cannot be used"); a client hearing `access_denied` is told
  the server's list refuses its address. A refused link is `BAD_VALUE`,
  never `GONE`. An identity file may hold the certificate's chain after
  it (`Identity::open_or_create` reads it), and every listener presents
  the whole chain.
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
  The modern policy by default (TLS 1.3 only), in `TlsConfig::new` and
  in a `policy` map naming no `profile` (coqui-peer-tls's own default is
  `intermediate`); `intermediate` and `legacy` add TLS 1.2, which QUIC
  and WebTransport never run (a policy allowing only 1.2 is refused
  there); a client's policy may hold no `acl`. ALPN `guatiao-remote` is required at both
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
- **WebTransport** (`webtransport`, through coqui-web): the session's
  TLS from the same `TlsConfig` (identity, pins, trust store, roots,
  policy) under ALPN `h3`, at mutual TLS, server TLS or no security. The
  channel reports the certificate the peer presented (`peer_certificates`,
  the chain TLS accepted), and `peer_id()` is its fingerprint at mutual
  TLS on both ends and a client's at server TLS. One bidirectional stream
  per session carries the frames as over QUIC (coqui-web's own `u32` length
  framing, the same bytes), and `datagrams()` are HTTP/3 datagrams. At no
  security the client presents no certificate and a native one checks
  none. Each connection's TLS configuration is built from its source
  and local address (coqui-web's `with_tls_per_connection`, through
  coqui-peer-tls's `ServerConfigs::for_connection`, built once at bind so
  a policy or a rule it refuses, a fingerprint that is not SHA-256 among
  them, fails the bind): the policy's list decides in the TLS, its
  fingerprint rules at the client's certificate, and a trust store records
  a stranger waiting under its source, as on `tls://` and `quic://`. A
  resolver over the configuration's own keeps its address gate and
  presents the certificate the listener holds now (rotation); the
  listener's base configuration is the fail-closed fallback. The list's
  address rules are also decided in coqui-web's admission, which reports the
  sources turned away as `Accepted::Refused`. A client the server's TLS
  refuses (not pinned, not accepted, denied by the list) fails its dial
  with `Failure::Certificate` and is told `BAD_VALUE`, "the server does not
  trust this client". A source the list denies by address, or any
  connection the server declines, is QUIC's CONNECTION_REFUSED, which
  coqui-web reports as `Failure::Permission`; a client cannot tell which,
  and says "refused the connection (by its access control list, ... or the
  server refused the connection)" (`BAD_VALUE`).
  A browser takes a certificate pinned by hash only when it is valid for
  at most 14 days: `rotating(every)` presents certificates it makes (each
  valid two periods and two hours, the next made a period ahead, a fresh
  one after an idle period), `certificate_hashes()` names the one
  presented and the next. A server's handshake, HTTP/3 session and stream
  all run on the first read, on the thread serving the link, which the
  `closer` (or `shutdown`) cancels at once (`ConnectionAborted`, the
  session closed); a client's in `connect`, which tries each address the
  name resolves to. Chrome 155 refuses a pinned certificate with no X.509v3
  extension: the ones `rotating` makes carry a subjectAltName, a subject
  key identifier and basicConstraints, and one given to `presenting` needs
  at least one extension. A client's own failure is a failure of the dial:
  a refused certificate reads as the verifier's words, a server that did
  not answer as `TimedOut` (on Windows an unreachable server is too: quinn
  surfaces no ICMP error), a QUIC handshake that did not finish as
  `ConnectionRefused` (so `GONE`).
- **QUIC's datagrams, runtime and ends.** Datagrams (`FrameRead::datagrams`,
  `Link::datagrams`, over QUIC and WebTransport) are
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
- **WebSocket** (`ws`, through coqui-web): a frame's body is one binary
  message, with no length in front (coqui-web's binary frames). `WsListen`
  admits peers by its access control list as `TcpListen` does, by address,
  in coqui-web's admission, and reports the sources it turned away as
  `Accepted::Refused`; `accept` reads nothing, and a connection answers
  the HTTP upgrade on its first read, on the thread serving the link,
  which the `closer` cancels at once (the socket closes), so a client that
  connects and stalls holds only that thread, for coqui-web's handshake
  bound (10 s); a write before the upgrade is refused (`NotConnected`).
  **`ws://` is for this machine alone** unless named: coqui-web refuses to
  listen on, or dial, a plain WebSocket beyond loopback (`PermissionDenied`,
  naming `plain_beyond_loopback` and `wss://`), and
  `WsListen::bind_beyond_loopback` / `WsConnect::beyond_loopback` (the
  `plain_beyond_loopback` key, `--plain-beyond-loopback`) choose it: nothing
  is encrypted and the key proof and the list alone decide, a weakening
  `RemoteSource::weakened()` lists for a source made from a configuration
  and the server prints at start (a source made by `RemoteSource::new`
  holds a connector its host can ask: `WsConnect::plain_beyond_loopback`).
  On a wildcard bind a list rule naming the address or port a client came
  to cannot be decided and refuses the client, as on the TLS listeners.
  Any path is accepted, the `Origin` header unread (the key proof is
  the gate); a text message or a message over `max` ends the channel
  (`InvalidData`); a close or a dropped connection ends the reads as a
  stream's end of file does. The channel is told apart what it reads
  (`set_max_receive`, the link's `max_frame`, raised by
  `FrameRead::set_max_frame`: a channel judges a message by the bound in
  force when it reads it off the socket, ahead of the reader, so a link
  raises it before it can be sent a large frame, on a server before its
  reply to the HELLO goes (where a PROOF is read, staged with
  `FrameRead::set_max_frame_after(1, max_frame)` through the channel's
  `set_max_receive_after`: the PROOF is judged at the handshake's bound
  and every frame after it at the full one, so a stranger cannot send a
  large message in its place and a call pipelined behind it is read; a
  transport that cannot stage keeps the handshake's bound until the PROOF
  checks) and on either side before the link is returned, not when the
  reader first asks. The first message is judged at the handshake's bound
  from the channel's start (`limits()` sets coqui-web's `max_message`).
  A channel also reads one message ahead of the link (`limits()` sets
  coqui-web's `pending` to 1) until the link admits the peer and raises it
  to 64 (`FrameRead::set_read_ahead`, through the channel's `set_pending`;
  a transport without one answers `Unsupported` and changes nothing): a
  server once the PROOF holds, or before its reply where no PROOF is read,
  a client once the WELCOME and its proof hold. So a client past the
  address rules that sends a junk PROOF and then large messages makes this
  side hold one message of `max_frame` behind the PROOF, plus coqui-web's
  buffers below whole messages (under 128 KiB on a WebSocket, under 16 KiB
  on WebTransport, TLS's, and the TCP receive buffer or QUIC stream
  window), until the link checks the PROOF and closes the channel
  (milliseconds; at most the handshake timeout if the serving thread
  stalls); its sends wait meanwhile. A stream never holds more than the
  handshake's bound before the proof) and what it sends
  (`set_max_send`, `FrameWrite::set_max_frame`, the peer's, known once the
  handshake agreed it); a frame past the peer's is refused by the writer
  (`InvalidInput`) and the reader keeps its own size check. A write waits
  for room in coqui-web's send queue for the link's write timeout
  (`FrameWrite::set_timeout`, through `send_timeout`; none: while the
  channel lives) and then fails `TimedOut`; a dead worker is `PathLost`,
  which ends the link (`GONE`). **`wss://`** (with `tls`): `WsConnect::tls`
  and `WsListen::bind_tls` run the TLS of `tls://` (identity, pins or
  store, roots, policy and its `acl`, refusals as `BAD_VALUE`) under the
  upgrade, naming ALPN `http/1.1` and requiring none (a browser offers it
  or nothing). The channel reports the peer's certificate chain
  (`peer_certificates`, a resumed TLS 1.3 session's too), and `peer_id()` is
  its fingerprint at mutual TLS on both ends and a client's at server TLS.
  A mutual-TLS listener keeps the policy's list in its TLS and runs each
  connection's handshake on the thread serving it inside
  `coqui_peer_tls::with_addresses(source, bound)`, so the list's
  fingerprint rules and a store's waiting entries see the connection's
  addresses; a rule of the list naming the local address or port is decided
  by the address the listener is bound to, and refuses every client only on
  a wildcard bind. A `wss://` URL without TLS given is `Unsupported`;
  `ws://` with it, `InvalidInput`.
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
| `plain_beyond_loopback` | `true` lets a `ws://` URL name a host beyond this machine, with nothing encrypted (default `false`); a weakening, listed by `RemoteSource::weakened()` (feature `tls` or `ws`) |
| `child` | list: the server program and its arguments |
| `security` | `none`, `server-tls`, `mutual-tls` (`config::Security`); none on a child, `local://`, `shm://`, mutual-tls elsewhere unless named |
| `key`, `key_file` | the pre-shared key (none; server TLS when asked) |
| `token`, `token_file` | a client's token, for a server that requires one: text, or a file's text with a BOM and one last line end left out (`config::token_in`, as `--token-file` reads it); both is `BAD_VALUE`; empty or only white space refused |
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
`wss://` and `tls://` TLS alone; another is `BAD_VALUE` naming the fix.
`tcp://`, `quic://` and `https://` run any.
`config::{security_of, Security::default_for, Security::trust, acl_of,
policy_of, roots_of, tls_config_of, token_in}` read those values.

A peer manifest (`docs/encodings.md`, "A peer"): `type = "remote"`,
`name` (no `/`), `connect`, `security`, `key_file` and `token_file`
(beside the manifest, replacing the source's `key` and `token`),
`timeout_ms`, `max_frame`, `max_calls`, `rdma_device`, `pins`, `roots`
(a bundle beside the manifest, or `platform`), `policy`,
`identity_file` and `state_dir` (beside the manifest); any other key is
refused, `key` and `token` (secrets, naming `key_file` and `token_file`)
and `child` with words of their own. A peer's stamp hashes its key,
token, identity and CA bundle files: an edited one is a new peer.

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
