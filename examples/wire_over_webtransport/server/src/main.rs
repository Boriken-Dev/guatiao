// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Serves WebTransport on `https://localhost:4433` (or `WIRE_PORT`): to each
//! session it opens one stream, announces the schema on channel 1, then
//! sends a value every `WIRE_INTERVAL_MS` (default 1000).
//!
//! The certificate is self-signed and short-lived, which a browser accepts
//! only by its SHA-256 (`serverCertificateHashes`); the hash is printed at
//! start.

use std::error::Error;
use std::time::Duration;

use guatiao::value::wire::channel;
use wire_over_webtransport::{CHANNEL, delimited, nth, schema};
use wtransport::endpoint::IncomingSession;
use wtransport::tls::Sha256DigestFmt;
use wtransport::{Endpoint, Identity, ServerConfig};

type Failure = Box<dyn Error + Send + Sync>;

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[tokio::main]
async fn main() -> Result<(), Failure> {
    let port = env_u64("WIRE_PORT", 4433) as u16;
    let interval = Duration::from_millis(env_u64("WIRE_INTERVAL_MS", 1000));

    let identity = Identity::self_signed(["localhost", "127.0.0.1", "::1"])?;
    let hash = identity.certificate_chain().as_slice()[0].hash();
    let hex: String = hash.as_ref().iter().map(|b| format!("{b:02x}")).collect();
    println!("certificate-sha256={hex}");
    println!(
        "serverCertificateHashes: [{{ algorithm: \"sha-256\", value: new Uint8Array({}) }}]",
        hash.fmt(Sha256DigestFmt::BytesArray)
    );

    let config = ServerConfig::builder()
        .with_bind_default(port)
        .with_identity(identity)
        .keep_alive_interval(Some(Duration::from_secs(3)))
        .build();
    let server = Endpoint::server(config)?;
    println!("listening on https://localhost:{port}");

    loop {
        let incoming = server.accept().await;
        tokio::spawn(async move {
            if let Err(e) = serve(incoming, interval).await {
                eprintln!("session ended: {e}");
            }
        });
    }
}

async fn serve(incoming: IncomingSession, interval: Duration) -> Result<(), Failure> {
    let request = incoming.await?;
    let connection = request.accept().await?;
    let mut stream = connection.open_uni().await?.await?;
    stream
        .write_all(&delimited(&channel::announce(CHANNEL, &schema())?))
        .await?;
    for n in 1.. {
        stream
            .write_all(&delimited(&channel::send(CHANNEL, &nth(n))?))
            .await?;
        tokio::time::sleep(interval).await;
    }
    Ok(())
}
