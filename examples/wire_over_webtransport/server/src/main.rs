// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Serves WebTransport on UDP port 4433 (`WIRE_PORT`) through
//! coqui-web-channel: to each session that opens its stream and says hello it
//! announces the schema on channel 1, then sends a value every
//! `WIRE_INTERVAL_MS` (default 1000). Each channel frame is one coqui-web-channel
//! message.
//!
//! The certificate is self-signed and short-lived, which a browser accepts
//! only by its SHA-256 (`serverCertificateHashes`); the hash is printed at
//! start.

extern crate alloc;

use alloc::sync::Arc;
use core::error::Error;
use core::net::{Ipv4Addr, SocketAddr};
use core::time::Duration;

use coqui_peer_tls::{Identity, Policy};
use coqui_web_channel::Channel;
use coqui_web_channel::webtransport::{Listener, Pending};
use guatiao::value::wire::channel;
use wire_over_webtransport::{CHANNEL, nth, schema};

type Failure = Box<dyn Error + Send + Sync>;

/// A browser takes a certificate pinned by its hash for at most 14 days.
const VALID_FOR: Duration = Duration::from_secs(10 * 24 * 3600);

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn main() -> Result<(), Failure> {
    let port = u16::try_from(env_u64("WIRE_PORT", 4433))?;
    let interval = Duration::from_millis(env_u64("WIRE_INTERVAL_MS", 1000));

    let identity = Identity::generate_valid_for(VALID_FOR)?;
    let hash = identity.fingerprint();
    println!("certificate-sha256={}", hash.hex());
    let bytes: Vec<String> = hash.as_bytes().iter().map(u8::to_string).collect();
    println!(
        "serverCertificateHashes: [{{ algorithm: \"sha-256\", value: new Uint8Array([{}]) }}]",
        bytes.join(", ")
    );

    let tls =
        coqui_peer_tls::server_config_without_client_auth_with(&identity, &[], &Policy::modern())?
            .config;
    // A client that resolves `localhost` to `::1` first falls back to
    // the IPv4 address (coqui-web-channel dials each address a name resolves to).
    let bind = SocketAddr::from((Ipv4Addr::UNSPECIFIED, port));
    let listener = Listener::bind(bind, Arc::new(tls))
        .map_err(|e| format!("cannot listen on UDP {bind}: {e}"))?;
    println!("listening on https://localhost:{port}");

    loop {
        let pending = listener.next(None).map_err(|e| e.to_string())?;
        std::thread::spawn(move || {
            if let Err(e) = serve(pending, interval) {
                eprintln!("session ended: {e}");
            }
        });
    }
}

/// One session: the page's hello, the schema, then a value per interval
/// until the page goes.
fn serve(pending: Pending, interval: Duration) -> Result<(), Failure> {
    let channel = pending.handshake().map_err(|e| e.to_string())?;
    // A stream is seen by the server once its opener has sent on it.
    channel
        .receive(Some(Duration::from_secs(10)))
        .map_err(|e| e.to_string())?
        .ok_or("the page closed before it said hello")?;
    channel
        .send(&channel::announce(CHANNEL, &schema())?)
        .map_err(|e| e.to_string())?;
    for n in 1.. {
        channel
            .send(&channel::send(CHANNEL, &nth(n))?)
            .map_err(|e| e.to_string())?;
        std::thread::sleep(interval);
    }
    Ok(())
}
