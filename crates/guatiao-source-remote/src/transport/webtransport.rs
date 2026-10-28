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
use core::net::SocketAddr;
use core::time::Duration;
use std::io;
use std::net::UdpSocket;
use std::sync::Mutex;
use std::time::{Instant, SystemTime};

use coqui_net_acl::Acl;
use coqui_peer_tls::{Identity, PeerId, Trust};
use quinn::{Incoming, VarInt};
use rustls::ServerConfig;
use rustls::client::danger::ServerCertVerifier;
use wtransport::endpoint::endpoint_side::Client;
use wtransport::endpoint::{IncomingSessionFuture, SessionRequest};
use wtransport::error::{ConnectingError, ConnectionError, SendDatagramError};

use super::quic::{
    CLEAN, Handshake, Kept, Over, REFUSED, Rt, Shared, Watched, accept_incoming, checked,
    connection, fingerprint, for_connection, for_quinn, left, serving, transport_config, trusted,
    wait_at_most,
};
use super::tcp::{Accepted, CONNECT_TIMEOUT, bind_address};
use super::tls::{
    TlsConfig, ended_before_answering, host_of, invalid, server_name, unbuilt, untrusted_by_server,
    untrusted_server, words,
};
use super::{Connect, Connection, Datagrams};
use crate::lock;

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

/// A link over a WebTransport session: wtransport's connection and stream
/// over quinn's, ended as `quic://`'s are.
struct Wt;

/// A connection's endpoint, waited on as a dropped connection's close
/// leaves.
enum Ends {
    Server(quinn::Endpoint),
    Client(wtransport::Endpoint<Client>),
}

impl Over for Wt {
    type Conn = wtransport::Connection;
    type Stream = wtransport::SendStream;
    type Recv = wtransport::RecvStream;
    type Pending = Pending;
    type Ends = Ends;
    const NOUN: &'static str = "WebTransport session";
    const STREAM: &'static str = "WebTransport stream";
    const CLEAN: u32 = H3_NO_ERROR;

    fn clean(code: VarInt) -> bool {
        code == VarInt::from_u32(CLEAN) || code == VarInt::from_u32(H3_NO_ERROR)
    }

    fn quic(conn: &wtransport::Connection) -> &quinn::Connection {
        conn.quic_connection()
    }

    fn stream(stream: &mut wtransport::SendStream) -> &mut quinn::SendStream {
        stream.quic_stream_mut()
    }

    fn recv(stream: &mut wtransport::RecvStream) -> &mut quinn::RecvStream {
        stream.quic_stream_mut()
    }

    fn datagrams(shared: &Arc<Shared<Wt>>) -> Arc<dyn Datagrams> {
        Arc::new(WtDatagrams {
            shared: Arc::clone(shared),
        })
    }

    async fn idle(ends: &Ends) {
        match ends {
            Ends::Server(endpoint) => endpoint.wait_idle().await,
            Ends::Client(endpoint) => endpoint.wait_idle().await,
        }
    }
}

/// A certificate a listener presents, and the TLS configuration that
/// presents it to a connection that asks no certificate or checks none.
struct Presented {
    identity: Identity,
    tls: Arc<ServerConfig>,
}

impl Presented {
    fn of(config: &TlsConfig, identity: Identity) -> io::Result<Presented> {
        // Without the list, as `quic://`'s listener (see `for_connection`).
        let tls = config
            .listing_nobody()
            .presenting(identity.clone())
            .server_naming(H3)?;
        Ok(Presented { identity, tls })
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
    /// Its identity, whom it trusts and the policy, its list included.
    config: TlsConfig,
    /// At mutual TLS checking certificates: whom each connection's own
    /// configuration verifies its client by.
    trust: Option<Trust>,
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
    /// presented now, and at mutual TLS a client verifier made for the
    /// connection's addresses.
    fn config(
        &self,
        source: SocketAddr,
        local: SocketAddr,
        refusal: &Kept,
    ) -> io::Result<Arc<quinn::ServerConfig>> {
        let presented = self.current()?;
        let tls = match &self.trust {
            Some(trust) => for_connection(
                &self.config,
                &presented.identity,
                &presented.tls,
                trust,
                H3,
                source,
                local,
                refusal,
            )?,
            None => Arc::clone(&presented.tls),
        };
        for_quinn(tls)
    }
}

/// A server's handshake, run on the serving thread's first read.
struct Pending {
    incoming: Incoming,
    listener: Arc<Listener>,
    source: SocketAddr,
    local: SocketAddr,
}

impl Handshake<Wt> for Pending {
    fn run(
        self,
        shared: &Shared<Wt>,
        bound: Option<Duration>,
    ) -> io::Result<wtransport::RecvStream> {
        let until = bound.map(|b| Instant::now() + b);
        let refusal = Kept::default();
        let config = self.listener.config(self.source, self.local, &refusal)?;
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
        // At server TLS the client presents no certificate, and at no
        // security the one it presents proves nothing.
        let peer = if self.listener.trust.is_some() {
            Some(fingerprint(conn.quic_connection()).map_err(refuse)?)
        } else {
            None
        };
        let (send, recv) = wait_at_most(handle, left(until)?, conn.accept_bi())?
            .map_err(|e| io::Error::new(io::ErrorKind::ConnectionAborted, e.to_string()))?;
        shared.open(conn, send, peer.as_ref());
        Ok(recv)
    }
}

/// A session's datagrams: HTTP/3 datagrams, each the payload alone to
/// either side's code (a browser's `transport.datagrams`).
struct WtDatagrams {
    shared: Arc<Shared<Wt>>,
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
            .map_err(|e| unbuilt(&e))?;
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
            // A server refusing a client after the TLS handshake (no
            // certificate it can read) closes the connection as the session
            // opens.
            ConnectingError::ConnectionError(ConnectionError::ApplicationClosed(close)) => {
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "the server at {addr} ended the WebTransport session as it opened ({close}): it refused this client, {}",
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
        let shared = Shared::<Wt>::new(
            self.proves,
            Some(self.me.clone()),
            Ends::Client(endpoint),
            Arc::clone(&self.rt),
        );
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
/// at mutual TLS by fingerprint as the client presents its certificate, in
/// the handshake, which runs on the serving thread's first read. Every
/// session is taken, whatever its path.
pub struct WtListen {
    endpoint: quinn::Endpoint,
    listener: Arc<Listener>,
    rt: Arc<Rt>,
}

impl core::fmt::Debug for WtListen {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WtListen")
            .field("addr", &self.endpoint.local_addr().ok())
            .field("acl", &self.listener.config.tls_policy().acl.is_some())
            .finish()
    }
}

impl WtListen {
    /// Binds the UDP `address` (a wildcard included) as `config` says,
    /// presenting its identity.
    pub fn bind(address: &str, config: &TlsConfig) -> io::Result<WtListen> {
        WtListen::from_socket(UdpSocket::bind(bind_address(address)?)?, config)
    }

    /// Binds the UDP `address` at no security, admitting the peers `acl`
    /// permits by address: a certificate it made itself, taking any
    /// client's, the link's key proof deciding.
    pub fn bind_trusted(address: &str, acl: Acl) -> io::Result<WtListen> {
        WtListen::from_socket_trusted(UdpSocket::bind(bind_address(address)?)?, acl)
    }

    /// A UDP socket the caller bound itself, at no security. No client is
    /// asked for a certificate: a page has none to present.
    pub fn from_socket_trusted(socket: UdpSocket, acl: Acl) -> io::Result<WtListen> {
        WtListen::from_socket(socket, &trusted(acl)?.server_tls())
    }

    /// A UDP socket the caller bound itself, as `config` says.
    pub fn from_socket(socket: UdpSocket, config: &TlsConfig) -> io::Result<WtListen> {
        checked(config, H3)?;
        let trust = if config.is_mutual() && config.proves_peers() {
            Some(config.client_trust()?)
        } else {
            None
        };
        let presented = Presented::of(config, config.identity().clone())?;
        let (endpoint, rt) = serving(socket, Arc::clone(&presented.tls))?;
        Ok(WtListen {
            endpoint,
            listener: Arc::new(Listener {
                config: config.clone(),
                trust,
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
        let (incoming, source, local) =
            match accept_incoming(&self.endpoint, &self.rt, &self.listener.config)? {
                Ok(accepted) => accepted,
                Err(refused) => return Ok(Accepted::Refused(refused)),
            };
        let shared = Shared::<Wt>::new(
            self.listener.trust.is_some(),
            None,
            Ends::Server(self.endpoint.clone()),
            Arc::clone(&self.rt),
        );
        let pending = Pending {
            incoming,
            listener: Arc::clone(&self.listener),
            source,
            local,
        };
        Ok(Accepted::Connection(connection(
            shared,
            Some(pending),
            None,
            format!("https {source}"),
        )))
    }
}
