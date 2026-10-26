# guatiao-source-remote

[![Crate](https://img.shields.io/crates/v/guatiao-source-remote.svg)](https://crates.io/crates/guatiao-source-remote)
[![docs.rs](https://img.shields.io/docsrs/guatiao-source-remote)](https://docs.rs/guatiao-source-remote)
[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

[guatiao](https://crates.io/crates/guatiao) providers called across a
process, a machine or a network. A device runs a **server** over its own
registry; a host adds a **remote source** and, after `refresh()`, calls
the device's providers through the same kind traits and C tables as a
library it loaded itself, in any language.

> **Status: alpha.** Released in lockstep with `guatiao`.

## Features

- **Every slot shape** — borrowed, taken and lent arguments, every
  answer, objects both ways, refusals with their status and words.
- **Calls both ways** — a listener lent to a remote provider is called
  back over the same link during the call, on the thread that lent it.
- **Lifetimes across the link** — a remote object holds its library on
  the far side until its handle is dropped, its call is cancelled or the
  link is lost; a dead peer answers `GONE`, and `refresh()` connects again.
- **Few bytes, few round trips** — ids interned at the handshake,
  arguments typed by the kind's description, calls pipelined and answered
  in any order: `greet("world")` is a 15-byte frame.
- **One protocol, stated for every language** — frames, the handshake and
  the trust levels in `docs/remote.md`, shared vectors, and a C client and
  server in the tests.
- **Transports** — a server in a child process (isolation); the same
  machine through a Unix socket or a named pipe, or through shared memory
  (a few microseconds a call); TCP at the trusted level: peers admitted
  by an access control list, a pre-shared key proved both ways; TLS 1.3
  at the secure level: mutual TLS, each side known by its certificate's
  fingerprint, unknown clients waiting until an operator accepts them;
  the same over QUIC, with datagrams beside the calls.

## Installation

```toml
[dependencies]
guatiao-source-remote = { version = "=0.0.0-alpha.1", features = ["tcp"] }
```

| Flag | Adds | Needed for |
| --- | --- | --- |
| `tcp` | `hmac`, `sha2`, `getrandom`, `coqui-net-acl` | TCP at the trusted level |
| `tls` | `tcp`, `coqui-peer`, `coqui-peer-tls`, `rustls` (ring) | TLS 1.3 at the secure level |
| `quic` | `tls`, `quinn`, `tokio` | the secure level over QUIC, with datagrams |
| `local` | `interprocess` (Windows only) | a Unix socket or a named pipe |
| `shm` | `local`, `memmap2` | shared memory beside a local connection |
| `ws` | `tcp`, `tungstenite` | WebSocket at the trusted level: what a browser page connects to |

Kind tables are built at run time through `guatiao-libffi` (libffi, built
from C). `coqui-net-acl`, `coqui-peer` and `coqui-peer-tls` are git
dependencies pinned by commit until they are on crates.io.

## Usage

A host:

```rust,ignore
use guatiao_source_remote::transport::tcp::TcpConnect;
use guatiao_source_remote::{LinkOptions, RemoteSource, Trust};

let source = RemoteSource::new(
    Box::new(TcpConnect::new("192.0.2.21:7000")),
    LinkOptions::new(Trust::Trusted).key(&key),
);
registry.add_source("device", source)?;
registry.refresh();
let greeter = registry.offers::<dyn Greeter>().next().unwrap().shared().unwrap();
greeter.greet("ana")?;
```

A host in any other language adds the library file with a configuration
map (`{"connect": "tcp://192.0.2.21:7000", "trust": "trusted", "key_file":
"link.key"}`, or `{"child": [...]}` for a server in a child process), or
names directories of peer manifests (`{"dirs": ["peers"]}`): each
`<name>.guatiao.toml` of `type = "remote"` is a peer, its libraries
served as `<name>/<id>`.

A server:

```text
guatiao-remote-server --listen tcp://192.0.2.21:7000 --trust trusted \
    --range 192.0.2.0/24 --key-file link.key \
    --kinds greeter_kind.kinds.json --scan /opt/providers
guatiao-remote-server --listen tls://0.0.0.0:7000 --trust secure \
    --state-dir /var/lib/guatiao-remote --kinds greeter_kind.kinds.json --scan /opt/providers
```

A host reaches the secure one with `{"connect": "tls://192.0.2.21:7000",
"trust": "secure", "state_dir": "state", "pins": ["sha256:..."]}`; the
server lists it as waiting (`--state-dir DIR --waiting`) until accepted
(`--accept sha256:...`).

## License

[MPL-2.0](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
