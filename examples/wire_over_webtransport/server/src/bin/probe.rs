// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Connects as a browser would — the certificate pinned by its hash, the
//! server's stream read frame by frame into a `Receiver` — and prints what
//! each frame said.
//!
//! `wire-probe <url> <certificate-sha256 hex> <frames>`

use std::error::Error;

use guatiao::value::read::Dump;
use guatiao::value::wire::channel::{Received, Receiver};
use wire_over_webtransport::frames;
use wtransport::tls::Sha256Digest;
use wtransport::{ClientConfig, Endpoint};

type Failure = Box<dyn Error + Send + Sync>;

#[tokio::main]
async fn main() -> Result<(), Failure> {
    let args: Vec<String> = std::env::args().collect();
    let [_, url, hex, count] = args.as_slice() else {
        return Err("usage: wire-probe <url> <certificate-sha256 hex> <frames>".into());
    };
    let count: usize = count.parse()?;
    let mut digest = [0u8; 32];
    for (i, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(2 * i..2 * i + 2).ok_or("a 64-digit hash")?, 16)?;
    }

    let config = ClientConfig::builder()
        .with_bind_default()
        .with_server_certificate_hashes([Sha256Digest::new(digest)])
        .build();
    let connection = Endpoint::client(config)?.connect(url.as_str()).await?;
    let mut stream = connection.accept_uni().await?;

    let mut receiver = Receiver::new();
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut seen = 0;
    while seen < count {
        let Some(n) = stream.read(&mut chunk).await? else {
            return Err("the server closed the stream".into());
        };
        buffer.extend_from_slice(&chunk[..n]);
        for frame in frames(&mut buffer) {
            match receiver.accept(&frame) {
                Ok(Received::Schema(channel)) => println!("schema on channel {channel}"),
                Ok(Received::Value { channel, value }) => {
                    println!("value on channel {channel}: {:?}", Dump(&value))
                }
                Ok(Received::Closed(channel)) => println!("channel {channel} closed"),
                Err(e) => println!("refused: {e}"),
            }
            seen += 1;
            if seen == count {
                break;
            }
        }
    }
    Ok(())
}
