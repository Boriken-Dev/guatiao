// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `guatiao-remote-server` facing many connections: past its bound on
//! handshakes a connection is closed unread, and a source the access
//! control list denies is reported once, not once per connection.

#![cfg(feature = "tcp")]

mod common;

use core::time::Duration;
use std::io::{ErrorKind, Read};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::mpsc::Receiver;
use std::time::Instant;

use common::procs;
use guatiao::library::Registry;
use guatiao::value::convert::TryAsRef;
use guatiao_source_remote::transport::tcp::TcpConnect;
use guatiao_source_remote::{LinkOptions, RemoteSource, Trust};

const KEY: &[u8] = b"server-front-key";

/// Kills the server however the test ends.
struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Starts a trusted server on a loopback port with `args` added; answers
/// it, its output and its port.
fn start(name: &str, args: &[&str]) -> (Running, Receiver<String>, u16) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let key = dir.join("link.key");
    std::fs::write(&key, KEY).unwrap();
    let (child, lines) = procs::start(
        Command::new(env!("CARGO_BIN_EXE_guatiao-remote-server"))
            .args(["--listen", "tcp://127.0.0.1:0", "--trust", "trusted"])
            .arg("--key-file")
            .arg(&key)
            .args(args),
    );
    let child = Running(child);
    let first = lines
        .recv_timeout(procs::DEADLINE)
        .expect("the server says where it serves");
    let port = first
        .split("serving tcp://127.0.0.1:")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .and_then(|p| p.parse().ok())
        .unwrap_or_else(|| panic!("no port in `{first}`"));
    (child, lines, port)
}

/// Whether the server closed `conn` without answering, within `within`.
fn closed_unread(conn: &mut TcpStream, within: Duration) -> bool {
    conn.set_read_timeout(Some(within)).unwrap();
    let mut byte = [0u8; 1];
    match conn.read(&mut byte) {
        Ok(0) => true,
        Ok(_) => false,
        Err(e) => matches!(
            e.kind(),
            ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted
        ),
    }
}

/// What the server printed within `wait`.
fn said(lines: &Receiver<String>, wait: Duration) -> Vec<String> {
    let until = Instant::now() + wait;
    let mut out = Vec::new();
    while let Some(left) = until.checked_duration_since(Instant::now()) {
        match lines.recv_timeout(left) {
            Ok(line) => out.push(line),
            Err(_) => break,
        }
    }
    out
}

#[test]
fn past_the_bound_on_handshakes_a_connection_is_closed_unread() {
    let Some(derived) = procs::library("derived_greeter") else {
        panic!("derived_greeter is not built: `cargo build -p derived_greeter` first");
    };
    let kinds = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/greeter_kind/include/greeter_kind.kinds.json");
    let (_server, lines, port) = start(
        "handshake_bound",
        &[
            "--range",
            "127.0.0.1/32",
            "--max-handshakes",
            "1",
            "--kinds",
            kinds.to_str().unwrap(),
            "--load",
            derived.to_str().unwrap(),
        ],
    );
    // A connection that never says HELLO holds the one handshake for as
    // long as the handshake may take (10 s).
    // Accepted in order, it holds the place before the next is accepted.
    let silent = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut second = TcpStream::connect(("127.0.0.1", port)).unwrap();
    assert!(
        closed_unread(&mut second, Duration::from_secs(5)),
        "a connection past the bound was kept"
    );
    let printed = said(&lines, Duration::from_millis(500));
    assert!(
        printed
            .iter()
            .any(|l| l.contains("at most 1 handshakes run at once: closed tcp 127.0.0.1:")),
        "{printed:?}"
    );

    // Once the silent one is gone, a host links and calls.
    drop(silent);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let source = RemoteSource::new(
            Box::new(TcpConnect::new(&format!("127.0.0.1:{port}"))),
            LinkOptions::new(Trust::Trusted).key(KEY),
        );
        let mut registry = Registry::new("host", "1.0");
        registry.add_source("remote", source).unwrap();
        let refreshed = registry.refresh();
        if refreshed.refused.is_empty() {
            assert!(!refreshed.loaded.is_empty(), "{refreshed:?}");
            let greeter = registry
                .offer::<dyn greeter_kind::Greeter>("derived_greeter_hello")
                .expect("the greeter")
                .unwrap()
                .shared()
                .unwrap();
            let answer = greeter.greet("ana").unwrap();
            let greeting = answer.get("greeting").and_then(TryAsRef::<str>::try_as_ref);
            assert_eq!(greeting, Some("hello, ana"));
            break;
        }
        assert!(Instant::now() < deadline, "{refreshed:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_denied_source_is_reported_once_however_often_it_connects() {
    let (_server, lines, port) = start("denied_once", &["--range", "192.0.2.0/24"]);
    for _ in 0..5 {
        let mut conn = TcpStream::connect(("127.0.0.1", port)).unwrap();
        assert!(closed_unread(&mut conn, Duration::from_secs(5)));
    }
    let printed = said(&lines, Duration::from_millis(500));
    let reports = printed
        .iter()
        .filter(|l| l.contains("the access control list denies 127.0.0.1"))
        .count();
    assert_eq!(reports, 1, "{printed:?}");
}
