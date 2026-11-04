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

/// A server whose reader is slow to hand over the PROOF: the channel reads
/// on, so the frames behind it are judged while the server still checks the
/// PROOF.
struct SlowAfterFirst {
    inner: Box<dyn guatiao_source_remote::transport::FrameRead>,
    reads: usize,
    /// How long it holds the PROOF before handing it over.
    pause: Duration,
}

impl guatiao_source_remote::transport::FrameRead for SlowAfterFirst {
    fn read_frame(&mut self, buf: &mut Vec<u8>, max: usize) -> std::io::Result<bool> {
        let read = self.inner.read_frame(buf, max);
        self.reads += 1;
        if self.reads == 2 {
            std::thread::sleep(self.pause);
        }
        read
    }

    fn set_timeout(&mut self, timeout: Option<Duration>) -> std::io::Result<()> {
        self.inner.set_timeout(timeout)
    }

    fn set_max_frame(&mut self, bytes: usize) {
        self.inner.set_max_frame(bytes);
    }

    fn set_max_frame_after(&mut self, messages: usize, bytes: usize) -> bool {
        self.inner.set_max_frame_after(messages, bytes)
    }

    fn set_read_ahead(&mut self, frames: usize) -> std::io::Result<()> {
        self.inner.set_read_ahead(frames)
    }

    fn peer_id(&self) -> Option<String> {
        self.inner.peer_id()
    }
}

/// A client sends a large call right behind its PROOF, before the server
/// has read the PROOF: the server judges the PROOF small and what follows
/// large, so the call is served rather than ending the link.
#[test]
fn a_large_call_pipelined_right_behind_the_proof_is_served() {
    let server = common::server(LinkOptions::new(Trust::Trusted).key(KEY));
    let listener = WsListen::bind("127.0.0.1:0", common::permit(&["127.0.0.1"])).unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        while let Ok(accepted) = listener.accept() {
            if let Accepted::Connection(mut conn) = accepted {
                conn.reader = Box::new(SlowAfterFirst {
                    inner: conn.reader,
                    reads: 0,
                    pause: Duration::from_millis(400),
                });
                let server = server.clone();
                std::thread::spawn(move || {
                    let _ = server.serve(conn);
                });
            }
        }
    });
    let big = "x".repeat(1 << 20);
    let source = over_ws(
        port,
        LinkOptions::new(Trust::Trusted)
            .key(KEY)
            .timeout(Some(Duration::from_secs(10))),
    );
    let link = source.link().expect("the link is made");
    let _ = link.open(&big, "stamp");
    assert!(link.alive(), "the link survives: {}", link.why());
    link.ping().expect("and answers");
}

/// A client's writer that puts a large message where its second frame (the
/// PROOF) goes.
struct ProofReplaced {
    inner: Box<dyn guatiao_source_remote::transport::FrameWrite>,
    writes: usize,
}

impl guatiao_source_remote::transport::FrameWrite for ProofReplaced {
    fn write_frame(&mut self, frame: &[u8]) -> std::io::Result<()> {
        self.writes += 1;
        if self.writes == 2 {
            // The client may send past the handshake's bound only once it has
            // agreed a larger one, so the wrapper agrees it for this frame.
            self.inner.set_max_frame(64 << 20);
            let body = vec![0u8; 3 << 20];
            let mut big = u32::try_from(body.len()).unwrap().to_le_bytes().to_vec();
            big.extend_from_slice(&body);
            return self.inner.write_frame(&big);
        }
        self.inner.write_frame(frame)
    }

    fn set_timeout(&mut self, timeout: Option<Duration>) -> std::io::Result<()> {
        self.inner.set_timeout(timeout)
    }

    fn shutdown(&mut self) {
        self.inner.shutdown();
    }
}

struct SendsBigProof(WsConnect);

impl guatiao_source_remote::transport::Connect for SendsBigProof {
    fn connect(&self) -> std::io::Result<guatiao_source_remote::transport::Connection> {
        let mut conn = self.0.connect()?;
        conn.writer = Box::new(ProofReplaced {
            inner: conn.writer,
            writes: 0,
        });
        Ok(conn)
    }

    fn describe(&self) -> String {
        self.0.describe()
    }
}

/// A stranger puts a 3 MiB message where the PROOF goes: the channel judges
/// it at the handshake's 64 KiB (the staged bound), refusing it as it
/// arrives, not after buffering it under the full bound.
#[test]
fn a_large_message_in_the_proofs_place_is_refused_under_64_kib() {
    let server = common::server(LinkOptions::new(Trust::Trusted).key(KEY));
    let listener = WsListen::bind("127.0.0.1:0", common::permit(&["127.0.0.1"])).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (told, heard) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(accepted) = listener.accept() {
            if let Accepted::Connection(conn) = accepted {
                let server = server.clone();
                let told = told.clone();
                std::thread::spawn(move || {
                    let _ = told.send(server.serve(conn).map(|_| ()).map_err(|e| format!("{e:?}")));
                });
            }
        }
    });
    let source = guatiao_source_remote::RemoteSource::new(
        Box::new(SendsBigProof(WsConnect::new(&format!(
            "ws://127.0.0.1:{port}/remote"
        )))),
        LinkOptions::new(Trust::Trusted)
            .key(KEY)
            .timeout(Some(Duration::from_secs(10))),
    );
    // The client returns once its PROOF is written; the server judges it.
    let _client = source.link();
    let why = heard
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .expect_err("the server refuses it");
    assert!(
        why.contains("breaks the wire"),
        "refused by the channel ending at its bound (not read whole and found too large): {why}"
    );
}

/// A 1 MiB message sent in one burst with the upgrade is the first the
/// channel judges: under the handshake's bound, not coqui-web's default.
#[test]
fn a_one_mib_first_message_sent_with_the_upgrade_is_refused() {
    use std::io::Write;
    let listener = WsListen::bind("127.0.0.1:0", common::permit(&["127.0.0.1"])).unwrap();
    let at = listener.local_addr().unwrap();
    let sending = std::thread::spawn(move || {
        let mut stream = std::net::TcpStream::connect(at).unwrap();
        let mut burst = format!(
            "GET /remote HTTP/1.1\r\nHost: {at}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )
        .into_bytes();
        let payload = vec![0u8; 1 << 20];
        burst.push(0x82);
        burst.push(0x80 | 127);
        burst.extend_from_slice(&u64::try_from(payload.len()).unwrap().to_be_bytes());
        burst.extend_from_slice(&[0, 0, 0, 0]);
        burst.extend_from_slice(&payload);
        let _ = stream.write_all(&burst);
        std::thread::sleep(Duration::from_secs(2));
    });
    let Accepted::Connection(mut conn) = listener.accept().unwrap() else {
        panic!("admitted");
    };
    conn.reader
        .set_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut buf = Vec::new();
    let e = conn
        .reader
        .read_frame(&mut buf, 64 << 10)
        .expect_err("a 1 MiB first message is refused");
    assert!(
        e.to_string().contains("broke the wire format"),
        "refused by the channel's bound: {e}"
    );
    sending.join().unwrap();
}

/// One WebSocket message as a client sends it: binary, masked with a key of
/// zeros, which leaves the payload as it is.
fn masked(payload: &[u8]) -> Vec<u8> {
    let mut out = vec![0x82];
    match payload.len() {
        n if n < 126 => out.push(0x80 | u8::try_from(n).unwrap()),
        n if n <= 0xffff => {
            out.push(0x80 | 126);
            out.extend_from_slice(&u16::try_from(n).unwrap().to_be_bytes());
        }
        n => {
            out.push(0x80 | 127);
            out.extend_from_slice(&u64::try_from(n).unwrap().to_be_bytes());
        }
    }
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(payload);
    out
}

/// The payload of the next message a server sends.
fn unmasked(stream: &mut std::net::TcpStream) -> Vec<u8> {
    use std::io::Read;
    let mut head = [0u8; 2];
    stream.read_exact(&mut head).unwrap();
    let len = match head[1] & 0x7f {
        126 => {
            let mut n = [0u8; 2];
            stream.read_exact(&mut n).unwrap();
            usize::from(u16::from_be_bytes(n))
        }
        127 => {
            let mut n = [0u8; 8];
            stream.read_exact(&mut n).unwrap();
            usize::try_from(u64::from_be_bytes(n)).unwrap()
        }
        n => usize::from(n),
    };
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).unwrap();
    payload
}

/// A frame's body as the link encodes it, without its length.
fn body(frame: &guatiao_source_remote::protocol::Frame<'_>) -> Vec<u8> {
    let mut out = Vec::new();
    frame.encode(&mut out, usize::MAX).unwrap();
    out.split_off(4)
}

/// A stranger past the address rules answers the WELCOME with a junk PROOF
/// and then 1 MiB messages while the server is slow to check the PROOF: the
/// channel reads one of those messages ahead and no more, so the stranger's
/// socket stops taking bytes once that one and the transport's own buffers
/// are full, rather than the server reading up to 64 of them.
#[test]
fn a_stranger_behind_a_junk_proof_has_one_message_read_ahead() {
    use guatiao_source_remote::protocol::Frame;
    use std::io::{Read, Write};
    const MESSAGE: usize = 1 << 20;
    const SENT: usize = 48;
    let server = common::server(LinkOptions::new(Trust::Trusted).key(KEY));
    let listener = WsListen::bind("127.0.0.1:0", common::permit(&["127.0.0.1"])).unwrap();
    let at = listener.local_addr().unwrap();
    let (told, heard) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        if let Ok(Accepted::Connection(mut conn)) = listener.accept() {
            conn.reader = Box::new(SlowAfterFirst {
                inner: conn.reader,
                reads: 0,
                pause: Duration::from_secs(4),
            });
            let _ = told.send(server.serve(conn).map(|_| ()).map_err(|e| format!("{e:?}")));
        }
    });

    let mut stream = std::net::TcpStream::connect(at).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream
        .write_all(
            format!(
                "GET /remote HTTP/1.1\r\nHost: {at}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
            )
            .as_bytes(),
        )
        .unwrap();
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).unwrap();
        head.push(byte[0]);
    }
    assert!(
        head.starts_with(b"HTTP/1.1 101"),
        "{}",
        String::from_utf8_lossy(&head)
    );
    let hello = Frame::Hello {
        trust: Trust::Trusted,
        nonce: [7; 16],
        options: guatiao::Value::from(Map::new()),
    };
    stream.write_all(&masked(&body(&hello))).unwrap();
    let welcome = unmasked(&mut stream);
    assert!(
        matches!(
            Frame::decode(&welcome, guatiao::Alloc::rust()),
            Ok(Frame::Welcome { .. })
        ),
        "the server answers with a WELCOME"
    );
    stream
        .write_all(&masked(&body(&Frame::Proof { proof: &[0; 32] })))
        .unwrap();

    // While the server holds the PROOF, how much of what follows the socket
    // takes before it stops taking any.
    stream
        .set_write_timeout(Some(Duration::from_millis(1500)))
        .unwrap();
    let message = masked(&vec![0u8; MESSAGE]);
    let mut taken = 0;
    'sending: for _ in 0..SENT {
        let mut at = 0;
        while at < message.len() {
            let end = message.len().min(at + (64 << 10));
            match stream.write(&message[at..end]) {
                Ok(0) | Err(_) => break 'sending,
                Ok(n) => {
                    at += n;
                    taken += n;
                }
            }
        }
    }
    eprintln!(
        "the socket took {taken} bytes of {} sent behind the PROOF",
        SENT * message.len()
    );
    assert!(
        taken < 16 * MESSAGE,
        "the server read {taken} bytes of {SENT} messages of {MESSAGE} behind a PROOF it had not checked: \
         more than one message ahead and the transport's buffers"
    );
    let why = heard
        .recv_timeout(Duration::from_secs(20))
        .unwrap()
        .expect_err("the server refuses the junk PROOF");
    assert!(why.contains("does not hold this link's key"), "{why}");
}
