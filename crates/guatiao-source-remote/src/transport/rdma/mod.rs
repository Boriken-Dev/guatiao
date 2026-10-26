// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! RDMA, experimental and Linux only: frames written with one-sided RDMA
//! writes into rings both sides registered, at the trusted level.
//!
//! A link starts as a TCP connection, admitted by address range as
//! [`tcp::TcpListen`](super::tcp::TcpListen) admits one; the two sides
//! exchange their queue pairs' numbers and their rings' addresses over it,
//! then every frame crosses by RDMA. The TCP connection stays open beside
//! the link: its end is the peer's. libibverbs (rdma-core) is opened when
//! a link is made, so building needs nothing of it; elsewhere than Linux
//! every call answers [`io::ErrorKind::Unsupported`].
//!
//! Each link registers 8 MiB of memory with the device (a ring to read
//! and a staging area to write from), locked in RAM: a process making
//! links needs `RLIMIT_MEMLOCK` to allow it, or `CAP_IPC_LOCK`.

#![deny(unsafe_code)]

#[cfg(target_os = "linux")]
mod ring;
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
mod verbs;

use core::net::SocketAddr;
use core::time::Duration;
use std::io;
use std::net::TcpListener;

use coqui_net_acl::Acl;

use super::tcp::{Accepted, TcpListen};
use super::{Connect, Connection};

/// Bytes each direction's ring holds.
pub const RING: usize = 4 << 20;

/// How long setting a link up may take.
pub const SETUP_TIMEOUT: Duration = Duration::from_secs(10);

/// The RDMA device, port and address (GID) a link goes through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RdmaPort {
    device: Option<String>,
    port: u8,
    gid_index: Option<u8>,
}

impl Default for RdmaPort {
    fn default() -> RdmaPort {
        RdmaPort::first()
    }
}

impl RdmaPort {
    /// The first device libibverbs lists, its port 1, and its first IPv4
    /// address (RoCE v2), else its address 0.
    pub fn first() -> RdmaPort {
        RdmaPort {
            device: None,
            port: 1,
            gid_index: None,
        }
    }

    /// The device named `name` (`rxe0`, `mlx5_0`).
    pub fn named(name: &str) -> RdmaPort {
        RdmaPort {
            device: Some(name.to_string()),
            ..RdmaPort::first()
        }
    }

    /// Port `port` of the device.
    pub fn port(mut self, port: u8) -> RdmaPort {
        self.port = port;
        self
    }

    /// The address at `index` in the port's GID table.
    pub fn gid_index(mut self, index: u8) -> RdmaPort {
        self.gid_index = Some(index);
        self
    }
}

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "RDMA links are Linux only in this build",
    )
}

/// The names of the RDMA devices present: an error when libibverbs
/// cannot be loaded (rdma-core is not installed) or off Linux.
pub fn devices() -> io::Result<Vec<String>> {
    #[cfg(target_os = "linux")]
    {
        verbs::device_names()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(unsupported())
    }
}

/// Connects to an [`RdmaListen`] at a TCP address (`host:port`).
#[derive(Debug, Clone)]
pub struct RdmaConnect {
    address: String,
    port: RdmaPort,
    timeout: Duration,
}

impl RdmaConnect {
    /// To `address`, through [`RdmaPort::first`], set up within
    /// [`SETUP_TIMEOUT`].
    pub fn new(address: &str) -> RdmaConnect {
        RdmaConnect {
            address: address.to_string(),
            port: RdmaPort::first(),
            timeout: SETUP_TIMEOUT,
        }
    }

    /// Through `port`.
    pub fn port(mut self, port: RdmaPort) -> RdmaConnect {
        self.port = port;
        self
    }

    /// Connecting and setting up within `timeout`.
    pub fn timeout(mut self, timeout: Duration) -> RdmaConnect {
        self.timeout = timeout;
        self
    }
}

impl Connect for RdmaConnect {
    fn connect(&self) -> io::Result<Connection> {
        #[cfg(target_os = "linux")]
        {
            use std::net::{TcpStream, ToSocketAddrs};
            let mut last = io::Error::new(io::ErrorKind::NotFound, "no address to connect to");
            for addr in self.address.to_socket_addrs()? {
                match TcpStream::connect_timeout(&addr, self.timeout) {
                    Ok(stream) => {
                        return ring::establish(
                            stream,
                            &self.port,
                            self.timeout,
                            format!("rdma {addr}"),
                        );
                    }
                    Err(e) => last = e,
                }
            }
            Err(last)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (&self.port, self.timeout);
            Err(unsupported())
        }
    }

    fn describe(&self) -> String {
        format!("rdma {}", self.address)
    }
}

/// Listens for RDMA links on a TCP address, admitting peers by an access
/// control list.
#[derive(Debug)]
pub struct RdmaListen {
    tcp: TcpListen,
    port: RdmaPort,
    timeout: Duration,
}

impl RdmaListen {
    /// Binds `address` as [`TcpListen::bind`] does, admitting the peers
    /// `acl` permits, through [`RdmaPort::first`].
    pub fn bind(address: &str, acl: Acl) -> io::Result<RdmaListen> {
        if cfg!(not(target_os = "linux")) {
            return Err(unsupported());
        }
        Ok(RdmaListen {
            tcp: TcpListen::bind(address, acl)?,
            port: RdmaPort::first(),
            timeout: SETUP_TIMEOUT,
        })
    }

    /// A listener the caller bound itself, as [`TcpListen::from_listener`].
    pub fn from_listener(listener: TcpListener, acl: Acl) -> io::Result<RdmaListen> {
        if cfg!(not(target_os = "linux")) {
            return Err(unsupported());
        }
        Ok(RdmaListen {
            tcp: TcpListen::from_listener(listener, acl),
            port: RdmaPort::first(),
            timeout: SETUP_TIMEOUT,
        })
    }

    /// Through `port`.
    pub fn port(mut self, port: RdmaPort) -> RdmaListen {
        self.port = port;
        self
    }

    /// Where it listens.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.tcp.local_addr()
    }

    /// The next link, set up, or a peer the list denies, turned away
    /// unread.
    pub fn accept(&self) -> io::Result<Accepted> {
        let (stream, peer) = match self.tcp.accept_stream()? {
            Ok(accepted) => accepted,
            Err(refused) => return Ok(Accepted::Refused(refused)),
        };
        #[cfg(target_os = "linux")]
        {
            Ok(Accepted::Connection(ring::establish(
                stream,
                &self.port,
                self.timeout,
                format!("rdma {peer}"),
            )?))
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (stream, peer, &self.port, self.timeout);
            Err(unsupported())
        }
    }
}
