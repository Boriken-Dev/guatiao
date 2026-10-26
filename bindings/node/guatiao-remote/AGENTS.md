# guatiao-remote — API header

guatiao's remote protocol in TypeScript: a browser page, or any
JavaScript program, calls the providers a guatiao server serves, over a
WebSocket. https://github.com/Boriken-Dev/guatiao/ Self-contained — this
file ships inside the npm package, so it names no repository path.

ESM only, typed. No Node API in the package (a browser loads it as is);
Node 22 and later run it with their own `WebSocket`. One dependency,
`@noble/hashes` (HMAC-SHA256 for the key proof): WebCrypto's
`crypto.subtle` exists only in a secure context, and a page that may
open `ws://` is `http://`, secure only on localhost. Nothing of the
`guatiao` package (koffi) is needed.

## A link

```ts
RemoteLink.connect(url: string | URL, options?: LinkOptions): Promise<RemoteLink>  // ws://, wss:// or https:// (WebTransport)
RemoteLink.over(channel: RemoteChannel, options: LinkOptions & { trust?: RemoteTrust }): Promise<RemoteLink>
interface LinkOptions {
  security?: RemoteSecurity;          // 'none' | 'server-tls' | 'mutual-tls' (default); ws:// runs none alone, wss:// the others, https:// any
  pins?: readonly string[];           // https:// only: 'sha256:<hex>' of the server's certificate, the browser's serverCertificateHashes
  key?: Uint8Array | string;          // the pre-shared key (a string is UTF-8): required with none; at server TLS proved when the server asks
  token?: string;                     // what this side presents when the server requires a token (server TLS: key or token required)
  timeout?: number | null;            // ms a call waits, for a place under the peer's max_calls and then its answer; default 30 000; null: as long as the link lives
  handshakeTimeout?: number;          // ms; default 10 000 (also bounds the WebSocket's opening)
  maxFrame?: number;                  // the largest message read; default 64 MiB
  maxCalls?: number;                  // the peer's calls run at once; default 64, past it BUSY
}
RemoteTrust = { LOCAL: 0, TRUSTED: 1, SECURE: 2, SERVER_TLS: 3 }   // the HELLO's byte: none is TRUSTED, mutual TLS SECURE
link.list(): Promise<RemoteCatalog>   // must come first: its kinds type every call
link.open(library): Promise<void>     // its providers answer only once opened
link.closeLibrary(library): void
link.ping(): Promise<number>          // ms, one round trip
link.close(): void                    // GOODBYE, then the channel closes; idempotent
link.alive: boolean; link.why: string | undefined; link.done: Promise<string>
link.objectsHeld: number              // objects of this side's the peer holds
link.trust: RemoteTrust
link.datagrams: RemoteDatagrams | undefined   // WebTransport's; undefined over a WebSocket
interface RemoteDatagrams {
  readonly maxSize: number;           // the browser's maxDatagramSize: the largest payload the path takes now
  send(payload: Uint8Array): void;    // BadValue past maxSize, Gone once the link ended; a lost one is not said
  receive(): Promise<Uint8Array>;     // the next to arrive; rejects Gone when the link ends
}
```

- **Datagrams** (WebTransport): each delivered whole or not at all, in
  any order; the payload alone, nothing added, so a server reads it with
  its own QUIC stack; for values, a wire channel's frame. A server that
  sends each one back is guatiao's `datagram_echo` example.

- **WebTransport** (`https://host:port/path`): a browser's only (Node has
  none: `Unsupported`). `pins` (the hashes the server says as
  `certificate hashes`) become `serverCertificateHashes`; without them
  the browser trusts the server as any site. One bidirectional stream
  carries each frame behind its `u32` LE length; a frame over `maxFrame`
  ends the link. A session that does not open, or a certificate matching
  no pin, rejects `Gone`; `mutual-tls` needs a client certificate the
  browser presents (Chrome presented none to a profile holding none). `pins` with a `ws://` or `wss://` URL is a
  `TypeError`.
- A wrong key rejects `BadValue` ("the server does not hold this link's
  key") before the link is up; a server that cannot be reached, or does
  not answer the handshake in time, rejects `Gone`; a GOODBYE in the
  handshake rejects with its status's error.
- Losing the link rejects every open call `Gone` and drops every object
  either side held. A call past its timeout rejects `Gone` and is
  cancelled; an object its late answer carries is released. A call still
  waiting for a place under the peer's `max_calls` when its timeout runs
  out rejects `Busy` and is never sent.
- A frame larger than the peer's `max_frame` is refused (`BadValue`), not
  sent. A malformed frame from the peer ends the link with GOODBYE 7.

## The catalog and calls

```ts
catalog.kinds: ReadonlyMap<string, RemoteKind>; catalog.libraries: readonly RemoteLibrary[]; catalog.providers
library.id; library.version; library.stamp; library.providers
provider.id; provider.name; provider.meta; provider.tables: ReadonlyMap<string, RemoteTable>; provider.table(kind)
table.id; table.slots: ReadonlySet<string>; table.has(slot); table.kind: RemoteKind
callee.call(slot: string, args: readonly unknown[] = [], options?: { during?: InboundCall }): Promise<unknown>
```

- Arguments and answers are typed by the slot's description
  (`RemoteKind.slots[i].args[j].type`): `bool` a boolean; `i8`..`i64`,
  `u8`..`u64`, `isize`, `usize` a `number` or a `bigint` (checked against
  the type's range, `TypeError` outside it); `f32`/`f64` a number; `str` a
  string; `bytes` a `Uint8Array`; `bytes_mut` a `Uint8Array` the answer
  writes into; `value`/`map`/`list` plain data; `object` a `Served` or a
  `RemoteObject`.
- Plain data: `undefined` is absent, then `null`, booleans, numbers (a
  `number` when a safe integer or fractional, else a `bigint`), strings,
  `Uint8Array`, arrays, plain objects (a `Map` with string keys is taken
  too). Keys are defined, so `__proto__` is data. Nesting past 128 is
  refused.
- What the peer sends is checked as the wire encoding states it: lengths
  in their shortest form and within what remains, a number's text in the
  JSON number grammar (`1e999` reads as `Infinity`; a fraction keeps its
  value, not its spelling: `1.10` is `1.1`), a bool byte 0 or 1, no key
  repeated, UTF-8 text (a leading U+FEFF is kept). A value that breaks
  one ends the link (GOODBYE 7), as any malformed frame does.
- A lent map, array or `Map` is written back in place (cleared, then
  filled); a `bytes_mut` buffer gets what the callee wrote.
- A slot the far table does not fill rejects `Unsupported` and sends
  nothing (a description carries no default body); a slot the kind does
  not have rejects `NotFound`; the wrong number of arguments is a
  `TypeError`.

## Objects

```ts
new Served(kind: string, slots: Record<string, Handler>)
type Handler = (args: unknown[], call: InboundCall) => unknown   // may return a Promise
object: RemoteObject  // object.id; object.kind; object.alive; object.call(...); object.release()
call: InboundCall     // call.running
```

- A `Served` passed where a slot takes an object is exported to the peer:
  for the call (`lent`), or until the peer releases it (`taken`). The
  wrong kind rejects `WrongKind`, and an object already handed over in the
  same call is taken back.
- The peer's object arrives as a `RemoteObject`; `release()` lets it go
  (a `FinalizationRegistry` releases one dropped unreleased). A lent
  handle answers `Gone` once its call returns. One of this side's own
  coming back arrives as the `Served` itself.
- A handler's answer is typed by the slot; throwing a `GuatiaoError`
  refuses with its status and message, a `TypeError`/`RangeError` (an
  answer the slot does not take) with `BAD_VALUE`, anything else with
  `INTERNAL`.
- A call of the peer's refused before its handler runs (made during a call
  that has returned, past `maxCalls`, a slot the `Served` does not fill,
  an argument that cannot be taken) releases the peer's objects handed in
  it and drops this side's own given back. An object lent for a call
  cannot come back handed over, in a call or as an answer: `WrongKind`,
  and it stays lent.
- **A call made from inside a handler names it with `{ during: call }`**:
  the peer then runs it as part of that call, which a call back into an
  object busy with that call needs. Named only while the handler still
  runs; afterwards it goes as a call of its own. There is no implicit
  context: browsers have no async-local storage.

## Errors

`GuatiaoError` (`status`, `message`) and `BadValue` (7), `AllocFailed`
(8), `WrongKind` (9), `NotFound` (10), `NullArgument` (11), `Gone` (12),
`Unsupported` (13), `Busy` (14), `Internal` (100); `errorFor(status,
message)`; `Status`. `MalformedFrame extends BadValue` (`fault`, `at`):
bytes from the peer that are no frame.

## A channel of your own

```ts
interface RemoteChannel {
  start(message: (body: Uint8Array) => void, closed: (why: string) => void): void;
  send(body: Uint8Array): void;
  close(): void;
  readonly datagrams?: RemoteDatagrams;   // what link.datagrams answers
}
```

One message per frame body, in order, no length in front; `closed` runs
once. `RemoteLink.over(channel, { trust: RemoteTrust.LOCAL })` skips the
key proof (a transport whose ends are already trusted).

## Gotchas

- `list()` before any call: the catalog's kinds type the calls, and a
  kind the catalog has not described rejects `NotFound`.
- `security: 'none'` is not encryption: anyone on the network reads and
  changes the traffic, and whoever reads the page holds the key.
- Over `wss://` the TLS is the browser's (or Node's): the server's
  certificate must be one it trusts (a certificate authority's, for the
  server's name), and at mutual TLS a page presents the client
  certificate its user installed; nothing here pins or presents one.
  A URL that disagrees with `security`, or nothing to prove with, is a
  `TypeError` before anything opens.
- A text WebSocket message ends the link: frames are binary.
