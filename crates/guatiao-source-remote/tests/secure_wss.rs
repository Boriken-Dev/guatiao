// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Mutual TLS under a WebSocket (`wss://`): the TLS of `tls://` (pins, the
//! trust store, refusals) with the WebSocket's upgrade inside it, as a
//! browser holding a client certificate would connect.

#![cfg(all(feature = "ws", feature = "tls"))]

mod common;

use std::path::PathBuf;

use greeter_kind::Greeter;
use guatiao::library::Registry;
use guatiao::value::convert::TryAsRef;
use guatiao_source_remote::transport::tcp::Accepted;
use guatiao_source_remote::transport::tls::{Identity, TlsConfig, TrustStore};
use guatiao_source_remote::transport::ws::{WsConnect, WsListen};
use guatiao_source_remote::{LinkOptions, RemoteSource, Server, Trust};

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("secure_wss")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A `wss://` server on loopback as `config` says; its address.
fn listen(config: &TlsConfig, server: Server) -> String {
    let listener = WsListen::bind_tls("127.0.0.1:0", config).unwrap();
    let address = listener.local_addr().unwrap().to_string();
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
    address
}

fn secure_server() -> Server {
    common::server(LinkOptions::new(Trust::Secure))
}

fn source(address: &str, config: &TlsConfig) -> RemoteSource {
    RemoteSource::new(
        Box::new(
            WsConnect::new(&format!("wss://{address}/remote"))
                .tls(config)
                .unwrap(),
        ),
        LinkOptions::new(Trust::Secure),
    )
}

fn refresh(source: &RemoteSource) -> (Registry, Vec<String>) {
    let mut registry = Registry::new("host", "1.0");
    registry.add_source("remote", source.clone()).unwrap();
    let refreshed = registry.refresh();
    let refused = refreshed.refused.iter().map(|r| r.reason.clone()).collect();
    (registry, refused)
}

fn greeting(registry: &Registry) -> String {
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .expect("a greeter")
        .shared()
        .unwrap();
    let map = greeter.greet("wss").unwrap();
    map.get("greeting")
        .and_then(TryAsRef::<str>::try_as_ref)
        .unwrap_or_default()
        .to_string()
}

#[test]
fn pinned_ends_link_over_wss_and_each_knows_the_other() {
    let (server_id, client_id) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    let server = TlsConfig::new(server_id.clone()).pins([client_id.fingerprint()]);
    let client = TlsConfig::new(client_id).pins([server_id.fingerprint()]);
    let address = listen(&server, secure_server());
    let source = source(&address, &client);
    let (registry, refused) = refresh(&source);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(greeting(&registry), "hello, wss");
    let link = source.link().unwrap();
    assert_eq!(link.trust(), Trust::Secure);
    assert_eq!(
        link.peer_id(),
        Some(server_id.fingerprint().canonical().as_str())
    );
    assert!(link.peer().starts_with("wss 127.0.0.1:"), "{}", link.peer());
}

#[test]
fn a_client_the_server_does_not_pin_is_refused_as_a_trust_failure() {
    let (server_id, client_id) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    let server =
        TlsConfig::new(server_id.clone()).pins([Identity::generate().unwrap().fingerprint()]);
    let client = TlsConfig::new(client_id).pins([server_id.fingerprint()]);
    let address = listen(&server, secure_server());
    let source = source(&address, &client);
    let (registry, refused) = refresh(&source);
    assert_eq!(registry.offers::<dyn Greeter>().count(), 0);
    assert!(!refused.is_empty());
    let e = source.link().unwrap_err();
    assert_eq!(e.status, guatiao::Status::GUATIAO_ERR_BAD_VALUE, "{e:?}");
}

#[test]
fn a_peer_removed_from_the_store_cannot_link_again_over_wss() {
    let dir = scratch("removed");
    let server = TlsConfig::open(&dir).unwrap();
    let client_id = Identity::generate().unwrap();
    let store = TrustStore::open(&dir).unwrap();
    coqui_peer_tls::TrustStoreExt::accept_certificate(&store, client_id.certificate_der()).unwrap();
    let address = listen(&server, secure_server());
    let client = TlsConfig::new(client_id.clone()).pins([server.fingerprint()]);
    let source = source(&address, &client);
    let (registry, refused) = refresh(&source);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(greeting(&registry), "hello, wss");
    drop(registry);
    source.disconnect();

    assert!(store.remove(&client_id.fingerprint()).unwrap());
    let (registry, refused) = refresh(&source);
    assert_eq!(registry.offers::<dyn Greeter>().count(), 0, "{refused:?}");
    let e = source.link().unwrap_err();
    assert_eq!(e.status, guatiao::Status::GUATIAO_ERR_BAD_VALUE, "{e:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_configuration_names_a_wss_peer() {
    let dir = scratch("config");
    let server = TlsConfig::open(&dir.join("server")).unwrap();
    let client = TlsConfig::open(&dir.join("client")).unwrap();
    TrustStore::open(&dir.join("server"))
        .unwrap()
        .accept_key(&coqui_peer_tls::certificate_key(client.identity().certificate_der()).unwrap())
        .unwrap();
    let address = listen(&server, secure_server());
    let mut config = guatiao::Map::new();
    config.set("connect", format!("wss://{address}/").as_str());
    config.set(
        "state_dir",
        dir.join("client").display().to_string().as_str(),
    );
    let mut pins = guatiao::List::new();
    pins.push(server.fingerprint().canonical().as_str());
    config.set("pins", pins);
    // No security named: a network link runs mutual TLS.
    let (registry, refused) = refresh(&RemoteSource::from_config(&config).unwrap());
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(greeting(&registry), "hello, wss");

    // Server TLS against this mutual-TLS server: refused there.
    config.set("security", "server-tls");
    config.set("key", "k");
    let (registry, refused) = refresh(&RemoteSource::from_config(&config).unwrap());
    assert_eq!(registry.offers::<dyn Greeter>().count(), 0);
    assert!(!refused.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
