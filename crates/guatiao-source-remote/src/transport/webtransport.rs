// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! WebTransport: HTTP/3 over QUIC, what a browser opens with
//! `new WebTransport("https://host:port/")`, through coqui-web. A link is
//! one WebTransport session carrying one bidirectional stream, the frames
//! on it as on `quic://` (each behind its `u32` length); [`Datagrams`](super::Datagrams)
//! are the session's own.
//!
//! The TLS is `quic://`'s configuration, trust and policy: the server's
//! certificate is verified by its pins, trust store or roots, and the
//! policy's access control list decides a client's address before any
//! handshake (a Retry proves the address first). At no security
//! ([`WtConnect::trusted`], [`WtListen::bind_trusted`]) a client presents
//! no certificate, a native one checks none, and the link's key proof
//! decides; a page's browser still checks the one it was told to trust.
//!
//! At mutual TLS a client presents a certificate, which the channel
//! reports (`peer_certificates`) once the handshake is through: the link
//! names the peer by it. The handshake of a connection runs on coqui-web's
//! threads, where coqui-peer-tls has no connection addresses to decide the
//! policy's list by, so the list's address rules are decided before the
//! handshake and its fingerprint rules after it.
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
use std::sync::Mutex;
use std::time::SystemTime;

use coqui_net_acl::Acl;
use coqui_peer_tls::{Identity, PeerId};
use coqui_web::webtransport::{Dialer as WtDialer, Listener};
use coqui_web::{Channel, Dialer as _, Failure, Kind, Limits};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use url::Url;

use super::tcp::{Accepted, CONNECT_TIMEOUT, bind_address};
use super::tls::{TlsConfig, host_of, invalid, server_name, unbuilt, words};
use super::web::{
    Open, Refused, Shared, Watching, admits, connection, io_error, list_permits, server_config,
};
use super::{Connect, Connection};
use crate::lock;

/// HTTP/3's application protocol, which a WebTransport session runs in.
const H3: &[u8] = b"h3";

/// How often a listener looks up from waiting for a session to report the
/// sources its list turned away.
const REPORT_EVERY: Duration = Duration::from_millis(100);

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

/// A certificate a listener presents, with the key it signs by.
struct Presented {
    identity: Identity,
    key: Arc<CertifiedKey>,
}

impl Presented {
    fn of(identity: Identity) -> io::Result<Presented> {
        let provider = coqui_peer_tls::crypto_provider();
        let signing = provider
            .key_provider
            .load_private_key(identity.private_key_der())
            .map_err(|e| invalid(format!("the identity's key cannot sign: {e}")))?;
        let key = Arc::new(CertifiedKey::new(identity.certificate_chain(), signing));
        Ok(Presented { identity, key })
    }

    /// A certificate made now, valid for two periods of `every`, a margin
    /// on either side: one period as the next, one as the one presented.
    fn made(every: Duration) -> io::Result<Presented> {
        let now = SystemTime::now();
        let from = now.checked_sub(MARGIN).unwrap_or(now);
        let identity = Identity::generate_with(from, now + 2 * every + MARGIN)
            .map_err(|e| invalid(words(&e)))?;
        Presented::of(identity)
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

/// The certificates every session a listener accepts is presented, which
/// the TLS configuration asks for at each handshake.
struct Presenting {
    certs: Mutex<Certs>,
}

impl core::fmt::Debug for Presenting {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Presenting").finish_non_exhaustive()
    }
}

impl Presenting {
    /// The certificate to present now, a rotating listener's moved on when
    /// its period has passed.
    fn current(&self) -> io::Result<Arc<Presented>> {
        let mut certs = lock(&self.certs);
        Presenting::roll(&mut certs)?;
        Ok(Arc::clone(&certs.presented))
    }

    /// Moves a rotating listener's certificates on by the periods passed.
    fn roll(certs: &mut Certs) -> io::Result<()> {
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
                    Arc::new(Presented::made(every)?)
                };
                certs.presented = presented;
                certs.next = Some((Arc::new(Presented::made(every)?), every));
                certs.since = u32::try_from(periods)
                    .ok()
                    .and_then(|n| every.checked_mul(n))
                    .and_then(|passed| certs.since.checked_add(passed))
                    .unwrap_or_else(SystemTime::now);
            }
        }
        Ok(())
    }
}

impl ResolvesServerCert for Presenting {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        self.current().ok().map(|p| Arc::clone(&p.key))
    }
}

/// No security: a generated identity checking no certificate, the peers
/// `acl` permits admitted by address.
fn trusted(acl: Acl) -> io::Result<TlsConfig> {
    let config = TlsConfig::checking_nobody()?;
    let mut policy = config.tls_policy().clone();
    policy.acl = Some(acl);
    Ok(config.policy(policy))
}

/// The bounds of a link's channel: coqui-web's, with the largest message
/// a link allows set by the link as it reads.
fn limits() -> Limits {
    Limits::default()
}

/// A bad `https://` URL: `host:port` of the URL, or why it is not one.
fn authority(url: &str) -> io::Result<String> {
    let parsed = Url::parse(url).map_err(|_| {
        invalid(format!(
            "`{url}` is not a WebTransport URL: one is https://host:port/path"
        ))
    })?;
    if parsed.scheme() != "https" {
        return Err(invalid(format!(
            "`{url}` is not a WebTransport URL: one is https://host:port/path"
        )));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| invalid(format!("`{url}` names no host")))?;
    // An IPv6 host comes bracketed.
    let port = parsed.port_or_known_default().unwrap_or(443);
    Ok(format!("{host}:{port}"))
}

/// Connects to a WebTransport server at `https://host:port/path`, as a
/// browser does: the QUIC and HTTP/3 handshakes run in
/// [`connect`](Connect::connect), so a server this side does not trust
/// fails it there.
pub struct WtConnect {
    url: String,
    tls: Arc<rustls::ClientConfig>,
    verifier: Arc<dyn rustls::client::danger::ServerCertVerifier>,
    me: String,
    /// Whether the server's certificate is checked: false at no security.
    proves: bool,
    timeout: Duration,
}

impl core::fmt::Debug for WtConnect {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WtConnect")
            .field("url", &self.url)
            .field("as", &self.me)
            .finish()
    }
}

impl WtConnect {
    /// To `url` (`https://host:port/path`), as `config` says (the same
    /// configuration as [`QuicConnect`](super::quic::QuicConnect)'s),
    /// within [`CONNECT_TIMEOUT`].
    pub fn new(url: &str, config: &TlsConfig) -> io::Result<WtConnect> {
        // The URL's host is the name the server's certificate is checked
        // for under roots.
        server_name(host_of(&authority(url)?), config.by_roots())?;
        let verifier = config
            .server_trust()?
            .server_cert_verifier_with(config.tls_policy())
            .map_err(|e| unbuilt(&e))?;
        Ok(WtConnect {
            url: url.to_string(),
            tls: config.client_naming(H3)?,
            verifier,
            me: config.me(),
            proves: config.proves_peers(),
            timeout: CONNECT_TIMEOUT,
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
    fn refusal(&self, failure: Failure, watching: &Watching) -> io::Error {
        let addr = authority(&self.url).unwrap_or_else(|_| self.url.clone());
        if let Some(rejection) = watching.rejection(&addr, host_of(&addr)) {
            return rejection;
        }
        match failure {
            Failure::Timeout => io::Error::new(
                io::ErrorKind::TimedOut,
                format!("the WebTransport server at {addr} did not answer in time"),
            ),
            Failure::PathLost => io::Error::new(
                io::ErrorKind::ConnectionRefused,
                format!("the WebTransport handshake with the server at {addr} did not finish"),
            ),
            // QUIC's CONNECTION_REFUSED, which a list's refusal is and so is
            // any server declining a connection: the client cannot tell
            // which.
            Failure::Permission => io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "the WebTransport server at {addr} refused the connection (by its access control list, which does not admit this client's address, or the server refused the connection): this client is {}",
                    self.me
                ),
            ),
            other => io_error(other),
        }
    }
}

impl Connect for WtConnect {
    fn connect(&self) -> io::Result<Connection> {
        let url = Url::parse(&self.url).map_err(|e| invalid(e.to_string()))?;
        // One verifier for this connection, so what it saw is this
        // connection's.
        let watching = Watching::new(Arc::clone(&self.verifier));
        let mut tls = (*self.tls).clone();
        tls.dangerous().set_certificate_verifier(
            Arc::clone(&watching) as Arc<dyn rustls::client::danger::ServerCertVerifier>
        );
        let mut dialer = WtDialer::with_tls(Arc::new(tls))
            .map_err(io_error)?
            .with_limits(limits());
        let channel = dialer
            .dial(Kind::WebTransport, &url, self.timeout)
            .map_err(|failure| self.refusal(failure, &watching))?;
        let shared = Shared::new(true, true, self.proves);
        shared.open(Arc::new(channel) as Arc<dyn Channel>);
        let at = authority(&self.url)?;
        Ok(connection(shared, None, format!("https {at}")))
    }

    fn describe(&self) -> String {
        self.url.clone()
    }
}

/// A WebTransport listener: HTTP/3 over QUIC, at mutual TLS, server TLS or
/// no security as its configuration says. It admits peers by the policy's
/// access control list, every source when there is none: by address,
/// refusing a peer it denies before any handshake, and at mutual TLS by
/// fingerprint once the client's certificate is known, before any frame is
/// read. A session's handshake, HTTP/3 session and stream run on the
/// serving thread's first read, which the closer cancels.
pub struct WtListen {
    listener: Listener,
    presenting: Arc<Presenting>,
    refused: Arc<Refused>,
    config: Arc<TlsConfig>,
    bound: SocketAddr,
}

impl core::fmt::Debug for WtListen {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WtListen")
            .field("addr", &self.listener.local_addr())
            .finish()
    }
}

impl WtListen {
    /// Binds the UDP `address` (a wildcard included) as `config` says,
    /// presenting its identity.
    pub fn bind(address: &str, config: &TlsConfig) -> io::Result<WtListen> {
        let address = bind_address(address)?;
        let presenting = Arc::new(Presenting {
            certs: Mutex::new(Certs::new(
                Arc::new(Presented::of(config.identity().clone())?),
                None,
            )),
        });
        let tls = server_config(
            config,
            H3,
            Some(Arc::clone(&presenting) as Arc<dyn ResolvesServerCert>),
            false,
        )?;
        let listener = Listener::bind(address, tls).map_err(io_error)?;
        let bound = listener.local_addr();
        let refused = Arc::new(Refused::default());
        let turned = Arc::clone(&refused);
        let config = Arc::new(config.clone());
        let judging = Arc::clone(&config);
        let listener = listener
            .with_limits(limits())
            .with_admission(move |source| {
                let permitted = admits(&judging, source, bound);
                if !permitted {
                    turned.add(source);
                }
                permitted
            });
        Ok(WtListen {
            listener,
            presenting,
            refused,
            config,
            bound,
        })
    }

    /// Binds the UDP `address` at no security, admitting the peers `acl`
    /// permits by address: a certificate it made itself, taking any
    /// client's, the link's key proof deciding.
    pub fn bind_trusted(address: &str, acl: Acl) -> io::Result<WtListen> {
        WtListen::bind(address, &trusted(acl)?.server_tls())
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
        let presented = Arc::new(Presented::made(every)?);
        let next = Arc::new(Presented::made(every)?);
        *lock(&self.presenting.certs) = Certs::new(presented, Some((next, every)));
        Ok(self)
    }

    /// Presents `identity` rather than the configuration's (or a rotating
    /// one's): a certificate an authority issued, for pages that trust it as
    /// they trust any site, at any security.
    pub fn presenting(self, identity: Identity) -> io::Result<WtListen> {
        let presented = Arc::new(Presented::of(identity)?);
        *lock(&self.presenting.certs) = Certs::new(presented, None);
        Ok(self)
    }

    /// The fingerprints (`sha256:<hex>`, the SHA-256 of each certificate, what
    /// a page names in `serverCertificateHashes`) of the certificate presented
    /// now and, rotating, the next one.
    pub fn certificate_hashes(&self) -> io::Result<Vec<PeerId>> {
        let mut certs = lock(&self.presenting.certs);
        Presenting::roll(&mut certs)?;
        let mut hashes = vec![certs.presented.identity.fingerprint()];
        if let Some((next, _)) = &certs.next {
            hashes.push(next.identity.fingerprint());
        }
        Ok(hashes)
    }

    /// Where it listens.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(self.listener.local_addr())
    }

    /// The next session, before its handshake, or a peer the list denies by
    /// address, refused before any handshake.
    pub fn accept(&self) -> io::Result<Accepted> {
        loop {
            if let Some(source) = self.refused.take() {
                return Ok(Accepted::Refused(source));
            }
            match self.listener.next(Some(REPORT_EVERY)) {
                Ok(pending) => {
                    let source = pending.peer_addr();
                    let cancel = pending.canceller();
                    let mutual = self.config.is_mutual() && self.config.proves_peers();
                    let (config, bound) = (Arc::clone(&self.config), self.bound);
                    let open: Open = Box::new(move || {
                        let channel = pending.handshake().map_err(io_error)?;
                        if mutual {
                            // The list's fingerprint rules: the handshake had
                            // no addresses to decide them by.
                            if let Err(e) =
                                list_permits(&config, source, bound, channel.peer_certificates())
                            {
                                coqui_web::Channel::close(&channel);
                                return Err(e);
                            }
                        }
                        Ok(Arc::new(channel) as Arc<dyn Channel>)
                    });
                    let mutual = self.config.is_mutual() && self.config.proves_peers();
                    let shared = Shared::new(true, true, mutual);
                    return Ok(Accepted::Connection(connection(
                        shared,
                        Some((open, cancel)),
                        format!("https {source}"),
                    )));
                }
                Err(Failure::Timeout) => {}
                Err(failure) => return Err(io_error(failure)),
            }
        }
    }
}
