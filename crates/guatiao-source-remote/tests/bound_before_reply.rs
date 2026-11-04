// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A message channel judges a message by the bound in force when it
//! arrives, ahead of the reader. A link therefore raises its reader's bound
//! (`FrameRead::set_max_frame`) before the peer can send a frame larger than
//! the handshake's: on the server before its reply goes, on either side
//! before the link is returned.

mod common;

extern crate alloc;

use alloc::sync::Arc;
use core::time::Duration;
use std::io;
use std::sync::Mutex;

use guatiao_source_remote::transport::{Connect, Connection, FrameRead, FrameWrite, memory};
use guatiao_source_remote::{LinkOptions, RemoteSource, Trust};

type Log = Arc<Mutex<Vec<String>>>;

struct Reading {
    inner: Box<dyn FrameRead>,
    log: Log,
    side: &'static str,
}

impl FrameRead for Reading {
    fn read_frame(&mut self, buf: &mut Vec<u8>, max: usize) -> io::Result<bool> {
        let read = self.inner.read_frame(buf, max);
        self.log
            .lock()
            .unwrap()
            .push(format!("{} reads", self.side));
        read
    }

    fn set_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.inner.set_timeout(timeout)
    }

    fn set_max_frame(&mut self, bytes: usize) {
        self.log
            .lock()
            .unwrap()
            .push(format!("{} raises to {bytes}", self.side));
        self.inner.set_max_frame(bytes);
    }

    fn set_max_frame_after(&mut self, messages: usize, bytes: usize) -> bool {
        self.log
            .lock()
            .unwrap()
            .push(format!("{} stages {messages} then {bytes}", self.side));
        self.inner.set_max_frame_after(messages, bytes)
    }

    fn set_read_ahead(&mut self, frames: usize) -> io::Result<()> {
        self.log
            .lock()
            .unwrap()
            .push(format!("{} reads ahead {frames}", self.side));
        self.inner.set_read_ahead(frames)
    }
}

struct Writing {
    inner: Box<dyn FrameWrite>,
    log: Log,
    side: &'static str,
}

impl FrameWrite for Writing {
    fn write_frame(&mut self, frame: &[u8]) -> io::Result<()> {
        self.log
            .lock()
            .unwrap()
            .push(format!("{} writes", self.side));
        self.inner.write_frame(frame)
    }

    fn set_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.inner.set_timeout(timeout)
    }

    fn shutdown(&mut self) {
        self.inner.shutdown();
    }
}

fn watched(conn: Connection, log: &Log, side: &'static str) -> Connection {
    Connection {
        reader: Box::new(Reading {
            inner: conn.reader,
            log: Arc::clone(log),
            side,
        }),
        writer: Box::new(Writing {
            inner: conn.writer,
            log: Arc::clone(log),
            side,
        }),
        peer: conn.peer,
        closer: conn.closer,
    }
}

struct Once(Mutex<Option<Connection>>);

impl Connect for Once {
    fn connect(&self) -> io::Result<Connection> {
        self.0
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| io::Error::other("used"))
    }

    fn describe(&self) -> String {
        "memory".to_string()
    }
}

#[test]
fn a_link_raises_its_readers_bound_before_the_peer_can_send_a_large_frame() {
    let log: Log = Arc::default();
    let (host_end, server_end) = memory::pair();
    let server = common::server(LinkOptions::new(Trust::Local));
    let served = {
        let log = Arc::clone(&log);
        let server = server.clone();
        std::thread::spawn(move || server.serve(watched(server_end, &log, "server")))
    };
    let source = RemoteSource::new(
        Box::new(Once(Mutex::new(Some(watched(host_end, &log, "client"))))),
        LinkOptions::new(Trust::Local),
    );
    let _link = source.link().expect("the link is made");
    let _served = served.join().unwrap().expect("the server's link is made");

    let log = log.lock().unwrap().clone();
    let at = |what: &str| log.iter().position(|e| e == what);
    let raised = at("server raises to 67108864").unwrap_or_else(|| panic!("never raised: {log:?}"));
    let reply = at("server writes").expect("a reply");
    assert!(
        raised < reply,
        "the server raises its bound before its reply goes, which frees the client to send: {log:?}"
    );
    assert!(
        at("client raises to 67108864").is_some(),
        "the client raises its bound before its link is returned: {log:?}"
    );
    // With no PROOF to read, the HELLO admits the client: the server reads
    // ahead before its reply, the client once it has read the reply.
    let ahead = at("server reads ahead 64").unwrap_or_else(|| panic!("{log:?}"));
    assert!(ahead < reply, "{log:?}");
    let client_ahead = at("client reads ahead 64").unwrap_or_else(|| panic!("{log:?}"));
    let client_read = at("client reads").expect("the reply read");
    assert!(client_read < client_ahead, "{log:?}");
}

/// Where a key PROOF is read, the server lets its channel read ahead only
/// once the PROOF holds: a peer not yet proved makes it hold one message.
#[cfg(feature = "tcp")]
#[test]
fn a_server_that_reads_a_proof_reads_ahead_only_after_it() {
    let log: Log = Arc::default();
    let (host_end, server_end) = memory::pair();
    let key = b"a key both ends hold".as_slice();
    let server = common::server(LinkOptions::new(Trust::Trusted).key(key));
    let served = {
        let log = Arc::clone(&log);
        let server = server.clone();
        std::thread::spawn(move || server.serve(watched(server_end, &log, "server")))
    };
    let source = RemoteSource::new(
        Box::new(Once(Mutex::new(Some(watched(host_end, &log, "client"))))),
        LinkOptions::new(Trust::Trusted).key(key),
    );
    let _link = source.link().expect("the link is made");
    let _served = served.join().unwrap().expect("the server's link is made");

    let log = log.lock().unwrap().clone();
    let server_events: Vec<&str> = log
        .iter()
        .map(String::as_str)
        .filter(|e| e.starts_with("server"))
        .collect();
    let ahead = server_events
        .iter()
        .position(|e| *e == "server reads ahead 64")
        .unwrap_or_else(|| panic!("never read ahead: {log:?}"));
    let reads_before = server_events[..ahead]
        .iter()
        .filter(|e| **e == "server reads")
        .count();
    assert_eq!(
        reads_before, 2,
        "the HELLO and the PROOF are read before the channel reads ahead: {log:?}"
    );
    assert_eq!(
        server_events
            .iter()
            .filter(|e| e.starts_with("server reads ahead"))
            .count(),
        1,
        "{log:?}"
    );
}

#[cfg(feature = "tcp")]
/// Where a key PROOF is still to be read, the bound stays the handshake's
/// until the proof is through: a stranger cannot send a large message in
/// its place.
#[test]
fn a_server_that_reads_a_proof_raises_its_bound_only_after_it() {
    let log: Log = Arc::default();
    let (host_end, server_end) = memory::pair();
    let key = b"a key both ends hold".as_slice();
    let server = common::server(LinkOptions::new(Trust::Trusted).key(key));
    let served = {
        let log = Arc::clone(&log);
        let server = server.clone();
        std::thread::spawn(move || server.serve(watched(server_end, &log, "server")))
    };
    let source = RemoteSource::new(
        Box::new(Once(Mutex::new(Some(watched(host_end, &log, "client"))))),
        LinkOptions::new(Trust::Trusted).key(key),
    );
    let _link = source.link().expect("the link is made");
    let _served = served.join().unwrap().expect("the server's link is made");

    let log = log.lock().unwrap().clone();
    let server_events: Vec<&str> = log
        .iter()
        .map(String::as_str)
        .filter(|e| e.starts_with("server"))
        .collect();
    let raised = server_events
        .iter()
        .position(|e| e.contains("raises"))
        .unwrap_or_else(|| panic!("never raised: {log:?}"));
    let reads_before = server_events[..raised]
        .iter()
        .filter(|e| **e == "server reads")
        .count();
    assert_eq!(
        reads_before, 2,
        "the HELLO and the PROOF are read before the bound is raised: {log:?}"
    );
}

/// Where a PROOF is read, the server stages its bound (the PROOF small,
/// what follows large) before its WELCOME goes, which is before the client
/// can send either.
#[cfg(feature = "tcp")]
#[test]
fn a_server_that_reads_a_proof_stages_its_bound_before_the_welcome() {
    let log: Log = Arc::default();
    let (host_end, server_end) = memory::pair();
    let key = b"a key both ends hold".as_slice();
    let server = common::server(LinkOptions::new(Trust::Trusted).key(key));
    let served = {
        let log = Arc::clone(&log);
        let server = server.clone();
        std::thread::spawn(move || server.serve(watched(server_end, &log, "server")))
    };
    let source = RemoteSource::new(
        Box::new(Once(Mutex::new(Some(watched(host_end, &log, "client"))))),
        LinkOptions::new(Trust::Trusted).key(key),
    );
    let _link = source.link().expect("the link is made");
    let _served = served.join().unwrap().expect("the server's link is made");

    let log = log.lock().unwrap().clone();
    let at = |what: &str| log.iter().position(|e| e == what);
    let staged = at("server stages 1 then 67108864").unwrap_or_else(|| panic!("{log:?}"));
    let welcome = at("server writes").expect("a welcome");
    assert!(staged < welcome, "staged before the WELCOME goes: {log:?}");
}
