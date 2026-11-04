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
//! threads, so each connection's TLS configuration is built from its
//! source and local address (`with_tls_per_connection`, through
//! coqui-peer-tls's `ServerConfigs`): the policy's list, with its
//! fingerprint rules, and a trust store's waiting entries see the
//! connection's addresses.
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
use coqui_peer_tls::{ConnectionConfig, Identity, PeerId};
use coqui_web::webtransport::{Dialer as WtDialer, Listener};
use coqui_web::{Channel, Dialer as _, Failure, Kind, Limits};
use rustls::ServerConfig;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use url::Url;

use super::tcp::{Accepted, CONNECT_TIMEOUT, bind_address};
use super::tls::{
    TlsConfig, host_of, invalid, server_name, unbuilt, untrusted_by_server, untrusted_client, words,
};
use super::web::{
    Open, Refused, Shared, Watching, admits, canonical_list, connection, io_error, list_permits,
    server_config,
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

/// The list a listener asks again once a handshake is through: only where a
/// resumed session would skip the client verifier (`allow_resumption` at
/// mutual TLS), else `None` because the handshake already decided it.
fn resuming_list(config: &TlsConfig) -> io::Result<Option<Arc<Acl>>> {
    if config.tls_policy().allow_resumption && config.is_mutual() && config.proves_peers() {
        canonical_list(config)
    } else {
        Ok(None)
    }
}

/// Decides a client whose handshake is through, by the list's fingerprint
/// rules for the certificate it presented (`chain`, its own first), from
/// `source` to `local`: `Ok` where there is no list or it permits; else the
/// refusal, after `refuse` is told its words.
fn after_handshake(
    list: Option<&Acl>,
    source: SocketAddr,
    local: SocketAddr,
    chain: Option<&[Vec<u8>]>,
    refuse: impl FnOnce(&str),
) -> io::Result<()> {
    let Some(list) = list else {
        return Ok(());
    };
    list_permits(list, source, local, chain).inspect_err(|e| refuse(&e.to_string()))
}

#[cfg(test)]
mod after_handshake_tests {
    use super::*;
    use crate::transport::tls::Policy;
    use coqui_net_acl::{Action, Rule};

    fn certificate() -> Vec<u8> {
        vec![7, 1, 2, 3]
    }

    /// A list permitting the certificate from 127.0.0.1 only.
    fn list() -> Acl {
        let mut permit = Rule::new(Action::Permit, "127.0.0.1/32".parse().unwrap());
        permit.fingerprint = Some(PeerId::of(&certificate()).canonical());
        let mut acl = Acl::default();
        acl.push(permit);
        acl.push(Rule::new(Action::Deny, "any".parse().unwrap()));
        acl
    }

    fn at(ip: &str) -> SocketAddr {
        format!("{ip}:50000").parse().unwrap()
    }

    #[test]
    fn a_fingerprint_permitted_from_one_source_is_refused_from_another() {
        let (list, chain) = (list(), [certificate()]);
        let local = at("127.0.0.1");
        let told = core::cell::Cell::new(false);
        after_handshake(Some(&list), at("127.0.0.1"), local, Some(&chain), |_| {
            told.set(true);
        })
        .expect("permitted from the source the rule names");
        assert!(!told.get());
        let e = after_handshake(Some(&list), at("127.0.0.2"), local, Some(&chain), |_| {
            told.set(true);
        })
        .unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::PermissionDenied, "{e}");
        assert!(told.get(), "the client is told why");
    }

    #[test]
    fn no_list_or_no_certificate_decides_nothing_or_refuses() {
        let local = at("127.0.0.1");
        after_handshake(None, at("127.0.0.2"), local, None, |_| {}).expect("no list");
        let list = list();
        after_handshake(Some(&list), at("127.0.0.1"), local, None, |_| {})
            .expect_err("a list cannot decide a client that presented none");
    }

    #[test]
    fn only_a_mutual_listener_that_may_resume_asks_the_list_again() {
        let mut policy = Policy::modern();
        policy.acl = Some(list());
        let pinned = |policy: Policy| {
            TlsConfig::new(Identity::generate().unwrap())
                .pins([Identity::generate().unwrap().fingerprint()])
                .policy(policy)
        };
        assert!(resuming_list(&pinned(policy.clone())).unwrap().is_none());
        policy.allow_resumption = true;
        assert!(resuming_list(&pinned(policy.clone())).unwrap().is_some());
        assert!(
            resuming_list(&pinned(policy.clone()).server_tls())
                .unwrap()
                .is_none(),
            "server TLS asks no client for a certificate"
        );
    }
}

/// A resolver that presents nothing: no handshake through it completes.
#[derive(Debug)]
struct Nobody;

impl ResolvesServerCert for Nobody {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        None
    }
}

/// A connection's certificate resolver: the configuration's own (which
/// refuses a hello from an address the list denies) deciding, then the
/// certificate the listener presents now.
#[derive(Debug)]
struct Gated {
    gate: Arc<dyn ResolvesServerCert>,
    presenting: Arc<Presenting>,
}

impl ResolvesServerCert for Gated {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        self.gate.resolve(hello)?;
        self.presenting.current().ok().map(|p| Arc::clone(&p.key))
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
/// the handshake's until the link raises it, so the first message a peer
/// sends is judged small, as the HELLO is, and one message read ahead until
/// the link admits the peer (`FrameRead::set_read_ahead`), so a peer not yet
/// proved makes this side hold one message, not 64.
fn limits() -> Limits {
    Limits::default()
        .with_max_message(crate::protocol::HANDSHAKE_MAX_FRAME)
        .with_pending(1)
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
#[derive(Clone)]
pub struct WtConnect {
    url: String,
    tls: Arc<rustls::ClientConfig>,
    verifier: Arc<dyn rustls::client::danger::ServerCertVerifier>,
    me: String,
    /// Whether the server's certificate is checked: false at no security.
    proves: bool,
    timeout: Duration,
    /// The local address each connection's socket binds first.
    local: Option<SocketAddr>,
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
            local: None,
        })
    }

    /// Binds each connection's socket to `address` before connecting, so
    /// the server sees that source (a port of 0 lets the system choose).
    /// A clone shares this one's TLS session store, so it can resume
    /// a session from another address.
    #[must_use]
    pub fn local_addr(mut self, address: SocketAddr) -> WtConnect {
        self.local = Some(address);
        self
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
            // The server's TLS refused this client's certificate (not pinned,
            // not accepted, denied by the list's fingerprint rules).
            Failure::Certificate => untrusted_client(&self.me),
            // QUIC's CONNECTION_REFUSED, which a list's refusal is and so is
            // any server declining a connection: the client cannot tell
            // which.
            Failure::Permission => io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "the WebTransport server at {addr} refused the connection (by its access control list, which does not admit this client's address, or the server refused the connection{}): this client is {}",
                    self.local.map_or_else(String::new, |a| format!(
                        ", or this machine refused the local address {a} the connection binds first"
                    )),
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
        if let Some(address) = self.local {
            dialer = dialer.with_local_addr(address);
        }
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
    /// The policy's list, its fingerprints canonical, checked again after
    /// the handshake when a resumed session would skip the client
    /// verifier (`allow_resumption`).
    list: Option<Arc<coqui_net_acl::Acl>>,
    bound: SocketAddr,
    /// What each connection's handshake refused it on, by (source, local).
    reasons: Reasons,
}

type Reasons = Arc<Mutex<Vec<((SocketAddr, SocketAddr), ConnectionConfig)>>>;

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
        // Built once here, so a policy or a rule coqui-peer-tls refuses
        // (a fingerprint that is not SHA-256) fails the bind, not each
        // client.
        let configs = Arc::new(config.server_configs(config.identity(), H3)?);
        configs
            .for_connection(address, address)
            .map_err(|e| unbuilt(&e))?;
        let presenting = Arc::new(Presenting {
            certs: Mutex::new(Certs::new(
                Arc::new(Presented::of(config.identity().clone())?),
                None,
            )),
        });
        // What a connection is given should building its own fail: a
        // configuration that presents nothing, so every handshake fails.
        let mut nobody = ServerConfig::clone(&*server_config(config, H3, None, true)?);
        nobody.cert_resolver = Arc::new(Nobody);
        let fallback = Arc::new(nobody);
        let listener = Listener::bind(address, Arc::clone(&fallback)).map_err(io_error)?;
        let serving = Arc::clone(&presenting);
        let reasons: Reasons = Arc::default();
        let kept = Arc::clone(&reasons);
        let listener = listener.with_tls_per_connection(move |source, local| {
            match configs.for_connection(source, local) {
                Ok(connection) => {
                    {
                        let mut held = lock(&kept);
                        // A connection that never reached its open leaves
                        // its entry: the oldest go first.
                        if held.len() >= 1024 {
                            held.remove(0);
                        }
                        held.push(((source, local), connection.clone()));
                    }
                    let mut tls = ServerConfig::clone(&connection.config());
                    // The address gate of the configuration stays, over the
                    // certificate the listener presents now.
                    tls.cert_resolver = Arc::new(Gated {
                        gate: Arc::clone(&tls.cert_resolver),
                        presenting: Arc::clone(&serving),
                    });
                    Arc::new(tls)
                }
                Err(_) => Arc::clone(&fallback),
            }
        });
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
        let list = resuming_list(&config)?;
        Ok(WtListen {
            listener,
            presenting,
            refused,
            config,
            list,
            bound,
            reasons,
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
                    let key = (source, pending.local_addr());
                    let (list, bound) = (self.list.clone(), self.bound);
                    let reasons = Arc::clone(&self.reasons);
                    let open: Open = Box::new(move || {
                        let handshook = pending.handshake();
                        let kept = {
                            let mut held = lock(&reasons);
                            held.iter()
                                .position(|(k, _)| *k == key)
                                .map(|at| held.remove(at).1)
                        };
                        let channel = match handshook {
                            Ok(channel) => channel,
                            Err(e) => {
                                return Err(match kept.and_then(|k| k.refusal()) {
                                    Some(refusal) => untrusted_by_server(Some(refusal), e),
                                    None => io_error(e),
                                });
                            }
                        };
                        after_handshake(
                            list.as_deref(),
                            source,
                            bound,
                            channel.peer_certificates(),
                            |why| super::web::refuse(&channel, why),
                        )?;
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
