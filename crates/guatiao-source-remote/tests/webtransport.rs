// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! WebTransport (`https://`): mutual TLS, no security, a rotating
//! listener's certificates, datagrams, a configuration naming it, the
//! server binary, and a client checking the server's certificate by its
//! hash as a browser does. Server TLS runs with every other TLS transport
//! in `server_tls.rs`.

#![cfg(feature = "webtransport")]

mod common;

use core::time::Duration;
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use common::procs;
use greeter_kind::Greeter;
use guatiao::Status;
use guatiao::library::Registry;
use guatiao::value::convert::TryAsRef;
use guatiao_source_remote::transport::tcp::Accepted;
use guatiao_source_remote::transport::tls::{Identity, PeerId, TlsConfig};
use guatiao_source_remote::transport::webtransport::{ROTATE_MAX, WtConnect, WtListen};
use guatiao_source_remote::transport::{Connect, Connection};
use guatiao_source_remote::{LinkOptions, RemoteSource, Server, Trust};

const KEY: &[u8] = b"test-key";

/// Serves `server` on `listener`; its address.
fn spawn(listener: WtListen, server: Server) -> String {
    let at = listener.local_addr().unwrap().to_string();
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
    at
}

fn url(at: &str) -> String {
    format!("https://{at}/remote")
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
    let map = greeter.greet("webtransport").unwrap();
    map.get("greeting")
        .and_then(TryAsRef::<str>::try_as_ref)
        .unwrap_or_default()
        .to_string()
}

fn frame(body: &[u8]) -> Vec<u8> {
    let mut f = u32::try_from(body.len()).unwrap().to_le_bytes().to_vec();
    f.extend_from_slice(body);
    f
}

/// A listener and a client as `server` and `client` say, linked at the
/// transport: the client's first frame has run the server's handshake.
fn pair(server: &TlsConfig, client: &TlsConfig) -> (Connection, Connection) {
    let listener = WtListen::bind("127.0.0.1:0", server).unwrap();
    let client = WtConnect::new(&url(&listener.local_addr().unwrap().to_string()), client).unwrap();
    let dialing = std::thread::spawn(move || {
        let mut c = client.connect().unwrap();
        c.writer.write_frame(&frame(b"start")).unwrap();
        c
    });
    let Accepted::Connection(mut accepted) = listener.accept().unwrap() else {
        panic!("no access control list: every source proceeds");
    };
    accepted
        .reader
        .set_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut buf = Vec::new();
    assert!(accepted.reader.read_frame(&mut buf, 1 << 10).unwrap());
    assert_eq!(buf, b"start");
    accepted.reader.set_timeout(None).unwrap();
    (dialing.join().unwrap(), accepted)
}

#[test]
fn pinned_ends_link_at_mutual_tls_and_each_knows_the_other() {
    let (server_id, client_id) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    let server = TlsConfig::new(server_id.clone()).pins([client_id.fingerprint()]);
    let client = TlsConfig::new(client_id.clone()).pins([server_id.fingerprint()]);
    let listener = WtListen::bind("127.0.0.1:0", &server).unwrap();
    assert_eq!(
        listener.certificate_hashes().unwrap(),
        vec![server_id.fingerprint()],
        "not rotating: the configuration's identity alone"
    );
    let at = spawn(listener, common::server(LinkOptions::new(Trust::Secure)));
    let source = RemoteSource::new(
        Box::new(WtConnect::new(&url(&at), &client).unwrap()),
        LinkOptions::new(Trust::Secure),
    );
    let (registry, refused) = refresh(&source);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(greeting(&registry), "hello, webtransport");
    let link = source.link().unwrap();
    assert_eq!(link.trust(), Trust::Secure);
    assert_eq!(
        link.peer_id(),
        Some(server_id.fingerprint().canonical().as_str())
    );
    assert!(
        link.peer().starts_with("https 127.0.0.1:"),
        "{}",
        link.peer()
    );
    assert!(link.datagrams().is_some());
}

#[test]
fn a_client_the_server_does_not_pin_is_refused_as_a_trust_failure() {
    let (server_id, client_id) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    let server =
        TlsConfig::new(server_id.clone()).pins([Identity::generate().unwrap().fingerprint()]);
    let client = TlsConfig::new(client_id).pins([server_id.fingerprint()]);
    let at = spawn(
        WtListen::bind("127.0.0.1:0", &server).unwrap(),
        common::server(LinkOptions::new(Trust::Secure)),
    );
    let source = RemoteSource::new(
        Box::new(WtConnect::new(&url(&at), &client).unwrap()),
        LinkOptions::new(Trust::Secure),
    );
    let e = source.link().unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_BAD_VALUE, "{e:?}");
}

#[test]
fn no_security_links_by_the_key_alone_and_the_list_decides_by_address() {
    let at = spawn(
        WtListen::bind_trusted("127.0.0.1:0", common::permit(&["127.0.0.1"])).unwrap(),
        common::server(LinkOptions::new(Trust::Trusted).key(KEY)),
    );
    let source = RemoteSource::new(
        Box::new(WtConnect::trusted(&url(&at)).unwrap()),
        LinkOptions::new(Trust::Trusted).key(KEY),
    );
    let (registry, refused) = refresh(&source);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(greeting(&registry), "hello, webtransport");
    assert_eq!(
        source.link().unwrap().peer_id(),
        None,
        "no certificate proves anyone"
    );

    let wrong = RemoteSource::new(
        Box::new(WtConnect::trusted(&url(&at)).unwrap()),
        LinkOptions::new(Trust::Trusted).key(b"other"),
    );
    let e = wrong.link().unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_BAD_VALUE, "{e:?}");

    let elsewhere = spawn(
        WtListen::bind_trusted("127.0.0.1:0", common::permit(&["192.0.2.0/24"])).unwrap(),
        common::server(LinkOptions::new(Trust::Trusted).key(KEY)),
    );
    let denied = RemoteSource::new(
        Box::new(
            WtConnect::trusted(&url(&elsewhere))
                .unwrap()
                .timeout(Duration::from_secs(3)),
        ),
        LinkOptions::new(Trust::Trusted).key(KEY),
    );
    let e = denied.link().unwrap_err();
    assert_eq!(
        e.status,
        Status::GUATIAO_ERR_BAD_VALUE,
        "a source the list denies is refused, not gone: {e:?}"
    );
}

#[test]
fn a_fingerprint_the_list_denies_is_refused_after_the_handshake() {
    use guatiao_source_remote::acl::{Acl, Action, Rule};
    use guatiao_source_remote::transport::tls::Policy;
    let (server_id, client_id) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    let mut deny = Rule::new(Action::Deny, "0.0.0.0/0".parse().unwrap());
    deny.fingerprint = Some(client_id.fingerprint().canonical());
    let mut acl = Acl::default();
    acl.push(deny);
    acl.push(Rule::new(Action::Permit, "127.0.0.1/32".parse().unwrap()));
    let mut policy = Policy::strict();
    policy.acl = Some(acl);
    let server = TlsConfig::new(server_id.clone())
        .pins([client_id.fingerprint()])
        .policy(policy);
    let client = TlsConfig::new(client_id).pins([server_id.fingerprint()]);
    let at = spawn(
        WtListen::bind("127.0.0.1:0", &server).unwrap(),
        common::server(LinkOptions::new(Trust::Secure)),
    );
    let source = RemoteSource::new(
        Box::new(WtConnect::new(&url(&at), &client).unwrap()),
        LinkOptions::new(Trust::Secure),
    );
    let e = source.link().unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_BAD_VALUE, "{}", e.message());
}

/// A store keeps waiting only a stranger its access control list could
/// admit: one whose fingerprint no rule permits from its address is
/// refused as it presents its certificate, so no operator is offered it.
#[test]
fn a_stranger_the_list_will_never_admit_is_not_kept_waiting() {
    use guatiao_source_remote::acl::{Acl, Action, Rule};
    use guatiao_source_remote::transport::tls::{Policy, TrustStore};
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("webtransport")
        .join("never_admitted");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (admitted, never) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    // Every source is deferred to the fingerprint rule.
    let mut only = Rule::new(Action::Permit, "any".parse().unwrap());
    only.fingerprint = Some(admitted.fingerprint().canonical());
    let mut acl = Acl::default();
    acl.push(only);
    let mut policy = Policy::strict();
    policy.acl = Some(acl);
    let server = TlsConfig::open(&dir).unwrap().policy(policy);
    let at = spawn(
        WtListen::bind("127.0.0.1:0", &server).unwrap(),
        common::server(LinkOptions::new(Trust::Secure)),
    );
    let link = |id: Identity| {
        let client = TlsConfig::new(id).pins([server.fingerprint()]);
        RemoteSource::new(
            Box::new(WtConnect::new(&url(&at), &client).unwrap()),
            LinkOptions::new(Trust::Secure),
        )
        .link()
    };
    assert!(link(never).is_err());
    // The stranger the list names waits for an operator to accept it.
    assert!(link(admitted.clone()).is_err());
    let store = TrustStore::open(&dir).unwrap();
    common::eventually("the stranger the list names waits", || {
        !store.waiting().unwrap().is_empty()
    });
    let waiting: Vec<PeerId> = store.waiting().unwrap().into_iter().map(|w| w.id).collect();
    assert_eq!(waiting, [admitted.fingerprint()]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn at_server_tls_the_server_knows_no_client_and_the_client_knows_the_server() {
    let id = Identity::generate().unwrap();
    let server = TlsConfig::new(id.clone()).server_tls();
    let client = TlsConfig::without_identity()
        .unwrap()
        .pins([id.fingerprint()]);
    let (c, s) = pair(&server, &client);
    assert_eq!(s.reader.peer_id(), None);
    assert!(s.reader.encrypted());
    assert_eq!(c.reader.peer_id(), Some(id.fingerprint().canonical()));
}

#[test]
fn a_rotating_listener_moves_to_the_next_certificate_each_period() {
    let config = TlsConfig::new(Identity::generate().unwrap()).server_tls();
    let every = Duration::from_secs(2);
    let listener = WtListen::bind("127.0.0.1:0", &config)
        .unwrap()
        .rotating(every)
        .unwrap();
    let before = listener.certificate_hashes().unwrap();
    assert_eq!(before.len(), 2, "the one presented, and the next");
    assert_ne!(before[0], before[1]);
    std::thread::sleep(every + Duration::from_millis(300));
    let after = listener.certificate_hashes().unwrap();
    assert_eq!(after[0], before[1], "the next is presented now");
    assert_ne!(after[1], before[0]);
    assert_ne!(after[1], before[1]);

    // A client pinning what the page was told before the rotation still links.
    let at = spawn(
        listener,
        common::server(LinkOptions::new(Trust::ServerTls).key(KEY)),
    );
    let client = TlsConfig::without_identity().unwrap().pins(before.clone());
    let source = RemoteSource::new(
        Box::new(WtConnect::new(&url(&at), &client).unwrap()),
        LinkOptions::new(Trust::ServerTls).key(KEY),
    );
    let link = source.link().unwrap();
    assert_eq!(link.peer_id(), Some(before[1].canonical().as_str()));

    // One pinning only the certificate it no longer presents does not.
    let stale = TlsConfig::without_identity().unwrap().pins([before[0]]);
    let source = RemoteSource::new(
        Box::new(WtConnect::new(&url(&at), &stale).unwrap()),
        LinkOptions::new(Trust::ServerTls).key(KEY),
    );
    let e = source.link().unwrap_err();
    assert!(e.message().contains("not pinned"), "{}", e.message());
}

#[test]
fn a_period_a_browser_would_refuse_is_refused() {
    let config = TlsConfig::new(Identity::generate().unwrap()).server_tls();
    for every in [Duration::ZERO, ROTATE_MAX + Duration::from_secs(1)] {
        let e = WtListen::bind("127.0.0.1:0", &config)
            .unwrap()
            .rotating(every)
            .unwrap_err();
        assert!(e.to_string().contains("14 days"), "{e}");
    }
    WtListen::bind("127.0.0.1:0", &config)
        .unwrap()
        .rotating(ROTATE_MAX)
        .unwrap();
}

#[test]
fn a_listener_presents_the_identity_it_is_given_at_no_security() {
    let id = Identity::generate().unwrap();
    let listener = WtListen::bind_trusted("127.0.0.1:0", common::permit(&["127.0.0.1"]))
        .unwrap()
        .presenting(id.clone())
        .unwrap();
    assert_eq!(
        listener.certificate_hashes().unwrap(),
        vec![id.fingerprint()]
    );
}

#[test]
fn datagrams_cross_beside_the_frames() {
    let (server_id, client_id) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    let server = TlsConfig::new(server_id.clone()).pins([client_id.fingerprint()]);
    let client = TlsConfig::new(client_id.clone()).pins([server_id.fingerprint()]);
    let (mut c, mut s) = pair(&server, &client);
    let (cd, sd) = (c.reader.datagrams().unwrap(), s.reader.datagrams().unwrap());
    assert!(cd.max_size().unwrap() > 1000);
    cd.send(b"one").unwrap();
    assert_eq!(
        sd.recv(Some(Duration::from_secs(5))).unwrap().unwrap(),
        b"one"
    );
    sd.send(b"two").unwrap();
    assert_eq!(
        cd.recv(Some(Duration::from_secs(5))).unwrap().unwrap(),
        b"two"
    );
    assert_eq!(sd.recv(Some(Duration::from_millis(50))).unwrap(), None);
    let e = cd.send(&vec![0; 64 << 10]).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(
        s.reader.peer_id(),
        Some(client_id.fingerprint().canonical())
    );
    c.writer.write_frame(&frame(b"after")).unwrap();
    let mut buf = Vec::new();
    assert!(s.reader.read_frame(&mut buf, 1 << 10).unwrap());
    assert_eq!(buf, b"after");
}

#[test]
fn a_configuration_names_webtransport_by_its_url() {
    let id = Identity::generate().unwrap();
    let at = spawn(
        WtListen::bind("127.0.0.1:0", &TlsConfig::new(id.clone()).server_tls()).unwrap(),
        common::server(LinkOptions::new(Trust::ServerTls).key(KEY)),
    );
    let mut config = guatiao::Map::new();
    config.set("connect", url(&at).as_str());
    config.set("security", "server-tls");
    config.set("key", "test-key");
    let mut pins = guatiao::List::new();
    pins.push(id.fingerprint().canonical().as_str());
    config.set("pins", pins);
    let (registry, refused) = refresh(&RemoteSource::from_config(&config).unwrap());
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(greeting(&registry), "hello, webtransport");

    let none = spawn(
        WtListen::bind_trusted("127.0.0.1:0", common::permit(&["127.0.0.1"])).unwrap(),
        common::server(LinkOptions::new(Trust::Trusted).key(KEY)),
    );
    let mut config = guatiao::Map::new();
    config.set("connect", url(&none).as_str());
    config.set("security", "none");
    config.set("key", "test-key");
    let (registry, refused) = refresh(&RemoteSource::from_config(&config).unwrap());
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(greeting(&registry), "hello, webtransport");
}

#[test]
fn the_server_binary_serves_webtransport_and_writes_the_hashes_a_page_pins() {
    let Some(derived) = procs::library("derived_greeter") else {
        panic!("derived_greeter is not built: `cargo build -p derived_greeter` first");
    };
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("webtransport_binary");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let key = dir.join("link.key");
    std::fs::write(&key, KEY).unwrap();
    let hashes_file = dir.join("hashes.json");
    let kinds = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/greeter_kind/include/greeter_kind.kinds.json");
    let (child, lines) = procs::start(
        Command::new(env!("CARGO_BIN_EXE_guatiao-remote-server"))
            .args([
                "--listen",
                "https://127.0.0.1:0",
                "--security",
                "server-tls",
            ])
            .arg("--key-file")
            .arg(&key)
            .arg("--hashes-file")
            .arg(&hashes_file)
            .arg("--kinds")
            .arg(&kinds)
            .arg("--load")
            .arg(&derived),
    );
    struct Kill(std::process::Child);
    impl Drop for Kill {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let _child = Kill(child);
    let until = Instant::now() + procs::DEADLINE;
    let (mut at, mut said) = (None, None);
    while said.is_none() {
        let left = until.saturating_duration_since(Instant::now());
        let line = lines
            .recv_timeout(left)
            .expect("the server says what it serves");
        if let Some(rest) = line.split("serving https://").nth(1) {
            at = rest.split('/').next().map(str::to_string);
        }
        if let Some(rest) = line.split("certificate hashes ").nth(1) {
            said = Some(rest.trim().to_string());
        }
    }
    let at = at.expect("an address before the hashes");
    let said: Vec<PeerId> = said
        .unwrap()
        .split(' ')
        .map(|h| h.parse().unwrap())
        .collect();
    assert_eq!(said.len(), 2);
    let written = std::fs::read_to_string(&hashes_file).unwrap();
    let listed: Vec<String> = serde_json::from_str(&written).unwrap();
    assert_eq!(
        listed,
        said.iter().map(PeerId::canonical).collect::<Vec<_>>()
    );

    let client = TlsConfig::without_identity().unwrap().pins(said);
    let source = RemoteSource::new(
        Box::new(WtConnect::new(&url(&at), &client).unwrap()),
        LinkOptions::new(Trust::ServerTls).key(KEY),
    );
    let (registry, refused) = refresh(&source);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(greeting(&registry), "hello, webtransport");
}

/// A client checking the server's certificate as a browser does
/// (wtransport's `ServerHashVerification`: the hash, a validity of at most
/// 14 days, ECDSA P-256) reaches a rotating listener.
#[test]
fn a_client_pinning_by_hash_as_a_browser_does_reaches_a_rotating_listener() {
    let config = TlsConfig::new(Identity::generate().unwrap()).server_tls();
    let listener = WtListen::bind("127.0.0.1:0", &config)
        .unwrap()
        .rotating(ROTATE_MAX)
        .unwrap();
    let hashes = listener.certificate_hashes().unwrap();
    let at = spawn(
        listener,
        common::server(LinkOptions::new(Trust::ServerTls).key(KEY)),
    );
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let digests: Vec<wtransport::tls::Sha256Digest> = hashes
        .iter()
        .map(|h| {
            let hex = h.canonical();
            let hex = hex.strip_prefix("sha256:").unwrap();
            let bytes: Vec<u8> = (0..32)
                .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
                .collect();
            wtransport::tls::Sha256Digest::new(bytes.try_into().unwrap())
        })
        .collect();
    rt.block_on(async {
        let config = wtransport::ClientConfig::builder()
            .with_bind_default()
            .with_server_certificate_hashes(digests)
            .build();
        let endpoint = wtransport::Endpoint::client(config).unwrap();
        let conn = tokio::time::timeout(Duration::from_secs(10), endpoint.connect(url(&at)))
            .await
            .expect("in time")
            .expect("the session opens");
        let (_send, _recv) = conn.open_bi().await.unwrap().await.unwrap();
    });
}
