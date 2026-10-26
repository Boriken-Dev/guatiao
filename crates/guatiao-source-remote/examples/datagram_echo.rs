// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A WebTransport server whose links send every datagram back: how a Rust
//! server reads and writes a link's datagrams, and what the page clients'
//! datagram suites run against.
//!
//! ```text
//! cargo run -p guatiao-source-remote --features webtransport --example datagram_echo -- KEY [ADDRESS]
//! ```
//!
//! At no security, the key proved both ways, every loopback source
//! admitted; it presents certificates it makes and says, as
//! guatiao-remote-server does, where it serves and the hashes a page pins.

use guatiao::library::Registry;
use guatiao_source_remote::acl::{Acl, Action, Rule};
use guatiao_source_remote::transport::tcp::Accepted;
use guatiao_source_remote::transport::webtransport::{ROTATE_EVERY, WtListen};
use guatiao_source_remote::{Described, LinkOptions, Server, Trust};

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let key = args.next().ok_or("usage: datagram_echo KEY [ADDRESS]")?;
    let address = args.next().unwrap_or_else(|| "127.0.0.1:0".to_string());
    let mut acl = Acl::default();
    for loopback in ["127.0.0.0/8", "::1"] {
        let range = loopback.parse().map_err(|e| format!("{e:?}"))?;
        acl.push(Rule::new(Action::Permit, range));
    }
    let listener = WtListen::bind_trusted(&address, acl)
        .and_then(|l| l.rotating(ROTATE_EVERY))
        .map_err(|e| e.to_string())?;
    let hashes: Vec<String> = listener
        .certificate_hashes()
        .map_err(|e| e.to_string())?
        .iter()
        .map(|h| h.canonical())
        .collect();
    eprintln!(
        "datagram_echo: serving https://{}/ (WebTransport)",
        listener.local_addr().map_err(|e| e.to_string())?
    );
    eprintln!("datagram_echo: certificate hashes {}", hashes.join(" "));
    let server = Server::new(
        Registry::new("datagram_echo", "1.0"),
        Described::new(),
        LinkOptions::new(Trust::Trusted).key(key.as_bytes()),
    );
    loop {
        let Accepted::Connection(conn) = listener.accept().map_err(|e| e.to_string())? else {
            continue;
        };
        let server = server.clone();
        std::thread::spawn(move || {
            let Ok(link) = server.serve(conn) else {
                return;
            };
            let Some(datagrams) = link.datagrams() else {
                return;
            };
            // Each one back as it came, until the link ends.
            while let Ok(Some(payload)) = datagrams.recv(None) {
                let _ = datagrams.send(&payload);
            }
            drop(link);
        });
    }
}
