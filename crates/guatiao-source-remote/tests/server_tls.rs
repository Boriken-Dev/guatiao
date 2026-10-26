// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Server TLS on every transport that runs TLS (`tls://`, `quic://`,
//! `wss://`, WebTransport's `https://`): the server proven by its certificate, the client by the key
//! or a token inside the link, presenting no certificate.

#![cfg(feature = "tls")]

mod common;

use greeter_kind::Greeter;
use guatiao::Status;
use guatiao::library::Registry;
use guatiao::value::convert::TryAsRef;
use guatiao_source_remote::transport::Connect;
use guatiao_source_remote::transport::tcp::Accepted;
use guatiao_source_remote::transport::tls::{Identity, TlsConfig, TlsConnect, TlsListen};
use guatiao_source_remote::{LinkOptions, RemoteSource, Server, Trust};

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

fn over_name(over: Over) -> &'static str {
    match over {
        Over::Tcp => "tcp",
        #[cfg(feature = "quic")]
        Over::Quic => "quic",
        #[cfg(feature = "ws")]
        Over::Wss => "wss",
        #[cfg(feature = "webtransport")]
        Over::Wt => "https",
    }
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

/// Serves `server` over `over` as `config` says; its address.
fn listen(over: Over, config: &TlsConfig, server: Server) -> String {
    match over {
        Over::Tcp => {
            let l = TlsListen::bind("127.0.0.1:0", config).unwrap();
            let at = l.local_addr().unwrap().to_string();
            spawn(at, Box::new(move || l.accept()), server)
        }
        #[cfg(feature = "quic")]
        Over::Quic => {
            let l = guatiao_source_remote::transport::quic::QuicListen::bind("127.0.0.1:0", config)
                .unwrap();
            let at = l.local_addr().unwrap().to_string();
            spawn(at, Box::new(move || l.accept()), server)
        }
        #[cfg(feature = "ws")]
        Over::Wss => {
            let l = guatiao_source_remote::transport::ws::WsListen::bind_tls("127.0.0.1:0", config)
                .unwrap();
            let at = l.local_addr().unwrap().to_string();
            spawn(at, Box::new(move || l.accept()), server)
        }
        #[cfg(feature = "webtransport")]
        Over::Wt => {
            let l = guatiao_source_remote::transport::webtransport::WtListen::bind(
                "127.0.0.1:0",
                config,
            )
            .unwrap();
            let at = l.local_addr().unwrap().to_string();
            spawn(at, Box::new(move || l.accept()), server)
        }
    }
}

fn spawn(
    at: String,
    accept: Box<dyn Fn() -> std::io::Result<Accepted> + Send>,
    server: Server,
) -> String {
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
    at
}

fn connect(over: Over, address: &str, config: &TlsConfig) -> Box<dyn Connect> {
    match over {
        Over::Tcp => Box::new(TlsConnect::new(address, config).unwrap()),
        #[cfg(feature = "quic")]
        Over::Quic => Box::new(
            guatiao_source_remote::transport::quic::QuicConnect::new(address, config).unwrap(),
        ),
        #[cfg(feature = "ws")]
        Over::Wss => Box::new(
            guatiao_source_remote::transport::ws::WsConnect::new(&format!("wss://{address}/"))
                .tls(config)
                .unwrap(),
        ),
        #[cfg(feature = "webtransport")]
        Over::Wt => Box::new(
            guatiao_source_remote::transport::webtransport::WtConnect::new(
                &format!("https://{address}/"),
                config,
            )
            .unwrap(),
        ),
    }
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
    let map = greeter.greet("server tls").unwrap();
    map.get("greeting")
        .and_then(TryAsRef::<str>::try_as_ref)
        .unwrap_or_default()
        .to_string()
}

/// A server at server TLS holding `options`, and the client configuration
/// that pins it.
fn device(over: Over, options: LinkOptions) -> (String, Identity, TlsConfig) {
    let id = Identity::generate().unwrap();
    let config = TlsConfig::new(id.clone()).server_tls();
    let address = listen(over, &config, common::server(options));
    let client = TlsConfig::without_identity()
        .unwrap()
        .pins([id.fingerprint()]);
    (address, id, client)
}

#[test]
fn a_client_with_no_certificate_links_by_the_key_on_every_transport() {
    for over in every() {
        let (address, server, client) = device(over, LinkOptions::new(Trust::ServerTls).key(KEY));
        let source = RemoteSource::new(
            connect(over, &address, &client),
            LinkOptions::new(Trust::ServerTls).key(KEY),
        );
        let (registry, refused) = refresh(&source);
        assert!(refused.is_empty(), "{over:?}: {refused:?}");
        assert_eq!(greeting(&registry), "hello, server tls", "{over:?}");
        let link = source.link().unwrap();
        assert_eq!(link.trust(), Trust::ServerTls);
        assert_eq!(
            link.peer_id(),
            Some(server.fingerprint().canonical().as_str()),
            "{over:?}: the client knows the server"
        );
        assert!(link.datagrams().is_some() == matches!(over_name(over), "quic" | "https"));

        let wrong = RemoteSource::new(
            connect(over, &address, &client),
            LinkOptions::new(Trust::ServerTls).key(b"other"),
        );
        let (registry, refused) = refresh(&wrong);
        assert_eq!(registry.offers::<dyn Greeter>().count(), 0, "{over:?}");
        assert!(
            refused.iter().any(|r| r.contains("key")),
            "{over:?}: {refused:?}"
        );
    }
}

#[test]
fn a_client_with_a_token_links_when_the_server_requires_one() {
    for over in every() {
        let (address, _, client) = device(over, LinkOptions::new(Trust::ServerTls).token("t0k"));
        let source = RemoteSource::new(
            connect(over, &address, &client),
            LinkOptions::new(Trust::ServerTls).token("t0k"),
        );
        let (registry, refused) = refresh(&source);
        assert!(refused.is_empty(), "{over:?}: {refused:?}");
        assert_eq!(greeting(&registry), "hello, server tls");
        let wrong = RemoteSource::new(
            connect(over, &address, &client),
            LinkOptions::new(Trust::ServerTls).token("t0x"),
        );
        let e = wrong.link().unwrap_err();
        assert_eq!(e.status, Status::GUATIAO_ERR_BAD_VALUE, "{over:?}: {e:?}");
    }
}

#[test]
fn a_server_the_client_does_not_pin_is_refused() {
    for over in every() {
        let (address, _, _) = device(over, LinkOptions::new(Trust::ServerTls).key(KEY));
        let stranger = TlsConfig::without_identity()
            .unwrap()
            .pins([Identity::generate().unwrap().fingerprint()]);
        let source = RemoteSource::new(
            connect(over, &address, &stranger),
            LinkOptions::new(Trust::ServerTls).key(KEY),
        );
        let e = source.link().unwrap_err();
        assert_eq!(e.status, Status::GUATIAO_ERR_BAD_VALUE, "{over:?}: {e:?}");
        assert!(
            e.message().contains("not pinned"),
            "{over:?}: {}",
            e.message()
        );
    }
}

#[test]
fn a_mutual_tls_server_refuses_a_client_with_no_certificate() {
    for over in every() {
        let (server_id, client_id) = (Identity::generate().unwrap(), Identity::generate().unwrap());
        let mutual = TlsConfig::new(server_id.clone()).pins([client_id.fingerprint()]);
        let address = listen(
            over,
            &mutual,
            common::server(LinkOptions::new(Trust::Secure)),
        );
        let client = TlsConfig::without_identity()
            .unwrap()
            .pins([server_id.fingerprint()]);
        let source = RemoteSource::new(
            connect(over, &address, &client),
            LinkOptions::new(Trust::ServerTls).key(KEY),
        );
        let e = source.link().unwrap_err();
        assert_eq!(e.status, Status::GUATIAO_ERR_BAD_VALUE, "{over:?}: {e:?}");
    }
}

#[test]
fn a_server_tls_server_refuses_a_client_asking_mutual_tls() {
    for over in every() {
        let (address, server, _) = device(over, LinkOptions::new(Trust::ServerTls).key(KEY));
        // A client holding an identity the server never asks for, at the
        // secure level: the trust levels differ.
        let client = TlsConfig::new(Identity::generate().unwrap()).pins([server.fingerprint()]);
        let source = RemoteSource::new(
            connect(over, &address, &client),
            LinkOptions::new(Trust::Secure),
        );
        let e = source.link().unwrap_err();
        assert_eq!(e.status, Status::GUATIAO_ERR_UNSUPPORTED, "{over:?}: {e:?}");
        assert!(
            e.message().contains("server-tls"),
            "{over:?}: {}",
            e.message()
        );
    }
}

#[test]
fn server_tls_is_listed_as_weaker_than_strict() {
    let config = TlsConfig::new(Identity::generate().unwrap()).server_tls();
    assert!(!config.is_mutual());
    assert!(
        config
            .weakened()
            .iter()
            .any(|w| w.setting == coqui_peer_tls::CLIENT_AUTH_SETTING),
        "{:?}",
        config.weakened()
    );
    let mutual = TlsConfig::new(Identity::generate().unwrap());
    assert!(mutual.weakened().is_empty());
}

#[test]
fn a_configuration_names_server_tls_with_pins_alone() {
    let (address, server, _) = device(Over::Tcp, LinkOptions::new(Trust::ServerTls).key(KEY));
    let mut config = guatiao::Map::new();
    config.set("connect", format!("tcp://{address}").as_str());
    config.set("security", "server-tls");
    config.set("key", "test-key");
    let mut pins = guatiao::List::new();
    pins.push(server.fingerprint().canonical().as_str());
    config.set("pins", pins);
    let (registry, refused) = refresh(&RemoteSource::from_config(&config).unwrap());
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(greeting(&registry), "hello, server tls");
}

/// A directory of its own under the target's scratch space, emptied.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("server_tls")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn server_program() -> std::process::Command {
    std::process::Command::new(env!("CARGO_BIN_EXE_guatiao-remote-server"))
}

#[test]
fn the_server_program_requires_the_token_its_token_file_holds() {
    let Some(derived) = common::procs::library("derived_greeter") else {
        panic!("derived_greeter is not built: `cargo build -p derived_greeter` first");
    };
    let dir = scratch("token_file");
    // As `echo` writes it: the line end is not the token's.
    std::fs::write(dir.join("token"), "s3cret\n").unwrap();
    let (child, lines) = common::procs::start(
        server_program()
            .args(["--listen", "tls://127.0.0.1:0", "--security", "server-tls"])
            .arg("--identity")
            .arg(dir.join("identity.pem"))
            .arg("--token-file")
            .arg(dir.join("token"))
            .arg("--load")
            .arg(&derived),
    );
    let _running = Running(child);
    let first = lines
        .recv_timeout(common::procs::DEADLINE)
        .expect("the server says where it serves");
    let serving = first
        .split("serving tls://")
        .nth(1)
        .unwrap_or_else(|| panic!("{first}"));
    let (address, rest) = serving.split_once(" as ").unwrap();
    let fingerprint: guatiao_source_remote::transport::tls::PeerId =
        rest.split(',').next().unwrap().parse().unwrap();
    let client = TlsConfig::without_identity().unwrap().pins([fingerprint]);

    let source = RemoteSource::new(
        connect(Over::Tcp, address, &client),
        LinkOptions::new(Trust::ServerTls).token("s3cret"),
    );
    let (registry, refused) = refresh(&source);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(greeting(&registry), "hello, server tls");

    let wrong = RemoteSource::new(
        connect(Over::Tcp, address, &client),
        LinkOptions::new(Trust::ServerTls).token("s3cret\n"),
    );
    let e = wrong.link().unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_BAD_VALUE, "{e:?}");
    assert!(e.message().contains("token"), "{}", e.message());
}

/// Kills the server however the test ends.
struct Running(std::process::Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn the_server_program_refuses_an_empty_token_and_a_token_with_no_tls() {
    let dir = scratch("token_refused");
    std::fs::write(dir.join("empty"), "\r\n").unwrap();
    std::fs::write(dir.join("token"), "t0k").unwrap();
    std::fs::write(dir.join("key"), "k").unwrap();
    let (ok, said) = common::procs::run(
        server_program()
            .args(["--listen", "tls://127.0.0.1:0", "--security", "server-tls"])
            .arg("--identity")
            .arg(dir.join("identity.pem"))
            .arg("--token-file")
            .arg(dir.join("empty")),
    );
    assert!(!ok);
    assert!(said.contains("the token is empty"), "{said}");
    let (ok, said) = common::procs::run(
        server_program()
            .args(["--listen", "tcp://127.0.0.1:0", "--security", "none"])
            .args(["--range", "127.0.0.1/32"])
            .arg("--key-file")
            .arg(dir.join("key"))
            .arg("--token-file")
            .arg(dir.join("token")),
    );
    assert!(!ok);
    assert!(said.contains("is for server-tls and mutual-tls"), "{said}");
}
