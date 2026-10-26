// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The secure level: mutual TLS between a host and a server, in this
//! process and across two; pinning, pairing through a trust store, the
//! access control list by address and by fingerprint, a removed peer, and
//! large calls beside small ones.

#![cfg(feature = "tls")]

mod common;

use alloc::sync::Arc;
use core::time::Duration;
use std::io::Read;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::Instant;

extern crate alloc;

use common::procs;
use greeter_kind::{Counter, Greeter};
use guatiao::library::Registry;
use guatiao::value::convert::TryAsRef;
use guatiao_source_remote::acl::{Acl, Action, Rule};
use guatiao_source_remote::transport::Connect;
use guatiao_source_remote::transport::tcp::Accepted;
use guatiao_source_remote::transport::tls::{
    Identity, PeerId, Policy, TlsConfig, TlsConnect, TlsListen, TrustStore,
};
use guatiao_source_remote::{Link, LinkOptions, RemoteSource, Server, Trust};

/// A directory of its own under the target's scratch space, emptied.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("secure_tls")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Links a server set up, kept for the test to look at.
type Served = Arc<Mutex<Vec<Arc<Link>>>>;

/// A secure server of the test greeter on loopback, serving every link
/// it accepts; answers its address and the links it set up.
fn serve(config: &TlsConfig, server: Server) -> (String, Served) {
    let listener = TlsListen::bind("127.0.0.1:0", config).unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let served = Served::default();
    let keep = Arc::clone(&served);
    std::thread::spawn(move || {
        while let Ok(accepted) = listener.accept() {
            if let Accepted::Connection(conn) = accepted {
                let server = server.clone();
                let keep = Arc::clone(&keep);
                std::thread::spawn(move || {
                    if let Ok(link) = server.serve(conn) {
                        keep.lock().unwrap().push(link);
                    }
                });
            }
        }
    });
    (address, served)
}

fn listen(config: &TlsConfig, server: Server) -> String {
    serve(config, server).0
}

fn secure_server() -> Server {
    common::server(LinkOptions::new(Trust::Secure))
}

fn source(address: &str, config: &TlsConfig) -> RemoteSource {
    RemoteSource::new(
        Box::new(TlsConnect::new(address, config).unwrap()),
        LinkOptions::new(Trust::Secure),
    )
}

/// Refreshes a registry over `source`: the registry and what was refused.
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
    let map = greeter.greet("tls").unwrap();
    map.get("greeting")
        .and_then(TryAsRef::<str>::try_as_ref)
        .unwrap_or_default()
        .to_string()
}

#[test]
fn pinned_ends_link_and_each_knows_the_other_by_fingerprint() {
    let (server_id, client_id) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    let server = TlsConfig::new(server_id.clone()).pins([client_id.fingerprint()]);
    let client = TlsConfig::new(client_id.clone()).pins([server_id.fingerprint()]);
    let (address, served) = serve(&server, secure_server());
    let source = source(&address, &client);
    let (registry, refused) = refresh(&source);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(greeting(&registry), "hello, tls");
    let link = source.link().unwrap();
    assert_eq!(link.trust(), Trust::Secure);
    assert_eq!(
        link.peer_id(),
        Some(server_id.fingerprint().canonical().as_str())
    );
    let held = served.lock().unwrap();
    assert_eq!(held.len(), 1);
    assert_eq!(
        held[0].peer_id(),
        Some(client_id.fingerprint().canonical().as_str())
    );
    assert!(
        held[0].peer().starts_with("tls 127.0.0.1:"),
        "{}",
        held[0].peer()
    );
}

#[test]
fn a_server_matching_no_pin_is_refused_by_the_client() {
    let (server_id, client_id) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    let stranger = Identity::generate().unwrap().fingerprint();
    let server = TlsConfig::new(server_id).pins([client_id.fingerprint()]);
    let client = TlsConfig::new(client_id).pins([stranger]);
    let address = listen(&server, secure_server());
    let e = TlsConnect::new(&address, &client)
        .unwrap()
        .connect()
        .unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied, "{e}");
    assert!(
        e.to_string().contains("is not one this client trusts"),
        "{e}"
    );
}

#[test]
fn a_client_the_store_does_not_know_waits_with_its_address_until_accepted() {
    let dir = scratch("store");
    let server = TlsConfig::open(&dir).unwrap();
    let client_id = Identity::generate().unwrap();
    let client = TlsConfig::new(client_id.clone()).pins([server.fingerprint()]);
    let address = listen(&server, secure_server());

    let source = source(&address, &client);
    let (_, refused) = refresh(&source);
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert!(
        refused[0].contains("does not trust this client"),
        "{refused:?}"
    );
    assert!(
        refused[0].contains(&client_id.fingerprint().canonical()),
        "{refused:?}"
    );

    let store = TrustStore::open(&dir).unwrap();
    common::eventually("the client waits", || !store.waiting().unwrap().is_empty());
    let waiting = store.waiting().unwrap();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].id, client_id.fingerprint());
    let from = waiting[0].source.expect("the address it came from");
    assert!(from.ip().is_loopback(), "{from}");

    store.accept(&client_id.fingerprint()).unwrap();
    source.disconnect();
    let (registry, refused) = refresh(&source);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(greeting(&registry), "hello, tls");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A rustls client presenting `victim`'s certificate, signing with a key
/// that is not its own.
fn impostor(victim: &Identity) -> Arc<rustls::ClientConfig> {
    use rustls::client::ResolvesClientCert;
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use rustls::sign::CertifiedKey;

    #[derive(Debug)]
    struct Impostor(Arc<CertifiedKey>);
    impl ResolvesClientCert for Impostor {
        fn resolve(&self, _: &[&[u8]], _: &[rustls::SignatureScheme]) -> Option<Arc<CertifiedKey>> {
            Some(Arc::clone(&self.0))
        }
        fn has_certs(&self) -> bool {
            true
        }
    }
    let other = Identity::generate().unwrap().to_pem().unwrap();
    let key = PrivateKeyDer::from_pem_slice(other.as_bytes()).unwrap();
    let signing = rustls::crypto::ring::sign::any_supported_type(&key).unwrap();
    let certificate = CertificateDer::from(victim.certificate_der().to_vec());
    let verifier = coqui_peer_tls::Trust::AnyWithKeyProof
        .server_cert_verifier()
        .unwrap();
    let mut config = rustls::ClientConfig::builder_with_provider(coqui_peer_tls::crypto_provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_client_cert_resolver(Arc::new(Impostor(Arc::new(CertifiedKey::new(
            vec![certificate],
            signing,
        )))));
    config.alpn_protocols = vec![b"guatiao-remote".to_vec()];
    Arc::new(config)
}

#[test]
fn a_stolen_certificate_without_its_key_leaves_no_waiting_entry() {
    let dir = scratch("stolen");
    let server = TlsConfig::open(&dir).unwrap();
    let address = listen(&server, secure_server());
    let victim = Identity::generate().unwrap();

    let handshake = coqui_peer_tls::TlsHandshake::client(impostor(&victim)).unwrap();
    let mut socket = TcpStream::connect(&address).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    // TLS 1.3 lets the client finish; the server refuses at its signature.
    if coqui_peer::drive_stream(handshake, &mut socket).is_ok() {
        let mut byte = [0u8; 1];
        let _ = socket.read(&mut byte);
    }
    // A genuine stranger, for contrast, is recorded.
    let genuine = TlsConfig::new(Identity::generate().unwrap()).pins([server.fingerprint()]);
    let _ = refresh(&source(&address, &genuine));
    let store = TrustStore::open(&dir).unwrap();
    common::eventually("the genuine client waits", || {
        !store.waiting().unwrap().is_empty()
    });
    let waiting: Vec<PeerId> = store.waiting().unwrap().into_iter().map(|w| w.id).collect();
    assert_eq!(waiting, [genuine.fingerprint()]);
    assert!(!waiting.contains(&victim.fingerprint()));
    let _ = std::fs::remove_dir_all(&dir);
}

fn acl(rules: Vec<Rule>) -> Acl {
    Acl {
        rules,
        default: Action::Deny,
    }
}

#[test]
fn the_access_control_list_closes_a_denied_source_before_the_handshake() {
    let (server_id, client_id) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    let mut policy = Policy::strict();
    policy.acl = Some(acl(vec![Rule::new(
        Action::Permit,
        "10.0.0.0/8".parse().unwrap(),
    )]));
    let server = TlsConfig::new(server_id.clone())
        .pins([client_id.fingerprint()])
        .policy(policy);
    let listener = TlsListen::bind("127.0.0.1:0", &server).unwrap();
    let mut raw = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    assert!(matches!(listener.accept().unwrap(), Accepted::Refused(_)));
    raw.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut byte = [0u8; 1];
    assert!(
        matches!(raw.read(&mut byte), Ok(0) | Err(_)),
        "nothing is sent to it"
    );

    // The same client through the front door is refused the same way.
    let client = TlsConfig::new(client_id).pins([server_id.fingerprint()]);
    let connect = TlsConnect::new(&listener.local_addr().unwrap().to_string(), &client).unwrap();
    let dialing = std::thread::spawn(move || connect.connect());
    assert!(matches!(listener.accept().unwrap(), Accepted::Refused(_)));
    assert!(dialing.join().unwrap().is_err());
}

#[test]
fn ending_the_links_ends_one_still_in_its_tls_handshake() {
    let server_id = Identity::generate().unwrap();
    let config = TlsConfig::new(server_id).pins([Identity::generate().unwrap().fingerprint()]);
    let listener = TlsListen::bind("127.0.0.1:0", &config).unwrap();
    // Connected, and never sends its ClientHello: the TLS handshake would
    // wait out the link's 10 s bound.
    let _silent = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let Accepted::Connection(conn) = listener.accept().unwrap() else {
        panic!("a permitted source");
    };
    let took = common::ended_in_handshake(&secure_server(), conn);
    assert!(took < Duration::from_secs(3), "{took:?}");
}

#[test]
fn a_fingerprint_rule_refuses_a_trusted_peer_once_the_handshake_names_it() {
    let (server_id, allowed, denied) = (
        Identity::generate().unwrap(),
        Identity::generate().unwrap(),
        Identity::generate().unwrap(),
    );
    let mut deny = Rule::new(Action::Deny, "any".parse().unwrap());
    // Any spelling of a fingerprint names it.
    deny.fingerprint = Some(denied.fingerprint().to_base32_grouped());
    let mut policy = Policy::strict();
    policy.acl = Some(acl(vec![
        deny,
        Rule::new(Action::Permit, "127.0.0.0/8".parse().unwrap()),
    ]));
    let server = TlsConfig::new(server_id.clone())
        .pins([allowed.fingerprint(), denied.fingerprint()])
        .policy(policy);
    let address = listen(&server, secure_server());

    let ok = TlsConfig::new(allowed).pins([server_id.fingerprint()]);
    let (registry, refused) = refresh(&source(&address, &ok));
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(greeting(&registry), "hello, tls");

    let no = TlsConfig::new(denied).pins([server_id.fingerprint()]);
    let (registry, refused) = refresh(&source(&address, &no));
    assert_eq!(registry.offers::<dyn Greeter>().count(), 0);
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert!(
        refused[0].contains("access control list refuses it"),
        "{refused:?}"
    );
}

#[test]
fn a_list_permitting_one_fingerprint_admits_it_alone() {
    let (server_id, one, other) = (
        Identity::generate().unwrap(),
        Identity::generate().unwrap(),
        Identity::generate().unwrap(),
    );
    // Deferred for every address, then decided by the fingerprint.
    let mut only = Rule::new(Action::Permit, "any".parse().unwrap());
    only.fingerprint = Some(one.fingerprint().canonical());
    let mut policy = Policy::strict();
    policy.acl = Some(acl(vec![only]));
    let server = TlsConfig::new(server_id.clone())
        .pins([one.fingerprint(), other.fingerprint()])
        .policy(policy);
    let address = listen(&server, secure_server());
    let yes = TlsConfig::new(one).pins([server_id.fingerprint()]);
    let (registry, refused) = refresh(&source(&address, &yes));
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(greeting(&registry), "hello, tls");
    let no = TlsConfig::new(other).pins([server_id.fingerprint()]);
    let (registry, refused) = refresh(&source(&address, &no));
    assert_eq!(registry.offers::<dyn Greeter>().count(), 0, "{refused:?}");
}

#[test]
fn a_client_has_no_access_control_list() {
    let id = Identity::generate().unwrap();
    let mut policy = Policy::strict();
    policy.acl = Some(acl(Vec::new()));
    let client = TlsConfig::new(id.clone())
        .pins([id.fingerprint()])
        .policy(policy);
    let e = TlsConnect::new("127.0.0.1:9", &client).unwrap_err();
    assert!(e.to_string().contains("a client has none"), "{e}");
}

/// A TLS server that pins the client but names no application protocol
/// is not a guatiao server.
#[test]
fn a_server_agreeing_no_application_protocol_is_refused() {
    let (server_id, client_id) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    let trust =
        coqui_peer_tls::Trust::Pinned(coqui_peer_tls::Pins::fixed([client_id.fingerprint()]));
    let config = Arc::new(coqui_peer_tls::server_config(&server_id, &trust, &[]).unwrap());
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let serving = std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let _ = coqui_peer_tls::accept(config, socket);
    });
    let client = TlsConfig::new(client_id).pins([server_id.fingerprint()]);
    let e = TlsConnect::new(&address, &client)
        .unwrap()
        .connect()
        .unwrap_err();
    assert!(e.to_string().contains("application protocol"), "{e}");
    serving.join().unwrap();
}

#[test]
fn a_peer_removed_from_the_store_cannot_link_again() {
    let dir = scratch("removed");
    let server = TlsConfig::open(&dir).unwrap();
    let client_id = Identity::generate().unwrap();
    let store = TrustStore::open(&dir).unwrap();
    coqui_peer_tls::TrustStoreExt::accept_certificate(&store, client_id.certificate_der()).unwrap();
    let address = listen(&server, secure_server());
    let client = TlsConfig::new(client_id.clone()).pins([server.fingerprint()]);
    // One connector for both links: a TLS session it kept could resume.
    let source = source(&address, &client);
    let (registry, refused) = refresh(&source);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(greeting(&registry), "hello, tls");
    drop(registry);
    source.disconnect();

    assert!(store.remove(&client_id.fingerprint()).unwrap());
    let (registry, refused) = refresh(&source);
    assert_eq!(registry.offers::<dyn Greeter>().count(), 0, "{refused:?}");
    assert!(
        refused
            .iter()
            .any(|r| r.contains("does not trust this client")),
        "{refused:?}"
    );
    // A refusal is a trust failure, not a peer gone.
    let e = source.link().unwrap_err();
    assert_eq!(e.status, guatiao::Status::GUATIAO_ERR_BAD_VALUE, "{e:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The control for the test above: resumption kept by the policy (listed
/// as a weakening) lets a removed peer back in on the session it holds.
#[test]
fn resumption_kept_by_the_policy_lets_a_removed_peer_link_again() {
    let dir = scratch("resumed");
    let mut policy = Policy::strict();
    policy.allow_resumption = true;
    let server = TlsConfig::open(&dir).unwrap().policy(policy.clone());
    assert!(
        server
            .weakened()
            .iter()
            .any(|w| w.setting == "allow_resumption"),
        "{:?}",
        server.weakened()
    );
    let client_id = Identity::generate().unwrap();
    let store = TrustStore::open(&dir).unwrap();
    coqui_peer_tls::TrustStoreExt::accept_certificate(&store, client_id.certificate_der()).unwrap();
    let address = listen(&server, secure_server());
    let client = TlsConfig::new(client_id.clone())
        .pins([server.fingerprint()])
        .policy(policy);
    let source = source(&address, &client);
    let (registry, refused) = refresh(&source);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(greeting(&registry), "hello, tls");
    drop(registry);
    source.disconnect();

    assert!(store.remove(&client_id.fingerprint()).unwrap());
    let (registry, refused) = refresh(&source);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(greeting(&registry), "hello, tls");
    drop(registry);
    source.disconnect();
    let _ = std::fs::remove_dir_all(&dir);
}

/// A certificate and its key as rustls takes them.
fn certified(
    id: &Identity,
) -> (
    rustls::pki_types::CertificateDer<'static>,
    rustls::pki_types::PrivateKeyDer<'static>,
) {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    let pem = id.to_pem().unwrap();
    (
        CertificateDer::from(id.certificate_der().to_vec()),
        PrivateKeyDer::from_pem_slice(pem.as_bytes()).unwrap(),
    )
}

/// A TLS server presenting `id`'s certificate with a chain behind it,
/// accepting any client that proves its key.
fn chain_server(id: &Identity) -> String {
    let (certificate, key) = certified(id);
    let extra = certified(&Identity::generate().unwrap()).0;
    let verifier = coqui_peer_tls::Trust::AnyWithKeyProof
        .client_cert_verifier()
        .unwrap();
    let mut config = rustls::ServerConfig::builder_with_provider(coqui_peer_tls::crypto_provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_client_cert_verifier(verifier)
        .with_single_cert(vec![certificate, extra], key)
        .unwrap();
    config.alpn_protocols = vec![b"guatiao-remote".to_vec()];
    let config = Arc::new(config);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let _ = coqui_peer_tls::accept(config, socket);
    });
    address
}

/// What `client` is told connecting to `address`: the status and the words.
fn refused_link(address: &str, client: &TlsConfig) -> (guatiao::Status, String) {
    let e = source(address, client).link().unwrap_err();
    (e.status, e.message().to_string())
}

#[test]
fn a_client_says_why_it_refuses_a_server() {
    let server_id = Identity::generate().unwrap();
    let client_id = Identity::generate().unwrap();
    let server = TlsConfig::new(server_id.clone()).pins([client_id.fingerprint()]);
    let address = listen(&server, secure_server());

    // Pinning another server.
    let stranger = Identity::generate().unwrap().fingerprint();
    let pinned = TlsConfig::new(client_id.clone()).pins([stranger]);
    let (status, why) = refused_link(&address, &pinned);
    assert_eq!(status, guatiao::Status::GUATIAO_ERR_BAD_VALUE, "{why}");
    assert!(why.contains("is not one this client trusts"), "{why}");
    assert!(why.contains("is not pinned"), "{why}");

    // Trusting what its store accepted, which this server is not.
    let dir = scratch("client_store");
    let stored = TlsConfig::open(&dir).unwrap();
    let server = TlsConfig::new(server_id.clone()).pins([stored.fingerprint()]);
    let address = listen(&server, secure_server());
    let (status, why) = refused_link(&address, &stored);
    assert_eq!(status, guatiao::Status::GUATIAO_ERR_BAD_VALUE, "{why}");
    assert!(why.contains("is not accepted"), "{why}");

    // A pinned server presenting a chain.
    let address = chain_server(&server_id);
    let client = TlsConfig::new(client_id).pins([server_id.fingerprint()]);
    let (status, why) = refused_link(&address, &client);
    assert_eq!(status, guatiao::Status::GUATIAO_ERR_BAD_VALUE, "{why}");
    assert!(why.contains("presented a certificate chain"), "{why}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A listener serving one link: its address, and what serving it answered.
fn serve_once(config: &TlsConfig) -> (String, std::thread::JoinHandle<String>) {
    let listener = TlsListen::bind("127.0.0.1:0", config).unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let serving = std::thread::spawn(move || {
        let Accepted::Connection(conn) = listener.accept().unwrap() else {
            panic!("no access control list: every source proceeds");
        };
        match secure_server().serve(conn) {
            Ok(_) => "served".to_string(),
            Err(e) => e.to_string(),
        }
    });
    (address, serving)
}

/// A rustls client presenting `id`'s certificate with a chain behind it.
fn chain_client(id: &Identity) -> Arc<rustls::ClientConfig> {
    let (certificate, key) = certified(id);
    let extra = certified(&Identity::generate().unwrap()).0;
    let verifier = coqui_peer_tls::Trust::AnyWithKeyProof
        .server_cert_verifier()
        .unwrap();
    let mut config = rustls::ClientConfig::builder_with_provider(coqui_peer_tls::crypto_provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_client_auth_cert(vec![certificate, extra], key)
        .unwrap();
    config.alpn_protocols = vec![b"guatiao-remote".to_vec()];
    Arc::new(config)
}

#[test]
fn a_server_says_why_it_refuses_a_client() {
    let server_id = Identity::generate().unwrap();
    let (known, stranger) = (Identity::generate().unwrap(), Identity::generate().unwrap());

    // Pinning another client.
    let server = TlsConfig::new(server_id.clone()).pins([known.fingerprint()]);
    let (address, serving) = serve_once(&server);
    let client = TlsConfig::new(stranger.clone()).pins([server_id.fingerprint()]);
    let _ = refresh(&source(&address, &client));
    let why = serving.join().unwrap();
    assert!(why.contains("the client is not pinned"), "{why}");

    // A store that has not accepted it: it waits there.
    let dir = scratch("server_refuses");
    let server = TlsConfig::open(&dir).unwrap();
    let (address, serving) = serve_once(&server);
    let client = TlsConfig::new(stranger.clone()).pins([server.fingerprint()]);
    let _ = refresh(&source(&address, &client));
    let why = serving.join().unwrap();
    assert!(why.contains("the client is not accepted"), "{why}");
    assert!(why.contains("waits there"), "{why}");

    // A pinned client presenting a chain.
    let server = TlsConfig::new(server_id).pins([known.fingerprint()]);
    let (address, serving) = serve_once(&server);
    let handshake = coqui_peer_tls::TlsHandshake::client(chain_client(&known)).unwrap();
    let mut socket = TcpStream::connect(&address).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let _ = coqui_peer::drive_stream(handshake, &mut socket);
    let why = serving.join().unwrap();
    assert!(why.contains("presented a certificate chain"), "{why}");
    drop(socket);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The alert a refusing server sends is `certificate_unknown`, and the
/// client says so: heard before it wrote anything, nothing races it.
#[test]
fn a_refused_client_hears_its_certificate_was_refused() {
    let dir = scratch("alert");
    let server = TlsConfig::open(&dir).unwrap();
    let address = listen(&server, secure_server());
    let client_id = Identity::generate().unwrap();
    let client = TlsConfig::new(client_id.clone()).pins([server.fingerprint()]);
    let mut conn = TlsConnect::new(&address, &client)
        .unwrap()
        .connect()
        .unwrap();
    conn.reader
        .set_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut buf = Vec::new();
    let e = conn.reader.read_frame(&mut buf, 1 << 10).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied, "{e}");
    let why = e.to_string();
    assert!(why.contains("certificate_unknown"), "{why}");
    assert!(why.contains("does not trust this client"), "{why}");
    assert!(why.contains(&client_id.fingerprint().canonical()), "{why}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A secure link and a trusted one to the same greeter, for comparing.
fn linked_pair() -> (RemoteSource, Registry) {
    let (server_id, client_id) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    let server = TlsConfig::new(server_id.clone()).pins([client_id.fingerprint()]);
    let client = TlsConfig::new(client_id).pins([server_id.fingerprint()]);
    let address = listen(&server, secure_server());
    let source = source(&address, &client);
    let (registry, refused) = refresh(&source);
    assert!(refused.is_empty(), "{refused:?}");
    (source, registry)
}

#[test]
fn arguments_far_past_a_record_cross_both_ways_at_once() {
    let (_source, registry) = linked_pair();
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    let name = "x".repeat(8 << 20);
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let greeter = greeter.clone();
            let name = name.clone();
            std::thread::spawn(move || greeter.greet(&name).unwrap())
        })
        .collect();
    for t in threads {
        let map = t.join().unwrap();
        let said = map
            .get("greeting")
            .and_then(TryAsRef::<str>::try_as_ref)
            .unwrap();
        assert_eq!(said.len(), "hello, ".len() + (8 << 20));
    }
}

fn median(v: &mut [Duration]) -> Duration {
    v.sort();
    v[v.len() / 2]
}

/// The reader thread is never held up by a writer: small calls while
/// large ones stream stay within reach of small calls alone. The bound is
/// loose (a shared machine moves timings); a reader starved by a lock held
/// across a read waits whole transfers, hundreds of milliseconds.
#[test]
fn small_calls_stay_quick_while_large_ones_stream() {
    let (_source, registry) = linked_pair();
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    let counter = registry
        .offers::<dyn Counter>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    for _ in 0..50 {
        counter.count();
    }
    let mut idle: Vec<Duration> = (0..200)
        .map(|_| {
            let t = Instant::now();
            counter.count();
            t.elapsed()
        })
        .collect();
    let stop = Arc::new(core::sync::atomic::AtomicBool::new(false));
    let streaming: Vec<_> = (0..2)
        .map(|_| {
            let greeter = greeter.clone();
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                let name = "y".repeat(4 << 20);
                let mut n = 0;
                while !stop.load(core::sync::atomic::Ordering::Relaxed) {
                    greeter.greet(&name).unwrap();
                    n += 1;
                }
                n
            })
        })
        .collect();
    std::thread::sleep(Duration::from_millis(100));
    let mut busy: Vec<Duration> = (0..200)
        .map(|_| {
            let t = Instant::now();
            counter.count();
            t.elapsed()
        })
        .collect();
    stop.store(true, core::sync::atomic::Ordering::Relaxed);
    let large: usize = streaming.into_iter().map(|t| t.join().unwrap()).sum();
    let (idle, busy) = (median(&mut idle), median(&mut busy));
    println!("small call p50: {idle:?} alone, {busy:?} beside {large} calls of 4 MiB");
    assert!(large > 0);
    assert!(
        busy < Duration::from_millis(100),
        "small calls waited {busy:?} beside large ones ({idle:?} alone)"
    );
}

fn server_program() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_guatiao-remote-server"))
}

fn kinds_json() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/greeter_kind/include/greeter_kind.kinds.json")
}

/// Mutual TLS between this process and a `guatiao-remote-server` child:
/// the first link waits, the operator accepts the client with the
/// server's own command, the next link calls the greeter it loaded.
#[test]
fn pairing_across_two_processes_through_the_servers_commands() {
    let Some(derived) = procs::library("derived_greeter") else {
        panic!("derived_greeter is not built: `cargo build -p derived_greeter` first");
    };
    let dir = scratch("two_processes");
    let (server_dir, client_dir) = (dir.join("server"), dir.join("client"));
    let (mut child, lines) = procs::start(
        Command::new(server_program())
            .args(["--listen", "tls://127.0.0.1:0", "--state-dir"])
            .arg(&server_dir)
            .arg("--kinds")
            .arg(kinds_json())
            .arg("--load")
            .arg(&derived),
    );
    let first = lines
        .recv_timeout(procs::DEADLINE)
        .expect("the server says where it serves");
    let serving = first
        .split("serving tls://")
        .nth(1)
        .unwrap_or_else(|| panic!("{first}"));
    let (address, rest) = serving.split_once(" as ").unwrap();
    let server_fp: PeerId = rest.split(',').next().unwrap().parse().unwrap();

    let client = TlsConfig::open(&client_dir).unwrap().pins([server_fp]);
    let me = client.fingerprint();
    let source = source(address, &client);
    let (_, refused) = refresh(&source);
    assert!(
        refused.iter().any(|r| r.contains(&me.canonical())),
        "{refused:?}"
    );

    let waiting = || {
        procs::run(
            Command::new(server_program())
                .arg("--state-dir")
                .arg(&server_dir)
                .arg("--waiting"),
        )
    };
    common::eventually("the client is listed as waiting", || {
        waiting().1.contains(&me.canonical())
    });
    let (ok, said) = waiting();
    assert!(ok && said.contains(" from 127.0.0.1:"), "{said}");
    let (ok, said) = procs::run(
        Command::new(server_program())
            .arg("--state-dir")
            .arg(&server_dir)
            .arg("--accept")
            .arg(me.canonical()),
    );
    assert!(ok, "{said}");

    let (registry, refused) = refresh(&source);
    assert!(refused.is_empty(), "{refused:?}");
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .expect("derived_greeter's greeter")
        .shared()
        .unwrap();
    assert!(greeter.greet("across").is_ok());
    assert_eq!(
        source.link().unwrap().peer_id(),
        Some(server_fp.canonical().as_str())
    );
    drop(greeter);
    drop(registry);
    source.disconnect();
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
}

/// The secure level from a source file's configuration, as a host in any
/// language hands it over.
#[test]
fn a_configuration_map_reaches_the_secure_level() {
    let dir = scratch("config");
    let server = TlsConfig::open(&dir.join("server")).unwrap();
    let client = TlsConfig::open(&dir.join("client")).unwrap();
    TrustStore::open(&dir.join("server"))
        .unwrap()
        .accept_key(&coqui_peer_tls::certificate_key(client.identity().certificate_der()).unwrap())
        .unwrap();
    let address = listen(&server, secure_server());

    let mut config = guatiao::Map::new();
    config.set("connect", format!("tcp://{address}").as_str());
    config.set("security", "mutual-tls");
    config.set(
        "state_dir",
        dir.join("client").display().to_string().as_str(),
    );
    let mut pins = guatiao::List::new();
    pins.push(server.fingerprint().to_base32_grouped().as_str());
    config.set("pins", pins);
    let mut policy = guatiao::Map::new();
    policy.set("profile", "strict");
    policy.set("check_validity", true);
    config.set("policy", policy);
    let source = RemoteSource::from_config(&config).unwrap();
    let (registry, refused) = refresh(&source);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(greeting(&registry), "hello, tls");

    // A policy key it does not know is refused, naming the policy.
    let mut wrong = guatiao::Map::new();
    wrong.set("tls_version", "1.3");
    config.set("policy", wrong);
    let refused = RemoteSource::from_config(&config).unwrap_err();
    assert!(
        refused.message().contains("not a TLS policy"),
        "{}",
        refused.message()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A host sees every setting looser than strict that its own
/// configuration and its peer manifests name, by peer, as the server
/// binary prints its own.
#[test]
fn a_host_sees_the_settings_its_configuration_weakens() {
    let dir = scratch("weakened");
    let identity = dir.join("host.pem");
    let mut config = guatiao::Map::new();
    config.set("connect", "tls://127.0.0.1:9");
    config.set("security", "mutual-tls");
    config.set("identity_file", identity.display().to_string().as_str());
    let pin = format!("sha256:{}", "0".repeat(64));
    let mut pins = guatiao::List::new();
    pins.push(pin.as_str());
    config.set("pins", pins);
    let source = RemoteSource::from_config(&config).unwrap();
    assert!(source.weakened().is_empty(), "{:?}", source.weakened());

    let mut policy = guatiao::Map::new();
    policy.set("profile", "strict");
    policy.set("allow_resumption", true);
    config.set("policy", policy.clone());
    let source = RemoteSource::from_config(&config).unwrap();
    let weakened = source.weakened();
    assert_eq!(weakened.len(), 1, "{weakened:?}");
    assert_eq!(weakened[0].0, "");
    assert_eq!(weakened[0].1.setting, "allow_resumption");

    // No TLS runs at none, so no policy holds.
    let mut plain = guatiao::Map::new();
    plain.set("connect", "tcp://127.0.0.1:9");
    plain.set("security", "none");
    plain.set("key", "a key");
    plain.set("policy", policy);
    let source = RemoteSource::from_config(&plain).unwrap();
    assert!(source.weakened().is_empty(), "{:?}", source.weakened());

    // A peer manifest's own policy, under the peer's name, once a refresh
    // has read it.
    let peers = dir.join("peers");
    std::fs::create_dir_all(&peers).unwrap();
    std::fs::write(
        peers.join("lab.guatiao.toml"),
        format!(
            "type = \"remote\"\nname = \"lab\"\nconnect = \"tls://127.0.0.1:9\"\n\
             security = \"mutual-tls\"\nidentity_file = \"lab.pem\"\npins = [\"{pin}\"]\n\
             [policy]\nprofile = \"strict\"\nallow_resumption = true\n"
        ),
    )
    .unwrap();
    let mut described = guatiao::Map::new();
    let mut dirs = guatiao::List::new();
    dirs.push(peers.display().to_string().as_str());
    described.set("dirs", dirs);
    let source = RemoteSource::from_config(&described).unwrap();
    assert!(
        source.weakened().is_empty(),
        "nothing read before a refresh"
    );
    let _ = refresh(&source);
    let weakened = source.weakened();
    assert_eq!(weakened.len(), 1, "{weakened:?}");
    assert_eq!(weakened[0].0, "lab");
    assert_eq!(weakened[0].1.setting, "allow_resumption");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A manifest's identity file made by the first refresh leaves the peer
/// as it was: the next refresh neither replaces it nor reports it again.
#[test]
fn an_identity_file_the_first_refresh_makes_leaves_the_peer_as_it_was() {
    let dir = scratch("identity_made");
    let pin = format!("sha256:{}", "0".repeat(64));
    std::fs::write(
        dir.join("lab.guatiao.toml"),
        format!(
            "type = \"remote\"\nname = \"lab\"\nconnect = \"tls://127.0.0.1:9\"\n\
             security = \"mutual-tls\"\nidentity_file = \"lab.pem\"\npins = [\"{pin}\"]\n"
        ),
    )
    .unwrap();
    let mut config = guatiao::Map::new();
    let mut dirs = guatiao::List::new();
    dirs.push(dir.display().to_string().as_str());
    config.set("dirs", dirs);
    let source = RemoteSource::from_config(&config).unwrap();
    let (mut registry, refused) = refresh(&source);
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert!(dir.join("lab.pem").is_file(), "the identity is made");
    let refreshed = registry.refresh();
    assert!(
        refreshed.is_empty(),
        "reported once while it stays away: {refreshed:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_secure_level_over_a_transport_that_proves_no_peer_is_refused() {
    let mut config = guatiao::Map::new();
    config.set("connect", "tls://127.0.0.1:9");
    config.set("security", "none");
    let refused = RemoteSource::from_config(&config).unwrap_err();
    assert!(refused.message().contains("`tls://` runs TLS"));
    // In Rust, a link at the secure level over memory proves no peer.
    let source = RemoteSource::new(
        Box::new(common::Memory(secure_server())),
        LinkOptions::new(Trust::Secure),
    );
    let e = source.link().unwrap_err();
    assert!(
        e.message().contains("transport that proves the peer"),
        "{}",
        e.message()
    );
}

/// A peer manifest names a secure peer: its pins, its policy and the
/// identity this host presents to it, by a path beside the manifest.
#[test]
fn a_peer_manifest_names_a_secure_peer() {
    let dir = scratch("manifest");
    let server = TlsConfig::open(&dir.join("server")).unwrap();
    let address = listen(&server, secure_server());
    let peers = dir.join("peers");
    std::fs::create_dir_all(&peers).unwrap();
    let client = Identity::open_or_create(&peers.join("lab.identity.pem")).unwrap();
    TrustStore::open(&dir.join("server"))
        .unwrap()
        .accept_key(&coqui_peer_tls::certificate_key(client.certificate_der()).unwrap())
        .unwrap();
    std::fs::write(
        peers.join("lab.guatiao.toml"),
        format!(
            "type = \"remote\"\nname = \"lab\"\nconnect = \"tls://{address}\"\n\
             identity_file = \"lab.identity.pem\"\npins = [\"{}\"]\n\n[policy]\nprofile = \"strict\"\n",
            server.fingerprint()
        ),
    )
    .unwrap();
    let mut config = guatiao::Map::new();
    let mut dirs = guatiao::List::new();
    dirs.push(peers.display().to_string().as_str());
    config.set("dirs", dirs);
    let source = RemoteSource::from_config(&config).unwrap();
    let mut registry = Registry::new("host", "1.0");
    registry.add_source("peers", source).unwrap();
    let refreshed = registry.refresh();
    assert!(refreshed.refused.is_empty(), "{refreshed:?}");
    assert_eq!(refreshed.loaded.len(), 1, "{refreshed:?}");
    assert_eq!(greeting(&registry), "hello, tls");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_token_the_server_requires_is_required_at_mutual_tls_too() {
    let (server_id, client_id) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    let server = TlsConfig::new(server_id.clone()).pins([client_id.fingerprint()]);
    let client = TlsConfig::new(client_id).pins([server_id.fingerprint()]);
    let address = listen(
        &server,
        common::server(LinkOptions::new(Trust::Secure).token("t0k")),
    );
    let with = RemoteSource::new(
        Box::new(TlsConnect::new(&address, &client).unwrap()),
        LinkOptions::new(Trust::Secure).token("t0k"),
    );
    let (registry, refused) = refresh(&with);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(greeting(&registry), "hello, tls");
    let e = source(&address, &client).link().unwrap_err();
    assert_eq!(e.status, guatiao::Status::GUATIAO_ERR_BAD_VALUE, "{e:?}");
    assert!(e.message().contains("token"), "{}", e.message());
}
