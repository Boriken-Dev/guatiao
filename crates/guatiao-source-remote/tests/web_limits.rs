// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a message transport (coqui-web-channel's WebSocket and WebTransport) must
//! do as a stream does: carry a frame past the largest this side reads, up
//! to what the peer takes, and wait for a peer slow to take a burst rather
//! than end the link.

#![cfg(feature = "ws")]

mod common;

use core::time::Duration;
use std::io::ErrorKind;
use std::time::Instant;

use guatiao_source_remote::transport::tcp::{Accepted, TcpConnect, TcpListen};
use guatiao_source_remote::transport::ws::{WsConnect, WsListen};
use guatiao_source_remote::transport::{Connect, Connection};

fn frame(body: &[u8]) -> Vec<u8> {
    let mut f = u32::try_from(body.len()).unwrap().to_le_bytes().to_vec();
    f.extend_from_slice(body);
    f
}

fn taken(accepted: Accepted) -> Connection {
    match accepted {
        Accepted::Connection(c) => c,
        other => panic!("{other:?}"),
    }
}

/// A server's and a client's end of one transport, the client's hello
/// sent and read (a server's end opens on its first read).
fn pair(over: &str) -> (Connection, Connection) {
    let (mut server, dialing) = unprimed(over);
    server
        .reader
        .set_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    let mut hello = Vec::new();
    assert!(server.reader.read_frame(&mut hello, 1 << 10).unwrap());
    assert_eq!(hello, b"hello");
    server.reader.set_timeout(None).unwrap();
    (server, dialing.join().unwrap())
}

type Dialing = std::thread::JoinHandle<Connection>;

fn hello(mut c: Connection) -> Connection {
    c.writer.write_frame(&frame(b"hello")).unwrap();
    c
}

fn unprimed(over: &str) -> (Connection, Dialing) {
    match over {
        "tcp" => {
            let l = TcpListen::bind("127.0.0.1:0", common::permit(&["127.0.0.1"])).unwrap();
            let at = l.local_addr().unwrap().to_string();
            let dialing =
                std::thread::spawn(move || hello(TcpConnect::new(&at).connect().unwrap()));
            (taken(l.accept().unwrap()), dialing)
        }
        "ws" => {
            let l = WsListen::bind("127.0.0.1:0", common::permit(&["127.0.0.1"])).unwrap();
            let at = l.local_addr().unwrap();
            let dialing = std::thread::spawn(move || {
                hello(WsConnect::new(&format!("ws://{at}/")).connect().unwrap())
            });
            (taken(l.accept().unwrap()), dialing)
        }
        #[cfg(feature = "tls")]
        "wss" => {
            use guatiao_source_remote::transport::tls::{Identity, TlsConfig};
            let id = Identity::generate().unwrap();
            let server = TlsConfig::new(id.clone()).server_tls();
            let client = TlsConfig::without_identity()
                .unwrap()
                .pins([id.fingerprint()]);
            let l = WsListen::bind_tls("127.0.0.1:0", &server).unwrap();
            let at = l.local_addr().unwrap();
            let dialing = std::thread::spawn(move || {
                hello(
                    WsConnect::new(&format!("wss://{at}/"))
                        .tls(&client)
                        .unwrap()
                        .connect()
                        .unwrap(),
                )
            });
            (taken(l.accept().unwrap()), dialing)
        }
        #[cfg(feature = "webtransport")]
        "https" => {
            use guatiao_source_remote::transport::tls::{Identity, TlsConfig};
            use guatiao_source_remote::transport::webtransport::{WtConnect, WtListen};
            let id = Identity::generate().unwrap();
            let server = TlsConfig::new(id.clone()).server_tls();
            let client = TlsConfig::without_identity()
                .unwrap()
                .pins([id.fingerprint()]);
            let l = WtListen::bind("127.0.0.1:0", &server).unwrap();
            let at = l.local_addr().unwrap();
            let dialing = std::thread::spawn(move || {
                hello(
                    WtConnect::new(&format!("https://{at}/"), &client)
                        .unwrap()
                        .connect()
                        .unwrap(),
                )
            });
            (taken(l.accept().unwrap()), dialing)
        }
        other => panic!("{other}"),
    }
}

fn transports() -> Vec<&'static str> {
    #[allow(unused_mut)]
    let mut all = vec!["tcp", "ws"];
    #[cfg(feature = "tls")]
    all.push("wss");
    #[cfg(feature = "webtransport")]
    all.push("https");
    all
}

/// A server that reads at most 1 KiB itself still answers with what its
/// peer takes (a link's `max_frame` is what it reads, and its peer's is
/// what it may send): over a stream that is so, and a message transport
/// bounds a message one way for both.
#[test]
fn a_frame_past_what_this_side_reads_crosses_up_to_what_the_peer_takes() {
    for over in transports() {
        let (mut server, mut client) = pair(over);
        let serving = std::thread::spawn(move || {
            server.writer.set_max_frame(1 << 20);
            server.writer.write_frame(&frame(&[7; 4000])).unwrap();
            server
        });
        let mut got = Vec::new();
        client
            .reader
            .set_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        assert!(
            client.reader.read_frame(&mut got, 1 << 20).unwrap(),
            "{over}"
        );
        assert_eq!(got.len(), 4000, "{over}");
        let _server = serving.join().unwrap_or_else(|_| panic!("{over}"));
    }
}

/// A frame past what the peer takes is still refused, here and not by a
/// reader that then cannot read it.
#[test]
fn a_frame_past_what_the_peer_takes_is_refused_by_the_writer() {
    let (mut server, mut client) = pair("ws");
    let serving = std::thread::spawn(move || {
        server.writer.set_max_frame(2 << 10);
        let refused = server.writer.write_frame(&frame(&[7; 4000]));
        (server, refused)
    });
    let (_server, refused) = serving.join().unwrap();
    assert_eq!(refused.unwrap_err().kind(), ErrorKind::InvalidInput);
    client.writer.shutdown();
}

/// Four 30 MiB frames written to a peer slow to take them all arrive:
/// the writer waits while coqui-web-channel's queue is full.
#[test]
fn a_burst_of_large_frames_waits_for_a_slow_peer_to_take_them() {
    let (mut server, mut client) = pair("ws");
    // Raised before the client sends: a channel judges a message as it arrives.
    server.reader.set_max_frame(64 << 20);
    client.writer.set_max_frame(64 << 20);
    client
        .writer
        .set_timeout(Some(Duration::from_secs(60)))
        .unwrap();
    let body = vec![3u8; 30 << 20];
    client.writer.write_frame(&frame(&body)).unwrap();
    let serving = std::thread::spawn(move || {
        let mut buf = Vec::new();
        server
            .reader
            .set_timeout(Some(Duration::from_secs(60)))
            .unwrap();
        let mut sizes = Vec::new();
        assert!(server.reader.read_frame(&mut buf, 64 << 20).unwrap());
        sizes.push(buf.len());
        // Slow: the rest wait in the client's queue meanwhile.
        std::thread::sleep(Duration::from_secs(2));
        for _ in 0..3 {
            assert!(server.reader.read_frame(&mut buf, 64 << 20).unwrap());
            sizes.push(buf.len());
        }
        sizes
    });
    for n in 2..=4 {
        client
            .writer
            .write_frame(&frame(&body))
            .unwrap_or_else(|e| panic!("frame {n}: {e}"));
    }
    assert_eq!(serving.join().unwrap(), [30 << 20; 4]);
}

/// How many 1 MiB frames a client writes to a server whose reader has read
/// only the hello, each write waiting at most 700 ms for room, before one
/// finds none; the server's channel reads `read_ahead` frames ahead, or what
/// it starts at.
fn written_until_full(over: &str, read_ahead: Option<usize>) -> usize {
    let (mut server, mut client) = pair(over);
    server.reader.set_max_frame(4 << 20);
    if let Some(frames) = read_ahead {
        server.reader.set_read_ahead(frames).unwrap();
    }
    client.writer.set_max_frame(4 << 20);
    client
        .writer
        .set_timeout(Some(Duration::from_millis(700)))
        .unwrap();
    let body = frame(&vec![3u8; 1 << 20]);
    let mut written = 0;
    while written < 400 && client.writer.write_frame(&body).is_ok() {
        written += 1;
    }
    // The server's end first: a WebSocket client's close waits up to 30 s
    // for its queue to go, which the server's close cuts short.
    server.writer.shutdown();
    client.writer.shutdown();
    written
}

/// A server's channel reads one frame ahead of its reader until the link
/// raises it (`set_read_ahead`), so a peer writing to a reader that has
/// stopped finds no room some 63 frames sooner than at the raised 64. A
/// stream has no read-ahead to set.
#[test]
fn a_server_channel_reads_one_frame_ahead_until_raised() {
    for over in transports() {
        if over == "tcp" {
            let (mut server, _client) = pair(over);
            let refused = server.reader.set_read_ahead(64).unwrap_err();
            assert_eq!(refused.kind(), ErrorKind::Unsupported);
            continue;
        }
        let held = written_until_full(over, None);
        let raised = written_until_full(over, Some(64));
        assert!(
            raised >= held + 48,
            "{over}: {held} frames of 1 MiB went to a server reading one ahead, {raised} to one reading 64 ahead"
        );
    }
}

/// A peer that takes nothing (its channel full: coqui-web-channel stops reading at
/// its read-ahead) ends a write at the timeout the link set,
/// as a stream's does.
#[test]
fn a_write_the_peer_takes_nothing_of_ends_at_the_timeout() {
    let (mut server, mut client) = pair("ws");
    server.reader.set_max_frame(64 << 20);
    client.writer.set_max_frame(64 << 20);
    client
        .writer
        .set_timeout(Some(Duration::from_millis(700)))
        .unwrap();
    let body = vec![3u8; 1 << 20];
    client.writer.write_frame(&frame(&body)).unwrap();
    let (opened_tx, opened) = std::sync::mpsc::channel();
    let (done_tx, done) = std::sync::mpsc::channel::<()>();
    let holding = std::thread::spawn(move || {
        let mut buf = Vec::new();
        assert!(server.reader.read_frame(&mut buf, 64 << 20).unwrap());
        opened_tx.send(()).unwrap();
        // Reads no more until the test is over.
        let _ = done.recv();
        drop(server);
    });
    opened.recv_timeout(Duration::from_secs(60)).unwrap();
    let started = Instant::now();
    let mut ended = None;
    for _ in 0..1000 {
        if let Err(e) = client.writer.write_frame(&frame(&body)) {
            ended = Some(e);
            break;
        }
    }
    let e = ended.expect("a peer taking nothing ends a write");
    assert_eq!(e.kind(), ErrorKind::TimedOut, "{e}");
    assert!(started.elapsed() < Duration::from_secs(60));
    done_tx.send(()).unwrap();
    holding.join().unwrap();
}
