// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! WebSocket (RFC 6455), what a browser can open, through coqui-web:
//! `ws://` with no TLS, on loopback unless told otherwise, and `wss://`
//! over a server certificate (feature `tls`). Each frame's body is one binary message,
//! with no length in front. A listener admits peers by an access control
//! list as [`TcpListen`](super::tcp::TcpListen) does, by address, before
//! the upgrade; a connection upgrades on its first read, on the thread
//! serving it.
//!
//! coqui-web runs the TLS of a `wss://` link itself, from a rustls
//! configuration, and reports the certificate each side presented
//! (`peer_certificates`), by which a link names its peer at mutual TLS.
//! The handshake of a connection runs on the thread serving it, which is
//! given the connection's addresses for the policy's list.
//!
//! A plain `ws://` beyond loopback carries nothing secret and is a
//! weakening: [`WsListen::bind_beyond_loopback`] and
//! [`WsConnect::beyond_loopback`] choose it, and a host reports it.

#![forbid(unsafe_code)]

use alloc::sync::Arc;
use core::net::SocketAddr;
use core::time::Duration;
use std::io;

use coqui_net_acl::{Acl, Outcome};
use coqui_web::websocket::{Dialer as WsDialer, Frames, Listener};
use coqui_web::{Channel, Dialer as _, Failure, Kind, Limits};
use url::Url;

use super::tcp::{Accepted, CONNECT_TIMEOUT, bind_address};
use super::web::{Open, Refused, Shared, connection, io_error};
use super::{Connect, Connection};

/// How often a listener looks up from waiting for a connection to report
/// the sources its list turned away.
const REPORT_EVERY: Duration = Duration::from_millis(100);

fn invalid(why: impl core::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, why.to_string())
}

/// The bounds of a link's channel: coqui-web's, with the largest message
/// a link allows set by the link as it reads.
fn limits() -> Limits {
    Limits::default()
}

/// Connects to a WebSocket URL: `ws://host:port/path` (loopback unless
/// [`beyond_loopback`](WsConnect::beyond_loopback)), or
/// `wss://host:port/path` once given TLS (`WsConnect::tls`).
#[derive(Clone)]
pub struct WsConnect {
    url: String,
    timeout: Duration,
    beyond_loopback: bool,
    #[cfg(feature = "tls")]
    tls: Option<Wss>,
}

/// What a `wss://` connect dials with: the client configuration, the
/// verifier it checks the server by.
#[cfg(feature = "tls")]
#[derive(Clone)]
struct Wss {
    config: Arc<rustls::ClientConfig>,
    verifier: Arc<dyn rustls::client::danger::ServerCertVerifier>,
    /// Whether the server's certificate is checked.
    proves: bool,
}

impl core::fmt::Debug for WsConnect {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WsConnect").field("url", &self.url).finish()
    }
}

impl WsConnect {
    /// To `url`, connecting and upgrading within [`CONNECT_TIMEOUT`].
    pub fn new(url: &str) -> WsConnect {
        WsConnect {
            url: url.to_string(),
            timeout: CONNECT_TIMEOUT,
            beyond_loopback: false,
            #[cfg(feature = "tls")]
            tls: None,
        }
    }

    /// Lets a plain `ws://` URL name a host beyond this machine: nothing
    /// is encrypted, and the key proof alone decides the link. A
    /// weakening, which a host reports ([`plain_beyond_loopback`](Self::plain_beyond_loopback)).
    #[must_use]
    pub fn beyond_loopback(mut self) -> WsConnect {
        self.beyond_loopback = true;
        self
    }

    /// Whether a plain `ws://` beyond loopback is allowed.
    pub fn plain_beyond_loopback(&self) -> bool {
        self.beyond_loopback
    }

    /// Over TLS as `config` says (a `wss://` URL): the server known by its
    /// pins, the trust store or the roots (for the URL's host). The TLS
    /// offers HTTP's application protocol, as a browser does, and requires
    /// none.
    #[cfg(feature = "tls")]
    pub fn tls(mut self, config: &super::tls::TlsConfig) -> io::Result<WsConnect> {
        let verifier = config
            .server_trust()?
            .server_cert_verifier_with(config.tls_policy())
            .map_err(|e| super::tls::unbuilt(&e))?;
        self.tls = Some(Wss {
            config: config.client_naming(super::tls::WEBSOCKET_ALPN)?,
            verifier,
            proves: config.proves_peers(),
        });
        let url = self.parsed()?;
        if let Some(host) = url.host_str() {
            super::tls::server_name(host.trim_matches(['[', ']']), config.by_roots())?;
        }
        Ok(self)
    }

    /// Within `timeout` as a whole: the connection, the TLS and the upgrade.
    pub fn timeout(mut self, timeout: Duration) -> WsConnect {
        self.timeout = timeout;
        self
    }

    fn encrypted(&self) -> bool {
        #[cfg(feature = "tls")]
        return self.tls.is_some();
        #[cfg(not(feature = "tls"))]
        false
    }

    fn parsed(&self) -> io::Result<Url> {
        let url =
            Url::parse(&self.url).map_err(|_| invalid(format!("`{}` is not a URL", self.url)))?;
        match (url.scheme(), self.encrypted()) {
            ("ws", false) | ("wss", true) => {}
            ("ws", true) => {
                return Err(invalid(format!(
                    "`{}` carries no TLS, and this connect was given TLS: connect to wss://",
                    self.url
                )));
            }
            ("wss", false) => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    if cfg!(feature = "tls") {
                        "wss:// runs TLS: give the connect a TLS configuration (`WsConnect::tls`)"
                    } else {
                        "wss:// runs TLS, which this build lacks (guatiao-source-remote's `tls` feature)"
                    },
                ));
            }
            _ => {
                return Err(invalid(format!(
                    "`{}` is not a ws:// or wss:// URL",
                    self.url
                )));
            }
        }
        if url.host_str().is_none() {
            return Err(invalid(format!("`{}` names no host", self.url)));
        }
        Ok(url)
    }

    /// A dial's failure in words a person can act on; `rejection` is the
    /// refusal of the server's certificate, when there was one.
    fn refused(&self, failure: Failure, url: &Url, rejection: Option<io::Error>) -> io::Error {
        if let Some(rejection) = rejection {
            return rejection;
        }
        let addr = format!(
            "{}:{}",
            url.host_str().unwrap_or_default(),
            url.port_or_known_default().unwrap_or_default()
        );
        match failure {
            Failure::Permission if !self.encrypted() => io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "`ws://` reaches only this machine (loopback) unless the connection allows a plain WebSocket beyond it (`plain_beyond_loopback`), and {addr} is not loopback: connect to wss://"
                ),
            ),
            Failure::Timeout => io::Error::new(
                io::ErrorKind::TimedOut,
                format!("the WebSocket server at {addr} did not answer in time"),
            ),
            Failure::PathLost => io::Error::new(
                io::ErrorKind::ConnectionRefused,
                format!("the WebSocket server at {addr} did not accept the connection"),
            ),
            other => io_error(other),
        }
    }
}

impl Connect for WsConnect {
    fn connect(&self) -> io::Result<Connection> {
        let url = self.parsed()?;
        let mut dialer = WsDialer::new()
            .with_frames(Frames::Binary)
            .with_limits(limits());
        if self.beyond_loopback {
            dialer = dialer.with_plain_beyond_loopback();
        }
        #[cfg(feature = "tls")]
        let watching = self.tls.as_ref().map(|wss| {
            // One verifier for this connection, so what it saw is this
            // connection's.
            let watching = super::web::Watching::new(Arc::clone(&wss.verifier));
            let mut config = (*wss.config).clone();
            config.dangerous().set_certificate_verifier(
                Arc::clone(&watching) as Arc<dyn rustls::client::danger::ServerCertVerifier>
            );
            dialer = core::mem::take(&mut dialer).with_tls(Arc::new(config));
            watching
        });
        #[cfg(feature = "tls")]
        let rejected = || {
            let host = url.host_str().unwrap_or_default().trim_matches(['[', ']']);
            let addr = format!("{host}:{}", url.port_or_known_default().unwrap_or_default());
            watching.as_ref().and_then(|w| w.rejection(&addr, host))
        };
        #[cfg(not(feature = "tls"))]
        let rejected = || None;
        let channel = dialer
            .dial(Kind::WebSocket, &url, self.timeout)
            .map_err(|failure| self.refused(failure, &url, rejected()))?;
        #[cfg(feature = "tls")]
        let names_peer = self.tls.as_ref().is_some_and(|wss| wss.proves);
        #[cfg(not(feature = "tls"))]
        let names_peer = false;
        let shared = Shared::new(self.encrypted(), false, names_peer);
        shared.open(Arc::new(channel) as Arc<dyn Channel>);
        let scheme = if self.encrypted() { "wss" } else { "ws" };
        let at = format!(
            "{}:{}",
            url.host_str().unwrap_or_default(),
            url.port_or_known_default().unwrap_or_default()
        );
        Ok(connection(shared, None, format!("{scheme} {at}")))
    }

    fn describe(&self) -> String {
        format!("ws {}", self.url)
    }
}

/// A WebSocket listener that admits peers by an access control list, as
/// [`TcpListen`](super::tcp::TcpListen) does, or by a TLS configuration's
/// (`bind_tls`). It reads nothing from a connection it hands over: the
/// upgrade, and over TLS the handshake before it, run on the first read,
/// so a client that connects and stalls holds only the thread serving it,
/// for the handshake's bound; a write before then is refused.
pub struct WsListen {
    listener: Listener,
    refused: Arc<Refused>,
    encrypted: bool,
    /// Whether clients present a certificate, which a link names them by.
    mutual: bool,
    /// The address it listens on, which the list's rules can name.
    #[cfg_attr(not(feature = "tls"), allow(dead_code))]
    bound: SocketAddr,
    beyond_loopback: bool,
}

impl core::fmt::Debug for WsListen {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WsListen")
            .field("addr", &self.listener.local_addr().ok())
            .field("encrypted", &self.encrypted)
            .field("plain_beyond_loopback", &self.beyond_loopback)
            .finish()
    }
}

/// A bind's failure in words: coqui-web refuses a plain listener beyond
/// loopback.
fn bind_failed(failure: Failure, address: SocketAddr, tls: bool) -> io::Error {
    match failure {
        Failure::Permission if !tls => io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "ws:// listens on loopback only unless told it may not (`plain_beyond_loopback`), and {address} is not: listen on wss:// to be reached from another machine"
            ),
        ),
        other => io_error(other),
    }
}

impl WsListen {
    /// Binds `address`, which must be loopback (use
    /// [`bind_tls`](Self::bind_tls), or
    /// [`bind_beyond_loopback`](Self::bind_beyond_loopback), to be reached
    /// from another machine), admitting the peers `acl` permits.
    pub fn bind(address: &str, acl: Acl) -> io::Result<WsListen> {
        WsListen::plain(address, acl, false)
    }

    /// Binds `address`, any address a wildcard included, with no TLS:
    /// nothing is encrypted and the key proof and the list alone decide
    /// who links. A weakening, which a host reports
    /// ([`plain_beyond_loopback`](Self::plain_beyond_loopback)).
    pub fn bind_beyond_loopback(address: &str, acl: Acl) -> io::Result<WsListen> {
        WsListen::plain(address, acl, true)
    }

    /// Whether this listener may be reached beyond loopback with nothing
    /// encrypted.
    pub fn plain_beyond_loopback(&self) -> bool {
        self.listener.plain_beyond_loopback()
    }

    fn plain(address: &str, acl: Acl, beyond: bool) -> io::Result<WsListen> {
        let address = bind_address(address)?;
        let listener = if beyond {
            Listener::bind_plain(address)
        } else {
            Listener::bind(address, None)
        }
        .map_err(|f| bind_failed(f, address, false))?;
        let beyond_loopback = listener.plain_beyond_loopback();
        let bound = listener.local_addr().map_err(io_error)?;
        let refused = Arc::new(Refused::default());
        let turned = Arc::clone(&refused);
        let listener = listener
            .with_frames(Frames::Binary)
            .with_limits(limits())
            .with_admission(move |source| {
                // A peer proving no identity: a rule naming a fingerprint
                // never matches it.
                let permitted = acl.decide_anonymous(source, bound).outcome == Outcome::Permit;
                if !permitted {
                    turned.add(source);
                }
                permitted
            });
        Ok(WsListen {
            listener,
            refused,
            encrypted: false,
            mutual: false,
            bound,
            beyond_loopback,
        })
    }

    /// Binds `address` for `wss://`: the TLS `config` says (its identity,
    /// its policy, whose `acl` admits peers by address before the
    /// handshake). The TLS names HTTP's application protocol, as a browser
    /// offers it, and requires none. A mutual-TLS configuration asks the
    /// client for a certificate, trusts it as the configuration says, and
    /// names it to the link.
    #[cfg(feature = "tls")]
    pub fn bind_tls(address: &str, config: &super::tls::TlsConfig) -> io::Result<WsListen> {
        let address = bind_address(address)?;
        let mutual = config.is_mutual() && config.proves_peers();
        // The handshake runs on the thread serving the connection, which
        // is given its addresses: the list stays in the TLS.
        let tls = super::web::server_config(config, super::tls::WEBSOCKET_ALPN, None, true)?;
        let listener =
            Listener::bind(address, Some(tls)).map_err(|f| bind_failed(f, address, true))?;
        let bound = listener.local_addr().map_err(io_error)?;
        let refused = Arc::new(Refused::default());
        let turned = Arc::clone(&refused);
        let config = config.clone();
        let listener = listener
            .with_frames(Frames::Binary)
            .with_limits(limits())
            .with_admission(move |source| {
                let permitted = super::web::admits(&config, source, bound);
                if !permitted {
                    turned.add(source);
                }
                permitted
            });
        Ok(WsListen {
            listener,
            refused,
            encrypted: true,
            mutual,
            bound,
            beyond_loopback: false,
        })
    }

    /// Where it listens.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr().map_err(io_error)
    }

    /// The next connection, or a peer the list denies. The upgrade is not
    /// read: it runs on the connection's first read.
    pub fn accept(&self) -> io::Result<Accepted> {
        loop {
            if let Some(source) = self.refused.take() {
                return Ok(Accepted::Refused(source));
            }
            match self.listener.next(Some(REPORT_EVERY)) {
                Ok(pending) => {
                    let scheme = if self.encrypted { "wss" } else { "ws" };
                    let source = pending.peer_addr();
                    let peer = format!("{scheme} {source}");
                    let cancel = pending.canceller();
                    #[cfg(feature = "tls")]
                    let (mutual, bound) = (self.mutual, self.bound);
                    let open: Open = Box::new(move || {
                        // At mutual TLS the list reads this connection's
                        // addresses from the scope around the handshake,
                        // which runs on this thread alone.
                        #[cfg(feature = "tls")]
                        if mutual {
                            return coqui_peer_tls::with_addresses(
                                Some(source),
                                Some(bound),
                                || pending.handshake(),
                            )
                            .map(|channel| Arc::new(channel) as Arc<dyn Channel>)
                            .map_err(io_error);
                        }
                        pending
                            .handshake()
                            .map(|channel| Arc::new(channel) as Arc<dyn Channel>)
                            .map_err(io_error)
                    });
                    let shared = Shared::new(self.encrypted, false, self.mutual);
                    return Ok(Accepted::Connection(connection(
                        shared,
                        Some((open, cancel)),
                        peer,
                    )));
                }
                Err(Failure::Timeout) => {}
                Err(failure) => return Err(io_error(failure)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;

    fn listener() -> WsListen {
        let mut acl = Acl::default();
        acl.push(coqui_net_acl::Rule::new(
            coqui_net_acl::Action::Permit,
            "127.0.0.1".parse().unwrap(),
        ));
        WsListen::bind("127.0.0.1:0", acl).unwrap()
    }

    fn accepted(l: &WsListen) -> Connection {
        match l.accept().unwrap() {
            Accepted::Connection(c) => c,
            Accepted::Refused(peer) => panic!("{peer} refused"),
        }
    }

    #[test]
    fn a_browser_upgrade_is_answered_on_the_first_read() {
        let l = listener();
        let mut raw = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        raw.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        // Accepting reads nothing: a client that says nothing holds no one.
        let Connection { mut reader, .. } = accepted(&l);
        let serving = std::thread::spawn(move || {
            let mut frame = Vec::new();
            reader.read_frame(&mut frame, 1 << 10)
        });
        raw.write_all(
            b"GET /chat HTTP/1.1\r\nHost: 127.0.0.1\r\nUpgrade: websocket\r\n\
              Connection: keep-alive, Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
              Origin: http://example.com\r\nSec-WebSocket-Version: 13\r\n\r\n",
        )
        .unwrap();
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            raw.read_exact(&mut byte).unwrap();
            head.push(byte[0]);
        }
        let head = String::from_utf8(head).unwrap();
        assert!(head.starts_with("HTTP/1.1 101"), "{head}");
        assert!(
            head.contains("s3pPLMBiTxaQ9kYGzzhZRbK+xOo="),
            "RFC 6455's sample answer: {head}"
        );
        // A close ends the reads cleanly.
        raw.write_all(&[0x88, 0x82, 1, 2, 3, 4, 0x03 ^ 1, 0xe8 ^ 2])
            .unwrap();
        assert!(!serving.join().unwrap().unwrap());
    }

    #[test]
    fn a_write_before_the_upgrade_is_refused() {
        let l = listener();
        let _raw = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        let mut conn = accepted(&l);
        let refused = conn.writer.write_frame(&[1, 0, 0, 0, 9]).unwrap_err();
        assert_eq!(refused.kind(), io::ErrorKind::NotConnected);
    }

    #[test]
    fn a_client_and_a_server_carry_frames_both_ways_within_their_bound() {
        let l = listener();
        let addr = l.local_addr().unwrap();
        // A client that connects and says nothing, first.
        let _silent = TcpStream::connect(addr).unwrap();
        let _held = accepted(&l);
        let client = std::thread::spawn(move || {
            WsConnect::new(&format!("ws://{addr}/any/path"))
                .connect()
                .unwrap()
        });
        let mut server = accepted(&l);
        let (tx, rx) = std::sync::mpsc::channel();
        let reading = std::thread::spawn(move || {
            let mut frame = Vec::new();
            let mut got = Vec::new();
            for _ in 0..3 {
                assert!(server.reader.read_frame(&mut frame, 2 << 20).unwrap());
                got.push(frame.clone());
            }
            tx.send(got).unwrap();
            server.writer.write_frame(&[2, 0, 0, 0, 4, 5]).unwrap();
            let over = server.reader.read_frame(&mut frame, 1 << 10);
            (server, over)
        });
        let mut client = client.join().unwrap();
        assert!(client.peer.starts_with("ws 127.0.0.1:"), "{}", client.peer);
        client
            .writer
            .write_frames(&[&[1, 0, 0, 0, 1], &[2, 0, 0, 0, 2, 3]])
            .unwrap();
        let tail = vec![0xab; 512 << 10];
        let len = u32::try_from(1 + tail.len()).unwrap();
        let mut head = len.to_le_bytes().to_vec();
        head.push(7);
        client.writer.write_frame_parts(&head, &tail).unwrap();
        let got = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(got[0], [1]);
        assert_eq!(got[1], [2, 3]);
        assert_eq!(got[2].len(), 1 + tail.len());
        assert_eq!(got[2][0], 7);
        let mut back = Vec::new();
        assert!(client.reader.read_frame(&mut back, 1 << 20).unwrap());
        assert_eq!(back, [4, 5]);
        // A message past the bound the reader set ends the channel.
        client.writer.write_frame(&[0u8; 4 + 2048]).unwrap();
        let (_server, over) = reading.join().unwrap();
        assert_eq!(over.unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_url_that_is_not_ws_is_refused() {
        let kind = |url: &str| WsConnect::new(url).connect().unwrap_err().kind();
        assert_eq!(kind("wss://127.0.0.1:1/"), io::ErrorKind::Unsupported);
        assert_eq!(kind("http://127.0.0.1:1/"), io::ErrorKind::InvalidInput);
        assert_eq!(kind("tcp://127.0.0.1:1"), io::ErrorKind::InvalidInput);
        assert_eq!(kind("not a url"), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn plain_websocket_is_for_this_machine_alone() {
        let e = WsListen::bind("0.0.0.0:0", Acl::default()).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::PermissionDenied, "{e}");
        assert!(e.to_string().contains("loopback"), "{e}");
    }
}
