// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The handshake at server TLS (trust 3), over a transport standing in for
//! one that encrypts and proved the server: the client proves itself by the
//! key when the server's WELCOME carries a proof, else by a token; an
//! empty token and a transport that encrypts nothing are refused at both
//! ends.

#![cfg(feature = "tcp")]

mod common;

use std::io;

use guatiao::Status;
use guatiao_source_remote::transport::{Connect, Connection, FrameRead, memory};
use guatiao_source_remote::{LinkOptions, RemoteSource, Server, Trust};

/// A memory end that says it encrypts and, on the client's side, that it
/// proved the server's certificate.
struct Sealed {
    inner: Box<dyn FrameRead>,
    server: Option<String>,
    encrypted: bool,
}

impl FrameRead for Sealed {
    fn read_frame(&mut self, buf: &mut Vec<u8>, max: usize) -> io::Result<bool> {
        self.inner.read_frame(buf, max)
    }

    fn read_frame_in(
        &mut self,
        buf: &mut Vec<u8>,
        max: usize,
        room: &mut dyn FnMut(usize) -> Vec<u8>,
    ) -> io::Result<bool> {
        self.inner.read_frame_in(buf, max, room)
    }

    fn set_timeout(&mut self, timeout: Option<core::time::Duration>) -> io::Result<()> {
        self.inner.set_timeout(timeout)
    }

    fn peer_id(&self) -> Option<String> {
        self.server.clone()
    }

    fn encrypted(&self) -> bool {
        self.encrypted
    }
}

/// Connects to `server` over memory; `sealed` says whether either end
/// claims the TLS a server-TLS link needs.
struct Through {
    server: Server,
    sealed: bool,
}

impl Connect for Through {
    fn connect(&self) -> io::Result<Connection> {
        let (mut host, mut device) = memory::pair();
        let sealed = self.sealed;
        host.reader = Box::new(Sealed {
            inner: host.reader,
            server: sealed.then(|| "sha256:00".to_string()),
            encrypted: sealed,
        });
        device.reader = Box::new(Sealed {
            inner: device.reader,
            server: None,
            encrypted: sealed,
        });
        let server = self.server.clone();
        std::thread::spawn(move || {
            let _ = server.serve(device);
        });
        Ok(host)
    }

    fn describe(&self) -> String {
        "sealed memory".to_string()
    }
}

fn link(server: &Server, sealed: bool, client: LinkOptions) -> Result<(), (Status, String)> {
    let source = RemoteSource::new(
        Box::new(Through {
            server: server.clone(),
            sealed,
        }),
        client,
    );
    let linked = source
        .link()
        .map(|_| ())
        .map_err(|e| (e.status, e.message().to_string()));
    source.disconnect();
    linked
}

fn at_server_tls() -> LinkOptions {
    LinkOptions::new(Trust::ServerTls)
}

#[test]
fn a_server_with_a_key_asks_for_it_and_a_client_holding_it_links() {
    let server = common::server(at_server_tls().key(b"k"));
    link(&server, true, at_server_tls().key(b"k")).unwrap();
    let (status, why) = link(&server, true, at_server_tls().key(b"other")).unwrap_err();
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE, "{why}");
    assert!(why.contains("key"), "{why}");
    // A client with only a token is asked for a key it does not have.
    let (status, why) = link(&server, true, at_server_tls().token("t")).unwrap_err();
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE, "{why}");
    assert!(why.contains("has none"), "{why}");
}

#[test]
fn a_server_with_a_token_takes_the_token_alone() {
    let server = common::server(at_server_tls().token("open sesame"));
    link(&server, true, at_server_tls().token("open sesame")).unwrap();
    // A client's key is not asked for: the WELCOME carries no proof.
    link(
        &server,
        true,
        at_server_tls().token("open sesame").key(b"k"),
    )
    .unwrap();
    let (status, why) = link(&server, true, at_server_tls().token("wrong")).unwrap_err();
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE, "{why}");
    assert!(why.contains("token"), "{why}");
}

#[test]
fn a_server_with_an_empty_token_admits_nobody() {
    for token in ["", " \t"] {
        let server = common::server(at_server_tls().token(token));
        let (status, why) = link(&server, true, at_server_tls().key(b"k")).unwrap_err();
        assert_eq!(status, Status::GUATIAO_ERR_UNSUPPORTED, "{why}");
        assert!(why.contains("this server's token is empty"), "{why}");
    }
}

#[test]
fn a_client_with_an_empty_token_is_refused_before_it_sends() {
    let server = common::server(at_server_tls().token("t"));
    let (status, why) = link(&server, true, at_server_tls().token("")).unwrap_err();
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE, "{why}");
    assert!(why.contains("the token is empty"), "{why}");
}

#[test]
fn a_server_asking_both_needs_both() {
    let server = common::server(at_server_tls().key(b"k").token("t"));
    link(&server, true, at_server_tls().key(b"k").token("t")).unwrap();
    assert!(link(&server, true, at_server_tls().key(b"k")).is_err());
    assert!(link(&server, true, at_server_tls().token("t")).is_err());
}

#[test]
fn neither_end_runs_server_tls_over_a_transport_that_encrypts_nothing() {
    let server = common::server(at_server_tls().key(b"k"));
    let (status, why) = link(&server, false, at_server_tls().key(b"k")).unwrap_err();
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE, "{why}");
    assert!(why.contains("proves the server"), "{why}");
}

#[test]
fn a_server_refuses_an_unencrypted_client_asking_server_tls() {
    // The client's transport claims to have proved the server; the
    // server's encrypts nothing.
    struct Lying(Server);
    impl Connect for Lying {
        fn connect(&self) -> io::Result<Connection> {
            let (mut host, device) = memory::pair();
            host.reader = Box::new(Sealed {
                inner: host.reader,
                server: Some("sha256:00".to_string()),
                encrypted: true,
            });
            let server = self.0.clone();
            std::thread::spawn(move || {
                let _ = server.serve(device);
            });
            Ok(host)
        }
        fn describe(&self) -> String {
            "lying memory".to_string()
        }
    }
    let server = common::server(at_server_tls().key(b"k"));
    let source = RemoteSource::new(Box::new(Lying(server)), at_server_tls().key(b"k"));
    let e = source.link().unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_UNSUPPORTED, "{}", e.message());
    assert!(e.message().contains("encrypts"), "{}", e.message());
}

#[test]
fn a_server_tls_server_with_neither_key_nor_token_refuses_every_client() {
    let server = common::server(at_server_tls());
    let (status, why) = link(&server, true, at_server_tls().key(b"k")).unwrap_err();
    assert_eq!(status, Status::GUATIAO_ERR_UNSUPPORTED, "{why}");
    assert!(why.contains("has neither"), "{why}");
}

#[test]
fn a_client_with_neither_key_nor_token_is_refused_before_it_sends() {
    let server = common::server(at_server_tls().key(b"k"));
    let (status, why) = link(&server, true, at_server_tls()).unwrap_err();
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE, "{why}");
    assert!(why.contains("key or a token"), "{why}");
}

#[test]
fn a_server_at_another_level_refuses_server_tls_and_names_both() {
    let server = common::server(LinkOptions::new(Trust::Trusted).key(b"k"));
    let (status, why) = link(&server, true, at_server_tls().key(b"k")).unwrap_err();
    assert_eq!(status, Status::GUATIAO_ERR_UNSUPPORTED, "{why}");
    assert!(
        why.contains("trusted") && why.contains("server-tls"),
        "{why}"
    );
}
