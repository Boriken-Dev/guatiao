// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The trusted level over TCP: the right key links, a wrong or missing key
//! is refused, a silent server is given up on within the handshake's
//! bound, and a client asking another level is turned away.

#![cfg(feature = "tcp")]

mod common;

use core::time::Duration;
use std::net::TcpListener;
use std::time::Instant;

use greeter_kind::Greeter;
use guatiao::library::Registry;
use guatiao_source_remote::transport::tcp::{Accepted, TcpConnect, TcpListen};
use guatiao_source_remote::{LinkOptions, RemoteSource, Server, Trust};

const KEY: &[u8] = b"test-key";

/// A trusted server on loopback, accepting every link; answers its port.
fn listen(server: Server) -> u16 {
    let listener = TcpListen::bind("127.0.0.1:0", common::permit(&["127.0.0.1"])).unwrap();
    let port = listener.local_addr().unwrap().port();
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
    port
}

fn refresh(port: u16, options: LinkOptions) -> (Registry, Vec<String>) {
    let source = RemoteSource::new(
        Box::new(TcpConnect::new(&format!("127.0.0.1:{port}"))),
        options,
    );
    let mut registry = Registry::new("host", "1.0");
    registry.add_source("remote", source).unwrap();
    let refreshed = registry.refresh();
    let refused = refreshed.refused.iter().map(|r| r.reason.clone()).collect();
    (registry, refused)
}

fn trusted_server() -> u16 {
    listen(common::server(LinkOptions::new(Trust::Trusted).key(KEY)))
}

#[test]
fn the_right_key_links_and_calls() {
    let port = trusted_server();
    let (registry, refused) = refresh(port, LinkOptions::new(Trust::Trusted).key(KEY));
    assert!(refused.is_empty(), "{refused:?}");
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    assert!(greeter.greet("tcp").is_ok());
}

#[test]
fn a_wrong_key_is_refused() {
    let port = trusted_server();
    let (registry, refused) = refresh(port, LinkOptions::new(Trust::Trusted).key(b"other"));
    assert_eq!(registry.offers::<dyn Greeter>().count(), 0);
    assert!(
        refused
            .iter()
            .any(|r| r.contains("does not hold this link's key")),
        "{refused:?}"
    );
}

#[test]
fn a_missing_key_is_refused_before_anything_is_sent() {
    let port = trusted_server();
    let (_, refused) = refresh(port, LinkOptions::new(Trust::Trusted));
    assert!(
        refused.iter().any(|r| r.contains("pre-shared key")),
        "{refused:?}"
    );
}

#[test]
fn another_trust_level_is_turned_away() {
    let port = trusted_server();
    let (_, refused) = refresh(port, LinkOptions::new(Trust::Local));
    assert!(
        refused
            .iter()
            .any(|r| r.contains("serves the trusted level")),
        "{refused:?}"
    );
}

#[test]
fn a_silent_server_is_given_up_on_within_the_handshake_bound() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    // Accepts, holds the connection, says nothing.
    let held = std::thread::spawn(move || listener.accept().map(|(s, _)| s));
    let started = Instant::now();
    let (_, refused) = refresh(
        port,
        LinkOptions::new(Trust::Trusted)
            .key(KEY)
            .handshake_timeout(Duration::from_millis(300)),
    );
    assert!(!refused.is_empty());
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    drop(held.join());
}

#[test]
fn ending_the_links_ends_one_still_in_its_handshake() {
    let server = common::server(LinkOptions::new(Trust::Trusted).key(KEY));
    let listener = TcpListen::bind("127.0.0.1:0", common::permit(&["127.0.0.1"])).unwrap();
    // Connected, and never says HELLO: the handshake would wait 10 s.
    let _silent = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let Accepted::Connection(conn) = listener.accept().unwrap() else {
        panic!("a permitted source");
    };
    let took = common::ended_in_handshake(&server, conn);
    assert!(took < Duration::from_secs(3), "{took:?}");
}
