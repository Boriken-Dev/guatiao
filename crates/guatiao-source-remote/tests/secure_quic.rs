// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The secure level over QUIC: what `secure_tls.rs` asks of TLS over TCP,
//! asked again of `quic://` (pinning, pairing with the address a client
//! came from, a stolen certificate, the access control list, a removed
//! peer, refusals in words), and datagrams beside a link.

#![cfg(feature = "quic")]

mod common;

use alloc::sync::Arc;
use core::net::SocketAddr;
use core::time::Duration;
use std::net::UdpSocket;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::Instant;

extern crate alloc;

use common::procs;
use greeter_kind::{Counter, Greeter};
use guatiao::library::Registry;
use guatiao::value::convert::TryAsRef;
use guatiao::value::wire::channel::{self, Received, Receiver};
use guatiao_source_remote::acl::{Acl, Action, Rule};
use guatiao_source_remote::transport::Connect;
use guatiao_source_remote::transport::quic::{QuicConnect, QuicListen};
use guatiao_source_remote::transport::tcp::Accepted;
use guatiao_source_remote::transport::tls::{
    Identity, PeerId, Policy, TlsConfig, TlsConnect, TlsListen, TrustStore,
};
use guatiao_source_remote::{Link, LinkOptions, RemoteSource, Server, Trust};
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};

/// A directory of its own under the target's scratch space, emptied.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("secure_quic")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Links a server set up, kept for the test to look at.
type Served = Arc<Mutex<Vec<Arc<Link>>>>;

/// A QUIC server of the test greeter on loopback, serving every link it
/// accepts; answers its address and the links it set up.
fn serve(config: &TlsConfig, server: Server) -> (String, Served) {
    let listener = QuicListen::bind("127.0.0.1:0", config).unwrap();
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
        Box::new(QuicConnect::new(address, config).unwrap()),
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
    let map = greeter.greet("quic").unwrap();
    map.get("greeting")
        .and_then(TryAsRef::<str>::try_as_ref)
        .unwrap_or_default()
        .to_string()
}

/// A runtime for the tests' own QUIC peers, written against quinn and
/// rustls directly.
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap()
}

/// A QUIC server completing every handshake `config` lets through, then
/// holding the connection until its peer leaves.
fn raw_server(config: rustls::ServerConfig) -> String {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let address = socket.local_addr().unwrap().to_string();
    let crypto = QuicServerConfig::try_from(Arc::new(config)).unwrap();
    std::thread::spawn(move || {
        runtime().block_on(async move {
            let endpoint = quinn::Endpoint::new(
                quinn::EndpointConfig::default(),
                Some(quinn::ServerConfig::with_crypto(Arc::new(crypto))),
                socket,
                Arc::new(quinn::TokioRuntime),
            )
            .unwrap();
            while let Some(incoming) = endpoint.accept().await {
                tokio::spawn(async move {
                    if let Ok(conn) = incoming.await {
                        let _ = conn.accept_bi().await;
                        conn.closed().await;
                    }
                });
            }
        });
    });
    address
}

/// A QUIC client handshaking as `config` says with `address`, then
/// opening its stream, writing a byte and reading what comes back.
fn raw_client(config: Arc<rustls::ClientConfig>, address: &str) {
    let addr: SocketAddr = address.parse().unwrap();
    let crypto = QuicClientConfig::try_from(config).unwrap();
    runtime().block_on(async move {
        let endpoint = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).unwrap();
        let connecting = endpoint
            .connect_with(
                quinn::ClientConfig::new(Arc::new(crypto)),
                addr,
                coqui_peer_tls::SERVER_NAME,
            )
            .unwrap();
        if let Ok(conn) = connecting.await
            && let Ok((mut send, mut recv)) = conn.open_bi().await
        {
            let _ = send.write_all(b"x").await;
            let mut byte = [0u8; 1];
            let _ = tokio::time::timeout(Duration::from_secs(10), recv.read(&mut byte)).await;
        }
    });
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
    assert_eq!(greeting(&registry), "hello, quic");
    let link = source.link().unwrap();
    assert_eq!(link.trust(), Trust::Secure);
    assert_eq!(
        link.peer_id(),
        Some(server_id.fingerprint().canonical().as_str())
    );
    common::eventually("the server holds the link", || {
        !served.lock().unwrap().is_empty()
    });
    let held = served.lock().unwrap();
    assert_eq!(
        held[0].peer_id(),
        Some(client_id.fingerprint().canonical().as_str())
    );
    assert!(
        held[0].peer().starts_with("quic 127.0.0.1:"),
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
    let e = QuicConnect::new(&address, &client)
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
    // Recorded on quinn's threads, scoped to this connection's address.
    let from = waiting[0].source.expect("the address it came from");
    assert!(from.ip().is_loopback(), "{from}");

    store.accept(&client_id.fingerprint()).unwrap();
    source.disconnect();
    let (registry, refused) = refresh(&source);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(greeting(&registry), "hello, quic");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A rustls client presenting `victim`'s certificate, signing with a key
/// that is not its own.
fn impostor(victim: &Identity) -> Arc<rustls::ClientConfig> {
    use rustls::client::ResolvesClientCert;
    use rustls::pki_types::CertificateDer;
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
    let key = Identity::generate().unwrap().private_key_der();
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
    // The server refuses at the impostor's signature.
    raw_client(impostor(&victim), &address);
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
fn the_access_control_list_refuses_a_denied_source_before_the_handshake() {
    let (server_id, client_id) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    let mut policy = Policy::modern();
    policy.acl = Some(acl(vec![Rule::new(
        Action::Permit,
        "10.0.0.0/8".parse().unwrap(),
    )]));
    let server = TlsConfig::new(server_id.clone())
        .pins([client_id.fingerprint()])
        .policy(policy);
    let listener = QuicListen::bind("127.0.0.1:0", &server).unwrap();
    let client = TlsConfig::new(client_id).pins([server_id.fingerprint()]);
    let connect = QuicConnect::new(&listener.local_addr().unwrap().to_string(), &client).unwrap();
    let dialing = std::thread::spawn(move || connect.connect());
    assert!(matches!(listener.accept().unwrap(), Accepted::Refused(_)));
    let e = dialing.join().unwrap().unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::ConnectionRefused, "{e}");
}

#[test]
fn a_fingerprint_rule_refuses_a_trusted_peer_once_the_handshake_names_it() {
    let (server_id, allowed, denied) = (
        Identity::generate().unwrap(),
        Identity::generate().unwrap(),
        Identity::generate().unwrap(),
    );
    let mut deny = Rule::new(Action::Deny, "any".parse().unwrap());
    deny.fingerprint = Some(denied.fingerprint().to_base32_grouped());
    let mut policy = Policy::modern();
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
    assert_eq!(greeting(&registry), "hello, quic");

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
    let mut only = Rule::new(Action::Permit, "any".parse().unwrap());
    only.fingerprint = Some(one.fingerprint().canonical());
    let mut policy = Policy::modern();
    policy.acl = Some(acl(vec![only]));
    let server = TlsConfig::new(server_id.clone())
        .pins([one.fingerprint(), other.fingerprint()])
        .policy(policy);
    let address = listen(&server, secure_server());
    let yes = TlsConfig::new(one).pins([server_id.fingerprint()]);
    let (registry, refused) = refresh(&source(&address, &yes));
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(greeting(&registry), "hello, quic");
    let no = TlsConfig::new(other).pins([server_id.fingerprint()]);
    let (registry, refused) = refresh(&source(&address, &no));
    assert_eq!(registry.offers::<dyn Greeter>().count(), 0, "{refused:?}");
}

/// A store keeps waiting only a stranger its access control list could
/// admit: one whose fingerprint no rule permits from its address is
/// refused as it presents its certificate, so no operator is offered it.
#[test]
fn a_stranger_the_list_will_never_admit_is_not_kept_waiting() {
    let dir = scratch("never_admitted");
    let (admitted, never) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    // Every source is deferred to the fingerprint rule.
    let mut only = Rule::new(Action::Permit, "any".parse().unwrap());
    only.fingerprint = Some(admitted.fingerprint().canonical());
    let mut policy = Policy::modern();
    policy.acl = Some(acl(vec![only]));
    let server = TlsConfig::open(&dir).unwrap().policy(policy);
    let address = listen(&server, secure_server());

    let no = TlsConfig::new(never).pins([server.fingerprint()]);
    let (_, refused) = refresh(&source(&address, &no));
    assert_eq!(refused.len(), 1, "{refused:?}");
    // The stranger the list names waits for an operator to accept it.
    let yes = TlsConfig::new(admitted.clone()).pins([server.fingerprint()]);
    let (_, refused) = refresh(&source(&address, &yes));
    assert_eq!(refused.len(), 1, "{refused:?}");
    let store = TrustStore::open(&dir).unwrap();
    common::eventually("the stranger the list names waits", || {
        !store.waiting().unwrap().is_empty()
    });
    let waiting: Vec<PeerId> = store.waiting().unwrap().into_iter().map(|w| w.id).collect();
    assert_eq!(waiting, [admitted.fingerprint()]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_client_has_no_access_control_list() {
    let id = Identity::generate().unwrap();
    let mut policy = Policy::modern();
    policy.acl = Some(acl(Vec::new()));
    let client = TlsConfig::new(id.clone())
        .pins([id.fingerprint()])
        .policy(policy);
    let e = QuicConnect::new("127.0.0.1:9", &client).unwrap_err();
    assert!(e.to_string().contains("a client has none"), "{e}");
}

/// A QUIC server that pins the client but names no application protocol
/// is not a guatiao server.
#[test]
fn a_server_agreeing_no_application_protocol_is_refused() {
    let (server_id, client_id) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    let trust =
        coqui_peer_tls::Trust::Pinned(coqui_peer_tls::Pins::fixed([client_id.fingerprint()]));
    let address = raw_server(coqui_peer_tls::server_config(&server_id, &trust, &[]).unwrap());
    let client = TlsConfig::new(client_id).pins([server_id.fingerprint()]);
    let e = QuicConnect::new(&address, &client)
        .unwrap()
        .connect()
        .unwrap_err();
    assert!(e.to_string().contains("application protocol"), "{e}");
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
    assert_eq!(greeting(&registry), "hello, quic");
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
    let e = source.link().unwrap_err();
    assert_eq!(e.status, guatiao::Status::GUATIAO_ERR_BAD_VALUE, "{e:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The control for the test above: resumption kept by the policy lets a
/// removed peer back in on the session it holds.
#[test]
fn resumption_kept_by_the_policy_lets_a_removed_peer_link_again() {
    let dir = scratch("resumed");
    let mut policy = Policy::modern();
    policy.allow_resumption = true;
    let server = TlsConfig::open(&dir).unwrap().policy(policy.clone());
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
    assert_eq!(greeting(&registry), "hello, quic");
    drop(registry);
    source.disconnect();

    assert!(store.remove(&client_id.fingerprint()).unwrap());
    let (registry, refused) = refresh(&source);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(greeting(&registry), "hello, quic");
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
    (
        rustls::pki_types::CertificateDer::from(id.certificate_der().to_vec()),
        id.private_key_der(),
    )
}

/// A QUIC server presenting `id`'s certificate with a chain behind it,
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
    raw_server(config)
}

/// What `client` is told connecting to `address`: the status and the words.
fn refused_link(address: &str, client: &TlsConfig) -> (guatiao::Status, String) {
    let e = source(address, client).link().unwrap_err();
    (e.status, e.message().to_string())
}

/// A server presenting `victim`'s certificate without its key: what a
/// client pinning the victim refuses at the handshake's signature.
#[test]
fn a_server_holding_a_pinned_certificate_without_its_key_is_refused() {
    use rustls::server::{ClientHello, ResolvesServerCert};
    use rustls::sign::CertifiedKey;

    #[derive(Debug)]
    struct Impostor(Arc<CertifiedKey>);
    impl ResolvesServerCert for Impostor {
        fn resolve(&self, _: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
            Some(Arc::clone(&self.0))
        }
    }
    let victim = Identity::generate().unwrap();
    let (certificate, _) = certified(&victim);
    let (_, key) = certified(&Identity::generate().unwrap());
    let signing = rustls::crypto::ring::sign::any_supported_type(&key).unwrap();
    let verifier = coqui_peer_tls::Trust::AnyWithKeyProof
        .client_cert_verifier()
        .unwrap();
    let mut config = rustls::ServerConfig::builder_with_provider(coqui_peer_tls::crypto_provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_client_cert_verifier(verifier)
        .with_cert_resolver(Arc::new(Impostor(Arc::new(CertifiedKey::new(
            vec![certificate],
            signing,
        )))));
    config.alpn_protocols = vec![b"guatiao-remote".to_vec()];
    let address = raw_server(config);
    let client = TlsConfig::new(Identity::generate().unwrap()).pins([victim.fingerprint()]);
    // A refusal, not a peer gone.
    let (status, why) = refused_link(&address, &client);
    assert_eq!(status, guatiao::Status::GUATIAO_ERR_BAD_VALUE, "{why}");
    assert!(
        why.contains("presented a certificate this client refuses"),
        "{why}"
    );
}

#[test]
fn a_client_says_why_it_refuses_a_server() {
    let server_id = Identity::generate().unwrap();
    let client_id = Identity::generate().unwrap();
    let server = TlsConfig::new(server_id.clone()).pins([client_id.fingerprint()]);
    let address = listen(&server, secure_server());

    let stranger = Identity::generate().unwrap().fingerprint();
    let pinned = TlsConfig::new(client_id.clone()).pins([stranger]);
    let (status, why) = refused_link(&address, &pinned);
    assert_eq!(status, guatiao::Status::GUATIAO_ERR_BAD_VALUE, "{why}");
    assert!(why.contains("is not one this client trusts"), "{why}");
    assert!(why.contains("is not pinned"), "{why}");

    let dir = scratch("client_store");
    let stored = TlsConfig::open(&dir).unwrap();
    let server = TlsConfig::new(server_id.clone()).pins([stored.fingerprint()]);
    let address = listen(&server, secure_server());
    let (status, why) = refused_link(&address, &stored);
    assert_eq!(status, guatiao::Status::GUATIAO_ERR_BAD_VALUE, "{why}");
    assert!(why.contains("is not accepted"), "{why}");

    let address = chain_server(&server_id);
    let client = TlsConfig::new(client_id).pins([server_id.fingerprint()]);
    let (status, why) = refused_link(&address, &client);
    assert_eq!(status, guatiao::Status::GUATIAO_ERR_BAD_VALUE, "{why}");
    assert!(why.contains("presented a certificate chain"), "{why}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A listener serving one link: its address, and what serving it answered.
fn serve_once(config: &TlsConfig) -> (String, std::thread::JoinHandle<String>) {
    let listener = QuicListen::bind("127.0.0.1:0", config).unwrap();
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

    let server = TlsConfig::new(server_id.clone()).pins([known.fingerprint()]);
    let (address, serving) = serve_once(&server);
    let client = TlsConfig::new(stranger.clone()).pins([server_id.fingerprint()]);
    let _ = refresh(&source(&address, &client));
    let why = serving.join().unwrap();
    assert!(why.contains("the client is not pinned"), "{why}");

    let dir = scratch("server_refuses");
    let server = TlsConfig::open(&dir).unwrap();
    let (address, serving) = serve_once(&server);
    let client = TlsConfig::new(stranger.clone()).pins([server.fingerprint()]);
    let _ = refresh(&source(&address, &client));
    let why = serving.join().unwrap();
    assert!(why.contains("the client is not accepted"), "{why}");
    assert!(why.contains("waits there"), "{why}");

    let server = TlsConfig::new(server_id).pins([known.fingerprint()]);
    let (address, serving) = serve_once(&server);
    raw_client(chain_client(&known), &address);
    let why = serving.join().unwrap();
    assert!(why.contains("presented a certificate chain"), "{why}");

    // A list refusing it by its fingerprint, from any address.
    let mut deny = Rule::new(Action::Deny, "any".parse().unwrap());
    deny.fingerprint = Some(stranger.fingerprint().canonical());
    let mut policy = Policy::modern();
    policy.acl = Some(Acl {
        rules: vec![deny],
        default: Action::Permit,
    });
    let server = TlsConfig::new(Identity::generate().unwrap())
        .pins([stranger.fingerprint()])
        .policy(policy);
    let (address, serving) = serve_once(&server);
    let client = TlsConfig::new(stranger).pins([server.fingerprint()]);
    let _ = refresh(&source(&address, &client));
    let why = serving.join().unwrap();
    assert!(
        why.contains("the access control list denies the client (rule 10)"),
        "{why}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A QUIC server of coqui-peer-tls's own whose list refuses this client's
/// address at its hello answers `access_denied`: the client says the
/// server's list refuses its address.
#[test]
fn a_client_refused_by_address_at_its_hello_says_so() {
    let (server_id, client_id) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    let mut policy = Policy::modern();
    policy.acl = Some(acl(vec![Rule::new(
        Action::Permit,
        "10.0.0.0/8".parse().unwrap(),
    )]));
    let trust =
        coqui_peer_tls::Trust::Pinned(coqui_peer_tls::Pins::fixed([client_id.fingerprint()]));
    let at: SocketAddr = "127.0.0.1:1".parse().unwrap();
    let config =
        coqui_peer_tls::ServerConfigs::new(&server_id, &trust, &[b"guatiao-remote"], &policy)
            .unwrap()
            .for_connection(at, at)
            .unwrap()
            .config();
    let address = raw_server(Arc::unwrap_or_clone(config));
    let client = TlsConfig::new(client_id).pins([server_id.fingerprint()]);
    let e = QuicConnect::new(&address, &client)
        .unwrap()
        .connect()
        .unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied, "{e}");
    assert!(e.to_string().contains("access_denied"), "{e}");
    assert!(
        e.to_string()
            .contains("its access control list does not admit this client's address"),
        "{e}"
    );
}

/// A refused client is told so in words naming its fingerprint, whether
/// the refusal reaches it during its handshake or at its first read.
#[test]
fn a_refused_client_hears_its_certificate_was_refused() {
    let dir = scratch("alert");
    let server = TlsConfig::open(&dir).unwrap();
    let address = listen(&server, secure_server());
    let client_id = Identity::generate().unwrap();
    let client = TlsConfig::new(client_id.clone()).pins([server.fingerprint()]);
    let e = match QuicConnect::new(&address, &client).unwrap().connect() {
        Err(e) => e,
        Ok(mut conn) => {
            conn.reader
                .set_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut buf = Vec::new();
            conn.reader.read_frame(&mut buf, 1 << 10).unwrap_err()
        }
    };
    assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied, "{e}");
    let why = e.to_string();
    assert!(why.contains("certificate_unknown"), "{why}");
    assert!(why.contains("does not trust this client"), "{why}");
    assert!(why.contains(&client_id.fingerprint().canonical()), "{why}");
    let _ = std::fs::remove_dir_all(&dir);
}

fn linked_pair() -> (RemoteSource, Registry, Served) {
    let (server_id, client_id) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    let server = TlsConfig::new(server_id.clone()).pins([client_id.fingerprint()]);
    let client = TlsConfig::new(client_id).pins([server_id.fingerprint()]);
    let (address, served) = serve(&server, secure_server());
    let source = source(&address, &client);
    let (registry, refused) = refresh(&source);
    assert!(refused.is_empty(), "{refused:?}");
    (source, registry, served)
}

#[test]
fn arguments_far_past_a_packet_cross_both_ways_at_once() {
    let (_source, registry, _) = linked_pair();
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

/// Small calls while large ones stream wait less than a large call takes:
/// at most behind a frame already on the link's one stream, never behind a
/// reader held up by a writer. Both are timed in the same run, so a loaded
/// machine slows them together.
#[test]
fn small_calls_stay_quick_while_large_ones_stream() {
    let (_source, registry, _) = linked_pair();
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
                let mut took = Vec::new();
                while !stop.load(core::sync::atomic::Ordering::Relaxed) {
                    let t = Instant::now();
                    greeter.greet(&name).unwrap();
                    took.push(t.elapsed());
                }
                took
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
    let mut large: Vec<Duration> = streaming
        .into_iter()
        .flat_map(|t| t.join().unwrap())
        .collect();
    assert!(!large.is_empty());
    let n = large.len();
    let (idle, busy, large) = (median(&mut idle), median(&mut busy), median(&mut large));
    println!(
        "small call p50: {idle:?} alone, {busy:?} beside {n} calls of 4 MiB taking {large:?} each"
    );
    assert!(
        busy < large,
        "small calls waited {busy:?} beside large ones taking {large:?} ({idle:?} alone)"
    );
}

/// A wire channel's schema and values in datagrams beside a link's calls,
/// both ways; the frames are the channel's, readable from C and every
/// binding.
#[test]
fn datagrams_carry_a_wire_channels_values_beside_the_calls() {
    use guatiao::schema::{FieldBuilder, KindBuilder, SchemaBuilder};
    let (source, registry, served) = linked_pair();
    assert_eq!(greeting(&registry), "hello, quic");
    let client = source
        .link()
        .unwrap()
        .datagrams()
        .expect("QUIC has datagrams");
    common::eventually("the server holds the link", || {
        !served.lock().unwrap().is_empty()
    });
    let server = served.lock().unwrap()[0]
        .datagrams()
        .expect("QUIC has datagrams");

    let schema: guatiao::Value = SchemaBuilder::new()
        .field(FieldBuilder::new("celsius", KindBuilder::int()))
        .finish()
        .unwrap();
    let announce = channel::announce(3, &schema).unwrap();
    let mut reading = guatiao::Map::new();
    reading.set("celsius", 21i64);
    let sample = channel::send(3, &reading.into()).unwrap();

    for (from, to) in [(&client, &server), (&server, &client)] {
        let mut receiver = Receiver::new();
        // A datagram may be lost: send until each arrives.
        let mut announced = false;
        let deadline = Instant::now() + Duration::from_secs(10);
        while !announced && Instant::now() < deadline {
            from.send(&announce).unwrap();
            if let Some(got) = to.recv(Some(Duration::from_millis(200))).unwrap() {
                announced = matches!(receiver.accept(&got).unwrap(), Received::Schema(3));
            }
        }
        assert!(announced, "the schema crossed");
        let mut valued = false;
        while !valued && Instant::now() < deadline {
            from.send(&sample).unwrap();
            if let Some(got) = to.recv(Some(Duration::from_millis(200))).unwrap() {
                valued = matches!(
                    receiver.accept(&got).unwrap(),
                    Received::Value { channel: 3, .. }
                );
            }
        }
        assert!(valued, "a value crossed");
    }
}

#[test]
fn a_link_over_tls_has_no_datagrams() {
    let (server_id, client_id) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    let server = TlsConfig::new(server_id.clone()).pins([client_id.fingerprint()]);
    let client = TlsConfig::new(client_id).pins([server_id.fingerprint()]);
    let listener = TlsListen::bind("127.0.0.1:0", &server).unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let serving = secure_server();
    std::thread::spawn(move || {
        if let Ok(Accepted::Connection(conn)) = listener.accept() {
            let _ = serving.serve(conn);
        }
    });
    let source = RemoteSource::new(
        Box::new(TlsConnect::new(&address, &client).unwrap()),
        LinkOptions::new(Trust::Secure),
    );
    assert!(source.link().unwrap().datagrams().is_none());
}

fn server_program() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_guatiao-remote-server"))
}

fn kinds_json() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/greeter_kind/include/greeter_kind.kinds.json")
}

/// QUIC between this process and a `guatiao-remote-server` child: the
/// first link waits, the operator accepts the client with the server's
/// own command, the next link calls the greeter it loaded.
#[test]
fn pairing_across_two_processes_through_the_servers_commands() {
    let Some(derived) = procs::library("derived_greeter") else {
        panic!("derived_greeter is not built: `cargo build -p derived_greeter` first");
    };
    let dir = scratch("two_processes");
    let (server_dir, client_dir) = (dir.join("server"), dir.join("client"));
    let (mut child, lines) = procs::start(
        Command::new(server_program())
            .args([
                "--listen",
                "quic://127.0.0.1:0",
                "--security",
                "mutual-tls",
                "--state-dir",
            ])
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
        .split("serving quic://")
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
    drop(greeter);
    drop(registry);
    source.disconnect();
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
}

/// `quic://` from a source file's configuration, as a host in any
/// language hands it over.
#[test]
fn a_configuration_map_reaches_the_secure_level_over_quic() {
    let dir = scratch("config");
    let server = TlsConfig::open(&dir.join("server")).unwrap();
    let client = TlsConfig::open(&dir.join("client")).unwrap();
    TrustStore::open(&dir.join("server"))
        .unwrap()
        .accept_key(&coqui_peer_tls::certificate_key(client.identity().certificate_der()).unwrap())
        .unwrap();
    let address = listen(&server, secure_server());

    let mut config = guatiao::Map::new();
    config.set("connect", format!("quic://{address}").as_str());
    config.set(
        "state_dir",
        dir.join("client").display().to_string().as_str(),
    );
    let mut pins = guatiao::List::new();
    pins.push(server.fingerprint().to_base32_grouped().as_str());
    config.set("pins", pins);
    let source = RemoteSource::from_config(&config).unwrap();
    let (registry, refused) = refresh(&source);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(greeting(&registry), "hello, quic");

    // Server TLS against this mutual-TLS server: refused there.
    config.set("security", "server-tls");
    config.set("key", "k");
    let (registry, refused) = refresh(&RemoteSource::from_config(&config).unwrap());
    assert_eq!(registry.offers::<dyn Greeter>().count(), 0);
    assert!(!refused.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A peer manifest names a peer over QUIC.
#[test]
fn a_peer_manifest_names_a_peer_over_quic() {
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
            "type = \"remote\"\nname = \"lab\"\nconnect = \"quic://{address}\"\nsecurity = \"mutual-tls\"\n\
             identity_file = \"lab.identity.pem\"\npins = [\"{}\"]\n",
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
    assert_eq!(greeting(&registry), "hello, quic");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A QUIC server at no security on loopback, admitting `permit`: its
/// address.
fn serve_trusted(permit: &str, key: &[u8]) -> String {
    let listener = QuicListen::bind_trusted(
        "127.0.0.1:0",
        acl(vec![Rule::new(Action::Permit, permit.parse().unwrap())]),
    )
    .unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let server = common::server(LinkOptions::new(Trust::Trusted).key(key));
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

#[test]
fn at_no_security_quic_links_by_the_key_and_proves_no_peer() {
    let address = serve_trusted("127.0.0.1", b"k");
    let keyed = RemoteSource::new(
        Box::new(QuicConnect::trusted(&address).unwrap()),
        LinkOptions::new(Trust::Trusted).key(b"k"),
    );
    let (registry, refused) = refresh(&keyed);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(greeting(&registry), "hello, quic");
    let link = keyed.link().unwrap();
    assert_eq!(link.trust(), Trust::Trusted);
    assert_eq!(
        link.peer_id(),
        None,
        "a certificate nobody checked proves nothing"
    );
    assert!(link.datagrams().is_some());

    let wrong = RemoteSource::new(
        Box::new(QuicConnect::trusted(&address).unwrap()),
        LinkOptions::new(Trust::Trusted).key(b"other"),
    );
    let (registry, refused) = refresh(&wrong);
    assert_eq!(registry.offers::<dyn Greeter>().count(), 0);
    assert!(
        refused
            .iter()
            .any(|r| r.contains("does not hold this link's key")),
        "{refused:?}"
    );

    // A client pinning some server is refused: no certificate it pins.
    let client = TlsConfig::new(Identity::generate().unwrap())
        .pins([Identity::generate().unwrap().fingerprint()]);
    let (_, refused) = refresh(&source(&address, &client));
    assert!(!refused.is_empty());
}

#[test]
fn at_no_security_a_denied_source_is_refused_before_the_handshake() {
    let address = serve_trusted("10.0.0.0/8", b"k");
    let connect = QuicConnect::trusted(&address)
        .unwrap()
        .timeout(Duration::from_secs(3));
    let e = connect.connect().unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::ConnectionRefused, "{e}");
}

#[test]
fn a_configuration_names_quic_at_no_security() {
    let address = serve_trusted("127.0.0.1", b"k");
    let mut config = guatiao::Map::new();
    config.set("connect", format!("quic://{address}").as_str());
    config.set("security", "none");
    config.set("key", "k");
    let (registry, refused) = refresh(&RemoteSource::from_config(&config).unwrap());
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(greeting(&registry), "hello, quic");
}

#[test]
fn ending_the_links_ends_one_still_in_its_quic_handshake() {
    let (server_id, client_id) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    let server_config = TlsConfig::new(server_id.clone()).pins([client_id.fingerprint()]);
    let client_config = TlsConfig::new(client_id).pins([server_id.fingerprint()]);
    let server = secure_server();
    let listener = QuicListen::bind("127.0.0.1:0", &server_config).unwrap();
    let connect =
        QuicConnect::new(&listener.local_addr().unwrap().to_string(), &client_config).unwrap();
    // Connected, and never opens its stream: the server's handshake waits.
    let dialing = std::thread::spawn(move || {
        let conn = connect.connect().unwrap();
        std::thread::sleep(Duration::from_secs(4));
        drop(conn);
    });
    let Accepted::Connection(conn) = listener.accept().unwrap() else {
        panic!("every source proceeds");
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let serving = server.clone();
    std::thread::spawn(move || {
        let started = Instant::now();
        let failed = serving.serve(conn).is_err();
        let _ = tx.send((failed, started.elapsed()));
    });
    std::thread::sleep(Duration::from_millis(300));
    server.end_links();
    let (failed, took) = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the handshake ended");
    assert!(failed);
    assert!(took < Duration::from_secs(2), "{took:?}");
    dialing.join().unwrap();
}

#[test]
fn localhost_links_whichever_of_its_addresses_the_server_listens_on() {
    // On 127.0.0.1 alone: `localhost` names ::1 as well, often first, where
    // QUIC hears nothing back.
    let address = serve_trusted("127.0.0.1", b"k");
    let port = address.rsplit_once(':').unwrap().1;
    let started = Instant::now();
    let connect = QuicConnect::trusted(&format!("localhost:{port}")).unwrap();
    let linked = connect.connect().map(drop);
    assert!(linked.is_ok(), "{linked:?}");
    let took = started.elapsed();
    assert!(took < Duration::from_secs(3), "{took:?}");
}
