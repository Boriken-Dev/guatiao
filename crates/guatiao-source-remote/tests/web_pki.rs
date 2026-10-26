// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A client verifying its server by the web PKI (`roots`) on every
//! transport that runs TLS: a certificate a test authority issued for
//! `localhost`, trusted under that name, refused under another, refused
//! when another authority issued it or it has expired; pins beside the
//! roots; the configuration's `roots` key.

#![cfg(feature = "tls")]

mod common;

use core::net::SocketAddr;
use std::net::ToSocketAddrs;
use std::path::{Path, PathBuf};

use greeter_kind::Greeter;
use guatiao::library::Registry;
use guatiao::value::convert::TryAsRef;
use guatiao::{List, Map, Status};
use guatiao_source_remote::transport::Connect;
use guatiao_source_remote::transport::tcp::Accepted;
use guatiao_source_remote::transport::tls::{Identity, Roots, TlsConfig, TlsConnect, TlsListen};
use guatiao_source_remote::{LinkOptions, RemoteSource, Server, Trust};
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};

const KEY: &[u8] = b"test-key";

/// Which transport a test runs over.
#[derive(Clone, Copy, Debug)]
enum Over {
    Tcp,
    #[cfg(feature = "quic")]
    Quic,
    #[cfg(feature = "ws")]
    Wss,
    #[cfg(feature = "webtransport")]
    Wt,
}

fn every() -> Vec<Over> {
    #[allow(unused_mut)]
    let mut all = vec![Over::Tcp];
    #[cfg(feature = "quic")]
    all.push(Over::Quic);
    #[cfg(feature = "ws")]
    all.push(Over::Wss);
    #[cfg(feature = "webtransport")]
    all.push(Over::Wt);
    all
}

/// A directory of its own under the target's scratch space, emptied.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("web_pki")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A certificate authority of the tests' own.
struct Authority {
    issuer: Issuer<'static, KeyPair>,
    pem: String,
}

impl Authority {
    fn new(name: &str) -> Authority {
        let key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.distinguished_name.push(DnType::CommonName, name);
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let pem = params.self_signed(&key).unwrap().pem();
        Authority {
            issuer: Issuer::new(params, key),
            pem,
        }
    }

    /// Its certificate as a CA bundle file in `dir`.
    fn bundle(&self, dir: &Path) -> PathBuf {
        let path = dir.join("roots.pem");
        std::fs::write(&path, &self.pem).unwrap();
        path
    }

    /// A server identity it issued for `names`, valid now or, when
    /// `expired`, only in 2020.
    fn issue(&self, names: &[&str], expired: bool) -> Identity {
        let key = KeyPair::generate().unwrap();
        let names: Vec<String> = names.iter().map(|n| n.to_string()).collect();
        let mut params = CertificateParams::new(names).unwrap();
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        if expired {
            params.not_before = rcgen::date_time_ymd(2020, 1, 1);
            params.not_after = rcgen::date_time_ymd(2020, 6, 1);
        }
        let cert = params.signed_by(&key, &self.issuer).unwrap();
        Identity::from_pem_chain(&format!("{}{}", cert.pem(), key.serialize_pem())).unwrap()
    }
}

/// Where `localhost` resolves first: the address a server answering to
/// that name listens at, whichever family the resolver puts first.
fn localhost() -> SocketAddr {
    ("localhost", 0).to_socket_addrs().unwrap().next().unwrap()
}

/// Serves the test greeter over `over` at server TLS presenting
/// `identity`, at `localhost`'s first address; its port.
fn listen(over: Over, identity: &Identity) -> u16 {
    let config = TlsConfig::new(identity.clone()).server_tls();
    let server = common::server(LinkOptions::new(Trust::ServerTls).key(KEY));
    let at = localhost().to_string();
    let (port, accept): (u16, Box<dyn Fn() -> std::io::Result<Accepted> + Send>) = match over {
        Over::Tcp => {
            let l = TlsListen::bind(&at, &config).unwrap();
            (l.local_addr().unwrap().port(), Box::new(move || l.accept()))
        }
        #[cfg(feature = "quic")]
        Over::Quic => {
            let l = guatiao_source_remote::transport::quic::QuicListen::bind(&at, &config).unwrap();
            (l.local_addr().unwrap().port(), Box::new(move || l.accept()))
        }
        #[cfg(feature = "ws")]
        Over::Wss => {
            let l = guatiao_source_remote::transport::ws::WsListen::bind_tls(&at, &config).unwrap();
            (l.local_addr().unwrap().port(), Box::new(move || l.accept()))
        }
        #[cfg(feature = "webtransport")]
        Over::Wt => {
            let l = guatiao_source_remote::transport::webtransport::WtListen::bind(&at, &config)
                .unwrap();
            (l.local_addr().unwrap().port(), Box::new(move || l.accept()))
        }
    };
    serve(accept, server);
    port
}

fn serve(accept: Box<dyn Fn() -> std::io::Result<Accepted> + Send>, server: Server) {
    std::thread::spawn(move || {
        while let Ok(accepted) = accept() {
            if let Accepted::Connection(conn) = accepted {
                let server = server.clone();
                std::thread::spawn(move || {
                    let _ = server.serve(conn);
                });
            }
        }
    });
}

/// The `connect` URL of `host:port` over `over`.
fn url(over: Over, host: &str, port: u16) -> String {
    match over {
        Over::Tcp => format!("tls://{host}:{port}"),
        #[cfg(feature = "quic")]
        Over::Quic => format!("quic://{host}:{port}"),
        #[cfg(feature = "ws")]
        Over::Wss => format!("wss://{host}:{port}/"),
        #[cfg(feature = "webtransport")]
        Over::Wt => format!("https://{host}:{port}/"),
    }
}

/// Connects to `host:port` over `over` as `config` says.
fn connect(over: Over, host: &str, port: u16, config: &TlsConfig) -> Box<dyn Connect> {
    let address = format!("{host}:{port}");
    match over {
        Over::Tcp => Box::new(TlsConnect::new(&address, config).unwrap()),
        #[cfg(feature = "quic")]
        Over::Quic => Box::new(
            guatiao_source_remote::transport::quic::QuicConnect::new(&address, config).unwrap(),
        ),
        #[cfg(feature = "ws")]
        Over::Wss => Box::new(
            guatiao_source_remote::transport::ws::WsConnect::new(&url(over, host, port))
                .tls(config)
                .unwrap(),
        ),
        #[cfg(feature = "webtransport")]
        Over::Wt => Box::new(
            guatiao_source_remote::transport::webtransport::WtConnect::new(
                &url(over, host, port),
                config,
            )
            .unwrap(),
        ),
    }
}

fn source(over: Over, host: &str, port: u16, config: &TlsConfig) -> RemoteSource {
    RemoteSource::new(
        connect(over, host, port, config),
        LinkOptions::new(Trust::ServerTls).key(KEY),
    )
}

fn greeting(source: &RemoteSource) -> Result<String, Vec<String>> {
    let mut registry = Registry::new("host", "1.0");
    registry.add_source("remote", source.clone()).unwrap();
    let refreshed = registry.refresh();
    if !refreshed.refused.is_empty() {
        return Err(refreshed.refused.iter().map(|r| r.reason.clone()).collect());
    }
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .expect("a greeter")
        .shared()
        .unwrap();
    let map = greeter.greet("roots").unwrap();
    Ok(map
        .get("greeting")
        .and_then(TryAsRef::<str>::try_as_ref)
        .unwrap_or_default()
        .to_string())
}

/// A client with no identity of its own trusting `bundle`'s authorities.
fn by_roots(bundle: &Path) -> TlsConfig {
    TlsConfig::without_identity()
        .unwrap()
        .roots(Roots::bundle_file(bundle))
}

/// `localhost`'s first address, a name the test certificates do not
/// hold: as a host to dial (an IPv6 one in brackets), and as named.
fn address_host() -> (String, String) {
    let ip = localhost().ip();
    let named = ip.to_string();
    let host = if ip.is_ipv6() {
        format!("[{named}]")
    } else {
        named.clone()
    };
    (host, named)
}

#[test]
fn a_server_an_authority_issued_for_its_name_is_trusted_under_that_name() {
    let dir = scratch("trusted");
    let ca = Authority::new("guatiao test authority");
    let client = by_roots(&ca.bundle(&dir));
    let server = ca.issue(&["localhost"], false);
    for over in every() {
        let port = listen(over, &server);
        let source = source(over, "localhost", port, &client);
        assert_eq!(
            greeting(&source).unwrap_or_else(|e| panic!("{over:?}: {e:?}")),
            "hello, roots",
            "{over:?}"
        );
        let link = source.link().unwrap();
        assert_eq!(
            link.peer_id(),
            Some(server.fingerprint().canonical().as_str()),
            "{over:?}: the client knows the server by its certificate"
        );
    }
}

#[test]
fn a_server_dialled_by_a_name_its_certificate_lacks_is_refused_naming_the_name() {
    let dir = scratch("wrong_name");
    let ca = Authority::new("guatiao test authority");
    let client = by_roots(&ca.bundle(&dir));
    let server = ca.issue(&["localhost"], false);
    let (host, named) = address_host();
    for over in every() {
        let port = listen(over, &server);
        let e = source(over, &host, port, &client).link().unwrap_err();
        assert_eq!(e.status, Status::GUATIAO_ERR_BAD_VALUE, "{over:?}: {e:?}");
        assert!(
            e.message()
                .contains(&format!("not for `{named}`, the name dialled")),
            "{over:?}: {}",
            e.message()
        );
    }
}

#[test]
fn a_server_another_authority_issued_for_its_name_is_refused() {
    let dir = scratch("other_authority");
    let ca = Authority::new("guatiao test authority");
    let client = by_roots(&ca.bundle(&dir));
    let stranger = Authority::new("another authority").issue(&["localhost"], false);
    for over in every() {
        let port = listen(over, &stranger);
        let e = source(over, "localhost", port, &client).link().unwrap_err();
        assert_eq!(e.status, Status::GUATIAO_ERR_BAD_VALUE, "{over:?}: {e:?}");
        assert!(
            e.message()
                .contains("no authority the roots (`roots`) hold issued it"),
            "{over:?}: {}",
            e.message()
        );
    }
}

#[test]
fn an_expired_certificate_is_refused_by_the_roots() {
    let dir = scratch("expired");
    let ca = Authority::new("guatiao test authority");
    let client = by_roots(&ca.bundle(&dir));
    let server = ca.issue(&["localhost"], true);
    for over in every() {
        let port = listen(over, &server);
        let e = source(over, "localhost", port, &client).link().unwrap_err();
        assert_eq!(e.status, Status::GUATIAO_ERR_BAD_VALUE, "{over:?}: {e:?}");
        assert!(
            e.message().to_lowercase().contains("expired"),
            "{over:?}: {}",
            e.message()
        );
    }
}

#[test]
fn a_pinned_certificate_is_trusted_under_any_name_beside_the_roots() {
    let dir = scratch("pinned_or_roots");
    let ca = Authority::new("guatiao test authority");
    let bundle = ca.bundle(&dir);
    // A self-issued certificate no authority vouches for, named
    // `coqui-peer.invalid`: the pin alone takes it.
    let pinned = Identity::generate().unwrap();
    let issued = ca.issue(&["localhost"], false);
    let client = by_roots(&bundle).pins([pinned.fingerprint()]);
    for over in every() {
        let port = listen(over, &pinned);
        assert_eq!(
            greeting(&source(over, "localhost", port, &client))
                .unwrap_or_else(|e| panic!("{over:?}: pinned: {e:?}")),
            "hello, roots"
        );
        let port = listen(over, &issued);
        assert_eq!(
            greeting(&source(over, "localhost", port, &client))
                .unwrap_or_else(|e| panic!("{over:?}: issued: {e:?}")),
            "hello, roots"
        );
        let e = source(over, &address_host().0, port, &client)
            .link()
            .unwrap_err();
        assert!(
            e.message().contains("the name dialled"),
            "{over:?}: an unpinned certificate is still checked for its name: {}",
            e.message()
        );
    }
}

#[test]
fn a_listener_given_roots_alone_names_whom_it_lacks() {
    let dir = scratch("listener");
    let ca = Authority::new("guatiao test authority");
    let config =
        TlsConfig::new(Identity::generate().unwrap()).roots(Roots::bundle_file(ca.bundle(&dir)));
    let lacks = |e: std::io::Error| {
        assert!(
            e.to_string()
                .contains("a listener names the clients it trusts"),
            "{e}"
        );
    };
    lacks(TlsListen::bind("127.0.0.1:0", &config).unwrap_err());
    #[cfg(feature = "quic")]
    lacks(
        guatiao_source_remote::transport::quic::QuicListen::bind("127.0.0.1:0", &config)
            .unwrap_err(),
    );
    #[cfg(feature = "webtransport")]
    lacks(
        guatiao_source_remote::transport::webtransport::WtListen::bind("127.0.0.1:0", &config)
            .unwrap_err(),
    );
}

#[test]
fn roots_that_cannot_be_used_are_named() {
    let dir = scratch("unusable");
    std::fs::write(
        dir.join("broken.pem"),
        "-----BEGIN CERTIFICATE-----\nnot base64 at all\n-----END CERTIFICATE-----\n",
    )
    .unwrap();
    for bundle in [dir.join("absent.pem"), dir.join("broken.pem")] {
        let client = by_roots(&bundle);
        let e = TlsConnect::new("localhost:1", &client).unwrap_err();
        assert!(
            e.to_string().contains("the roots (`roots`) cannot be used"),
            "{}: {e}",
            bundle.display()
        );
        #[cfg(feature = "quic")]
        assert!(
            guatiao_source_remote::transport::quic::QuicConnect::new("localhost:1", &client)
                .unwrap_err()
                .to_string()
                .contains("`roots`")
        );
    }
    let e = RemoteSource::from_config(&configured(
        "tls://localhost:1",
        &dir.join("broken.pem").display().to_string(),
    ))
    .unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_BAD_VALUE, "{e:?}");
    assert!(e.message().contains("`roots`"), "{}", e.message());
}

#[test]
fn a_host_no_certificate_can_name_is_refused_under_roots_alone() {
    let dir = scratch("unnameable");
    let ca = Authority::new("guatiao test authority");
    let by_roots = by_roots(&ca.bundle(&dir));
    let pinned = TlsConfig::without_identity()
        .unwrap()
        .pins([Identity::generate().unwrap().fingerprint()]);
    // An address with a zone is no DNS name and no address a certificate holds.
    let zoned = "[fe80::1%eth0]:7443";
    let e = TlsConnect::new(zoned, &by_roots).unwrap_err();
    assert!(
        e.to_string()
            .contains("is not a name a certificate can hold"),
        "{e}"
    );
    TlsConnect::new(zoned, &pinned).unwrap();
    #[cfg(feature = "quic")]
    {
        use guatiao_source_remote::transport::quic::QuicConnect;
        assert!(QuicConnect::new(zoned, &by_roots).is_err());
        QuicConnect::new(zoned, &pinned).unwrap();
    }
}

/// A source file's configuration of a server-TLS peer at `connect`.
fn configured(connect: &str, roots: &str) -> Map {
    let mut config = Map::new();
    config.set("connect", connect);
    config.set("security", "server-tls");
    config.set("key", "test-key");
    config.set("roots", roots);
    config
}

#[test]
fn a_configuration_names_roots_by_a_bundle_path() {
    let dir = scratch("configured");
    let ca = Authority::new("guatiao test authority");
    let bundle = ca.bundle(&dir);
    let port = listen(Over::Tcp, &ca.issue(&["localhost"], false));
    let source = RemoteSource::from_config(&configured(
        &url(Over::Tcp, "localhost", port),
        &bundle.display().to_string(),
    ))
    .unwrap();
    assert_eq!(greeting(&source).unwrap(), "hello, roots");

    let missing = dir.join("absent.pem").display().to_string();
    let e = RemoteSource::from_config(&configured("tls://localhost:1", &missing)).unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_BAD_VALUE, "{e:?}");
    assert!(e.message().contains("`roots`"), "{}", e.message());

    std::fs::write(dir.join("empty.pem"), "no certificate here").unwrap();
    let empty = dir.join("empty.pem").display().to_string();
    let e = RemoteSource::from_config(&configured("tls://localhost:1", &empty)).unwrap_err();
    assert!(
        e.message().contains("holds no certificate"),
        "{}",
        e.message()
    );
}

#[test]
fn a_peer_manifest_names_its_bundle_beside_it() {
    let dir = scratch("manifest");
    let ca = Authority::new("guatiao test authority");
    // Another authority's bundle first: replaced, the peer is a new one
    // built over the new bundle.
    Authority::new("another authority").bundle(&dir);
    let port = listen(Over::Tcp, &ca.issue(&["localhost"], false));
    std::fs::write(
        dir.join("lab.guatiao.toml"),
        format!(
            "type = \"remote\"\nname = \"lab\"\nconnect = \"tls://localhost:{port}\"\nsecurity = \"server-tls\"\nroots = \"roots.pem\"\n"
        ),
    )
    .unwrap();
    let mut config = Map::new();
    let mut dirs = List::new();
    dirs.push(dir.display().to_string().as_str());
    config.set("dirs", dirs);
    config.set("key", "test-key");
    let source = RemoteSource::from_config(&config).unwrap();
    let mut registry = Registry::new("host", "1.0");
    registry.add_source("remote", source).unwrap();
    let refused = registry.refresh().refused;
    assert!(
        refused.iter().any(|r| r.reason.contains("no authority")),
        "{refused:?}"
    );
    ca.bundle(&dir);
    let refused = registry.refresh().refused;
    assert!(refused.is_empty(), "{refused:?}");
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .expect("a greeter")
        .shared()
        .unwrap();
    let answer = greeter.greet("roots").unwrap();
    assert_eq!(
        answer.get("greeting").and_then(TryAsRef::<str>::try_as_ref),
        Some("hello, roots")
    );
}

#[cfg(not(feature = "platform-roots"))]
#[test]
fn platform_roots_name_the_feature_they_need() {
    let e = RemoteSource::from_config(&configured("tls://localhost:1", "platform")).unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_UNSUPPORTED, "{e:?}");
    assert!(e.message().contains("`platform-roots`"), "{}", e.message());
}

/// No publicly trusted server is reachable offline, so the operating
/// system's roots are shown refusing a self-issued certificate only.
#[cfg(feature = "platform-roots")]
#[test]
fn platform_roots_refuse_a_self_issued_certificate() {
    let server = Identity::generate().unwrap();
    let client = TlsConfig::without_identity()
        .unwrap()
        .roots(Roots::platform());
    for over in every() {
        let port = listen(over, &server);
        let e = source(over, "localhost", port, &client).link().unwrap_err();
        assert_eq!(e.status, Status::GUATIAO_ERR_BAD_VALUE, "{over:?}: {e:?}");
        assert!(
            e.message().contains("not one this client trusts") || e.message().contains("refuses"),
            "{over:?}: {}",
            e.message()
        );
    }
    let port = listen(Over::Tcp, &server);
    let e = RemoteSource::from_config(&configured(&url(Over::Tcp, "localhost", port), "platform"))
        .unwrap()
        .link()
        .unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_BAD_VALUE, "{e:?}");
}
