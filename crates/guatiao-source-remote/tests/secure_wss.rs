// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! TLS under a WebSocket (`wss://`) through coqui-web-channel: mutual TLS (pins,
//! the trust store, refusals) with the WebSocket's upgrade inside it, as a
//! browser holding a client certificate would connect, and server TLS.

#![cfg(all(feature = "ws", feature = "tls"))]

extern crate alloc;

mod common;

use alloc::sync::Arc;
use std::path::PathBuf;
use std::sync::Mutex;

use greeter_kind::Greeter;
use guatiao::library::Registry;
use guatiao::value::convert::TryAsRef;
use guatiao_source_remote::transport::tcp::Accepted;
use guatiao_source_remote::transport::tls::{Identity, TlsConfig, TrustStore};
use guatiao_source_remote::transport::ws::{WsConnect, WsListen};
use guatiao_source_remote::{Link, LinkOptions, RemoteSource, Server, Trust};

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
    listen_keeping(config, server).0
}

/// The links a server has taken, newest last.
type Links = Arc<Mutex<Vec<Arc<Link>>>>;

/// As [`listen`], and the links the server takes.
fn listen_keeping(config: &TlsConfig, server: Server) -> (String, Links) {
    let listener = WsListen::bind_tls("127.0.0.1:0", config).unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let links: Links = Arc::default();
    let kept = Arc::clone(&links);
    std::thread::spawn(move || {
        while let Ok(accepted) = listener.accept() {
            if let Accepted::Connection(conn) = accepted {
                let server = server.clone();
                let kept = Arc::clone(&kept);
                std::thread::spawn(move || {
                    if let Ok(link) = server.serve(conn) {
                        kept.lock().unwrap().push(link);
                    }
                });
            }
        }
    });
    (address, links)
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
    let client_fingerprint = client_id.fingerprint().canonical();
    let client = TlsConfig::new(client_id).pins([server_id.fingerprint()]);
    let (address, served) = listen_keeping(&server, secure_server());
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
    // The server names the client by the certificate it proved: the point
    // of noting it on the handshake's own thread.
    common::eventually("the server holds the link", || {
        !served.lock().unwrap().is_empty()
    });
    let named: Vec<_> = served
        .lock()
        .unwrap()
        .iter()
        .map(|l| l.peer_id().map(str::to_string))
        .collect();
    assert_eq!(named, [Some(client_fingerprint)]);
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

const KEY: &[u8] = b"test-key";

/// A source over `wss://` at server TLS: the client holds no certificate.
fn server_tls_source(address: &str, config: &TlsConfig) -> RemoteSource {
    RemoteSource::new(
        Box::new(
            WsConnect::new(&format!("wss://{address}/remote"))
                .tls(config)
                .unwrap(),
        ),
        LinkOptions::new(Trust::ServerTls).key(KEY),
    )
}

#[test]
fn server_tls_ends_link_over_wss_and_the_client_knows_the_server() {
    let server_id = Identity::generate().unwrap();
    let server = TlsConfig::new(server_id.clone()).server_tls();
    let client = TlsConfig::without_identity()
        .unwrap()
        .pins([server_id.fingerprint()]);
    let address = listen(
        &server,
        common::server(LinkOptions::new(Trust::ServerTls).key(KEY)),
    );
    let source = server_tls_source(&address, &client);
    let (registry, refused) = refresh(&source);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(greeting(&registry), "hello, wss");
    let link = source.link().unwrap();
    assert_eq!(link.trust(), Trust::ServerTls);
    assert_eq!(
        link.peer_id(),
        Some(server_id.fingerprint().canonical().as_str())
    );
}

#[test]
fn a_server_the_client_does_not_pin_is_refused_as_a_trust_failure() {
    let server = TlsConfig::new(Identity::generate().unwrap()).server_tls();
    let client = TlsConfig::without_identity()
        .unwrap()
        .pins([Identity::generate().unwrap().fingerprint()]);
    let address = listen(
        &server,
        common::server(LinkOptions::new(Trust::ServerTls).key(KEY)),
    );
    let e = server_tls_source(&address, &client).link().unwrap_err();
    assert_eq!(e.status, guatiao::Status::GUATIAO_ERR_BAD_VALUE, "{e:?}");
    assert!(e.message().contains("not pinned"), "{}", e.message());
}

/// A listener and a client that both allow resumption name the client at
/// every link, resumed or not: the channel reports the chain a resumed
/// TLS 1.3 session carries too.
#[test]
fn a_resumed_session_still_names_the_client() {
    use coqui_peer_tls::{Pins, Trust as Pinned};
    use coqui_web_channel::websocket::{Dialer, Frames};
    use coqui_web_channel::{Channel, Dialer as _, Kind};
    use guatiao_source_remote::transport::tls::Policy;
    let (server_id, client_id) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    let mut resuming = Policy::modern();
    resuming.allow_resumption = true;
    let server = TlsConfig::new(server_id.clone())
        .pins([client_id.fingerprint()])
        .policy(resuming.clone());
    let listener = WsListen::bind_tls("127.0.0.1:0", &server).unwrap();
    let at = listener.local_addr().unwrap();
    // One client configuration for every dial, so its session cache is
    // shared and a later dial can resume an earlier one.
    let built = coqui_peer_tls::client_config_with(
        &client_id,
        &Pinned::Pinned(Pins::fixed([server_id.fingerprint()])),
        &[b"http/1.1"],
        &resuming,
    )
    .unwrap();
    let config = Arc::new(built.config);
    for _ in 0..3 {
        let config = Arc::clone(&config);
        let dialing = std::thread::spawn(move || {
            let url = url::Url::parse(&format!("wss://{at}/")).unwrap();
            let channel = Dialer::new()
                .with_frames(Frames::Binary)
                .with_tls(config)
                .dial(Kind::WebSocket, &url, core::time::Duration::from_secs(10))
                .unwrap();
            channel.send(b"hello").unwrap();
            channel
        });
        let Accepted::Connection(mut accepted) = listener.accept().unwrap() else {
            panic!("admitted");
        };
        accepted
            .reader
            .set_timeout(Some(core::time::Duration::from_secs(10)))
            .unwrap();
        let mut hello = Vec::new();
        assert!(accepted.reader.read_frame(&mut hello, 1 << 10).unwrap());
        assert_eq!(
            accepted.reader.peer_id(),
            Some(client_id.fingerprint().canonical())
        );
        let channel = dialing.join().unwrap();
        channel.close();
    }
}

/// An access control list rule that names the address a client came to is
/// decided by the address the listener is bound to, and refuses only when
/// that is a wildcard.
#[test]
fn a_rule_naming_the_local_address_decides_by_the_bound_address() {
    use guatiao_source_remote::acl::{Acl, Action, Rule};
    use guatiao_source_remote::transport::tls::Policy;
    let (server_id, client_id) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    let client = TlsConfig::new(client_id.clone()).pins([server_id.fingerprint()]);
    let attempt = |destination: &str| {
        let mut rule = Rule::new(Action::Permit, "any".parse().unwrap());
        rule.destination = Some(destination.parse().unwrap());
        let mut acl = Acl::default();
        acl.push(rule);
        let mut policy = Policy::modern();
        policy.acl = Some(acl);
        let server = TlsConfig::new(server_id.clone())
            .pins([client_id.fingerprint()])
            .policy(policy);
        let address = listen(&server, secure_server());
        source(&address, &client).link().map(|_| ())
    };
    attempt("127.0.0.1/32").expect("the address it is bound to");
    // The control: the same rule for an address it is not bound to.
    assert!(attempt("192.0.2.1/32").is_err());
}

/// A fingerprint the list denies is refused, and one it does not name is
/// not: the listener decides by the certificate the client proved.
#[test]
fn a_fingerprint_the_list_denies_is_refused_and_another_is_not() {
    use guatiao_source_remote::acl::{Acl, Action, Rule};
    use guatiao_source_remote::transport::tls::Policy;
    let server_id = Identity::generate().unwrap();
    let (denied, allowed) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    let mut deny = Rule::new(Action::Deny, "any".parse().unwrap());
    deny.fingerprint = Some(denied.fingerprint().canonical());
    let mut acl = Acl::default();
    acl.push(deny);
    acl.push(Rule::new(Action::Permit, "127.0.0.1/32".parse().unwrap()));
    let mut policy = Policy::modern();
    policy.acl = Some(acl);
    let server = TlsConfig::new(server_id.clone())
        .pins([denied.fingerprint(), allowed.fingerprint()])
        .policy(policy);
    let address = listen(&server, secure_server());
    let link = |id: &Identity| {
        let client = TlsConfig::new(id.clone()).pins([server_id.fingerprint()]);
        source(&address, &client).link().map(|_| ())
    };
    link(&allowed).expect("a client the list does not deny");
    assert!(link(&denied).is_err(), "a client the list denies");
}
