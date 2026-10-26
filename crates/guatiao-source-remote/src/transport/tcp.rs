// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! TCP: Nagle off, a bounded connect, and for the trusted level the
//! access control list a listener admits peers by.

#![forbid(unsafe_code)]

use alloc::sync::Arc;
use core::net::SocketAddr;
use core::time::Duration;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};

use coqui_net_acl::{Acl, Outcome};

use super::stream::{StreamReader, StreamWriter};
use super::{Connect, Connection};

/// How long a connect may take unless told otherwise.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// One socket, read on one thread and written and shut down on others.
/// Never a `try_clone`: on Windows a duplicated handle's `shutdown` or
/// read timeout does not reliably reach a read blocked on another.
struct Shared(Arc<TcpStream>);

impl Read for Shared {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        (&*self.0).read(buf)
    }
}

impl Write for Shared {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (&*self.0).write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        (&*self.0).flush()
    }
}

fn connection(stream: TcpStream, peer: String) -> io::Result<Connection> {
    stream.set_nodelay(true)?;
    let stream = Arc::new(stream);
    let shutting = Arc::clone(&stream);
    let closing = Arc::clone(&stream);
    Ok(Connection {
        reader: Box::new(StreamReader::with_timeout(
            Shared(Arc::clone(&stream)),
            |s: &Shared, t| s.0.set_read_timeout(t),
        )),
        writer: Box::new(
            StreamWriter::with_shutdown(Shared(stream), move || {
                let _ = shutting.shutdown(Shutdown::Both);
            })
            .bounded_by(|s: &mut Shared, t| s.0.set_write_timeout(t)),
        ),
        peer,
        closer: Some(Box::new(move || {
            let _ = closing.shutdown(Shutdown::Both);
        })),
    })
}

/// A connection over a stream the caller connected itself, with socket
/// options of its own.
pub fn from_stream(stream: TcpStream) -> io::Result<Connection> {
    let peer = format!("tcp {}", stream.peer_addr()?);
    connection(stream, peer)
}

/// Connects to a TCP address (`host:port`).
#[derive(Debug, Clone)]
pub struct TcpConnect {
    address: String,
    timeout: Duration,
}

impl TcpConnect {
    /// To `address`, within [`CONNECT_TIMEOUT`].
    pub fn new(address: &str) -> TcpConnect {
        TcpConnect {
            address: address.to_string(),
            timeout: CONNECT_TIMEOUT,
        }
    }

    /// Within `timeout` per address tried.
    pub fn timeout(mut self, timeout: Duration) -> TcpConnect {
        self.timeout = timeout;
        self
    }
}

impl Connect for TcpConnect {
    fn connect(&self) -> io::Result<Connection> {
        let mut last = io::Error::new(io::ErrorKind::NotFound, "no address to connect to");
        for addr in self.address.to_socket_addrs()? {
            match TcpStream::connect_timeout(&addr, self.timeout) {
                Ok(stream) => return connection(stream, format!("tcp {addr}")),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    fn describe(&self) -> String {
        format!("tcp {}", self.address)
    }
}

/// A TCP listener that admits peers by an access control list: the
/// trusted level's gate. Where it listens is the caller's choice.
#[derive(Debug)]
pub struct TcpListen {
    listener: TcpListener,
    acl: Acl,
}

/// What [`TcpListen::accept`] took in.
#[derive(Debug)]
#[non_exhaustive]
pub enum Accepted {
    /// A connection the access control list admits.
    Connection(Connection),
    /// One it denies, closed unread.
    Refused(SocketAddr),
}

/// Whether `acl` admits a peer at `source` that proves no identity, before
/// anything is read. A rule naming a fingerprint, whatever it names, cannot
/// match such a peer: the rules without one decide.
pub(crate) fn admits_plain(acl: &Acl, source: SocketAddr, local: SocketAddr) -> bool {
    let plain = Acl {
        rules: acl
            .rules
            .iter()
            .filter(|r| r.fingerprint.is_none())
            .cloned()
            .collect(),
        default: acl.default,
    };
    plain.decide(source, local).outcome == Outcome::Permit
}

impl TcpListen {
    /// Binds `address`, any address a wildcard (`0.0.0.0`, `::`)
    /// included, admitting the peers `acl` permits. Nothing about the
    /// address or the list is refused: an empty list denies every peer.
    pub fn bind(address: &str, acl: Acl) -> io::Result<TcpListen> {
        let addrs: Vec<SocketAddr> = address.to_socket_addrs()?.collect();
        let addr = addrs
            .first()
            .copied()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no address to bind"))?;
        Ok(TcpListen {
            listener: TcpListener::bind(addr)?,
            acl,
        })
    }

    /// A listener the caller bound itself (with socket options of its
    /// own, or handed over by a service manager), admitting peers by
    /// `acl` as [`bind`](TcpListen::bind) does.
    pub fn from_listener(listener: TcpListener, acl: Acl) -> TcpListen {
        TcpListen { listener, acl }
    }

    /// Where it listens.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// The access control list it admits peers by.
    pub fn acl(&self) -> &Acl {
        &self.acl
    }

    /// The next connection, or a peer the list denies, turned away.
    pub fn accept(&self) -> io::Result<Accepted> {
        Ok(match self.accept_stream()? {
            Ok((stream, peer)) => Accepted::Connection(connection(stream, format!("tcp {peer}"))?),
            Err(refused) => Accepted::Refused(refused),
        })
    }

    /// The next stream the list admits, or the address of a peer it
    /// denies, closed unread: what a transport set up over TCP (RDMA,
    /// WebSocket) accepts through.
    #[cfg_attr(not(any(feature = "rdma", feature = "ws")), allow(dead_code))]
    pub(crate) fn accept_stream(&self) -> io::Result<Result<(TcpStream, SocketAddr), SocketAddr>> {
        let (stream, peer) = self.listener.accept()?;
        let local = stream.local_addr()?;
        if !admits_plain(&self.acl, peer, local) {
            let _ = stream.shutdown(Shutdown::Both);
            return Ok(Err(peer));
        }
        Ok(Ok((stream, peer)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coqui_net_acl::{Action, Rule};

    fn permit(sources: &[&str]) -> Acl {
        let mut acl = Acl::default();
        for s in sources {
            acl.push(Rule::new(Action::Permit, s.parse().unwrap()));
        }
        acl
    }

    #[test]
    fn a_listener_binds_where_it_is_told_and_admits_by_its_list() {
        // Every interface: the peer at 127.0.0.1 is permitted.
        let any = TcpListen::bind("0.0.0.0:0", permit(&["127.0.0.0/8"])).expect("a wildcard binds");
        let port = any.local_addr().unwrap().port();
        let _c = TcpStream::connect(("127.0.0.1", port)).unwrap();
        assert!(matches!(any.accept().unwrap(), Accepted::Connection(_)));

        // Its own address outside the rules, as when peers route to it.
        let routed =
            TcpListen::bind("127.0.0.1:0", permit(&["10.0.0.0/8"])).expect("any address binds");
        let _c = TcpStream::connect(routed.local_addr().unwrap()).unwrap();
        assert!(matches!(routed.accept().unwrap(), Accepted::Refused(_)));

        // An empty list binds, and denies every peer.
        let none = TcpListen::bind("127.0.0.1:0", Acl::default()).expect("an empty list binds");
        let _c = TcpStream::connect(none.local_addr().unwrap()).unwrap();
        assert!(matches!(none.accept().unwrap(), Accepted::Refused(_)));
    }

    #[test]
    fn the_first_matching_rule_decides_and_a_fingerprint_rule_never_admits_a_plain_peer() {
        let source: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        let local: SocketAddr = "127.0.0.1:7000".parse().unwrap();
        let mut acl = permit(&["127.0.0.0/8"]);
        acl.prepend(Rule::new(Action::Deny, "127.0.0.1".parse().unwrap()));
        assert!(!admits_plain(&acl, source, local));
        let mut named = Rule::new(Action::Permit, "any".parse().unwrap());
        named.fingerprint = Some(format!("sha256:{}", "ab".repeat(32)));
        let mut acl = Acl::default();
        acl.push(named);
        assert!(
            !admits_plain(&acl, source, local),
            "no fingerprint, no match: the default denies"
        );
        let mut empty = Rule::new(Action::Permit, "any".parse().unwrap());
        empty.fingerprint = Some(String::new());
        acl.prepend(empty);
        assert!(
            !admits_plain(&acl, source, local),
            "not even a rule naming an empty fingerprint"
        );
        acl.push(Rule::new(Action::Permit, "127.0.0.1".parse().unwrap()));
        assert!(
            admits_plain(&acl, source, local),
            "a later rule without one decides"
        );
        let mut by_port = Rule::new(Action::Permit, "any".parse().unwrap());
        by_port.port = Some("7000".parse().unwrap());
        let acl = Acl {
            rules: vec![by_port],
            default: Action::Deny,
        };
        assert!(admits_plain(&acl, source, local));
        assert!(!admits_plain(
            &acl,
            source,
            "127.0.0.1:7001".parse().unwrap()
        ));
    }

    #[test]
    fn sockets_the_caller_set_up_carry_frames_under_the_same_gate() {
        let l = TcpListen::from_listener(
            TcpListener::bind("127.0.0.1:0").unwrap(),
            permit(&["127.0.0.0/8"]),
        );
        let mut client = from_stream(TcpStream::connect(l.local_addr().unwrap()).unwrap()).unwrap();
        assert!(client.peer.starts_with("tcp 127.0.0.1:"));
        let Accepted::Connection(mut server) = l.accept().unwrap() else {
            panic!("a permitted peer is accepted");
        };
        client.writer.write_frame(&[3, 0, 0, 0, 1, 2, 3]).unwrap();
        let mut frame = Vec::new();
        assert!(server.reader.read_frame(&mut frame, 1 << 10).unwrap());
        assert_eq!(frame, [1, 2, 3]);
    }

    #[test]
    fn a_write_to_a_peer_that_reads_nothing_gives_up_at_its_bound() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut c = from_stream(TcpStream::connect(l.local_addr().unwrap()).unwrap()).unwrap();
        let (_unread, _) = l.accept().unwrap();
        c.writer
            .set_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        let mut frame = vec![0u8; 16 << 20];
        frame[..4].copy_from_slice(&((16u32 << 20) - 4).to_le_bytes());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // Far more than the sockets' buffers hold.
            let written = (0..16).try_for_each(|_| c.writer.write_frame(&frame));
            let _ = tx.send(written);
        });
        let written = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the write gives up");
        assert!(written.is_err());
    }

    #[test]
    fn a_peer_outside_the_ranges_is_turned_away_unread() {
        use std::io::Read as _;
        // Permitting only 127.0.0.2: the loopback client at 127.0.0.1 is
        // denied, and finds the connection closed before anything is read.
        let l = TcpListen::bind("127.0.0.1:0", permit(&["127.0.0.2"])).unwrap();
        let mut c = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        assert!(matches!(l.accept().unwrap(), Accepted::Refused(_)));
        c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut byte = [0u8; 1];
        assert!(matches!(c.read(&mut byte), Ok(0) | Err(_)));
    }
}
