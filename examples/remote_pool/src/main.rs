// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! One call fanned out to several peers.
//!
//! Three servers stand in for three devices, each a `Server` over a
//! registry of its own on a loopback port. The host holds them in one
//! registry, a source per device. Identical devices serve the same
//! library, so the host files libraries and providers under
//! `%source/%id`: each device's under the name its source was added by.
//! It refreshes once, then asks every greeter the same question at once,
//! one thread per peer, and gathers the answers. A peer that is gone
//! answers `GUATIAO_ERR_GONE` and the others carry on.
//!
//! ```text
//! cargo run -p remote_pool
//! ```

extern crate alloc;

use alloc::sync::Arc;
use core::net::SocketAddr;
use core::time::Duration;
use std::time::Instant;

use greeter_kind::Greeter;
use guatiao::library::{ProviderError, Registry};
use guatiao::value::convert::TryAsRef;
use guatiao::{Map, Status};
use guatiao_source_remote::acl::{Acl, Action, Rule};
use guatiao_source_remote::transport::tcp::{Accepted, TcpConnect, TcpListen};
use guatiao_source_remote::{Described, LinkOptions, RemoteSource, Server, Trust};

/// The pre-shared key every link here proves. A real deployment reads
/// its own from a file.
const KEY: &[u8] = b"an example key, not a secret";

/// What each device serves: a greeter that takes a moment, as a device
/// doing real work would.
#[derive(Default, guatiao::Provider)]
#[provider(Greeter)]
pub struct Device;

impl Greeter for Device {
    fn greet(&self, name: &str) -> Result<Map, ProviderError> {
        if name.is_empty() {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_BAD_VALUE,
                "nobody to greet",
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
        let mut answer = Map::new();
        answer.set("greeting", format!("hello, {name}").as_str());
        Ok(answer)
    }
}

guatiao::local_providers!(id = "device", version = "1.0.0", providers = [Device]);

fn options() -> LinkOptions {
    LinkOptions::new(Trust::Trusted).key(KEY)
}

/// A device: a server on a loopback port, accepting loopback peers.
fn device() -> Result<SocketAddr, String> {
    let mut registry = Registry::new("device", "1.0");
    registry
        .register_local("device", library)
        .map_err(|e| e.to_string())?;
    let mut described = Described::new();
    described
        .add_json(&greeter_kind::GreeterVtable::description())
        .map_err(|e| e.message().to_string())?;
    let server = Server::new(registry, described, options());
    let mut acl = Acl::default();
    acl.push(Rule::new(
        Action::Permit,
        "127.0.0.0/8".parse().map_err(|e| format!("{e}"))?,
    ));
    let listener = TcpListen::bind("127.0.0.1:0", acl).map_err(|e| e.to_string())?;
    let address = listener.local_addr().map_err(|e| e.to_string())?;
    std::thread::spawn(move || {
        while let Ok(accepted) = listener.accept() {
            if let Accepted::Connection(conn) = accepted {
                let server = server.clone();
                std::thread::spawn(move || {
                    let _ = server.serve(conn);
                });
            }
        }
    });
    Ok(address)
}

/// One peer's greeter, and the source it came through.
struct Peer {
    name: String,
    greeter: Arc<dyn Greeter>,
}

/// Runs the example; answers how many peers greeted.
fn run() -> Result<usize, String> {
    let mut registry = Registry::new("pool-host", "1.0");
    registry
        .libraries_keyed_by("%source/%id")
        .map_err(|e| e.to_string())?;
    registry
        .keyed_by("%source/%id")
        .map_err(|e| e.to_string())?;
    for i in 0..3 {
        let address = device()?.to_string();
        let source = RemoteSource::new(Box::new(TcpConnect::new(&address)), options());
        registry
            .add_source(&format!("device-{i}"), source)
            .map_err(|e| e.to_string())?;
    }
    let refreshed = registry.refresh();
    if let Some(refused) = refreshed.refused.first() {
        return Err(format!("{}: {}", refused.source, refused.reason));
    }
    let greeters: Vec<Peer> = registry
        .all()
        .iter()
        .filter_map(|p| {
            let greeter = p.offer::<dyn Greeter>().ok()?.shared()?;
            Some(Peer {
                name: p.source().to_string(),
                greeter,
            })
        })
        .collect();
    println!(
        "{} peers offer a greeter: {}",
        greeters.len(),
        registry
            .all()
            .iter()
            .map(|p| p.key())
            .collect::<Vec<_>>()
            .join(", ")
    );

    // The same call to every peer at once: the wait is the slowest
    // peer's, not the sum of them.
    let started = Instant::now();
    let answers: Vec<_> = std::thread::scope(|scope| {
        let calls: Vec<_> = greeters
            .iter()
            .map(|p| scope.spawn(move || (&p.name, p.greeter.greet("pool"))))
            .collect();
        calls.into_iter().filter_map(|c| c.join().ok()).collect()
    });
    let mut greeted = 0;
    for (name, answer) in &answers {
        match answer {
            Ok(map) => {
                let greeting = map
                    .get("greeting")
                    .and_then(TryAsRef::<str>::try_as_ref)
                    .unwrap_or_default();
                println!("{name}: {greeting}");
                greeted += 1;
            }
            Err(e) => println!("{name}: refused, {}", e.message()),
        }
    }
    println!(
        "{} answers in {} ms",
        answers.len(),
        started.elapsed().as_millis()
    );
    Ok(greeted)
}

fn main() {
    if let Err(why) = run() {
        eprintln!("remote_pool: {why}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_peer_answers_the_call_fanned_out_to_them() {
        assert_eq!(super::run(), Ok(3));
    }
}
