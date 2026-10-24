# guatiao-remote — shipped API header

guatiao providers called across a process, a machine or a network: a
`Source` a host adds, a `Server` exposing a registry, and one framed
protocol over every transport. `#![deny(unsafe_code)]`; `unsafe` only in
`src/exports.rs` (the `guatiao::source!` expansion). Depends on `guatiao`
(`load`), `guatiao-libffi` (the kind tables' mechanism: libffi built from
C), `guatiao-bridge` (kind descriptions) and `guatiao-serde` (`json`).

| feature | adds |
| --- | --- |
| `tcp` | trusted TCP: `transport::tcp`, the HMAC-SHA256 key proof (`hmac`, `sha2`, `getrandom`) |
| `local` | `transport::local`: a Unix socket (std) or a named pipe (`interprocess`, Windows only) |
| `shm` | `local` and `transport::shm`: two rings in a mapped file (`memmap2`) |

## A host

```rust
RemoteSource::new(connect: Box<dyn Connect>, options: LinkOptions) -> RemoteSource   // Clone: one source
impl Source for RemoteSource                       // registry.add_source(name, source)
source.link() -> Result<Arc<Link>, ProviderError> // connects unless connected
source.disconnect(); source.objects_out() -> usize
```

- **list** connects when there is no live link (a `ChildCommand` starts
  its child again), asks the catalog, and lists each library under
  `<peer stamp>@<link generation>`: after a reconnect every library is new
  to the registry, which retires the old ones.
- **open** sends OPEN (the server holds the library's providers for the
  link) and builds a provider per catalog entry, one `ServedTable` per
  kind, filling only the slots the catalog lists: a slot the far table
  leaves out runs the kind's default here.
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
server.end_links(); server.links(); server.links_holding(); server.objects_out()
```

Serves every provider of a kind `described` describes; others are left
out of the catalog. The catalog's `kinds` lists each description once.

Binary `guatiao-remote-server` (`--stdio` | `--listen tcp://ADDR | local://NAME | shm://NAME`,
`--trust`, `--range CIDR`..., `--key-file`, `--kinds PATH`..., `--load`,
`--scan`, `--source PATH[=JSON]`, `--refresh-ms`, `--max-calls`,
`--max-frame`).

## Links

```rust
LinkOptions::new(Trust::{Local, Trusted, Secure})
    .key(&[u8]) .token(&str) .max_frame(bytes) .max_calls(n)
    .timeout(Option<Duration>) .handshake_timeout(Duration)
    .spin(Duration)    // how long a waiting thread polls before it sleeps
// defaults: 64 MiB frames, 64 calls, 30 s per call, 10 s handshake, 50 µs spin
link.alive() link.why() link.peer() link.trust() link.list() link.open(id, stamp)
link.close(id, stamp) link.kinds() link.ping() link.end()
LinkError::{Io, Protocol, Refused { status, why }, Trust}   // -> ProviderError
```

- **Dropping a `Link` ends it.** Calls waiting answer `GONE`; objects held
  for the peer are dropped.
- **A call past its timeout** is cancelled (CANCEL) and answers `GONE`; an
  object its late answer carries is released.
- **Nested calls** (a callback during a call) run on the thread waiting
  for the outer call; a call with no parent runs on the link's pool (at
  most `max_calls` threads; past it, `GUATIAO_ERR_BUSY`).
- **A call an object makes on itself from inside one of its calls** is
  refused `BUSY` rather than deadlocked.
- The tables behind handles on a peer's objects are shared by a
  source's or a server's links; when their owner goes with a handle still
  out, they are kept for the process's life, never freed under it.

## Transports (`transport`)

```rust
trait FrameRead { fn read_frame(&mut self, &mut Vec<u8>, max) -> io::Result<bool>; fn set_timeout(..) }
trait FrameWrite { fn write_frame(&mut self, &[u8]); fn write_frame_parts(&mut self, head, tail); fn shutdown(&mut self) }
struct Connection { reader, writer, peer: String, closer: Option<Box<dyn Fn()>> }
trait Connect { fn connect(&self) -> io::Result<Connection>; fn describe(&self) -> String }
memory::pair() -> (Connection, Connection)        // tests
child::ChildCommand::new(program).arg(..).args(..).env(..)   // Connect: the child's stdio
child::stdio() -> Connection                       // a server started as a child
stream::{StreamReader, StreamWriter}               // u32 LE length + body over any Read/Write
tcp::{TcpConnect::new("host:port").timeout(d), TcpListen::bind(addr, &[Range])?, Range::parse("10.0.0.0/8")?}   // tcp
tcp::TcpListen::accept() -> io::Result<Accepted::{Connection, Refused(SocketAddr)}>
local::{LocalConnect::new(name), LocalListen::bind(name)?.accept()}   // local: a path on Unix, a pipe name on Windows
shm::{ShmConnect::new(name), ShmListen::bind(name)?.accept(), RING}   // shm: the same names
```

- `TcpListen::bind` refuses an address outside every range, an empty
  range list, and `0.0.0.0`/`::`; `accept` closes a peer outside the
  ranges unread.
- A frame a writer gets already carries its 4-byte length; a message
  transport drops it.
- **A Unix socket** is created `0600`; `LocalListen::bind` replaces a
  socket left by a listener that is gone and refuses anything else at the
  path. **A named pipe** has Windows' default access, and nothing can end
  a read blocked on it: ending a link sends GOODBYE, and the peer closing
  its end ends that read. A pipe takes no read timeout, so its handshake
  is unbounded.
- **Shared memory**: the client creates the file (`/dev/shm`, else the
  temporary directory; `0600`; removed once mapped on Unix, deleted with
  its last handle on Windows), sends its path once over the local
  connection, and the server maps it. Each direction is a 4 MiB byte
  ring; frames of any size stream through it. A reader polls 50 µs, then
  sleeps on the local connection until rung; the peer's end there ends
  the link. Ring counters are checked on every read: a ring claiming more
  than 4 MiB ends the link.
- **Windows:** one TCP socket is shared by `Arc` between reader, writer
  and closer, never `try_clone`d: a duplicated handle's `shutdown` and
  read timeout do not reliably reach a read blocked on another.

## Configuration of the source file (`config::client_of`)

| key | holds |
| --- | --- |
| `connect` | `tcp://host:port`, `local://name`, `shm://name` |
| `child` | list: the server program and its arguments |
| `trust` | `local`, `trusted`, `secure` (`local` for a child; required for `connect`) |
| `key`, `key_file` | the trusted level's key |
| `token` | a secure client's token |
| `timeout_ms` | per call; `0` none |
| `max_frame`, `max_calls` | the link's bounds |

A scheme this build lacks is `GUATIAO_ERR_UNSUPPORTED`, naming it.

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
- **A server serves a kind only with its description**: pass the kind
  crate's `<crate>.kinds.json`.
- **Object proxies fill every slot**: a slot the far object leaves out
  answers `UNSUPPORTED`, not the kind's default.
