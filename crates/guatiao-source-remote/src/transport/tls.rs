// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! TLS 1.3 over TCP through coqui-peer-tls: mutual TLS, or server TLS
//! ([`TlsConfig::server_tls`]), where the server alone presents a
//! certificate and the client proves itself inside the link (its key or a
//! token). Each side presenting a certificate is known by its fingerprint
//! (`sha256:<hex>`): fixed pins, or the peers a trust store has accepted.
//! A server with a store records a client it does not know,
//! once the client has proved it holds its key, as waiting there until an
//! operator accepts it. A listener admits peers by an access control list
//! (the policy's `acl`): by address before anything is read, by
//! fingerprint once the handshake has proved one. A client the list will
//! not admit is refused as it presents its certificate, so the store never
//! records it as waiting.
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
use coqui_peer_tls::{Digest, PeerError, Pins, Refusal, TlsHandshake, Trust};
use rustls::client::danger::HandshakeSignatureValid;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{
    AlertDescription, ClientConfig, DigitallySignedStruct, DistinguishedName, ServerConfig,
    SignatureScheme,
};

pub use coqui_peer_tls::{Identity, PeerId, Policy, Roots, TrustStore, WaitingEntry, Weakening};

use super::stream::{StreamReader, StreamWriter};
use super::tcp::{Accepted, CONNECT_TIMEOUT};
use super::{Connect, Connection, FrameRead};

/// The application protocol both ends name and require, so a link never
/// completes with a TLS endpoint of something else.
pub(crate) const ALPN: &[u8] = b"guatiao-remote";

/// What a WebSocket over TLS names: the HTTP its upgrade is, as a
/// browser offers it (or nothing). The HELLO's magic names the protocol
/// there.
#[cfg(feature = "ws")]
pub(crate) const WEBSOCKET_ALPN: &[u8] = b"http/1.1";

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

/// Why this side refused the certificate of `whose` ("the client",
/// "the server at ..."), in words naming what to change.
pub(crate) fn refused(refusal: Refusal, whose: &str) -> String {
    match refusal {
        Refusal::NotPinned => {
            format!("{whose} is not pinned: its certificate's fingerprint matches none of the pins")
        }
        Refusal::NotAccepted => format!(
            "{whose} is not accepted: its certificate is not one the trust store has accepted"
        ),
        Refusal::ChainWhenPinned => format!(
            "{whose} presented a certificate chain: the policy holds a pinned peer to one certificate (`allow_chain_when_pinned`)"
        ),
        Refusal::NotTrusted => format!(
            "{whose} is not trusted: its certificate matches no pin, and no authority the roots (`roots`) hold issued it for a server"
        ),
        other => format!("{whose} is refused: {other}"),
    }
}

/// The host of `host:port` or `host` (an IPv6 address in brackets).
pub(crate) fn host_of(address: &str) -> &str {
    if let Some(rest) = address.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest);
    }
    match address.rsplit_once(':') {
        Some((host, _)) if !host.contains(':') => host,
        _ => address,
    }
}

/// The name a client dials `host` by, which a certificate the roots trust
/// must hold: a DNS name or an address. A pinned certificate is taken
/// whatever it names, so without roots a host no certificate could name
/// is dialled as coqui-peer-tls's placeholder name instead.
pub(crate) fn server_name(host: &str, by_roots: bool) -> io::Result<ServerName<'static>> {
    match ServerName::try_from(host.to_string()) {
        Ok(name) => Ok(name),
        Err(_) if !by_roots => Ok(ServerName::try_from(coqui_peer_tls::SERVER_NAME)
            .map_err(invalid)?
            .to_owned()),
        Err(_) => Err(invalid(format!(
            "`{host}` is not a name a certificate can hold, and the server is verified by the roots (`roots`): dial it by its DNS name or address"
        ))),
    }
}

/// Rewrites every fingerprint an access control list names in its
/// canonical text, the form a verified peer is decided by. A peer is known
/// by its certificate's SHA-256 fingerprint, so a rule naming another
/// digest could never match: refused rather than left to fail open.
pub(crate) fn canonical_acl(mut acl: Acl) -> io::Result<Acl> {
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

/// A client's link refused by the server's certificate check, heard as the
/// alert `certificate_unknown` before anything else: `me` is the client's
/// fingerprint.
pub(crate) fn untrusted_client(me: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "the server does not trust this client, {me}: it refused its certificate (certificate_unknown), which is not pinned there or not yet accepted by its trust store, or its access control list refuses it; a server with a trust store keeps an unknown client waiting until an operator accepts it"
        ),
    )
}

/// A client's link the server ended before answering anything.
pub(crate) fn ended_before_answering(e: &io::Error, me: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "the server ended the link before answering ({e}): most likely it does not trust this client, {me}, or its access control list refuses it; a server with a trust store keeps an unknown client waiting until an operator accepts it"
        ),
    )
}

/// The client's refusal of the server at `addr`, dialled as `name`, in
/// words naming what to change.
pub(crate) fn untrusted_server(
    refusal: Refusal,
    addr: impl core::fmt::Display,
    name: &str,
) -> io::Error {
    let why = match refusal {
        Refusal::NameMismatch => format!(
            "its certificate was issued by an authority the roots (`roots`) hold, but not for `{name}`, the name dialled: connect by a name the certificate holds"
        ),
        other => refused(other, "it"),
    };
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("the server at {addr} is not one this client trusts: {why}"),
    )
}

/// The server's refusal of a client, in words naming what to change; `why`
/// when the handshake failed for another reason.
pub(crate) fn untrusted_by_server(
    refusal: Option<Refusal>,
    why: impl core::fmt::Display,
) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        match refusal {
            Some(Refusal::NotAccepted) => format!(
                "{}; once it has proved its key it waits there, within the store's caps, until an operator accepts it",
                refused(Refusal::NotAccepted, "the client")
            ),
            Some(r) => refused(r, "the client"),
            None => format!("the client's TLS handshake failed: {why}"),
        },
    )
}

/// Which certificate an [`Admitting`] verifier refused, and by which rule
/// of the list (an index, `None` for its default).
pub(crate) type Denied = Arc<OnceLock<(PeerId, Option<usize>)>>;

/// A listener's client verifier for one connection: a certificate the
/// access control list denies from that connection's addresses is refused
/// as it is presented, before `inner` sees it, so a trust store never
/// records as waiting a client the list will never admit. The client hears
/// what any client the server does not trust hears.
#[derive(Debug)]
pub(crate) struct Admitting {
    pub(crate) inner: Arc<dyn ClientCertVerifier>,
    pub(crate) acl: Arc<Acl>,
    pub(crate) source: SocketAddr,
    pub(crate) local: SocketAddr,
    pub(crate) denied: Denied,
}

impl ClientCertVerifier for Admitting {
    fn offer_client_auth(&self) -> bool {
        self.inner.offer_client_auth()
    }

    fn client_auth_mandatory(&self) -> bool {
        self.inner.client_auth_mandatory()
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        self.inner.root_hint_subjects()
    }

    fn verify_client_cert(
        &self,
        certificate: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        let peer = PeerId::of(certificate.as_ref());
        let decided = self
            .acl
            .decide_peer(self.source, self.local, &peer.canonical());
        if decided.outcome != Outcome::Permit {
            let _ = self.denied.set((peer, decided.rule_index));
            return Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::Other(rustls::OtherError(Arc::new(Refusal::NotAccepted))),
            ));
        }
        self.inner
            .verify_client_cert(certificate, intermediates, now)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signed: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner
            .verify_tls12_signature(message, certificate, signed)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signed: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner
            .verify_tls13_signature(message, certificate, signed)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }

    fn requires_raw_public_keys(&self) -> bool {
        self.inner.requires_raw_public_keys()
    }
}

/// What a listener gives one connection a client verifier of its own with:
/// the verifier its configuration was built with, and the certificate and
/// key it presents.
pub(crate) struct Verifying {
    pub(crate) verifier: Arc<dyn ClientCertVerifier>,
    pub(crate) certificate: CertificateDer<'static>,
    pub(crate) key: PrivateKeyDer<'static>,
}

impl core::fmt::Debug for Verifying {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Verifying").finish_non_exhaustive()
    }
}

impl Verifying {
    /// What `config`, at mutual TLS, verifies its clients and presents
    /// itself with.
    pub(crate) fn of(config: &TlsConfig) -> io::Result<Verifying> {
        let verifier = config
            .client_trust()?
            .client_cert_verifier_with(config.tls_policy())
            .map_err(|e| invalid(words(&e)))?;
        let identity = config.identity();
        let pem = identity.to_pem().map_err(|e| invalid(words(&e)))?;
        Ok(Verifying {
            verifier,
            certificate: CertificateDer::from(identity.certificate_der().to_vec()),
            key: PrivateKeyDer::from_pem_slice(pem.as_bytes()).map_err(invalid)?,
        })
    }

    /// `base`, presenting `certificate` and `key`, its clients verified by
    /// `verifier`: rustls keeps a configuration's verifier to itself, so
    /// one of a connection's own is a configuration rebuilt around it.
    pub(crate) fn config(
        base: &ServerConfig,
        verifier: Arc<dyn ClientCertVerifier>,
        certificate: &CertificateDer<'static>,
        key: &PrivateKeyDer<'static>,
    ) -> io::Result<Arc<ServerConfig>> {
        let mut tls = ServerConfig::builder_with_provider(Arc::clone(base.crypto_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(invalid)?
            .with_client_cert_verifier(verifier)
            .with_single_cert(vec![certificate.clone()], key.clone_key())
            .map_err(invalid)?;
        tls.alpn_protocols.clone_from(&base.alpn_protocols);
        tls.session_storage = Arc::clone(&base.session_storage);
        tls.send_tls13_tickets = base.send_tls13_tickets;
        tls.ticketer = Arc::clone(&base.ticketer);
        Ok(Arc::new(tls))
    }
}

/// Whether `acl` has a rule naming a fingerprint: only then can a source
/// it admits to the handshake be one it would still deny.
fn defers(acl: &Acl) -> bool {
    acl.rules.iter().any(|r| r.fingerprint.is_some())
}

/// The access control list's refusal of `peer` from `source` once the
/// handshake named it, by rule `rule` (an index) or the list's default.
pub(crate) fn denied_by_acl(peer: &PeerId, source: SocketAddr, rule: Option<usize>) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        match rule {
            Some(i) => format!(
                "the access control list denies {peer} at {source} (rule {})",
                (i + 1) * 10
            ),
            None => format!("the access control list denies {peer} at {source} (its default)"),
        },
    )
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

/// The identity in the PEM file at `path`, its certificate's chain kept
/// (a CA-issued certificate a browser is to trust comes with one); made,
/// self-issued, when the file is absent.
pub fn identity_at(path: &Path) -> Result<Identity, PeerError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Identity::from_pem_chain(&text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Identity::open_or_create(path),
        Err(e) => Err(PeerError::Io(e)),
    }
}

/// What one side of the secure level is: its identity, whom it trusts and
/// the TLS policy both are held to.
///
/// Trust is the fixed pins when any are given, else the peers the trust
/// store has accepted; a server with a store records a client it does not
/// know as waiting there. A client may also trust a server by the web PKI
/// ([`roots`](TlsConfig::roots)). TLS session resumption is off at both
/// ends unless the policy's `allow_resumption` keeps it, so a peer removed
/// from the store is refused at its next link.
#[derive(Clone)]
pub struct TlsConfig {
    identity: Identity,
    pins: Vec<PeerId>,
    store: Option<TrustStore>,
    /// The authorities a client verifies a server by, beside any pins.
    roots: Option<Roots>,
    policy: Policy,
    /// Takes any certificate whose key the peer proves: QUIC at no
    /// security, where TLS encrypts and the link's key proof decides.
    checks_nobody: bool,
    /// Whether both sides present a certificate; false at server TLS.
    mutual: bool,
}

impl core::fmt::Debug for TlsConfig {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TlsConfig")
            .field("fingerprint", &self.identity.fingerprint().canonical())
            .field("pins", &self.pins.len())
            .field("store", &self.store.is_some())
            .field("roots", &self.roots)
            .field("policy", &self.policy.name)
            .field("mutual", &self.mutual)
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
            roots: None,
            policy: Policy::strict(),
            checks_nobody: false,
            mutual: true,
        }
    }

    /// A client's configuration at server TLS for a host with no identity
    /// of its own: it presents no certificate (it holds a generated one,
    /// never shown), and trusts the server its pins or store name.
    pub fn without_identity() -> io::Result<TlsConfig> {
        let identity = Identity::generate().map_err(|e| invalid(words(&e)))?;
        Ok(TlsConfig::new(identity).server_tls())
    }

    /// Server TLS rather than mutual TLS: as a server it presents its
    /// certificate and asks clients for none (they prove themselves inside
    /// the link, by the key or a token); as a client it presents none.
    /// Listed by [`weakened`](TlsConfig::weakened) as `client_auth`.
    pub fn server_tls(mut self) -> TlsConfig {
        self.mutual = false;
        self
    }

    /// Whether both sides present a certificate: false at server TLS.
    pub fn is_mutual(&self) -> bool {
        self.mutual
    }

    /// A generated identity that takes any peer's certificate: the TLS
    /// QUIC runs at no security, where only the link's key proof and the
    /// access control list decide.
    #[cfg_attr(not(feature = "quic"), allow(dead_code))]
    pub(crate) fn checking_nobody() -> io::Result<TlsConfig> {
        let identity = Identity::generate().map_err(|e| invalid(words(&e)))?;
        Ok(TlsConfig {
            checks_nobody: true,
            ..TlsConfig::new(identity)
        })
    }

    /// Whether it trusts certificates at all: false at no security.
    #[cfg_attr(not(feature = "quic"), allow(dead_code))]
    pub(crate) fn proves_peers(&self) -> bool {
        !self.checks_nobody
    }

    /// The identity in `dir`'s [`IDENTITY_FILE`], made there when absent
    /// (readable by its owner alone on Unix), and the trust store in `dir`
    /// (`peers/accepted`, `peers/waiting`).
    pub fn open(dir: &Path) -> io::Result<TlsConfig> {
        let identity = identity_at(&dir.join(IDENTITY_FILE)).map_err(|e| {
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

    /// As a client, also trusting a server whose certificate one of
    /// `roots`' authorities issued for the name dialled, within its dates:
    /// the web PKI, as a browser trusts a site. A certificate the pins or
    /// the store name is still taken whatever it names. A CA bundle is
    /// [`Roots::bundle_file`] or [`Roots::bundle_pem`]; the operating
    /// system's authorities and verifier are `Roots::platform()` (feature
    /// `platform-roots`). A listener trusts clients by pins or a store
    /// alone.
    pub fn roots(mut self, roots: Roots) -> TlsConfig {
        self.roots = Some(roots);
        self
    }

    /// Whether a server is checked against roots: its name then matters.
    pub(crate) fn by_roots(&self) -> bool {
        self.roots.is_some() && !self.checks_nobody
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

    /// The same configuration presenting `identity` instead: a WebTransport
    /// listener rotating short-lived certificates.
    #[cfg(feature = "webtransport")]
    pub(crate) fn presenting(&self, identity: Identity) -> TlsConfig {
        TlsConfig {
            identity,
            ..self.clone()
        }
    }

    /// The same configuration under its policy without the access control
    /// list: quinn runs rustls on its own threads, where coqui-peer-tls
    /// cannot learn a connection's addresses, so a QUIC listener applies
    /// the list itself.
    #[cfg(feature = "quic")]
    pub(crate) fn listing_nobody(&self) -> TlsConfig {
        let mut policy = self.policy.clone();
        policy.acl = None;
        TlsConfig {
            policy,
            ..self.clone()
        }
    }

    /// This side's fingerprint: what a peer pins or accepts.
    pub fn fingerprint(&self) -> PeerId {
        self.identity.fingerprint()
    }

    /// The trust store, if one is given.
    pub fn trust_store(&self) -> Option<&TrustStore> {
        self.store.as_ref()
    }

    /// The policy both ends are held to.
    #[cfg_attr(not(feature = "quic"), allow(dead_code))]
    pub(crate) fn tls_policy(&self) -> &Policy {
        &self.policy
    }

    /// Every setting of the policy looser than strict, its access control
    /// list's default included; empty under the strict policy.
    pub fn weakened(&self) -> Vec<Weakening> {
        if self.mutual {
            return self.policy.weakened();
        }
        coqui_peer_tls::server_config_without_client_auth_with(
            &self.identity,
            &[ALPN],
            &self.policy,
        )
        .map_or_else(|_| self.policy.weakened(), |built| built.weakened)
    }

    /// This side in a refusal's words: its fingerprint, or that it
    /// presents none.
    pub(crate) fn me(&self) -> String {
        if self.mutual {
            self.identity.fingerprint().canonical()
        } else {
            "which presents no certificate (server TLS)".to_string()
        }
    }

    /// The pins, else the store's accepted peers, if either is given.
    /// The list a listener of this configuration checks a client's
    /// certificate against before its trust store sees it: `acl`, when the
    /// store records clients it does not know as waiting and the list has
    /// a rule naming a fingerprint (a source it admits to the handshake may
    /// still be denied). `None` when the address decides, or nothing is
    /// recorded.
    pub(crate) fn admits_first(&self, acl: Option<&Arc<Acl>>) -> Option<Arc<Acl>> {
        let waits = self.mutual && !self.checks_nobody && self.pins.is_empty();
        acl.filter(|acl| waits && self.store.is_some() && defers(acl))
            .cloned()
    }

    fn pinned(&self) -> Option<Pins> {
        if !self.pins.is_empty() {
            return Some(Pins::fixed(self.pins.iter().copied()));
        }
        self.store.clone().map(Pins::store)
    }

    /// Whom a client trusts as its server: the pins or the store, the
    /// roots, or either.
    pub(crate) fn server_trust(&self) -> io::Result<Trust> {
        if self.checks_nobody {
            return Ok(Trust::AnyWithKeyProof);
        }
        // Built here once on their own, so a bundle that cannot be read or
        // holds no usable certificate is named as the roots' failure.
        if let Some(roots) = &self.roots {
            Trust::Roots(roots.clone())
                .server_cert_verifier_with(&self.policy)
                .map_err(|e| {
                    invalid(format!("the roots (`roots`) cannot be used: {}", words(&e)))
                })?;
        }
        match (self.pinned(), self.roots.clone()) {
            (Some(pins), Some(roots)) => Ok(Trust::PinnedOrRoots { pins, roots }),
            (Some(pins), None) => Ok(Trust::Pinned(pins)),
            (None, Some(roots)) => Ok(Trust::Roots(roots)),
            (None, None) => Err(invalid(
                "the secure level names whom it trusts: fingerprints to pin, a trust store, or the roots (`roots`) a server's certificate is issued by",
            )),
        }
    }

    /// Whom a listener trusts as its clients: the pins or the store.
    pub(crate) fn client_trust(&self) -> io::Result<Trust> {
        if self.checks_nobody {
            return Ok(Trust::AnyWithKeyProof);
        }
        self.pinned().map(Trust::Pinned).ok_or_else(|| {
            invalid(if self.roots.is_some() {
                "a listener names the clients it trusts: fingerprints to pin, or a trust store (`roots` verify a server this side dials)"
            } else {
                "the secure level names whom it trusts: fingerprints to pin, or a trust store"
            })
        })
    }

    pub(crate) fn client(&self) -> io::Result<Arc<ClientConfig>> {
        self.client_naming(ALPN)
    }

    /// A client configuration offering `alpn` alone.
    pub(crate) fn client_naming(&self, alpn: &[u8]) -> io::Result<Arc<ClientConfig>> {
        if self.policy.acl.is_some() {
            return Err(invalid(
                "the policy's `acl` admits peers to a listener; a client has none",
            ));
        }
        let built = if self.mutual {
            coqui_peer_tls::client_config_with(
                &self.identity,
                &self.server_trust()?,
                &[alpn],
                &self.policy,
            )
        } else {
            coqui_peer_tls::client_config_without_identity_with(
                &self.server_trust()?,
                &[alpn],
                &self.policy,
            )
        }
        .map_err(|e| invalid(words(&e)))?;
        Ok(Arc::new(built.config))
    }

    pub(crate) fn server(&self) -> io::Result<Arc<ServerConfig>> {
        self.server_naming(ALPN)
    }

    /// A server configuration naming `alpn` alone.
    pub(crate) fn server_naming(&self, alpn: &[u8]) -> io::Result<Arc<ServerConfig>> {
        let built = if self.mutual {
            coqui_peer_tls::server_config_with(
                &self.identity,
                &self.client_trust()?,
                &[alpn],
                &self.policy,
            )
        } else {
            coqui_peer_tls::server_config_without_client_auth_with(
                &self.identity,
                &[alpn],
                &self.policy,
            )
        }
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

    /// The session, and the peer it proved: none for a server's client at
    /// server TLS.
    fn open(&self, connection: rustls::Connection, peer: Option<&PeerId>) {
        *lock(&self.state) = Some(connection);
        if let Some(peer) = peer {
            let _ = self.peer.set(peer.canonical());
        }
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
        let alert = e
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<rustls::Error>())
            .and_then(|tls| match tls {
                rustls::Error::AlertReceived(alert) => Some(*alert),
                _ => None,
            });
        match &self.client {
            // What the server's verifier sends when it refuses a certificate.
            Some(me) if early && alert == Some(AlertDescription::CertificateUnknown) => {
                untrusted_client(me)
            }
            Some(me) if ended && early => ended_before_answering(&e, me),
            _ => e,
        }
    }

    /// Ends the socket both ways, a read blocked on it included, with
    /// nothing more said.
    fn shut(&self) {
        self.closed.store(true, Ordering::Release);
        super::tcp::shut(&self.socket);
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
    /// Whether the client must have agreed on [`ALPN`]; a browser's
    /// WebSocket offers HTTP's, or nothing.
    alpn: bool,
    /// Whether the client presents a certificate: false at server TLS.
    mutual: bool,
    /// Set when the list deferred this source to a fingerprint rule: the
    /// connection then verifies its client through an [`Admitting`].
    verifying: Option<Arc<Verifying>>,
}

impl Pending {
    fn run(self, core: &Core) -> io::Result<()> {
        let denied = Denied::default();
        let config = match (&self.verifying, &self.acl) {
            (Some(v), Some(acl)) => Verifying::config(
                &self.config,
                Arc::new(Admitting {
                    inner: Arc::clone(&v.verifier),
                    acl: Arc::clone(acl),
                    source: self.source,
                    local: self.local,
                    denied: Arc::clone(&denied),
                }),
                &v.certificate,
                &v.key,
            )?,
            _ => Arc::clone(&self.config),
        };
        let handshake = TlsHandshake::server(config, Some(self.source))
            .map_err(|e| invalid(words(&e)))?
            .with_local(self.local);
        // The bound the link set on a read bounds the whole handshake.
        let bound = core.socket.read_timeout()?;
        let mut io = Bounded {
            socket: &core.socket,
            until: bound.map(|b| Instant::now() + b),
        };
        let completed = coqui_peer::complete_stream(handshake, &mut io);
        core.socket.set_read_timeout(bound)?;
        core.socket.set_write_timeout(None)?;
        let completed = completed.map_err(|e| match (e, denied.get()) {
            (DriveError::Handshake(_), Some((peer, rule))) => {
                denied_by_acl(peer, self.source, *rule)
            }
            (DriveError::Io(e), _) => e,
            (DriveError::Handshake(e), None) => untrusted_by_server(e.refusal(), words(&e)),
            (other, _) => io::Error::new(io::ErrorKind::InvalidData, other.to_string()),
        })?;
        // A mutual handshake names the client; at server TLS there is none.
        let (session, peer) = completed
            .finish_unauthenticated()
            .map_err(|e| untrusted_by_server(e.refusal(), words(&e)))?;
        let peer = peer.map(|p| p.fingerprint);
        if self.mutual && peer.is_none() {
            core.shut();
            return Err(untrusted_by_server(
                None,
                "the client presented no certificate",
            ));
        }
        if self.alpn
            && let Err(e) = check_alpn(&session)
        {
            core.shut();
            return Err(e);
        }
        if let (Some(acl), Some(peer)) = (&self.acl, &peer) {
            let decided = acl.decide_peer(self.source, self.local, &peer.canonical());
            if decided.outcome != Outcome::Permit {
                core.close();
                return Err(denied_by_acl(peer, self.source, decided.rule_index));
            }
        }
        core.open(session, peer.as_ref());
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
        self.core.peer.get().cloned()
    }

    fn encrypted(&self) -> bool {
        true
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

/// A TLS session as one byte stream several threads share, what a
/// WebSocket runs over: the link's reader reads (and a server's first read
/// runs the handshake), any thread writes whole buffers.
#[cfg(feature = "ws")]
pub(crate) struct TlsDuplex {
    core: Arc<Core>,
    /// Only the reading thread takes it.
    read: Mutex<TlsRead>,
}

#[cfg(feature = "ws")]
impl TlsDuplex {
    fn new(core: Core, pending: Option<Pending>) -> TlsDuplex {
        let core = Arc::new(core);
        TlsDuplex {
            read: Mutex::new(TlsRead {
                core: Arc::clone(&core),
                pending,
                raw: vec![0; RAW_READ],
                fed: 0,
                filled: 0,
                ended: false,
            }),
            core,
        }
    }

    pub(crate) fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        lock(&self.read).read(buf)
    }

    pub(crate) fn write_all(&self, buf: &[u8]) -> io::Result<()> {
        self.core.send(&[buf]).map_err(|e| self.core.explain(e))
    }

    pub(crate) fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.core.socket.set_read_timeout(timeout)
    }

    pub(crate) fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.core.socket.set_write_timeout(timeout)
    }

    /// Tells the peer the session ends, then ends the socket.
    pub(crate) fn close(&self) {
        self.core.close();
    }

    /// Ends the socket both ways with nothing more said.
    pub(crate) fn shut(&self) {
        self.core.shut();
    }

    pub(crate) fn peer_id(&self) -> Option<String> {
        self.core.peer.get().cloned()
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
    /// Whether the server must agree on [`ALPN`].
    alpn: bool,
    /// Whether roots verify the server, so the name dialled matters.
    by_roots: bool,
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
    /// To `address` (`host:port`), as `config` says, within
    /// [`CONNECT_TIMEOUT`] for the connect and the handshake each. The
    /// host is the name the server's certificate is checked for under
    /// [`roots`](TlsConfig::roots).
    pub fn new(address: &str, config: &TlsConfig) -> io::Result<TlsConnect> {
        server_name(host_of(address), config.by_roots())?;
        Ok(TlsConnect {
            address: address.to_string(),
            config: config.client()?,
            me: config.me(),
            timeout: CONNECT_TIMEOUT,
            alpn: true,
            by_roots: config.by_roots(),
        })
    }

    /// The TLS under a WebSocket's upgrade: as [`new`](TlsConnect::new),
    /// offering HTTP's application protocol, none required.
    #[cfg(feature = "ws")]
    pub(crate) fn for_websocket(config: &TlsConfig) -> io::Result<TlsConnect> {
        Ok(TlsConnect {
            address: String::new(),
            config: config.client_naming(WEBSOCKET_ALPN)?,
            me: config.me(),
            timeout: CONNECT_TIMEOUT,
            alpn: false,
            by_roots: config.by_roots(),
        })
    }

    /// Within `timeout` per address tried, and again for the handshake.
    pub fn timeout(mut self, timeout: Duration) -> TlsConnect {
        self.timeout = timeout;
        self
    }

    /// A client handshake's failure, dialled as `name`, in words a person
    /// can act on.
    fn refusal(&self, e: &PeerError, addr: SocketAddr, name: &ServerName<'_>) -> io::Error {
        if let Some(refusal) = e.refusal() {
            return untrusted_server(refusal, addr, &name.to_str());
        }
        let (kind, why) = match tls_error(e) {
            // Its dates, its encoding or its signature.
            Some(rustls::Error::InvalidCertificate(why)) => (
                io::ErrorKind::PermissionDenied,
                format!(
                    "the server at {addr} presented a certificate this client refuses ({why:?})"
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
        let core = self.established(socket, addr, host_of(&self.address))?;
        connection(core, None, format!("tls {addr}"))
    }

    /// The TLS over `socket`, connected to `addr` dialled as `host`, under
    /// a WebSocket.
    #[cfg(feature = "ws")]
    pub(crate) fn duplex(
        &self,
        socket: TcpStream,
        addr: SocketAddr,
        host: &str,
    ) -> io::Result<TlsDuplex> {
        let core = self.established(socket, addr, host)?;
        Ok(TlsDuplex::new(core, None))
    }

    /// Runs the client's handshake over `socket`, dialled as `host`, within
    /// the timeout.
    fn established(&self, socket: TcpStream, addr: SocketAddr, host: &str) -> io::Result<Core> {
        socket.set_nodelay(true)?;
        let name = server_name(host, self.by_roots)?;
        let handshake = TlsHandshake::client_named(Arc::clone(&self.config), name.clone())
            .map_err(|e| invalid(words(&e)))?;
        let mut io = Bounded {
            socket: &socket,
            until: Some(Instant::now() + self.timeout),
        };
        let established = coqui_peer::drive_stream(handshake, &mut io).map_err(|e| match e {
            DriveError::Io(e) => e,
            DriveError::Handshake(e) => self.refusal(&e, addr, &name),
            other => io::Error::new(io::ErrorKind::InvalidData, other.to_string()),
        })?;
        socket.set_read_timeout(None)?;
        socket.set_write_timeout(None)?;
        if self.alpn {
            check_alpn(&established.session)?;
        }
        let core = Core::new(socket, Some(self.me.clone()));
        core.open(established.session, Some(&established.peer));
        Ok(core)
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

/// A TLS listener, at mutual TLS or server TLS as its configuration says.
/// It admits peers by the policy's access control list, every source when
/// there is none: by address as it accepts, reading nothing from a peer it
/// denies; at mutual TLS by fingerprint after the handshake, which runs on
/// the serving thread's first read, so a client that connects and stalls
/// holds only that thread.
pub struct TlsListen {
    listener: TcpListener,
    config: Arc<ServerConfig>,
    acl: Option<Arc<Acl>>,
    /// Whether clients must agree on [`ALPN`].
    alpn: bool,
    /// Whether clients present a certificate: false at server TLS.
    mutual: bool,
    /// At mutual TLS under a list with fingerprint rules: what a
    /// connection the list deferred verifies its client through.
    verifying: Option<Arc<Verifying>>,
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
        let verifying = match config.admits_first(acl.as_ref()) {
            Some(_) => Some(Arc::new(Verifying::of(config)?)),
            None => None,
        };
        Ok(TlsListen {
            listener,
            config: config.server()?,
            acl,
            alpn: true,
            mutual: config.is_mutual(),
            verifying,
        })
    }

    /// The TLS under a WebSocket's upgrade: as
    /// [`from_listener`](TlsListen::from_listener), naming HTTP's
    /// application protocol, none required of the client.
    #[cfg(feature = "ws")]
    pub(crate) fn for_websocket(
        listener: TcpListener,
        config: &TlsConfig,
    ) -> io::Result<TlsListen> {
        Ok(TlsListen {
            config: config.server_naming(WEBSOCKET_ALPN)?,
            alpn: false,
            ..TlsListen::from_listener(listener, config)?
        })
    }

    /// Where it listens.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// The next connection, before its handshake, or a peer the list
    /// denies by address, closed unread.
    pub fn accept(&self) -> io::Result<Accepted> {
        Ok(match self.accept_pending()? {
            Ok((core, pending, source)) => {
                Accepted::Connection(connection(core, Some(pending), format!("tls {source}"))?)
            }
            Err(source) => Accepted::Refused(source),
        })
    }

    /// The next connection with its handshake to come, or the address of
    /// a peer the list denies, closed unread.
    fn accept_pending(&self) -> io::Result<Result<(Core, Pending, SocketAddr), SocketAddr>> {
        let (socket, source) = self.listener.accept()?;
        let local = socket.local_addr()?;
        let outcome = self
            .acl
            .as_ref()
            .map(|acl| acl.decide(source, local).outcome);
        if outcome == Some(Outcome::Deny) {
            let _ = socket.shutdown(Shutdown::Both);
            return Ok(Err(source));
        }
        socket.set_nodelay(true)?;
        let pending = Pending {
            config: Arc::clone(&self.config),
            source,
            local,
            acl: self.acl.clone(),
            alpn: self.alpn,
            mutual: self.mutual,
            verifying: self
                .verifying
                .clone()
                .filter(|_| outcome == Some(Outcome::Deferred)),
        };
        Ok(Ok((Core::new(socket, None), pending, source)))
    }

    /// The next connection as the TLS under a WebSocket, its handshake run
    /// by the first read.
    #[cfg(feature = "ws")]
    pub(crate) fn accept_duplex(&self) -> io::Result<Result<(TlsDuplex, SocketAddr), SocketAddr>> {
        Ok(self
            .accept_pending()?
            .map(|(core, pending, source)| (TlsDuplex::new(core, Some(pending)), source)))
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
    fn an_identity_file_keeps_the_chain_after_its_certificate() {
        let dir = std::env::temp_dir().join(format!("guatiao-identity-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let leaf = identity();
        let issuer = identity();
        let issuer_pem = issuer.to_pem().unwrap();
        let issuer_cert = &issuer_pem[..issuer_pem.find("-----BEGIN PRIVATE KEY").unwrap()];
        let pem = leaf.to_pem().unwrap();
        let key_at = pem.find("-----BEGIN PRIVATE KEY").unwrap();
        let chained = format!("{}{issuer_cert}{}", &pem[..key_at], &pem[key_at..]);
        let file = dir.join(IDENTITY_FILE);
        std::fs::write(&file, chained).unwrap();
        let read = identity_at(&file).unwrap();
        assert_eq!(
            read.fingerprint(),
            leaf.fingerprint(),
            "the certificate's, not the chain's"
        );
        assert_eq!(read.intermediates_der().len(), 1);
        assert_eq!(
            Identity::read(&file).unwrap().unwrap().fingerprint(),
            leaf.fingerprint(),
            "coqui-peer-tls's reader keeps the chain too"
        );
        assert_eq!(
            TlsConfig::open(&dir).unwrap().fingerprint(),
            leaf.fingerprint()
        );
        // Absent, one is made.
        let made = dir.join("made.pem");
        let first = identity_at(&made).unwrap();
        assert_eq!(
            identity_at(&made).unwrap().fingerprint(),
            first.fingerprint()
        );
        let _ = std::fs::remove_dir_all(&dir);
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
