// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! TLS 1.3 over TCP, the secure level: mutual TLS through coqui-peer-tls.
//! Each side presents a certificate of its own and knows the other by
//! its fingerprint (`sha256:<hex>`): fixed pins, or the peers a trust store
//! has accepted. A server with a store records a client it does not know,
//! once the client has proved it holds its key, as waiting there until an
//! operator accepts it. A listener admits peers by an access control list
//! (the policy's `acl`): by address before anything is read, by
//! fingerprint once the handshake has proved one.
//!
//! The link reads on one thread and writes from others, and a TLS state is
//! one object. The socket is read with no lock held; the state's lock is
//! held only to feed it what was read, to decrypt, and to encrypt. Records
//! leave in the order they were sealed, under a second lock a writer holds
//! across its socket write. Once the handshake is through the reader never
//! writes: what reading produces (a key update's answer) leaves with the
//! next write, so a write blocked on a peer that reads nothing never stops
//! the reader.

#![forbid(unsafe_code)]

use alloc::sync::Arc;
use core::net::SocketAddr;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::io::{self, IoSlice, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Instant;

use coqui_net_acl::{Acl, Outcome};
use coqui_peer::DriveError;
use coqui_peer_tls::{Digest, PeerError, Pins, TlsHandshake, Trust};
use rustls::{ClientConfig, ServerConfig};

pub use coqui_peer_tls::{Identity, PeerId, Policy, TrustStore, WaitingEntry, Weakening};

use super::stream::{StreamReader, StreamWriter};
use super::tcp::{Accepted, CONNECT_TIMEOUT};
use super::{Connect, Connection, FrameRead};

/// The application protocol both ends name and require, so a link never
/// completes with a TLS endpoint of something else.
const ALPN: &[u8] = b"guatiao-remote";

/// The identity file [`TlsConfig::open`] keeps in its directory.
pub const IDENTITY_FILE: &str = "identity.pem";

/// Raw bytes read from the socket at a time, at first.
const RAW_READ: usize = 64 << 10;

/// What a read that keeps filling its buffer grows it to: few large
/// reads instead of many small ones while a large frame streams in, which
/// on Windows quadrupled a transfer.
const RAW_READ_MAX: usize = 1 << 20;

fn invalid(why: impl core::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, why.to_string())
}

/// A coqui-peer-tls error in words, with what lies under it: its own
/// words are generic ("storage or stream operation failed").
pub(crate) fn words(e: &PeerError) -> String {
    match core::error::Error::source(e) {
        Some(under) => format!("{e}: {under}"),
        None => e.to_string(),
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The rustls error under a handshake's failure, if that is what it is.
fn tls_error(e: &PeerError) -> Option<&rustls::Error> {
    match e {
        PeerError::Io(io) => io.get_ref()?.downcast_ref::<rustls::Error>(),
        _ => None,
    }
}

/// Rewrites every fingerprint an access control list names in its
/// canonical text, the form a verified peer is decided by. A peer is known
/// by its certificate's SHA-256 fingerprint, so a rule naming another
/// digest could never match: refused rather than left to fail open.
fn canonical_acl(mut acl: Acl) -> io::Result<Acl> {
    for rule in &mut acl.rules {
        if let Some(text) = &rule.fingerprint {
            let id: PeerId = text.parse().map_err(|_| {
                invalid(format!(
                    "`{text}` in the access control list is not a fingerprint"
                ))
            })?;
            if id.digest() != Digest::Sha256 {
                return Err(invalid(format!(
                    "`{text}` in the access control list is a {} fingerprint; a peer is known by its SHA-256 one, `sha256:<hex>`",
                    id.digest().name()
                )));
            }
            rule.fingerprint = Some(id.canonical());
        }
    }
    Ok(acl)
}

/// The socket under a handshake, every read and write bounded by what is
/// left of one deadline: a peer sending a byte at a time cannot hold the
/// handshake past it.
struct Bounded<'a> {
    socket: &'a TcpStream,
    until: Option<Instant>,
}

impl Bounded<'_> {
    fn left(&self) -> io::Result<Option<Duration>> {
        match self.until {
            None => Ok(None),
            Some(until) => match until.checked_duration_since(Instant::now()) {
                Some(left) if !left.is_zero() => Ok(Some(left)),
                _ => Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the TLS handshake did not finish in time",
                )),
            },
        }
    }
}

impl Read for Bounded<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.socket.set_read_timeout(self.left()?)?;
        self.socket.read(buf)
    }
}

impl Write for Bounded<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.socket.set_write_timeout(self.left()?)?;
        self.socket.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Refuses a handshake that did not agree on this protocol.
fn check_alpn(connection: &rustls::Connection) -> io::Result<()> {
    if connection.alpn_protocol() == Some(ALPN) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the peer does not speak guatiao's remote protocol: the TLS handshake agreed no `guatiao-remote` application protocol",
        ))
    }
}

/// What one side of the secure level is: its identity, whom it trusts and
/// the TLS policy both are held to.
///
/// Trust is the fixed pins when any are given, else the peers the trust
/// store has accepted; a server with a store records a client it does not
/// know as waiting there. TLS session resumption is off at both ends unless
/// the policy's `allow_resumption` keeps it, so a peer removed from the
/// store is refused at its next link.
#[derive(Clone)]
pub struct TlsConfig {
    identity: Identity,
    pins: Vec<PeerId>,
    store: Option<TrustStore>,
    policy: Policy,
}

impl core::fmt::Debug for TlsConfig {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TlsConfig")
            .field("fingerprint", &self.identity.fingerprint().canonical())
            .field("pins", &self.pins.len())
            .field("store", &self.store.is_some())
            .field("policy", &self.policy.name)
            .finish()
    }
}

impl TlsConfig {
    /// Presenting `identity`, under the strict policy, trusting no one yet.
    pub fn new(identity: Identity) -> TlsConfig {
        TlsConfig {
            identity,
            pins: Vec::new(),
            store: None,
            policy: Policy::strict(),
        }
    }

    /// The identity in `dir`'s [`IDENTITY_FILE`], made there when absent
    /// (readable by its owner alone on Unix), and the trust store in `dir`
    /// (`peers/accepted`, `peers/waiting`).
    pub fn open(dir: &Path) -> io::Result<TlsConfig> {
        let identity = Identity::open_or_create(&dir.join(IDENTITY_FILE)).map_err(|e| {
            invalid(format!(
                "{}: {}",
                dir.join(IDENTITY_FILE).display(),
                words(&e)
            ))
        })?;
        let store = TrustStore::open(dir)
            .map_err(|e| invalid(format!("{}: the trust store: {e}", dir.display())))?;
        Ok(TlsConfig::new(identity).store(store))
    }

    /// Trusting exactly these fingerprints (added to any given before);
    /// with any, the store no longer decides trust.
    pub fn pins(mut self, pins: impl IntoIterator<Item = PeerId>) -> TlsConfig {
        self.pins.extend(pins);
        self
    }

    /// Trusting the peers `store` has accepted, unless pins are given.
    pub fn store(mut self, store: TrustStore) -> TlsConfig {
        self.store = Some(store);
        self
    }

    /// Held to `policy` rather than the strict one.
    pub fn policy(mut self, policy: Policy) -> TlsConfig {
        self.policy = policy;
        self
    }

    /// The identity this side presents.
    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    /// This side's fingerprint: what a peer pins or accepts.
    pub fn fingerprint(&self) -> PeerId {
        self.identity.fingerprint()
    }

    /// The trust store, if one is given.
    pub fn trust_store(&self) -> Option<&TrustStore> {
        self.store.as_ref()
    }

    /// Every setting of the policy looser than strict, its access control
    /// list's default included; empty under the strict policy.
    pub fn weakened(&self) -> Vec<Weakening> {
        self.policy.weakened()
    }

    fn trust(&self) -> io::Result<Trust> {
        if !self.pins.is_empty() {
            return Ok(Trust::Pinned(Pins::fixed(self.pins.iter().copied())));
        }
        match &self.store {
            Some(store) => Ok(Trust::Pinned(Pins::store(store.clone()))),
            None => Err(invalid(
                "the secure level names whom it trusts: fingerprints to pin, or a trust store",
            )),
        }
    }

    fn client(&self) -> io::Result<Arc<ClientConfig>> {
        if self.policy.acl.is_some() {
            return Err(invalid(
                "the policy's `acl` admits peers to a listener; a client has none",
            ));
        }
        let built = coqui_peer_tls::client_config_with(
            &self.identity,
            &self.trust()?,
            &[ALPN],
            &self.policy,
        )
        .map_err(|e| invalid(words(&e)))?;
        Ok(Arc::new(built.config))
    }

    fn server(&self) -> io::Result<Arc<ServerConfig>> {
        let built = coqui_peer_tls::server_config_with(
            &self.identity,
            &self.trust()?,
            &[ALPN],
            &self.policy,
        )
        .map_err(|e| invalid(words(&e)))?;
        Ok(Arc::new(built.config))
    }
}

/// The TLS state and the socket under it, shared by both halves.
struct Core {
    socket: TcpStream,
    /// `None` until the handshake is through (a server's runs on its
    /// first read).
    state: Mutex<Option<rustls::Connection>>,
    /// Held from taking records out of the state until they are on the
    /// socket, so they leave in order; the buffer they pass through. Taken
    /// before `state`, never after; the reader only tries it.
    sending: Mutex<Vec<u8>>,
    /// The peer's verified fingerprint, canonical.
    peer: OnceLock<String>,
    /// A client: this side's own fingerprint, to name in a refusal.
    client: Option<String>,
    /// Whether anything has come from the peer since the handshake: a
    /// client's link failing before that was refused by the server.
    heard: AtomicBool,
    /// Whether this side ended the link.
    closed: AtomicBool,
}

impl Core {
    fn new(socket: TcpStream, client: Option<String>) -> Core {
        Core {
            socket,
            state: Mutex::new(None),
            sending: Mutex::new(Vec::new()),
            peer: OnceLock::new(),
            client,
            heard: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        }
    }

    fn open(&self, connection: rustls::Connection, peer: &PeerId) {
        *lock(&self.state) = Some(connection);
        let _ = self.peer.set(peer.canonical());
    }

    /// A failure in words a person can act on. A client's link the server
    /// ends before saying anything was most likely refused at the server's
    /// end of the handshake: TLS 1.3 lets a client finish before the server
    /// has checked its certificate.
    fn explain(&self, e: io::Error) -> io::Error {
        use io::ErrorKind::{
            BrokenPipe, ConnectionAborted, ConnectionReset, InvalidData, UnexpectedEof,
        };
        let ended = matches!(
            e.kind(),
            BrokenPipe | ConnectionAborted | ConnectionReset | InvalidData | UnexpectedEof
        );
        let early = !self.heard.load(Ordering::Acquire) && !self.closed.load(Ordering::Acquire);
        match &self.client {
            Some(me) if ended && early => io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "the server ended the link before answering ({e}): most likely it does not trust this client, {me}, or its access control list refuses it; a server with a trust store keeps an unknown client waiting until an operator accepts it"
                ),
            ),
            _ => e,
        }
    }

    /// Ends the socket both ways, with nothing more said.
    fn shut(&self) {
        self.closed.store(true, Ordering::Release);
        let _ = self.socket.shutdown(Shutdown::Both);
    }

    /// Seals `parts` and sends the records, one socket write per batch the
    /// state's buffer holds.
    fn send(&self, parts: &[&[u8]]) -> io::Result<()> {
        let mut out = lock(&self.sending);
        let mut parts = parts.iter().copied().filter(|p| !p.is_empty());
        let mut rest: &[u8] = parts.next().unwrap_or_default();
        loop {
            out.clear();
            {
                let mut state = lock(&self.state);
                let tls = state.as_mut().ok_or(io::ErrorKind::NotConnected)?;
                // The state seals what its buffer takes; past that, the
                // records go out and the rest follows.
                while !rest.is_empty() {
                    let n = tls.writer().write(rest)?;
                    if n == 0 {
                        break;
                    }
                    rest = &rest[n..];
                    if rest.is_empty() {
                        rest = parts.next().unwrap_or_default();
                    }
                }
                while tls.wants_write() {
                    tls.write_tls(&mut *out)?;
                }
            }
            if out.is_empty() {
                if rest.is_empty() {
                    break;
                }
                return Err(io::ErrorKind::WriteZero.into());
            }
            (&self.socket).write_all(&out)?;
            if rest.is_empty() {
                break;
            }
        }
        Ok(())
    }

    /// Sends what the state holds sealed and no write has taken yet: what
    /// reading produced.
    fn flush(&self) -> io::Result<()> {
        let mut out = lock(&self.sending);
        out.clear();
        {
            let mut state = lock(&self.state);
            let Some(tls) = state.as_mut() else {
                return Ok(());
            };
            while tls.wants_write() {
                tls.write_tls(&mut *out)?;
            }
        }
        if !out.is_empty() {
            (&self.socket).write_all(&out)?;
        }
        Ok(())
    }

    /// Tells the peer the link ends (when no write is in progress), then
    /// ends the socket both ways.
    fn close(&self) {
        self.closed.store(true, Ordering::Release);
        if let Ok(mut out) = self.sending.try_lock() {
            out.clear();
            if let Some(tls) = lock(&self.state).as_mut() {
                tls.send_close_notify();
                while tls.wants_write() {
                    if tls.write_tls(&mut *out).is_err() {
                        break;
                    }
                }
            }
            if !out.is_empty() {
                let _ = self
                    .socket
                    .set_write_timeout(Some(Duration::from_millis(100)));
                let _ = (&self.socket).write_all(&out);
            }
        }
        self.shut();
    }
}

/// A server's handshake, run on the serving thread's first read.
struct Pending {
    config: Arc<ServerConfig>,
    source: SocketAddr,
    local: SocketAddr,
    acl: Option<Arc<Acl>>,
}

impl Pending {
    fn run(self, core: &Core) -> io::Result<()> {
        let handshake =
            TlsHandshake::server(self.config, Some(self.source)).map_err(|e| invalid(words(&e)))?;
        // The bound the link set on a read bounds the whole handshake.
        let bound = core.socket.read_timeout()?;
        let mut io = Bounded {
            socket: &core.socket,
            until: bound.map(|b| Instant::now() + b),
        };
        let established = coqui_peer::drive_stream(handshake, &mut io);
        core.socket.set_read_timeout(bound)?;
        core.socket.set_write_timeout(None)?;
        let established = established.map_err(|e| match e {
            DriveError::Io(e) => e,
            DriveError::Handshake(e) => io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("the client's TLS handshake failed: {}", words(&e)),
            ),
            other => io::Error::new(io::ErrorKind::InvalidData, other.to_string()),
        })?;
        let peer = established.peer;
        if let Err(e) = check_alpn(&established.session) {
            core.shut();
            return Err(e);
        }
        if let Some(acl) = &self.acl {
            let decided = acl.decide_peer(self.source, self.local, &peer.canonical());
            if decided.outcome != Outcome::Permit {
                core.close();
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    match decided.rule_index {
                        Some(i) => format!(
                            "the access control list denies {peer} at {} (rule {})",
                            self.source,
                            (i + 1) * 10
                        ),
                        None => format!(
                            "the access control list denies {peer} at {} (its default)",
                            self.source
                        ),
                    },
                ));
            }
        }
        core.open(established.session, &peer);
        Ok(())
    }
}

/// The reading half: plaintext out of records read from the socket.
struct TlsRead {
    core: Arc<Core>,
    pending: Option<Pending>,
    /// Bytes read from the socket; `raw[fed..filled]` are not yet in the
    /// state, which is fed only while it holds no plaintext.
    raw: Vec<u8>,
    fed: usize,
    filled: usize,
    /// The socket's end, told to the state.
    ended: bool,
}

/// Plaintext the state holds, if any: `None` while it needs more records.
fn plaintext(tls: &mut rustls::Connection, out: &mut [u8]) -> io::Result<Option<usize>> {
    match tls.reader().read(out) {
        Ok(n) => Ok(Some(n)),
        Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
        Err(e) => Err(e),
    }
}

impl TlsRead {
    fn read_plain(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if let Some(pending) = self.pending.take() {
            pending.run(&self.core)?;
        }
        loop {
            {
                let mut state = lock(&self.core.state);
                let tls = state.as_mut().ok_or(io::ErrorKind::NotConnected)?;
                loop {
                    if let Some(n) = plaintext(tls, out)? {
                        if n > 0 {
                            self.core.heard.store(true, Ordering::Release);
                        }
                        return Ok(n);
                    }
                    if self.fed == self.filled {
                        if !self.ended {
                            break;
                        }
                        // The socket ended without the peer's close_notify.
                        return Err(io::ErrorKind::UnexpectedEof.into());
                    }
                    self.fed += tls.read_tls(&mut &self.raw[self.fed..self.filled])?;
                    tls.process_new_packets()
                        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                }
            }
            // Read raw with no lock held: a writer seals meanwhile.
            let n = (&self.core.socket).read(&mut self.raw)?;
            if n == self.raw.len() && n < RAW_READ_MAX {
                self.raw.resize(n * 2, 0);
            }
            if n == 0 {
                self.ended = true;
                let mut state = lock(&self.core.state);
                if let Some(tls) = state.as_mut() {
                    let _ = tls.read_tls(&mut &[][..]);
                }
            }
            (self.fed, self.filled) = (0, n);
        }
    }
}

impl Read for TlsRead {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.read_plain(out).map_err(|e| self.core.explain(e))
    }
}

/// The frames of the reading half, and the peer the handshake proved.
struct TlsFrames {
    frames: StreamReader<TlsRead>,
    core: Arc<Core>,
}

impl FrameRead for TlsFrames {
    fn read_frame(&mut self, buf: &mut Vec<u8>, max: usize) -> io::Result<bool> {
        self.frames.read_frame(buf, max)
    }

    fn set_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.frames.set_timeout(timeout)
    }

    fn peer_id(&self) -> Option<String> {
        self.core.peer.get().cloned()
    }
}

/// The writing half.
struct TlsWrite {
    core: Arc<Core>,
}

impl Write for TlsWrite {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.core.send(&[bytes]).map_err(|e| self.core.explain(e))?;
        Ok(bytes.len())
    }

    fn write_vectored(&mut self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        let parts: Vec<&[u8]> = bufs.iter().map(|b| &**b).collect();
        self.core.send(&parts).map_err(|e| self.core.explain(e))?;
        Ok(parts.iter().map(|p| p.len()).sum())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.core.flush()
    }
}

fn connection(core: Core, pending: Option<Pending>, peer: String) -> io::Result<Connection> {
    let core = Arc::new(core);
    let closing = Arc::clone(&core);
    let shutting = Arc::clone(&core);
    let reader = StreamReader::with_timeout(
        TlsRead {
            core: Arc::clone(&core),
            pending,
            raw: vec![0; RAW_READ],
            fed: 0,
            filled: 0,
            ended: false,
        },
        |r: &TlsRead, t| r.core.socket.set_read_timeout(t),
    );
    Ok(Connection {
        reader: Box::new(TlsFrames {
            frames: reader,
            core: Arc::clone(&core),
        }),
        writer: Box::new(
            StreamWriter::with_shutdown(TlsWrite { core }, move || shutting.close())
                .bounded_by(|w: &mut TlsWrite, t| w.core.socket.set_write_timeout(t)),
        ),
        peer,
        closer: Some(Box::new(move || closing.shut())),
    })
}

/// Connects to a secure server at `host:port`: the handshake runs in
/// [`connect`](Connect::connect), so a server this side does not trust
/// fails it there.
#[derive(Clone)]
pub struct TlsConnect {
    address: String,
    config: Arc<ClientConfig>,
    me: String,
    timeout: Duration,
}

impl core::fmt::Debug for TlsConnect {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TlsConnect")
            .field("address", &self.address)
            .field("as", &self.me)
            .finish()
    }
}

impl TlsConnect {
    /// To `address`, as `config` says, within [`CONNECT_TIMEOUT`] for the
    /// connect and the handshake each.
    pub fn new(address: &str, config: &TlsConfig) -> io::Result<TlsConnect> {
        Ok(TlsConnect {
            address: address.to_string(),
            config: config.client()?,
            me: config.fingerprint().canonical(),
            timeout: CONNECT_TIMEOUT,
        })
    }

    /// Within `timeout` per address tried, and again for the handshake.
    pub fn timeout(mut self, timeout: Duration) -> TlsConnect {
        self.timeout = timeout;
        self
    }

    /// A client handshake's failure in words a person can act on.
    fn refusal(&self, e: &PeerError, addr: SocketAddr) -> io::Error {
        let (kind, why) = match tls_error(e) {
            // coqui-peer-tls refuses a certificate with `General` and its reason.
            Some(rustls::Error::InvalidCertificate(_) | rustls::Error::General(_)) => (
                io::ErrorKind::PermissionDenied,
                format!(
                    "the server at {addr} is not one this client trusts: its certificate matches none of the pins or the trust store's peers, or the policy refuses it"
                ),
            ),
            Some(rustls::Error::AlertReceived(alert)) => (
                io::ErrorKind::PermissionDenied,
                format!(
                    "the server at {addr} refused the TLS handshake ({alert:?}): it may not trust this client, {}",
                    self.me
                ),
            ),
            _ => (
                io::ErrorKind::InvalidData,
                format!("the TLS handshake with the server at {addr} failed"),
            ),
        };
        io::Error::new(kind, format!("{why}: {}", words(e)))
    }

    fn handshake(&self, socket: TcpStream, addr: SocketAddr) -> io::Result<Connection> {
        socket.set_nodelay(true)?;
        let handshake =
            TlsHandshake::client(Arc::clone(&self.config)).map_err(|e| invalid(words(&e)))?;
        let mut io = Bounded {
            socket: &socket,
            until: Some(Instant::now() + self.timeout),
        };
        let established = coqui_peer::drive_stream(handshake, &mut io).map_err(|e| match e {
            DriveError::Io(e) => e,
            DriveError::Handshake(e) => self.refusal(&e, addr),
            other => io::Error::new(io::ErrorKind::InvalidData, other.to_string()),
        })?;
        socket.set_read_timeout(None)?;
        socket.set_write_timeout(None)?;
        check_alpn(&established.session)?;
        let core = Core::new(socket, Some(self.me.clone()));
        core.open(established.session, &established.peer);
        connection(core, None, format!("tls {addr}"))
    }
}

impl Connect for TlsConnect {
    fn connect(&self) -> io::Result<Connection> {
        let mut last = io::Error::new(io::ErrorKind::NotFound, "no address to connect to");
        for addr in self.address.to_socket_addrs()? {
            match TcpStream::connect_timeout(&addr, self.timeout) {
                Ok(socket) => return self.handshake(socket, addr),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    fn describe(&self) -> String {
        format!("tls {}", self.address)
    }
}

/// A secure listener. It admits peers by the policy's access control
/// list, every source when there is none: by address as it accepts,
/// reading nothing from a peer it denies; by fingerprint after the
/// handshake, which runs on the serving thread's first read, so a client
/// that connects and stalls holds only that thread.
pub struct TlsListen {
    listener: TcpListener,
    config: Arc<ServerConfig>,
    acl: Option<Arc<Acl>>,
}

impl core::fmt::Debug for TlsListen {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TlsListen")
            .field("addr", &self.listener.local_addr().ok())
            .field("acl", &self.acl.is_some())
            .finish()
    }
}

impl TlsListen {
    /// Binds `address` (a wildcard included) as `config` says.
    pub fn bind(address: &str, config: &TlsConfig) -> io::Result<TlsListen> {
        let addr = address
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| invalid("no address to bind"))?;
        TlsListen::from_listener(TcpListener::bind(addr)?, config)
    }

    /// A listener the caller bound itself, as `config` says.
    pub fn from_listener(listener: TcpListener, config: &TlsConfig) -> io::Result<TlsListen> {
        let acl = match config.policy.acl.clone() {
            Some(acl) => Some(Arc::new(canonical_acl(acl)?)),
            None => None,
        };
        Ok(TlsListen {
            listener,
            config: config.server()?,
            acl,
        })
    }

    /// Where it listens.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// The next connection, before its handshake, or a peer the list
    /// denies by address, closed unread.
    pub fn accept(&self) -> io::Result<Accepted> {
        let (socket, source) = self.listener.accept()?;
        let local = socket.local_addr()?;
        if let Some(acl) = &self.acl
            && acl.decide(source, local).outcome == Outcome::Deny
        {
            let _ = socket.shutdown(Shutdown::Both);
            return Ok(Accepted::Refused(source));
        }
        socket.set_nodelay(true)?;
        let pending = Pending {
            config: Arc::clone(&self.config),
            source,
            local,
            acl: self.acl.clone(),
        };
        Ok(Accepted::Connection(connection(
            Core::new(socket, None),
            Some(pending),
            format!("tls {source}"),
        )?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    fn identity() -> Identity {
        Identity::generate().unwrap()
    }

    fn frame(body: &[u8]) -> Vec<u8> {
        let mut f = (body.len() as u32).to_le_bytes().to_vec();
        f.extend_from_slice(body);
        f
    }

    /// A listener and a client that pin each other, linked: the client's
    /// first frame has run the server's handshake on its first read.
    fn pair() -> (Connection, Connection) {
        let (server_id, client_id) = (identity(), identity());
        let server = TlsConfig::new(server_id.clone()).pins([client_id.fingerprint()]);
        let client = TlsConfig::new(client_id).pins([server_id.fingerprint()]);
        let listen = TlsListen::bind("127.0.0.1:0", &server).unwrap();
        let connect = TlsConnect::new(&listen.local_addr().unwrap().to_string(), &client).unwrap();
        let dialing = thread::spawn(move || {
            let mut c = connect.connect().unwrap();
            c.writer.write_frame(&frame(b"start")).unwrap();
            c
        });
        let Accepted::Connection(mut accepted) = listen.accept().unwrap() else {
            panic!("no access control list: every source proceeds");
        };
        let mut buf = Vec::new();
        assert!(accepted.reader.read_frame(&mut buf, 1 << 10).unwrap());
        assert_eq!(buf, b"start");
        (dialing.join().unwrap(), accepted)
    }

    #[test]
    fn frames_cross_both_ways_and_each_end_knows_the_other() {
        let (mut client, mut server) = pair();
        client.writer.write_frame(&frame(b"hello")).unwrap();
        let mut buf = Vec::new();
        assert!(server.reader.read_frame(&mut buf, 1 << 10).unwrap());
        assert_eq!(buf, b"hello");
        assert!(server.reader.peer_id().unwrap().starts_with("sha256:"));
        server
            .writer
            .write_frames(&[&frame(b"one"), &frame(b""), &frame(b"three")])
            .unwrap();
        for want in [&b"one"[..], b"", b"three"] {
            assert!(client.reader.read_frame(&mut buf, 1 << 10).unwrap());
            assert_eq!(buf, want);
        }
        assert_ne!(client.reader.peer_id(), server.reader.peer_id());
    }

    #[test]
    fn a_server_writes_nothing_before_its_handshake() {
        let id = identity();
        let config = TlsConfig::new(id.clone()).pins([id.fingerprint()]);
        let listen = TlsListen::bind("127.0.0.1:0", &config).unwrap();
        let _raw = TcpStream::connect(listen.local_addr().unwrap()).unwrap();
        let Accepted::Connection(mut server) = listen.accept().unwrap() else {
            panic!("every source proceeds");
        };
        let e = server.writer.write_frame(&frame(b"early")).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::NotConnected);
        assert_eq!(server.reader.peer_id(), None);
    }

    #[test]
    fn sixteen_mebibytes_cross_each_way_at_once() {
        let (client, server) = pair();
        let big: Vec<u8> = (0..16u32 << 20).map(|i| (i % 251) as u8).collect();
        let sent = frame(&big);
        let (mut cw, mut sw) = (client.writer, server.writer);
        let (mut cr, mut sr) = (client.reader, server.reader);
        let (s1, s2) = (sent.clone(), sent);
        let a = thread::spawn(move || cw.write_frame(&s1).unwrap());
        let b = thread::spawn(move || sw.write_frame(&s2).unwrap());
        let mut got = Vec::new();
        assert!(cr.read_frame(&mut got, 32 << 20).unwrap());
        assert!(got == big);
        assert!(sr.read_frame(&mut got, 32 << 20).unwrap());
        assert!(got == big);
        a.join().unwrap();
        b.join().unwrap();
    }

    #[test]
    fn an_end_with_close_notify_reads_as_a_clean_end() {
        let (mut client, mut server) = pair();
        let mut buf = Vec::new();
        server.writer.shutdown();
        assert!(!client.reader.read_frame(&mut buf, 1 << 10).unwrap());
    }

    #[test]
    fn fingerprints_in_an_access_control_list_are_made_canonical() {
        use coqui_net_acl::{Action, Rule};
        let id = identity().fingerprint();
        let mut rule = Rule::new(Action::Permit, "any".parse().unwrap());
        rule.fingerprint = Some(id.to_base32_grouped());
        let acl = canonical_acl(Acl {
            rules: vec![rule],
            default: Action::Deny,
        })
        .unwrap();
        assert_eq!(
            acl.rules[0].fingerprint.as_deref(),
            Some(id.canonical().as_str())
        );
        let mut bad = Rule::new(Action::Permit, "any".parse().unwrap());
        bad.fingerprint = Some("nonsense".into());
        assert!(
            canonical_acl(Acl {
                rules: vec![bad],
                default: Action::Deny,
            })
            .is_err()
        );
        // Another digest could never match a peer known by SHA-256.
        let mut other = Rule::new(Action::Deny, "any".parse().unwrap());
        other.fingerprint = Some(format!("sha384:{}", "ab".repeat(48)));
        let e = canonical_acl(Acl {
            rules: vec![other],
            default: Action::Permit,
        })
        .unwrap_err();
        assert!(e.to_string().contains("sha384"), "{e}");
    }

    #[test]
    fn an_end_without_close_notify_is_an_error_not_an_end() {
        let (mut client, mut server) = pair();
        server.writer.write_frame(&frame(b"heard")).unwrap();
        let mut buf = Vec::new();
        assert!(client.reader.read_frame(&mut buf, 1 << 10).unwrap());
        // The socket alone ends: a truncation, which reads as one.
        (server.closer.take().unwrap())();
        let e = client.reader.read_frame(&mut buf, 1 << 10).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::UnexpectedEof, "{e}");
    }

    #[test]
    fn a_handshake_has_one_deadline_however_slowly_the_peer_sends() {
        let id = identity();
        let config = TlsConfig::new(id.clone()).pins([id.fingerprint()]);
        let listen = TlsListen::bind("127.0.0.1:0", &config).unwrap();
        let mut raw = TcpStream::connect(listen.local_addr().unwrap()).unwrap();
        let Accepted::Connection(mut server) = listen.accept().unwrap() else {
            panic!("every source proceeds");
        };
        // A byte each 100 ms, each read well within its 300 ms bound.
        let trickle = thread::spawn(move || {
            let hello = [0x16u8, 3, 1, 0x40, 0];
            for b in hello.iter().cycle().take(100) {
                if raw.write_all(&[*b]).is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
        });
        server
            .reader
            .set_timeout(Some(Duration::from_millis(300)))
            .unwrap();
        let started = Instant::now();
        let mut buf = Vec::new();
        assert!(server.reader.read_frame(&mut buf, 1 << 10).is_err());
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        drop(server);
        trickle.join().unwrap();
    }
}
