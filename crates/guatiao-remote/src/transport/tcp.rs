// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! TCP: Nagle off, a bounded connect, and for the trusted level the
//! address ranges a listener accepts peers from.

#![forbid(unsafe_code)]

use alloc::sync::Arc;
use core::net::{IpAddr, SocketAddr};
use core::time::Duration;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};

use super::stream::{StreamReader, StreamWriter};
use super::{Connect, Connection};

/// How long a connect may take unless told otherwise.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// An address range, `192.0.2.0/24` or `fd00::/8`; a bare address is a
/// range of one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    base: IpAddr,
    bits: u8,
}

impl Range {
    /// Reads `address[/bits]`.
    pub fn parse(text: &str) -> Result<Range, String> {
        let (addr, bits) = match text.split_once('/') {
            Some((a, b)) => (a, Some(b)),
            None => (text, None),
        };
        let base: IpAddr = addr
            .trim()
            .parse()
            .map_err(|_| format!("`{text}` is not an address range"))?;
        let max = if base.is_ipv4() { 32 } else { 128 };
        let bits = match bits {
            None => max,
            Some(b) => b
                .trim()
                .parse::<u8>()
                .ok()
                .filter(|&b| b <= max)
                .ok_or_else(|| format!("`{text}`: the prefix length is not 0 to {max}"))?,
        };
        Ok(Range { base, bits })
    }

    /// Whether `addr` lies in the range. An IPv4 address mapped into IPv6
    /// is read as the IPv4 address it is.
    pub fn contains(&self, addr: IpAddr) -> bool {
        let addr = match addr {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(addr, IpAddr::V4),
            v4 => v4,
        };
        match (self.base, addr) {
            (IpAddr::V4(b), IpAddr::V4(a)) => {
                let mask = u32::MAX.checked_shl(32 - u32::from(self.bits)).unwrap_or(0);
                u32::from(b) & mask == u32::from(a) & mask
            }
            (IpAddr::V6(b), IpAddr::V6(a)) => {
                let mask = u128::MAX
                    .checked_shl(128 - u32::from(self.bits))
                    .unwrap_or(0);
                u128::from(b) & mask == u128::from(a) & mask
            }
            _ => false,
        }
    }
}

impl core::fmt::Display for Range {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}/{}", self.base, self.bits)
    }
}

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
        writer: Box::new(StreamWriter::with_shutdown(Shared(stream), move || {
            let _ = shutting.shutdown(Shutdown::Both);
        })),
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

/// A TCP listener that accepts only peers inside its ranges: the trusted
/// level's gate. Where it listens is the caller's choice.
#[derive(Debug)]
pub struct TcpListen {
    listener: TcpListener,
    ranges: Vec<Range>,
}

/// A peer [`TcpListen::accept`] turned away.
#[derive(Debug)]
#[non_exhaustive]
pub enum Accepted {
    /// A connection from inside the ranges.
    Connection(Connection),
    /// One from outside them, closed unread.
    Refused(SocketAddr),
}

impl TcpListen {
    /// Binds `address`, any address a wildcard (`0.0.0.0`, `::`)
    /// included, accepting peers inside `ranges`. An empty list is
    /// refused: it would turn every peer away.
    pub fn bind(address: &str, ranges: &[Range]) -> io::Result<TcpListen> {
        if ranges.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "no address range: every peer would be turned away",
            ));
        }
        let addrs: Vec<SocketAddr> = address.to_socket_addrs()?.collect();
        let addr = addrs
            .first()
            .copied()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no address to bind"))?;
        Ok(TcpListen {
            listener: TcpListener::bind(addr)?,
            ranges: ranges.to_vec(),
        })
    }

    /// A listener the caller bound itself (with socket options of its
    /// own, or handed over by a service manager), accepting peers inside
    /// `ranges` as [`bind`](TcpListen::bind) does.
    pub fn from_listener(listener: TcpListener, ranges: &[Range]) -> io::Result<TcpListen> {
        if ranges.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "no address range: every peer would be turned away",
            ));
        }
        Ok(TcpListen {
            listener,
            ranges: ranges.to_vec(),
        })
    }

    /// Where it listens.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// The next connection, or a peer from outside the ranges turned away.
    pub fn accept(&self) -> io::Result<Accepted> {
        let (stream, peer) = self.listener.accept()?;
        if !self.ranges.iter().any(|r| r.contains(peer.ip())) {
            let _ = stream.shutdown(Shutdown::Both);
            return Ok(Accepted::Refused(peer));
        }
        Ok(Accepted::Connection(connection(
            stream,
            format!("tcp {peer}"),
        )?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_range_holds_what_its_prefix_says() {
        let r = Range::parse("192.0.2.0/24").unwrap();
        assert!(r.contains("192.0.2.21".parse().unwrap()));
        assert!(!r.contains("198.51.100.1".parse().unwrap()));
        assert!(r.contains("::ffff:192.0.2.5".parse().unwrap()));
        let one = Range::parse("127.0.0.1").unwrap();
        assert!(one.contains("127.0.0.1".parse().unwrap()));
        assert!(!one.contains("127.0.0.2".parse().unwrap()));
        let all = Range::parse("0.0.0.0/0").unwrap();
        assert!(all.contains("8.8.8.8".parse().unwrap()));
        let v6 = Range::parse("fd00::/8").unwrap();
        assert!(v6.contains("fd12::1".parse().unwrap()));
        assert!(!v6.contains("fe80::1".parse().unwrap()));
        assert!(Range::parse("10.0.0.0/33").is_err());
        assert!(Range::parse("nowhere").is_err());
    }

    #[test]
    fn a_listener_binds_where_it_is_told_and_admits_by_its_ranges() {
        let local = [Range::parse("127.0.0.0/8").unwrap()];
        // Every interface: the peer at 127.0.0.1 is inside the range.
        let any = TcpListen::bind("0.0.0.0:0", &local).expect("a wildcard binds");
        let port = any.local_addr().unwrap().port();
        let _c = TcpStream::connect(("127.0.0.1", port)).unwrap();
        assert!(matches!(any.accept().unwrap(), Accepted::Connection(_)));

        // Its own address outside the ranges, as when peers route to it.
        let elsewhere = [Range::parse("10.0.0.0/8").unwrap()];
        let routed = TcpListen::bind("127.0.0.1:0", &elsewhere).expect("any address binds");
        let _c = TcpStream::connect(routed.local_addr().unwrap()).unwrap();
        assert!(matches!(routed.accept().unwrap(), Accepted::Refused(_)));

        assert_eq!(
            TcpListen::bind("127.0.0.1:0", &[]).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn sockets_the_caller_set_up_carry_frames_under_the_same_gate() {
        let local = [Range::parse("127.0.0.0/8").unwrap()];
        assert_eq!(
            TcpListen::from_listener(TcpListener::bind("127.0.0.1:0").unwrap(), &[])
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        let l =
            TcpListen::from_listener(TcpListener::bind("127.0.0.1:0").unwrap(), &local).unwrap();
        let mut client = from_stream(TcpStream::connect(l.local_addr().unwrap()).unwrap()).unwrap();
        assert!(client.peer.starts_with("tcp 127.0.0.1:"));
        let Accepted::Connection(mut server) = l.accept().unwrap() else {
            panic!("a peer inside the range is accepted");
        };
        client.writer.write_frame(&[3, 0, 0, 0, 1, 2, 3]).unwrap();
        let mut frame = Vec::new();
        assert!(server.reader.read_frame(&mut frame, 1 << 10).unwrap());
        assert_eq!(frame, [1, 2, 3]);
    }

    #[test]
    fn a_peer_outside_the_ranges_is_turned_away_unread() {
        use std::io::Read as _;
        // Accepting only 127.0.0.2: the loopback client at 127.0.0.1 is
        // outside, and finds the connection closed before anything is read.
        let l = TcpListen::bind("127.0.0.1:0", &[Range::parse("127.0.0.2").unwrap()]).unwrap();
        let mut c = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        assert!(matches!(l.accept().unwrap(), Accepted::Refused(_)));
        c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut byte = [0u8; 1];
        assert!(matches!(c.read(&mut byte), Ok(0) | Err(_)));
    }
}
