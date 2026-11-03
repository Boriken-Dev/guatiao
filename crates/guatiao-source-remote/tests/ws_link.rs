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
use guatiao_source_remote::transport::Connect as _;
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

/// An address of this machine that is not loopback, if it has one.
fn lan_address() -> Option<core::net::IpAddr> {
    let probe = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    probe.connect("192.0.2.1:9").ok()?;
    let ip = probe.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

/// A plain WebSocket beyond loopback is a deliberate, named weakening: off
/// by default at the listener, the connector and the configuration, and
/// listed by the source when it is on. Nothing here needs a network.
#[test]
fn a_plain_websocket_beyond_loopback_is_off_by_default_and_reported() {
    let refused = WsListen::bind("0.0.0.0:0", common::permit(&["0.0.0.0/0"])).unwrap_err();
    assert_eq!(refused.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(
        refused.to_string().contains("plain_beyond_loopback"),
        "{refused}"
    );
    let listener =
        WsListen::bind_beyond_loopback("0.0.0.0:0", common::permit(&["0.0.0.0/0"])).unwrap();
    assert!(listener.plain_beyond_loopback());

    // A documentation address: the connector refuses it before it connects.
    let url = "ws://192.0.2.1:9/";
    let e = WsConnect::new(url).connect().unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied, "{e}");
    assert!(e.to_string().contains("plain_beyond_loopback"), "{e}");
    assert!(!WsConnect::new(url).plain_beyond_loopback());
    assert!(
        WsConnect::new(url)
            .beyond_loopback()
            .plain_beyond_loopback()
    );

    // A configuration names it, and the source lists it; one built from a
    // connector in Rust holds the connector, which its host asks.
    let mut config = Map::new();
    config.set("connect", url);
    config.set("security", "none");
    config.set("key", "test-key");
    let off = RemoteSource::from_config(&config).unwrap();
    assert!(off.weakened().is_empty());
    let e = off.link().unwrap_err();
    assert!(
        e.message().contains("plain_beyond_loopback"),
        "{}",
        e.message()
    );
    config.set("plain_beyond_loopback", true);
    let on = RemoteSource::from_config(&config).unwrap();
    let weakened = on.weakened();
    assert_eq!(weakened.len(), 1, "{weakened:?}");
    assert_eq!(weakened[0].0, "");
    assert_eq!(weakened[0].1.setting, "plain_beyond_loopback");
    assert_eq!(weakened[0].1.value, "true");
}

/// The same over an address of this machine beyond loopback, which only a
/// machine with a network has. Without one this fails, saying so, unless
/// `GUATIAO_SKIP_LAN` names the choice to skip it.
#[test]
fn a_plain_websocket_beyond_loopback_links_when_named() {
    let Some(ip) = lan_address() else {
        assert!(
            std::env::var_os("GUATIAO_SKIP_LAN").is_some(),
            "this machine has no address beyond loopback to link over: set GUATIAO_SKIP_LAN to skip this test"
        );
        eprintln!("SKIPPED (GUATIAO_SKIP_LAN): no address beyond loopback");
        return;
    };
    let listener =
        WsListen::bind_beyond_loopback("0.0.0.0:0", common::permit(&["0.0.0.0/0"])).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = common::server(LinkOptions::new(Trust::Trusted).key(KEY));
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
    let url = format!("ws://{ip}:{port}/");
    // Off by default, and the listener has no say in it.
    assert!(WsConnect::new(&url).connect().is_err());
    let mut config = Map::new();
    config.set("connect", url.as_str());
    config.set("security", "none");
    config.set("key", "test-key");
    config.set("plain_beyond_loopback", true);
    let (registry, refused) = refresh(RemoteSource::from_config(&config).unwrap());
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(registry.offers::<dyn Greeter>().count(), 1);
}

/// A list rule that names the address a client came to cannot be decided
/// by a listener bound to a wildcard, which does not know it: the rule
/// refuses the client, as it does a TLS listener. `tcp://` decides it by
/// the accepted socket's address (the control); a rule naming the address
/// denies here, so the WebSocket refuses too, not permits for want of a
/// match.
#[test]
fn a_wildcard_plain_websocket_listener_refuses_on_a_rule_naming_a_destination() {
    use guatiao_source_remote::acl::{Acl, Action, Rule};
    use guatiao_source_remote::transport::tcp::TcpListen;
    let mut deny = Rule::new(Action::Deny, "127.0.0.0/8".parse().unwrap());
    deny.destination = Some("127.0.0.1/32".parse().unwrap());
    let mut acl = Acl::default();
    acl.push(deny);
    acl.push(Rule::new(Action::Permit, "127.0.0.0/8".parse().unwrap()));

    // The control: TCP on a wildcard refuses a loopback client by the rule.
    let tcp = TcpListen::bind("0.0.0.0:0", acl.clone()).unwrap();
    let port = tcp.local_addr().unwrap().port();
    let _client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    assert!(matches!(tcp.accept().unwrap(), Accepted::Refused(_)));

    let ws = WsListen::bind_beyond_loopback("0.0.0.0:0", acl).unwrap();
    let port = ws.local_addr().unwrap().port();
    let _client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    assert!(matches!(ws.accept().unwrap(), Accepted::Refused(_)));
}

/// The server binary listens beyond loopback only when told, and says it
/// at start as a weakening.
#[test]
fn the_server_program_listens_beyond_loopback_only_when_told() {
    use common::procs;
    use std::process::Command;
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ws_beyond_loopback");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let key = dir.join("link.key");
    std::fs::write(&key, KEY).unwrap();
    let run = |extra: &[&str]| {
        let (child, lines) = procs::start(
            Command::new(env!("CARGO_BIN_EXE_guatiao-remote-server"))
                .args(["--listen", "ws://0.0.0.0:0", "--security", "none"])
                .args(["--range", "0.0.0.0/0"])
                .arg("--key-file")
                .arg(&key)
                .args(extra),
        );
        // It serves until killed, or ends refusing.
        let until = std::time::Instant::now() + Duration::from_secs(20);
        let mut said = Vec::new();
        while let Ok(line) =
            lines.recv_timeout(until.saturating_duration_since(std::time::Instant::now()))
        {
            let serving = line.contains("serving ws://");
            said.push(line);
            if serving {
                // The weakening follows the serving line.
                while let Ok(more) = lines.recv_timeout(Duration::from_millis(500)) {
                    said.push(more);
                }
                break;
            }
        }
        let mut child = child;
        let _ = child.kill();
        let _ = child.wait();
        said.join("\n")
    };
    let default = run(&[]);
    assert!(default.contains("loopback"), "{default}");
    assert!(!default.contains("serving ws://"), "{default}");
    let named = run(&["--plain-beyond-loopback"]);
    assert!(named.contains("serving ws://"), "{named}");
    assert!(
        named.contains("weaker than modern: plain_beyond_loopback = true"),
        "{named}"
    );
    let _ = std::fs::remove_dir_all(&dir);
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
fn ending_the_links_ends_one_still_in_its_upgrade() {
    let server = common::server(LinkOptions::new(Trust::Trusted).key(KEY));
    let listener = WsListen::bind("127.0.0.1:0", common::permit(&["127.0.0.1"])).unwrap();
    // Connected, and never sends its HTTP upgrade.
    let _silent = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let Accepted::Connection(conn) = listener.accept().unwrap() else {
        panic!("a permitted source");
    };
    let took = common::ended_in_handshake(&server, conn);
    assert!(took < Duration::from_secs(3), "{took:?}");
}

/// A closer cancels the upgrade under way: the peer's socket is closed at
/// once, not left open for the handshake's whole bound.
#[test]
fn a_closed_link_closes_the_socket_of_the_upgrade_it_cancelled() {
    use std::io::Read;
    let listener = WsListen::bind("127.0.0.1:0", common::permit(&["127.0.0.1"])).unwrap();
    let mut silent = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    silent
        .set_read_timeout(Some(Duration::from_secs(4)))
        .unwrap();
    let Accepted::Connection(mut conn) = listener.accept().unwrap() else {
        panic!("a permitted source");
    };
    let closer = conn.closer.take().expect("a closer");
    let reading = std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let mut frame = Vec::new();
        let ended = conn.reader.read_frame(&mut frame, 1 << 10);
        (ended, started.elapsed())
    });
    std::thread::sleep(Duration::from_millis(300));
    let closed = std::time::Instant::now();
    closer();
    let (ended, _) = reading.join().unwrap();
    assert_eq!(
        ended.unwrap_err().kind(),
        std::io::ErrorKind::ConnectionAborted
    );
    // The socket goes with it: the silent peer reads the end of the stream.
    let mut byte = [0u8; 1];
    let n = silent.read(&mut byte);
    assert!(matches!(n, Ok(0) | Err(_)), "{n:?}");
    if let Err(e) = n {
        // A reset is an end too; a timeout is not.
        assert!(
            !matches!(
                e.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ),
            "the socket stayed open: {e}"
        );
    }
    assert!(
        closed.elapsed() < Duration::from_secs(3),
        "{:?}",
        closed.elapsed()
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
