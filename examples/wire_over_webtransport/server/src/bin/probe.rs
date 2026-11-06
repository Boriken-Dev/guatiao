// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Connects as a browser would — the certificate pinned by its hash, the
//! server's messages read one at a time into a `Receiver` — and prints
//! what each frame said.
//!
//! `wire-probe <url> <certificate-sha256 hex> <frames>`

use core::error::Error;
use core::time::Duration;

use coqui_web_channel::webtransport::Dialer;
use coqui_web_channel::{CertificatePin, Channel, Dialer as _, Kind};
use guatiao::value::read::Dump;
use guatiao::value::wire::channel::{Received, Receiver};
use wire_over_webtransport::HELLO;

type Failure = Box<dyn Error + Send + Sync>;

fn main() -> Result<(), Failure> {
    let args: Vec<String> = std::env::args().collect();
    let [_, url, hex, count] = args.as_slice() else {
        return Err("usage: wire-probe <url> <certificate-sha256 hex> <frames>".into());
    };
    let count: usize = count.parse()?;
    let pin: CertificatePin = format!("sha256:{hex}")
        .parse()
        .map_err(|_| "a 64-digit hash")?;

    let channel = Dialer::pinned(&[pin])
        .map_err(|e| e.to_string())?
        .dial(Kind::WebTransport, &url.parse()?, Duration::from_secs(10))
        .map_err(|e| format!("cannot connect to {url}: {e}"))?;
    channel.send(HELLO).map_err(|e| e.to_string())?;

    let mut receiver = Receiver::new();
    for _ in 0..count {
        let Some(frame) = channel
            .receive(Some(Duration::from_secs(10)))
            .map_err(|e| e.to_string())?
        else {
            return Err("the server closed the stream".into());
        };
        match receiver.accept(&frame) {
            Ok(Received::Schema(channel)) => println!("schema on channel {channel}"),
            Ok(Received::Value { channel, value }) => {
                println!("value on channel {channel}: {:?}", Dump(&value))
            }
            Ok(Received::Closed(channel)) => println!("channel {channel} closed"),
            Err(e) => println!("refused: {e}"),
        }
    }
    channel.close();
    Ok(())
}
