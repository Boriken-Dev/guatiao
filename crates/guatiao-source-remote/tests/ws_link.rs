// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! WebSocket at the trusted level: calls and callbacks cross, a source's
//! configuration reaches it as `ws://`, and the key and the ranges gate
//! it as they do TCP.

#![cfg(feature = "ws")]

mod common;

use core::time::Duration;

use common::Ear;
use greeter_kind::Greeter;
use guatiao::Map;
use guatiao::library::Registry;
use guatiao_source_remote::transport::tcp::Accepted;
use guatiao_source_remote::transport::ws::{WsConnect, WsListen};
use guatiao_source_remote::{LinkOptions, RemoteSource, Server, Trust};

const KEY: &[u8] = b"test-key";

/// A server on loopback admitting `range`; answers its port.
fn listen(server: Server, range: &str) -> u16 {
    let listener = WsListen::bind("127.0.0.1:0", common::permit(&[range])).unwrap();
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

fn trusted_server(range: &str) -> u16 {
    listen(
        common::server(LinkOptions::new(Trust::Trusted).key(KEY)),
        range,
    )
}

fn refresh(source: RemoteSource) -> (Registry, Vec<String>) {
    let mut registry = Registry::new("host", "1.0");
    registry.add_source("remote", source).unwrap();
    let refreshed = registry.refresh();
    let refused = refreshed.refused.iter().map(|r| r.reason.clone()).collect();
    (registry, refused)
}

fn over_ws(port: u16, options: LinkOptions) -> RemoteSource {
    RemoteSource::new(
        Box::new(WsConnect::new(&format!("ws://127.0.0.1:{port}/remote"))),
        options,
    )
}

#[test]
fn calls_and_callbacks_cross_a_websocket() {
    let port = trusted_server("127.0.0.1");
    let (registry, refused) = refresh(over_ws(
        port,
        LinkOptions::new(Trust::Trusted)
            .key(KEY)
            .timeout(Some(Duration::from_secs(10))),
    ));
    assert!(refused.is_empty(), "{refused:?}");
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    assert!(greeter.greet("ws").is_ok());
    let mut ear = Ear::new();
    greeter.greet_into("socket", &mut ear).unwrap();
    assert_eq!(*ear.heard.lock().unwrap(), ["hello, socket", "twice"]);
}

#[test]
fn a_configuration_names_a_websocket_peer() {
    let port = trusted_server("127.0.0.1");
    let mut config = Map::new();
    config.set("connect", format!("ws://127.0.0.1:{port}/").as_str());
    config.set("security", "none");
    config.set("key", "test-key");
    let (registry, refused) = refresh(RemoteSource::from_config(&config).unwrap());
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(registry.offers::<dyn Greeter>().count(), 1);

    let mut mutual = config.clone();
    mutual.remove("security");
    let refused = RemoteSource::from_config(&mutual).unwrap_err();
    assert!(refused.message().contains("carries no TLS"), "{refused:?}");
    let mut secure = Map::new();
    secure.set("connect", "wss://127.0.0.1:1/");
    secure.set("security", "none");
    secure.set("key", "k");
    let refused = RemoteSource::from_config(&secure).unwrap_err();
    assert_eq!(refused.status, guatiao::Status::GUATIAO_ERR_BAD_VALUE);
    assert!(refused.message().contains("runs TLS"), "{refused:?}");
}

#[test]
fn a_wrong_key_is_refused() {
    let port = trusted_server("127.0.0.1");
    let (registry, refused) = refresh(over_ws(
        port,
        LinkOptions::new(Trust::Trusted).key(b"other"),
    ));
    assert_eq!(registry.offers::<dyn Greeter>().count(), 0);
    assert!(
        refused
            .iter()
            .any(|r| r.contains("does not hold this link's key")),
        "{refused:?}"
    );
}

#[test]
fn a_peer_outside_the_ranges_is_turned_away() {
    let port = trusted_server("127.0.0.2");
    let (registry, refused) = refresh(over_ws(
        port,
        LinkOptions::new(Trust::Trusted)
            .key(KEY)
            .handshake_timeout(Duration::from_secs(2)),
    ));
    assert_eq!(registry.offers::<dyn Greeter>().count(), 0);
    assert!(!refused.is_empty());
}
