// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! WebTransport: HTTP/3 over QUIC, what a browser opens with
//! `new WebTransport("https://host:port/")`. A link is one WebTransport
//! session carrying one bidirectional stream, the frames on it as on
//! `quic://` (each behind its `u32` length); [`Datagrams`] are the
//! session's own.
//!
//! The TLS is `quic://`'s: the same [`TlsConfig`], trust, policy and access
//! control list, a Retry proving a client's address before the list decides,
//! and a connection keeping the address it was admitted from. At no security
//! ([`WtConnect::trusted`], [`WtListen::bind_trusted`]) a client presents
//! no certificate, a native one checks none, and the link's key proof
//! decides; a page's browser still checks the one it was told to trust.
//!
//! A browser trusts a certificate no authority issued when the page pins
//! it by its SHA-256 (`serverCertificateHashes`), and only one valid for
//! at most 14 days. A listener made [`rotating`](WtListen::rotating)
//! presents certificates it makes itself for that, a new one at every
//! period, and [`certificate_hashes`](WtListen::certificate_hashes) names
//! the one presented and the next, for a page to pin both.

#![forbid(unsafe_code)]

use alloc::sync::Arc;
use core::cell::Cell;
use core::net::SocketAddr;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::io::{self, IoSlice, Read, Write};
use std::net::{ToSocketAddrs, UdpSocket};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Instant, SystemTime};

use coqui_net_acl::{Acl, Outcome};
use coqui_peer_tls::{Identity, PeerId};
use quinn::crypto::rustls::QuicServerConfig;
use quinn::{Incoming, VarInt};
use rustls::ServerConfig;
use rustls::client::danger::ServerCertVerifier;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::danger::ClientCertVerifier;
use wtransport::endpoint::endpoint_side::Client;
use wtransport::endpoint::{IncomingSessionFuture, SessionRequest};
use wtransport::error::{ConnectingError, ConnectionError, SendDatagramError};

use super::quic::{
    ABRUPT, CLEAN, CLOSE_WAIT, DROP_WAIT, Kept, REFUSED, Rt, Sourced, Watched, admitting,
    fingerprint, left, transport_config, wait, wait_at_most,
};
use super::stream::{StreamReader, StreamWriter};
use super::tcp::{Accepted, CONNECT_TIMEOUT};
use super::tls::{
    Denied, TlsConfig, Verifying, canonical_acl, denied_by_acl, ended_before_answering, host_of,
    server_name, untrusted_by_server, untrusted_server, words,
};
use super::{Connect, Connection, Datagrams, FrameRead};

/// HTTP/3's application protocol, which a WebTransport session runs in.
const H3: &[u8] = wtransport::tls::WEBTRANSPORT_ALPN;

/// The code HTTP/3 closes a connection with when it ends as it should
/// (`H3_NO_ERROR`): what a session ending cleanly closes with, which a
/// browser reads as a clean end. `quic://`'s own clean code is taken too.
const H3_NO_ERROR: u32 = 0x100;

/// How often a [`rotating`](WtListen::rotating) listener presents a new
/// certificate, unless told.
pub const ROTATE_EVERY: Duration = Duration::from_secs(6 * 24 * 3600);

/// The longest period a rotating listener takes: each certificate is valid
/// for two periods and two hours, and a browser takes one pinned by its
/// hash only when that is at most 14 days.
pub const ROTATE_MAX: Duration = Duration::from_secs(6 * 24 * 3600 + 22 * 3600);

/// How far a made certificate's validity reaches on either side of its two
/// periods: a peer whose clock is a little off still takes it.
const MARGIN: Duration = Duration::from_secs(3600);

fn invalid(why: impl core::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, why.to_string())
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A session lost, as what a reader answers: a clean end when either side
/// ended it as it should, else an error a person can act on.
fn lost(e: &quinn::ConnectionError, early_client: Option<&str>) -> io::Result<usize> {
    use quinn::ConnectionError as E;
    match e {
        E::LocallyClosed => return Ok(0),
        E::ApplicationClosed(c)
            if c.error_code == VarInt::from_u32(CLEAN)
                || c.error_code == VarInt::from_u32(H3_NO_ERROR) =>
        {
            return Ok(0);
        }
        _ => {}
    }
    let kind = match e {
        E::TimedOut => io::ErrorKind::TimedOut,
        E::ApplicationClosed(_) => io::ErrorKind::UnexpectedEof,
        _ => io::ErrorKind::ConnectionReset,
    };
    let lost = io::Error::new(kind, format!("the WebTransport session was lost: {e}"));
    Err(match early_client {
        Some(me) if kind != io::ErrorKind::TimedOut => ended_before_answering(&lost, me),
        _ => lost,
    })
}

/// What both halves of a link share.
struct Shared {
    conn: OnceLock<wtransport::Connection>,
    send: Mutex<Option<wtransport::SendStream>>,
    /// The peer's verified fingerprint, canonical.
    peer: OnceLock<String>,
    /// Whether the peer's certificate was checked: false at no security and
    /// for a client at server TLS, which presents none.
    proves: bool,
    /// A client: this side's own words, to name in a refusal.
    client: Option<String>,
    /// Whether anything has come from the peer since the handshake.
    heard: AtomicBool,
    /// The endpoint the connection runs on.
    ends: Ends,
    /// Last, so it stops only once the connection has been let go.
    rt: Arc<Rt>,
}

/// A connection's endpoint, waited on as a dropped connection's close
/// leaves.
enum Ends {
    Server(quinn::Endpoint),
    Client(wtransport::Endpoint<Client>),
}

impl Shared {
    fn open(
        &self,
        conn: wtransport::Connection,
        send: wtransport::SendStream,
        peer: Option<&PeerId>,
    ) {
        *lock(&self.send) = Some(send);
        if let Some(peer) = peer.filter(|_| self.proves) {
            let _ = self.peer.set(peer.canonical());
        }
        let _ = self.conn.set(conn);
    }

    fn lost(&self, e: &quinn::ConnectionError) -> io::Result<usize> {
        let early = !self.heard.load(Ordering::Acquire);
        lost(e, self.client.as_deref().filter(|_| early))
    }

    /// Ends the session without the writer, which a blocked write may hold:
    /// what was written still reaches the peer unless a write is under way,
    /// when the session closes at once as an error.
    fn shut(&self) {
        match self.send.try_lock() {
            Ok(mut send) => self.finish(send.as_mut()),
            Err(_) => {
                if let Some(conn) = self.conn.get() {
                    conn.close(wtransport::VarInt::from_u32(ABRUPT), b"ended");
                }
            }
        }
    }

    fn close(&self) {
        self.finish(lock(&self.send).as_mut());
    }

    /// Finishes the stream, waits for the peer to acknowledge it, then
    /// closes the session as a clean end.
    fn finish(&self, send: Option<&mut wtransport::SendStream>) {
        if let Some(send) = send {
            let stream = send.quic_stream_mut();
            if stream.finish().is_ok() {
                let stopped = stream.stopped();
                let _ = wait_at_most(self.rt.handle(), Some(CLOSE_WAIT), stopped);
            }
        }
        if let Some(conn) = self.conn.get() {
            conn.close(wtransport::VarInt::from_u32(H3_NO_ERROR), b"");
        }
    }
}

impl Drop for Shared {
    /// A session let go unclosed ends as an error; when it holds the last
    /// of its runtime, its close gets a moment to leave first.
    fn drop(&mut self) {
        let Some(conn) = self.conn.get() else {
            return;
        };
        conn.close(wtransport::VarInt::from_u32(ABRUPT), b"dropped");
        if Arc::strong_count(&self.rt) == 1 {
            let handle = self.rt.handle();
            let _ = match &self.ends {
                Ends::Server(endpoint) => {
                    wait_at_most(handle, Some(DROP_WAIT), endpoint.wait_idle())
                }
                Ends::Client(endpoint) => {
                    wait_at_most(handle, Some(DROP_WAIT), endpoint.wait_idle())
                }
            };
        }
    }
}

/// A certificate a listener presents, and the TLS configuration it is
/// presented in.
struct Presented {
    identity: Identity,
    tls: Arc<ServerConfig>,
    certificate: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
}

impl Presented {
    fn of(config: &TlsConfig, identity: Identity) -> io::Result<Presented> {
        let tls = config.presenting(identity.clone()).server_naming(H3)?;
        let pem = identity.to_pem().map_err(|e| invalid(words(&e)))?;
        let key = PrivateKeyDer::from_pem_slice(pem.as_bytes()).map_err(invalid)?;
        Ok(Presented {
            certificate: CertificateDer::from(identity.certificate_der().to_vec()),
            identity,
            tls,
            key,
        })
    }

    /// A certificate made now, valid for two periods of `every`, a margin
    /// on either side: one period as the next, one as the one presented.
    fn made(config: &TlsConfig, every: Duration) -> io::Result<Presented> {
        let now = SystemTime::now();
        let from = now.checked_sub(MARGIN).unwrap_or(now);
        let identity = Identity::generate_with(from, now + 2 * every + MARGIN)
            .map_err(|e| invalid(words(&e)))?;
        Presented::of(config, identity)
    }
}

/// What a listener presents: one certificate, or, rotating, the one
/// presented and the next.
struct Certs {
    presented: Arc<Presented>,
    /// Rotating: the next certificate and the period.
    next: Option<(Arc<Presented>, Duration)>,
    /// When the one presented began its period, by the wall clock, which
    /// certificates are valid by (a monotonic clock stops while a machine
    /// sleeps).
    since: SystemTime,
}

impl Certs {
    fn new(presented: Arc<Presented>, next: Option<(Arc<Presented>, Duration)>) -> Certs {
        Certs {
            presented,
            next,
            since: SystemTime::now(),
        }
    }
}

/// What every connection a listener accepts is built from.
struct Listener {
    config: TlsConfig,
    verifier: Option<Arc<dyn ClientCertVerifier>>,
    acl: Option<Arc<Acl>>,
    /// Whether clients' certificates are checked: false at no security.
    proves: bool,
    certs: Mutex<Certs>,
}

impl Listener {
    /// The certificate to present now, a rotating listener's moved on when
    /// its period has passed.
    fn current(&self) -> io::Result<Arc<Presented>> {
        let mut certs = lock(&self.certs);
        self.roll(&mut certs)?;
        Ok(Arc::clone(&certs.presented))
    }

    /// Moves a rotating listener's certificates on by the periods passed.
    fn roll(&self, certs: &mut Certs) -> io::Result<()> {
        if let Some((next, every)) = &certs.next {
            let every = *every;
            let passed = SystemTime::now()
                .duration_since(certs.since)
                .unwrap_or_default();
            let periods = passed.as_nanos() / every.as_nanos();
            if periods > 0 {
                // Periods follow each other from the first, however late they
                // are noticed. A listener asked nothing for more than a period
                // holds a next one that has expired, or would within the
                // period: a fresh one instead.
                let presented = if periods == 1 {
                    Arc::clone(next)
                } else {
                    Arc::new(Presented::made(&self.config, every)?)
                };
                certs.presented = presented;
                certs.next = Some((Arc::new(Presented::made(&self.config, every)?), every));
                certs.since = u32::try_from(periods)
                    .ok()
                    .and_then(|n| every.checked_mul(n))
                    .and_then(|passed| certs.since.checked_add(passed))
                    .unwrap_or_else(SystemTime::now);
            }
        }
        Ok(())
    }

    /// The server configuration for one connection: the certificate
    /// presented now, and at mutual TLS a client verifier scoped to the
    /// connection's address.
    fn config(
        &self,
        source: SocketAddr,
        local: SocketAddr,
        refusal: &Kept,
        denied: &Denied,
    ) -> io::Result<Arc<quinn::ServerConfig>> {
        let presented = self.current()?;
        let base = &presented.tls;
        let tls = match &self.verifier {
            Some(inner) => {
                let verifier = Arc::new(Sourced {
                    inner: admitting(
                        inner,
                        self.config.admits_first(self.acl.as_ref()).as_ref(),
                        source,
                        local,
                        denied,
                    ),
                    source,
                    refusal: Arc::clone(refusal),
                });
                Verifying::config(base, verifier, &presented.certificate, &presented.key)?
            }
            None => Arc::clone(base),
        };
        let crypto = QuicServerConfig::try_from(tls).map_err(invalid)?;
        let mut config = quinn::ServerConfig::with_crypto(Arc::new(crypto));
        config.transport_config(Arc::new(transport_config()));
        config.migration(false);
        Ok(Arc::new(config))
    }
}

/// A server's handshake, run on the serving thread's first read.
struct Pending {
    incoming: Incoming,
    listener: Arc<Listener>,
    source: SocketAddr,
    local: SocketAddr,
}

impl Pending {
    fn run(self, shared: &Shared, bound: Option<Duration>) -> io::Result<wtransport::RecvStream> {
        let until = bound.map(|b| Instant::now() + b);
        let refusal = Kept::default();
        let denied = Denied::default();
        let config = self
            .listener
            .config(self.source, self.local, &refusal, &denied)?;
        let handle = shared.rt.handle();
        // quinn spawns the connection's driver on the runtime it is called
        // inside.
        let connecting = {
            let _inside = handle.enter();
            self.incoming.accept_with(config)
        }
        .map_err(|e| io::Error::new(io::ErrorKind::ConnectionAborted, e.to_string()))?;
        let session = IncomingSessionFuture::with_quic_connecting(connecting);
        let request: SessionRequest =
            wait_at_most(handle, left(until)?, session)?.map_err(|e| {
                if let Some((peer, rule)) = denied.get() {
                    return denied_by_acl(peer, self.source, *rule);
                }
                match refusal.get().copied() {
                    Some(refused) => untrusted_by_server(Some(refused), &e),
                    None => io::Error::new(
                        io::ErrorKind::ConnectionAborted,
                        format!("the client's WebTransport handshake did not finish: {e}"),
                    ),
                }
            })?;
        let accepting = request.accept();
        let conn = wait_at_most(handle, left(until)?, accepting)?
            .map_err(|e| io::Error::new(io::ErrorKind::ConnectionAborted, e.to_string()))?;
        let refuse = |e: io::Error| {
            conn.close(wtransport::VarInt::from_u32(REFUSED), b"refused");
            e
        };
        // At server TLS the client presents no certificate.
        let peer = match self.listener.verifier {
            Some(_) => Some(fingerprint(conn.quic_connection()).map_err(refuse)?),
            None => None,
        };
        if let (Some(acl), Some(peer)) = (
            self.listener.acl.as_ref().filter(|_| self.listener.proves),
            &peer,
        ) {
            let decided = acl.decide_peer(self.source, self.local, &peer.canonical());
            if decided.outcome != Outcome::Permit {
                return Err(refuse(denied_by_acl(peer, self.source, decided.rule_index)));
            }
        }
        let (send, recv) = wait_at_most(handle, left(until)?, conn.accept_bi())?
            .map_err(|e| io::Error::new(io::ErrorKind::ConnectionAborted, e.to_string()))?;
        shared.open(conn, send, peer.as_ref());
        Ok(recv)
    }
}

/// The reading half.
struct WtRead {
    shared: Arc<Shared>,
    pending: Option<Pending>,
    recv: Option<wtransport::RecvStream>,
    /// The bound the link set on a read; the first bounds the handshake.
    timeout: Cell<Option<Duration>>,
}

impl Read for WtRead {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(pending) = self.pending.take() {
            self.recv = Some(pending.run(&self.shared, self.timeout.get())?);
        }
        let recv = self.recv.as_mut().ok_or(io::ErrorKind::NotConnected)?;
        let reading = recv.quic_stream_mut().read(buf);
        match wait_at_most(self.shared.rt.handle(), self.timeout.get(), reading)? {
            Ok(Some(n)) => {
                self.shared.heard.store(true, Ordering::Release);
                Ok(n)
            }
            Ok(None) => Ok(0),
            Err(quinn::ReadError::ConnectionLost(e)) => self.shared.lost(&e),
            Err(quinn::ReadError::Reset(code)) => Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                format!("the peer reset the link's WebTransport stream (code {code})"),
            )),
            Err(e) => Err(io::Error::other(e.to_string())),
        }
    }
}

/// The frames of the reading half, the peer the handshake proved and the
/// session's datagrams.
struct WtFrames {
    frames: StreamReader<WtRead>,
    shared: Arc<Shared>,
}

impl FrameRead for WtFrames {
    fn read_frame(&mut self, buf: &mut Vec<u8>, max: usize) -> io::Result<bool> {
        self.frames.read_frame(buf, max)
    }

    fn read_frame_in(
        &mut self,
        buf: &mut Vec<u8>,
        max: usize,
        room: &mut dyn FnMut(usize) -> Vec<u8>,
    ) -> io::Result<bool> {
        self.frames.read_frame_in(buf, max, room)
    }

    fn set_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.frames.set_timeout(timeout)
    }

    fn peer_id(&self) -> Option<String> {
        self.shared.peer.get().cloned()
    }

    fn encrypted(&self) -> bool {
        true
    }

    fn datagrams(&self) -> Option<Arc<dyn Datagrams>> {
        Some(Arc::new(WtDatagrams {
            shared: Arc::clone(&self.shared),
        }))
    }
}

/// The writing half.
struct WtWrite {
    shared: Arc<Shared>,
    timeout: Option<Duration>,
}

impl WtWrite {
    fn send(&self, parts: &[&[u8]]) -> io::Result<()> {
        let mut send = lock(&self.shared.send);
        let stream = send
            .as_mut()
            .ok_or(io::ErrorKind::NotConnected)?
            .quic_stream_mut();
        let writing = async move {
            for part in parts {
                stream.write_all(part).await?;
            }
            Ok::<_, quinn::WriteError>(())
        };
        wait_at_most(self.shared.rt.handle(), self.timeout, writing)?.map_err(|e| match e {
            quinn::WriteError::ConnectionLost(lost) => match self.shared.lost(&lost) {
                Ok(_) => io::Error::new(io::ErrorKind::BrokenPipe, "the link ended"),
                Err(e) => e,
            },
            other => io::Error::new(io::ErrorKind::BrokenPipe, other.to_string()),
        })
    }
}

impl Write for WtWrite {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.send(&[bytes])?;
        Ok(bytes.len())
    }

    fn write_vectored(&mut self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        let parts: Vec<&[u8]> = bufs.iter().map(|b| &**b).collect();
        self.send(&parts)?;
        Ok(parts.iter().map(|p| p.len()).sum())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A session's datagrams: HTTP/3 datagrams, each the payload alone to
/// either side's code (a browser's `transport.datagrams`).
struct WtDatagrams {
    shared: Arc<Shared>,
}

impl WtDatagrams {
    fn conn(&self) -> io::Result<&wtransport::Connection> {
        self.shared.conn.get().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotConnected,
                "the WebTransport handshake has not run yet",
            )
        })
    }
}

impl Datagrams for WtDatagrams {
    fn send(&self, payload: &[u8]) -> io::Result<()> {
        self.conn()?.send_datagram(payload).map_err(|e| match e {
            SendDatagramError::TooLarge => invalid(format!(
                "a datagram of {} bytes is larger than the path takes ({:?})",
                payload.len(),
                self.max_size()
            )),
            SendDatagramError::UnsupportedByPeer => {
                io::Error::new(io::ErrorKind::Unsupported, e.to_string())
            }
            SendDatagramError::NotConnected => {
                io::Error::new(io::ErrorKind::NotConnected, "the link ended")
            }
        })
    }

    fn recv(&self, timeout: Option<Duration>) -> io::Result<Option<Vec<u8>>> {
        let reading = self.conn()?.receive_datagram();
        match wait_at_most(self.shared.rt.handle(), timeout, reading) {
            Err(e) if e.kind() == io::ErrorKind::TimedOut => Ok(None),
            Err(e) => Err(e),
            Ok(Ok(datagram)) => Ok(Some(datagram.payload().to_vec())),
            Ok(Err(lost)) => Err(io::Error::new(
                io::ErrorKind::NotConnected,
                format!("the link ended: {lost}"),
            )),
        }
    }

    fn max_size(&self) -> Option<usize> {
        self.shared.conn.get()?.max_datagram_size()
    }
}

fn connection(
    shared: Shared,
    pending: Option<Pending>,
    recv: Option<wtransport::RecvStream>,
    peer: String,
) -> Connection {
    let shared = Arc::new(shared);
    let closing = Arc::clone(&shared);
    let shutting = Arc::clone(&shared);
    let reader = StreamReader::with_timeout(
        WtRead {
            shared: Arc::clone(&shared),
            pending,
            recv,
            timeout: Cell::new(None),
        },
        |r: &WtRead, t| {
            r.timeout.set(t);
            Ok(())
        },
    );
    Connection {
        reader: Box::new(WtFrames {
            frames: reader,
            shared: Arc::clone(&shared),
        }),
        writer: Box::new(
            StreamWriter::with_shutdown(
                WtWrite {
                    shared,
                    timeout: None,
                },
                move || shutting.close(),
            )
            .bounded_by(|w: &mut WtWrite, t| {
                w.timeout = t;
                Ok(())
            }),
        ),
        peer,
        closer: Some(Box::new(move || closing.shut())),
    }
}

/// Connects to a WebTransport server at `https://host:port/path`, as a
/// browser does: the QUIC and HTTP/3 handshakes run in
/// [`connect`](Connect::connect), so a server this side does not trust
/// fails it there.
pub struct WtConnect {
    url: String,
    tls: Arc<rustls::ClientConfig>,
    watched: Arc<Watched>,
    /// Held while connecting, so a refusal kept is that connection's.
    connecting: Mutex<()>,
    me: String,
    /// Whether the server's certificate is checked: false at no security.
    proves: bool,
    timeout: Duration,
    rt: Arc<Rt>,
}

impl core::fmt::Debug for WtConnect {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WtConnect")
            .field("url", &self.url)
            .field("as", &self.me)
            .finish()
    }
}

/// `url`'s `host:port`, after checking it is an `https://` URL.
fn authority(url: &str) -> io::Result<&str> {
    let rest = url.strip_prefix("https://").ok_or_else(|| {
        invalid(format!(
            "`{url}` is not a WebTransport URL: one is https://host:port/path"
        ))
    })?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() {
        return Err(invalid(format!("`{url}` names no host")));
    }
    Ok(authority)
}

impl WtConnect {
    /// To `url` (`https://host:port/path`), as `config` says (the same
    /// configuration as [`QuicConnect`](super::quic::QuicConnect)), within
    /// [`CONNECT_TIMEOUT`].
    pub fn new(url: &str, config: &TlsConfig) -> io::Result<WtConnect> {
        // The URL's host is the name the server's certificate is checked
        // for under roots: wtransport dials by it.
        server_name(host_of(authority(url)?), config.by_roots())?;
        let verifier = config
            .server_trust()?
            .server_cert_verifier_with(config.tls_policy())
            .map_err(|e| invalid(words(&e)))?;
        let watched = Arc::new(Watched {
            inner: verifier,
            refusal: Mutex::new(None),
        });
        let mut tls: rustls::ClientConfig = (*config.client_naming(H3)?).clone();
        tls.dangerous()
            .set_certificate_verifier(Arc::clone(&watched) as Arc<dyn ServerCertVerifier>);
        Ok(WtConnect {
            url: url.to_string(),
            tls: Arc::new(tls),
            watched,
            connecting: Mutex::new(()),
            me: config.me(),
            proves: config.proves_peers(),
            timeout: CONNECT_TIMEOUT,
            rt: Rt::new()?,
        })
    }

    /// To `url` at no security: TLS whose certificate nobody checks, the
    /// link's key proof deciding. A server listening with
    /// [`WtListen::bind_trusted`] answers it.
    pub fn trusted(url: &str) -> io::Result<WtConnect> {
        WtConnect::new(url, &TlsConfig::checking_nobody()?.server_tls())
    }

    /// Within `timeout` as a whole: the QUIC and HTTP/3 handshakes and the
    /// link's stream.
    pub fn timeout(mut self, timeout: Duration) -> WtConnect {
        self.timeout = timeout;
        self
    }

    /// A connection's failure in words a person can act on.
    fn refusal(&self, e: ConnectingError) -> io::Error {
        let addr = authority(&self.url).unwrap_or(&self.url);
        if let Some(r) = *lock(&self.watched.refusal) {
            return untrusted_server(r, addr, host_of(addr));
        }
        match e {
            ConnectingError::ConnectionError(ConnectionError::TimedOut) => io::Error::new(
                io::ErrorKind::TimedOut,
                format!("the WebTransport server at {addr} did not answer in time"),
            ),
            ConnectingError::ConnectionError(ConnectionError::ConnectionClosed(close)) => {
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "the server at {addr} ended the WebTransport handshake ({close}): it may not trust this client, {}",
                        self.me
                    ),
                )
            }
            // A server refusing a client after the TLS handshake (its access
            // control list, by fingerprint) closes the connection as the
            // session opens.
            ConnectingError::ConnectionError(ConnectionError::ApplicationClosed(close)) => {
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "the server at {addr} ended the WebTransport session as it opened ({close}): most likely its access control list refuses this client, {}",
                        self.me
                    ),
                )
            }
            ConnectingError::ConnectionError(ConnectionError::QuicProto(why)) => io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("the WebTransport handshake with the server at {addr} failed: {why}"),
            ),
            ConnectingError::SessionRejected => io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("the server at {addr} refused the WebTransport session"),
            ),
            other => io::Error::new(
                io::ErrorKind::ConnectionRefused,
                format!("the WebTransport handshake with the server at {addr} failed: {other}"),
            ),
        }
    }
}

impl Connect for WtConnect {
    fn connect(&self) -> io::Result<Connection> {
        let _one = lock(&self.connecting);
        *lock(&self.watched.refusal) = None;
        let config = wtransport::ClientConfig::builder()
            .with_bind_default()
            .with_custom_tls_and_transport((*self.tls).clone(), transport_config())
            .build();
        let handle = self.rt.handle();
        let until = Some(Instant::now() + self.timeout);
        let endpoint = {
            let _inside = handle.enter();
            wtransport::Endpoint::client(config)?
        };
        let conn = wait_at_most(handle, left(until)?, endpoint.connect(&self.url))?
            .map_err(|e| self.refusal(e))?;
        let peer = fingerprint(conn.quic_connection())?;
        let remote = conn.remote_address();
        let opening = async {
            match conn.open_bi().await {
                Ok(opening) => opening.await.map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            }
        };
        let (send, recv) = wait_at_most(handle, left(until)?, opening)?.map_err(|e| {
            ended_before_answering(&io::Error::new(io::ErrorKind::ConnectionReset, e), &self.me)
        })?;
        let shared = Shared {
            conn: OnceLock::new(),
            send: Mutex::new(None),
            peer: OnceLock::new(),
            proves: self.proves,
            client: Some(self.me.clone()),
            heard: AtomicBool::new(false),
            ends: Ends::Client(endpoint),
            rt: Arc::clone(&self.rt),
        };
        shared.open(conn, send, Some(&peer));
        // A dual-stack socket sees an IPv4 server as IPv4-mapped.
        let remote = SocketAddr::new(remote.ip().to_canonical(), remote.port());
        Ok(connection(
            shared,
            None,
            Some(recv),
            format!("https {remote}"),
        ))
    }

    fn describe(&self) -> String {
        self.url.clone()
    }
}

/// A WebTransport listener: HTTP/3 over QUIC, at mutual TLS, server TLS
/// or no security as its configuration says. It admits peers by the
/// policy's access control list, every source when there is none: by
/// address as it accepts, refusing a peer it denies before any handshake;
/// at mutual TLS by fingerprint once the handshake, which runs on the
/// serving thread's first read, has proved one. Every session is taken,
/// whatever its path.
pub struct WtListen {
    endpoint: quinn::Endpoint,
    listener: Arc<Listener>,
    rt: Arc<Rt>,
}

impl core::fmt::Debug for WtListen {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WtListen")
            .field("addr", &self.endpoint.local_addr().ok())
            .field("acl", &self.listener.acl.is_some())
            .finish()
    }
}

impl WtListen {
    /// Binds the UDP `address` (a wildcard included) as `config` says,
    /// presenting its identity.
    pub fn bind(address: &str, config: &TlsConfig) -> io::Result<WtListen> {
        WtListen::from_socket(UdpSocket::bind(first(address)?)?, config)
    }

    /// Binds the UDP `address` at no security, admitting the peers `acl`
    /// permits by address: a certificate it made itself, taking any
    /// client's, the link's key proof deciding.
    pub fn bind_trusted(address: &str, acl: Acl) -> io::Result<WtListen> {
        WtListen::from_socket_trusted(UdpSocket::bind(first(address)?)?, acl)
    }

    /// A UDP socket the caller bound itself, at no security. No client is
    /// asked for a certificate: a page has none to present.
    pub fn from_socket_trusted(socket: UdpSocket, acl: Acl) -> io::Result<WtListen> {
        let mut config = TlsConfig::checking_nobody()?.server_tls();
        let mut policy = config.tls_policy().clone();
        policy.acl = Some(acl);
        config = config.policy(policy);
        WtListen::from_socket(socket, &config)
    }

    /// A UDP socket the caller bound itself, as `config` says.
    pub fn from_socket(socket: UdpSocket, config: &TlsConfig) -> io::Result<WtListen> {
        let acl = match config.tls_policy().acl.clone() {
            Some(acl) => Some(Arc::new(canonical_acl(acl)?)),
            None => None,
        };
        let verifier: Option<Arc<dyn ClientCertVerifier>> = if config.is_mutual() {
            Some(
                config
                    .client_trust()?
                    .client_cert_verifier_with(config.tls_policy())
                    .map_err(|e| invalid(words(&e)))?,
            )
        } else {
            None
        };
        let presented = Presented::of(config, config.identity().clone())?;
        let crypto = QuicServerConfig::try_from(Arc::clone(&presented.tls))
            .map_err(|e| invalid(format!("the TLS policy does not fit QUIC: {e}")))?;
        let mut server = quinn::ServerConfig::with_crypto(Arc::new(crypto));
        server.transport_config(Arc::new(transport_config()));
        let rt = Rt::new()?;
        let endpoint = {
            let _inside = rt.handle().enter();
            quinn::Endpoint::new(
                quinn::EndpointConfig::default(),
                Some(server),
                socket,
                Arc::new(quinn::TokioRuntime),
            )?
        };
        Ok(WtListen {
            endpoint,
            listener: Arc::new(Listener {
                config: config.clone(),
                verifier,
                acl,
                proves: config.proves_peers(),
                certs: Mutex::new(Certs::new(Arc::new(presented), None)),
            }),
            rt,
        })
    }

    /// Presents certificates it makes itself rather than the
    /// configuration's identity, a new one every `every` (at most
    /// [`ROTATE_MAX`]; [`ROTATE_EVERY`] is six days): each valid for two
    /// periods and two hours, which a browser takes pinned by its hash. A
    /// page pins both of [`certificate_hashes`](WtListen::certificate_hashes).
    pub fn rotating(self, every: Duration) -> io::Result<WtListen> {
        if every.is_zero() || every > ROTATE_MAX {
            return Err(invalid(format!(
                "a certificate is presented for {every:?}: a browser takes one pinned by its hash only for at most 14 days, so the period is at most {ROTATE_MAX:?} and more than zero"
            )));
        }
        let presented = Arc::new(Presented::made(&self.listener.config, every)?);
        let next = Arc::new(Presented::made(&self.listener.config, every)?);
        *lock(&self.listener.certs) = Certs::new(presented, Some((next, every)));
        Ok(self)
    }

    /// Presents `identity` rather than the configuration's (or a rotating
    /// one's): a certificate an authority issued, for pages that trust it as
    /// they trust any site, at any security.
    pub fn presenting(self, identity: Identity) -> io::Result<WtListen> {
        let presented = Arc::new(Presented::of(&self.listener.config, identity)?);
        *lock(&self.listener.certs) = Certs::new(presented, None);
        Ok(self)
    }

    /// The fingerprints (`sha256:<hex>`, the SHA-256 of each certificate, what
    /// a page names in `serverCertificateHashes`) of the certificate presented
    /// now and, rotating, the next one.
    pub fn certificate_hashes(&self) -> io::Result<Vec<PeerId>> {
        let mut certs = lock(&self.listener.certs);
        self.listener.roll(&mut certs)?;
        let mut hashes = vec![certs.presented.identity.fingerprint()];
        if let Some((next, _)) = &certs.next {
            hashes.push(next.identity.fingerprint());
        }
        Ok(hashes)
    }

    /// Where it listens.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.endpoint.local_addr()
    }

    /// The next connection, before its handshake, or a peer the list
    /// denies by address, refused before any handshake.
    pub fn accept(&self) -> io::Result<Accepted> {
        let incoming = loop {
            let incoming = wait(self.rt.handle(), self.endpoint.accept()).ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotConnected, "the QUIC endpoint is closed")
            })?;
            // A first packet's address may be forged: a Retry proves the
            // client receives at it before the list decides.
            if incoming.remote_address_validated() || !incoming.may_retry() {
                break incoming;
            }
            let _ = incoming.retry();
        };
        let source = incoming.remote_address();
        let bound = self.endpoint.local_addr()?;
        let local = match incoming.local_ip() {
            Some(ip) => Some(SocketAddr::new(ip, bound.port())),
            None if !bound.ip().is_unspecified() => Some(bound),
            None => None,
        };
        if let Some(acl) = &self.listener.acl {
            let denied = match local {
                Some(local) => acl.decide(source, local).outcome == Outcome::Deny,
                None => {
                    acl.rules.iter().any(|r| r.destination.is_some())
                        || acl.decide(source, bound).outcome == Outcome::Deny
                }
            };
            if denied {
                incoming.refuse();
                return Ok(Accepted::Refused(source));
            }
        }
        let shared = Shared {
            conn: OnceLock::new(),
            send: Mutex::new(None),
            peer: OnceLock::new(),
            proves: self.listener.proves && self.listener.verifier.is_some(),
            client: None,
            heard: AtomicBool::new(false),
            ends: Ends::Server(self.endpoint.clone()),
            rt: Arc::clone(&self.rt),
        };
        let pending = Pending {
            incoming,
            listener: Arc::clone(&self.listener),
            source,
            local: local.unwrap_or(bound),
        };
        Ok(Accepted::Connection(connection(
            shared,
            Some(pending),
            None,
            format!("https {source}"),
        )))
    }
}

fn first(address: &str) -> io::Result<SocketAddr> {
    address
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| invalid("no address to bind"))
}
