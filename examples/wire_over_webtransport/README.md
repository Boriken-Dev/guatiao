# Values over WebTransport

A server that announces a schema on a channel and then streams values
on it, and a browser page that receives them — each value checked
against the announced schema before the page sees it. Every tenth value
leaves out a required field, so the page shows the refusal too.

- `server/` — the shared library (the channel, the schema, a frame's
  length prefix), `wire-server`, and `wire-probe`, a native client that
  connects exactly as the page does, for checking without a browser.
- `client/` — the page's WebAssembly, its own workspace (it builds only
  for `wasm32-unknown-unknown`), and `index.html`.

## Run it

```bash
# 1. The server. It prints the certificate's SHA-256.
cargo run -p wire_over_webtransport --bin wire-server
#   certificate-sha256=6f8f…
#   listening on https://localhost:4433

# 2. Without a browser: the probe reads 12 frames and exits.
cargo run -p wire_over_webtransport --bin wire-probe -- https://127.0.0.1:4433 <sha256> 12

# 3. The page (needs wasm-bindgen-cli at the version in client/Cargo.lock).
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129 --locked
cd examples/wire_over_webtransport/client
cargo build --release --locked
wasm-bindgen --target web --out-dir pkg target/wasm32-unknown-unknown/release/wire_client.wasm
python -m http.server 8000
# open http://localhost:8000/?hash=<sha256>
```

The certificate is self-signed and valid for under two weeks, which is
what a browser accepts through `serverCertificateHashes`; no certificate
authority is involved, and a new one is made at every start.
`WIRE_PORT` and `WIRE_INTERVAL_MS` change the port and the pace.
`WIRE_BIND` names the address it listens on (default IPv4, `0.0.0.0`):
QUIC does not fall back between address families, so a client dialing
`localhost` that resolves to `::1` needs `WIRE_BIND=::`.

## What crosses

The page opens one bidirectional stream and sends a hello on it (a
stream is seen by the server once its opener has sent on it). Each
message either way is a little-endian `u32` length and then, from the
server, a `guatiao::value::wire::channel` frame: a kind byte, the
channel, and a wire-encoded payload. The first frame announces the
schema; every later one carries a value, which the page's `Receiver`
checks with `validate_map` before handing it over.

## Over a WebSocket instead

Nothing above the stream changes. A WebSocket delivers whole messages,
so send each channel frame as one binary message and drop the length
prefix; the `Receiver` is the same.
