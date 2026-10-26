// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! QUIC over UDP: the mutual TLS 1.3 of [`tls`](super::tls) inside QUIC
//! (quinn), with the same [`TlsConfig`]: identity, pins or trust store,
//! policy and access control list. At no security
//! ([`QuicConnect::trusted`], [`QuicListen::bind_trusted`]) QUIC still
//! runs TLS, as it always does, but no certificate is checked: each side
//! presents one it generated, and the link's key proof and the listener's
//! access control list, by address, decide. A link is
//! one bidirectional stream carrying the frames as TCP does; [`Datagrams`]
//! beside it carry what a caller accepts losing. A listener proves a
//! client's address with a Retry before admitting it, and a connection
//! keeps the address it was admitted from.
//!
//! quinn runs on a small tokio runtime the transport owns. A link's
//! threads wait on it by polling under its handle and parking, so a thread
//! that is itself inside another runtime can still call.
//!
//! The verifiers of coqui-peer-tls record a waiting client with the address
//! it came from, which they read from the thread they run on; quinn runs
//! each handshake on its runtime's threads. So a listener accepts each
//! connection with a TLS configuration of its own whose client verifier
//! scopes that connection's address around every call.

#![forbid(unsafe_code)]

use alloc::sync::Arc;
use alloc::task::Wake;
use core::cell::Cell;
use core::future::Future;
use core::net::SocketAddr;
use core::pin::pin;
use core::sync::atomic::{AtomicBool, Ordering};
use core::task::{Context, Poll, Waker};
use core::time::Duration;
use std::io::{self, IoSlice, Read, Write};
use std::net::{ToSocketAddrs, UdpSocket};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread::{self, Thread};
use std::time::Instant;

use coqui_net_acl::{Acl, Outcome};
use coqui_peer_tls::{PeerId, Refusal};
use quinn::crypto::rustls::{HandshakeData, QuicClientConfig, QuicServerConfig};
use quinn::{ConnectionError, Incoming, RecvStream, SendStream, TransportErrorCode, VarInt};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{
    AlertDescription, ClientConfig, DigitallySignedStruct, DistinguishedName, ServerConfig,
    SignatureScheme,
};
use tokio::runtime::{Handle, Runtime};

use super::stream::{StreamReader, StreamWriter};
use super::tcp::{Accepted, CONNECT_TIMEOUT};
use super::tls::{
    ALPN, TlsConfig, canonical_acl, denied_by_acl, ended_before_answering, host_of, server_name,
    untrusted_by_server, untrusted_client, untrusted_server,
};
use super::{Connect, Connection, Datagrams, FrameRead};

/// The code a connection closes with when it ends without its stream
/// finished: the peer reads an error, not an end. quinn closes a
/// connection dropped unclosed with it too.
pub(super) const ABRUPT: u32 = 0;
/// The code a connection closes with when its link ended as it should.
pub(super) const CLEAN: u32 = 1;
/// The code a server closes a connection with when its access control
/// list or the application protocol refuses the peer.
pub(super) const REFUSED: u32 = 2;

/// How long ending a link waits for the peer to acknowledge what was
/// sent: past it the rest is given up. A round trip or two on any path.
pub(super) const CLOSE_WAIT: Duration = Duration::from_secs(2);

/// How long a connection dropped unclosed waits for its close to leave
/// before the runtime under it may stop.
pub(super) const DROP_WAIT: Duration = Duration::from_millis(100);

/// The runtime's worker threads: the endpoint's driver and the
/// connections' drivers.
const WORKERS: usize = 2;

fn invalid(why: impl core::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, why.to_string())
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The runtime quinn runs on, shut down without waiting when the last
/// holder lets it go (which may be a thread inside another runtime).
pub(super) struct Rt(Option<Runtime>);

impl Rt {
    pub(super) fn new() -> io::Result<Arc<Rt>> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(WORKERS)
            .thread_name("guatiao-remote quic")
            .enable_all()
            .build()?;
        Ok(Arc::new(Rt(Some(rt))))
    }

    pub(super) fn handle(&self) -> &Handle {
        self.0.as_ref().expect("a runtime until dropped").handle()
    }
}

impl Drop for Rt {
    fn drop(&mut self) {
        if let Some(rt) = self.0.take() {
            rt.shutdown_background();
        }
    }
}

/// Wakes the thread waiting on a future.
struct Unpark(Thread);

impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

/// Runs `work` to its end on this thread, inside `rt`'s context (its
/// timers, its sockets), parking between polls. Unlike `block_on`, it may
/// be called from a thread that is inside another runtime.
pub(super) fn wait<F: Future>(rt: &Handle, work: F) -> F::Output {
    let _inside = rt.enter();
    let waker = Waker::from(Arc::new(Unpark(thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut work = pin!(work);
    loop {
        if let Poll::Ready(out) = work.as_mut().poll(&mut cx) {
            return out;
        }
        thread::park();
    }
}

/// As [`wait`], giving up after `bound` (none: never).
pub(super) fn wait_at_most<F: Future>(
    rt: &Handle,
    bound: Option<Duration>,
    work: F,
) -> io::Result<F::Output> {
    match bound {
        None => Ok(wait(rt, work)),
        Some(bound) => {
            wait(rt, async move { tokio::time::timeout(bound, work).await }).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the QUIC peer did not answer in time",
                )
            })
        }
    }
}

/// Whether a link was ended, and who to wake when it is: what a server's
/// handshake waits on beside its own work.
#[derive(Default)]
struct Ending {
    ended: AtomicBool,
    waiting: Mutex<Option<Waker>>,
}

impl Ending {
    fn end(&self) {
        self.ended.store(true, Ordering::Release);
        if let Some(waker) = lock(&self.waiting).take() {
            waker.wake();
        }
    }

    fn ended(&self) -> bool {
        self.ended.load(Ordering::Acquire)
    }

    /// `work`'s output, or `None` once the link is ended.
    async fn or_ended<F: Future>(&self, work: F) -> Option<F::Output> {
        let mut work = pin!(work);
        core::future::poll_fn(|cx| {
            if self.ended() {
                return Poll::Ready(None);
            }
            *lock(&self.waiting) = Some(cx.waker().clone());
            // Ended between the check and the waker being left: seen here.
            if self.ended() {
                return Poll::Ready(None);
            }
            work.as_mut().poll(cx).map(Some)
        })
        .await
    }
}

fn ended_in_handshake() -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionAborted,
        "the link was ended during its QUIC handshake",
    )
}

/// What is left of a deadline, or a timeout once it passed.
pub(super) fn left(until: Option<Instant>) -> io::Result<Option<Duration>> {
    match until {
        None => Ok(None),
        Some(until) => match until.checked_duration_since(Instant::now()) {
            Some(left) if !left.is_zero() => Ok(Some(left)),
            _ => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the QUIC handshake did not finish in time",
            )),
        },
    }
}

fn transport() -> Arc<quinn::TransportConfig> {
    Arc::new(transport_config())
}

/// The transport settings every connection runs with.
pub(super) fn transport_config() -> quinn::TransportConfig {
    let mut t = quinn::TransportConfig::default();
    // Room for a large frame in flight on a fast path.
    t.stream_receive_window(VarInt::from_u32(16 << 20));
    t.receive_window(VarInt::from_u32(64 << 20));
    t.send_window(64 << 20);
    // An idle link stays up; a lost peer ends it after the idle timeout.
    t.keep_alive_interval(Some(Duration::from_secs(10)));
    t.datagram_receive_buffer_size(Some(1 << 20));
    t
}

/// The first refusal a connection's verifier returned, for the words.
pub(super) type Kept = Arc<OnceLock<Refusal>>;

fn keep(kept: &Kept, e: &rustls::Error) {
    if let Some(r) = Refusal::from_rustls(e) {
        let _ = kept.set(r);
    }
}

/// The listener's client verifier for one connection: every call runs
/// with that connection's address as the source a waiting entry records,
/// and the first refusal is kept for the words.
#[derive(Debug)]
pub(super) struct Sourced {
    pub(super) inner: Arc<dyn ClientCertVerifier>,
    pub(super) source: SocketAddr,
    pub(super) refusal: Kept,
}

impl Sourced {
    fn run<T>(&self, call: impl FnOnce() -> Result<T, rustls::Error>) -> Result<T, rustls::Error> {
        let out = coqui_peer_tls::with_source(Some(self.source), call);
        if let Err(e) = &out {
            keep(&self.refusal, e);
        }
        out
    }
}

impl ClientCertVerifier for Sourced {
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
        self.run(|| {
            self.inner
                .verify_client_cert(certificate, intermediates, now)
        })
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signed: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.run(|| {
            self.inner
                .verify_tls12_signature(message, certificate, signed)
        })
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signed: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.run(|| {
            self.inner
                .verify_tls13_signature(message, certificate, signed)
        })
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }

    fn requires_raw_public_keys(&self) -> bool {
        self.inner.requires_raw_public_keys()
    }
}

/// A client's server verifier, keeping the refusal of the connection being
/// made for the words: one per connection, or one for every connection of
/// a [`QuicConnect`] whose policy keeps resumption, since rustls resumes a
/// session only under the verifier that made it.
#[derive(Debug)]
pub(super) struct Watched {
    pub(super) inner: Arc<dyn ServerCertVerifier>,
    pub(super) refusal: Mutex<Option<Refusal>>,
}

impl Watched {
    fn run<T>(&self, out: Result<T, rustls::Error>) -> Result<T, rustls::Error> {
        if let Err(e) = &out
            && let Some(r) = Refusal::from_rustls(e)
        {
            lock(&self.refusal).get_or_insert(r);
        }
        out
    }
}

impl ServerCertVerifier for Watched {
    fn verify_server_cert(
        &self,
        certificate: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        self.run(
            self.inner
                .verify_server_cert(certificate, intermediates, name, ocsp, now),
        )
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signed: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.run(
            self.inner
                .verify_tls12_signature(message, certificate, signed),
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signed: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.run(
            self.inner
                .verify_tls13_signature(message, certificate, signed),
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }

    fn requires_raw_public_keys(&self) -> bool {
        self.inner.requires_raw_public_keys()
    }

    fn root_hint_subjects(&self) -> Option<&[DistinguishedName]> {
        self.inner.root_hint_subjects()
    }
}

fn no_alpn() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "the peer does not speak guatiao's remote protocol: the QUIC handshake agreed no `guatiao-remote` application protocol",
    )
}

/// Refuses a connection that did not agree on this protocol.
fn check_alpn(conn: &quinn::Connection) -> io::Result<()> {
    let agreed = conn
        .handshake_data()
        .and_then(|d| d.downcast::<HandshakeData>().ok())
        .and_then(|d| d.protocol);
    if agreed.as_deref() == Some(ALPN) {
        Ok(())
    } else {
        Err(no_alpn())
    }
}

/// Whether `e` is a handshake that agreed no application protocol (QUIC
/// requires one, so TLS ends it with `no_application_protocol`).
fn no_protocol(e: &ConnectionError) -> bool {
    let none = TransportErrorCode::crypto(u8::from(AlertDescription::NoApplicationProtocol));
    match e {
        ConnectionError::ConnectionClosed(close) => close.error_code == none,
        ConnectionError::TransportError(e) => e.code == none,
        _ => false,
    }
}

/// The fingerprint of the certificate the peer proved it holds.
pub(super) fn fingerprint(conn: &quinn::Connection) -> io::Result<PeerId> {
    conn.peer_identity()
        .and_then(|any| any.downcast::<Vec<CertificateDer<'static>>>().ok())
        .and_then(|chain| chain.first().map(|c| PeerId::of(c.as_ref())))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the QUIC handshake proved no peer certificate",
            )
        })
}

/// Whether `e` is the peer refusing this side's certificate: TLS's
/// `certificate_unknown`, carried in QUIC as a CRYPTO_ERROR.
fn certificate_refused(e: &ConnectionError) -> bool {
    alert(e) == Some(u8::from(AlertDescription::CertificateUnknown))
}

/// The TLS alert a QUIC connection ended with (a CRYPTO_ERROR, `0x100` and
/// the alert), sent by the peer or raised here.
pub(super) fn alert(e: &ConnectionError) -> Option<u8> {
    let code = match e {
        ConnectionError::ConnectionClosed(close) => close.error_code,
        ConnectionError::TransportError(e) => e.code,
        _ => return None,
    };
    (0..=u8::MAX).find(|a| TransportErrorCode::crypto(*a) == code)
}

/// A connection lost, as an error a person can act on. `client` is this
/// side's fingerprint when it is a client that has heard nothing yet, so
/// the server most likely refused it.
fn gone(e: ConnectionError, client: Option<&str>) -> io::Error {
    let refused = certificate_refused(&e);
    let kind = match &e {
        ConnectionError::TimedOut => io::ErrorKind::TimedOut,
        ConnectionError::ApplicationClosed(_) => io::ErrorKind::UnexpectedEof,
        _ => io::ErrorKind::ConnectionReset,
    };
    let e = io::Error::new(kind, format!("the QUIC connection was lost: {e}"));
    match client {
        Some(me) if refused => untrusted_client(me),
        Some(me) if kind != io::ErrorKind::TimedOut => ended_before_answering(&e, me),
        _ => e,
    }
}

/// What both halves of a link share: the connection once its handshake is
/// through, and the stream the writer writes.
struct Shared {
    conn: OnceLock<quinn::Connection>,
    send: Mutex<Option<SendStream>>,
    /// The peer's verified fingerprint, canonical.
    peer: OnceLock<String>,
    /// Whether the peer's certificate was checked: false at no security,
    /// where it proves nothing and [`peer`](Shared::peer) stays empty.
    proves: bool,
    /// A client: this side's own fingerprint, to name in a refusal.
    client: Option<String>,
    /// Whether anything has come from the peer since the handshake.
    heard: AtomicBool,
    /// Ended by the closer or the writer: a server's handshake not yet
    /// through stops at once.
    ending: Ending,
    /// The endpoint the connection runs on (a client's is its own).
    endpoint: Option<quinn::Endpoint>,
    /// Last, so it stops only once the connection has been let go.
    rt: Arc<Rt>,
}

impl Shared {
    fn handle(&self) -> &Handle {
        self.rt.handle()
    }

    fn open(&self, conn: quinn::Connection, send: SendStream, peer: Option<&PeerId>) {
        *lock(&self.send) = Some(send);
        if let Some(peer) = peer.filter(|_| self.proves) {
            let _ = self.peer.set(peer.canonical());
        }
        let _ = self.conn.set(conn);
    }

    /// The connection lost, in what a reader answers: a clean end when the
    /// peer or this side ended the link as it should, else an error a
    /// person can act on.
    fn lost(&self, e: ConnectionError) -> io::Result<usize> {
        let early = !self.heard.load(Ordering::Acquire);
        match &e {
            ConnectionError::LocallyClosed => return Ok(0),
            ConnectionError::ApplicationClosed(c) if c.error_code == VarInt::from_u32(CLEAN) => {
                return Ok(0);
            }
            _ => {}
        }
        Err(gone(e, self.client.as_deref().filter(|_| early)))
    }

    /// Ends the connection both ways without the writer, which a blocked
    /// write may hold: what was written still reaches the peer, as TCP
    /// delivers what was sent before its end, unless a write is under way,
    /// when the connection closes at once as an error.
    fn shut(&self) {
        self.ending.end();
        match self.send.try_lock() {
            Ok(mut send) => self.finish(send.as_mut()),
            Err(_) => {
                if let Some(conn) = self.conn.get() {
                    conn.close(VarInt::from_u32(ABRUPT), b"ended");
                }
            }
        }
    }

    /// Finishes the stream, waits for the peer to acknowledge it, then
    /// closes the connection as a clean end.
    fn close(&self) {
        self.ending.end();
        self.finish(lock(&self.send).as_mut());
    }

    fn finish(&self, send: Option<&mut SendStream>) {
        if let Some(send) = send
            && send.finish().is_ok()
        {
            let stopped = send.stopped();
            let _ = wait_at_most(self.handle(), Some(CLOSE_WAIT), stopped);
        }
        if let Some(conn) = self.conn.get() {
            conn.close(VarInt::from_u32(CLEAN), b"");
        }
    }
}

impl Drop for Shared {
    /// A connection let go unclosed ends as an error; when it holds the
    /// last of its runtime, its close gets a moment to leave before the
    /// runtime stops, or the peer would wait out its idle timeout.
    fn drop(&mut self) {
        let Some(conn) = self.conn.get() else {
            return;
        };
        conn.close(VarInt::from_u32(ABRUPT), b"dropped");
        if Arc::strong_count(&self.rt) == 1
            && let Some(endpoint) = &self.endpoint
        {
            let idle = endpoint.wait_idle();
            let _ = wait_at_most(self.rt.handle(), Some(DROP_WAIT), idle);
        }
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
    /// This connection's own server configuration: the listener's, with
    /// a client verifier scoped to `source`.
    fn config(&self, refusal: &Kept) -> io::Result<Arc<quinn::ServerConfig>> {
        let base = &self.listener.tls;
        // At server TLS no client certificate is asked, so no verifier
        // records a waiting client: the listener's own configuration does.
        let tls = match &self.listener.verifier {
            Some(inner) => {
                let verifier = Arc::new(Sourced {
                    inner: Arc::clone(inner),
                    source: self.source,
                    refusal: Arc::clone(refusal),
                });
                let mut tls =
                    ServerConfig::builder_with_provider(Arc::clone(base.crypto_provider()))
                        .with_protocol_versions(&[&rustls::version::TLS13])
                        .map_err(invalid)?
                        .with_client_cert_verifier(verifier)
                        .with_single_cert(
                            vec![self.listener.certificate.clone()],
                            self.listener.key.clone_key(),
                        )
                        .map_err(invalid)?;
                tls.alpn_protocols.clone_from(&base.alpn_protocols);
                tls.session_storage = Arc::clone(&base.session_storage);
                tls.send_tls13_tickets = base.send_tls13_tickets;
                tls.ticketer = Arc::clone(&base.ticketer);
                Arc::new(tls)
            }
            None => Arc::clone(base),
        };
        let crypto = QuicServerConfig::try_from(tls).map_err(invalid)?;
        let mut config = quinn::ServerConfig::with_crypto(Arc::new(crypto));
        config.transport_config(transport());
        // The access control list decided on the address the handshake came
        // from; a connection keeps it, as a TCP one does.
        config.migration(false);
        Ok(Arc::new(config))
    }

    fn run(self, shared: &Shared, bound: Option<Duration>) -> io::Result<RecvStream> {
        if shared.ending.ended() {
            self.incoming.refuse();
            return Err(ended_in_handshake());
        }
        let until = bound.map(|b| Instant::now() + b);
        let refusal = Kept::default();
        let config = self.config(&refusal)?;
        let source = self.source;
        // quinn spawns the connection's driver on the runtime it is called
        // inside.
        let connecting = {
            let _inside = shared.handle().enter();
            self.incoming.accept_with(config)
        }
        .map_err(|e| io::Error::new(io::ErrorKind::ConnectionAborted, e.to_string()))?;
        let handshake = shared.ending.or_ended(connecting);
        let conn = wait_at_most(shared.handle(), left(until)?, handshake)?
            .ok_or_else(ended_in_handshake)?
            .map_err(|e| match (refusal.get().copied(), alert(&e)) {
                (None, None) => io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    format!("the client's QUIC handshake did not finish: {e}"),
                ),
                (refused, _) => untrusted_by_server(refused, &e),
            })?;
        let refuse = |e: io::Error| {
            conn.close(VarInt::from_u32(REFUSED), b"refused");
            e
        };
        check_alpn(&conn).map_err(refuse)?;
        // At server TLS the client presents no certificate.
        let peer = match self.listener.verifier {
            Some(_) => Some(fingerprint(&conn).map_err(refuse)?),
            None => None,
        };
        // At no security the certificate proves nothing: the list decided
        // by address as the connection was accepted.
        if let (Some(acl), Some(peer)) = (
            self.listener.acl.as_ref().filter(|_| self.listener.proves),
            &peer,
        ) {
            let decided = acl.decide_peer(source, self.local, &peer.canonical());
            if decided.outcome != Outcome::Permit {
                return Err(refuse(denied_by_acl(peer, source, decided.rule_index)));
            }
        }
        let accepting = shared.ending.or_ended(conn.accept_bi());
        let ended = |conn: &quinn::Connection| {
            conn.close(VarInt::from_u32(ABRUPT), b"ended");
            ended_in_handshake()
        };
        let (send, recv) = wait_at_most(shared.handle(), left(until)?, accepting)?
            .ok_or_else(|| ended(&conn))?
            .map_err(|e| io::Error::new(io::ErrorKind::ConnectionAborted, e.to_string()))?;
        let opened = conn.clone();
        shared.open(conn, send, peer.as_ref());
        // Ended between the stream arriving and the connection being kept,
        // when the closer found nothing to close.
        if shared.ending.ended() {
            return Err(ended(&opened));
        }
        Ok(recv)
    }
}

/// The reading half.
struct QuicRead {
    shared: Arc<Shared>,
    pending: Option<Pending>,
    recv: Option<RecvStream>,
    /// The bound the link set on a read; the first bounds the handshake.
    timeout: Cell<Option<Duration>>,
}

impl Read for QuicRead {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(pending) = self.pending.take() {
            self.recv = Some(pending.run(&self.shared, self.timeout.get())?);
        }
        let recv = self.recv.as_mut().ok_or(io::ErrorKind::NotConnected)?;
        let reading = recv.read(buf);
        match wait_at_most(self.shared.handle(), self.timeout.get(), reading)? {
            Ok(Some(n)) => {
                self.shared.heard.store(true, Ordering::Release);
                Ok(n)
            }
            Ok(None) => Ok(0),
            Err(quinn::ReadError::ConnectionLost(e)) => self.shared.lost(e),
            Err(quinn::ReadError::Reset(code)) => Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                format!("the peer reset the link's QUIC stream (code {code})"),
            )),
            Err(e) => Err(io::Error::other(e.to_string())),
        }
    }
}

/// The frames of the reading half, the peer the handshake proved and the
/// datagrams beside them.
struct QuicFrames {
    frames: StreamReader<QuicRead>,
    shared: Arc<Shared>,
}

impl FrameRead for QuicFrames {
    fn read_frame(&mut self, buf: &mut Vec<u8>, max: usize) -> io::Result<bool> {
        self.frames.read_frame(buf, max)
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
        Some(Arc::new(QuicDatagrams {
            shared: Arc::clone(&self.shared),
        }))
    }
}

/// The writing half.
struct QuicWrite {
    shared: Arc<Shared>,
    timeout: Option<Duration>,
}

impl QuicWrite {
    fn send(&self, parts: &[&[u8]]) -> io::Result<()> {
        let mut send = lock(&self.shared.send);
        let stream = send.as_mut().ok_or(io::ErrorKind::NotConnected)?;
        let writing = async move {
            for part in parts {
                stream.write_all(part).await?;
            }
            Ok::<_, quinn::WriteError>(())
        };
        wait_at_most(self.shared.handle(), self.timeout, writing)?.map_err(|e| match e {
            quinn::WriteError::ConnectionLost(lost) => match self.shared.lost(lost) {
                Ok(_) => io::Error::new(io::ErrorKind::BrokenPipe, "the link ended"),
                Err(e) => e,
            },
            other => io::Error::new(io::ErrorKind::BrokenPipe, other.to_string()),
        })
    }
}

impl Write for QuicWrite {
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

/// Datagrams on a link's connection.
struct QuicDatagrams {
    shared: Arc<Shared>,
}

impl QuicDatagrams {
    fn conn(&self) -> io::Result<&quinn::Connection> {
        self.shared.conn.get().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotConnected,
                "the QUIC handshake has not run yet",
            )
        })
    }
}

impl Datagrams for QuicDatagrams {
    fn send(&self, payload: &[u8]) -> io::Result<()> {
        use quinn::SendDatagramError as E;
        self.conn()?
            .send_datagram(payload.to_vec().into())
            .map_err(|e| match e {
                E::TooLarge => invalid(format!(
                    "a datagram of {} bytes is larger than the path takes ({:?})",
                    payload.len(),
                    self.max_size()
                )),
                E::UnsupportedByPeer | E::Disabled => {
                    io::Error::new(io::ErrorKind::Unsupported, e.to_string())
                }
                E::ConnectionLost(lost) => io::Error::new(
                    io::ErrorKind::NotConnected,
                    format!("the link ended: {lost}"),
                ),
            })
    }

    fn recv(&self, timeout: Option<Duration>) -> io::Result<Option<Vec<u8>>> {
        let reading = self.conn()?.read_datagram();
        match wait_at_most(self.shared.handle(), timeout, reading) {
            Err(e) if e.kind() == io::ErrorKind::TimedOut => Ok(None),
            Err(e) => Err(e),
            Ok(Ok(bytes)) => Ok(Some(bytes.to_vec())),
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
    recv: Option<RecvStream>,
    peer: String,
) -> Connection {
    let shared = Arc::new(shared);
    let closing = Arc::clone(&shared);
    let shutting = Arc::clone(&shared);
    let reader = StreamReader::with_timeout(
        QuicRead {
            shared: Arc::clone(&shared),
            pending,
            recv,
            timeout: Cell::new(None),
        },
        |r: &QuicRead, t| {
            r.timeout.set(t);
            Ok(())
        },
    );
    Connection {
        reader: Box::new(QuicFrames {
            frames: reader,
            shared: Arc::clone(&shared),
        }),
        writer: Box::new(
            StreamWriter::with_shutdown(
                QuicWrite {
                    shared,
                    timeout: None,
                },
                move || shutting.close(),
            )
            .bounded_by(|w: &mut QuicWrite, t| {
                w.timeout = t;
                Ok(())
            }),
        ),
        peer,
        closer: Some(Box::new(move || closing.shut())),
    }
}

/// How long a connect waits on one address before it tries the next as
/// well (RFC 8305's connection attempt delay).
const NEXT_ADDRESS: Duration = Duration::from_millis(250);

/// Connects to a server at `host:port` over QUIC: the handshake runs in
/// [`connect`](Connect::connect), so a server this side does not trust
/// fails it there. A name with several addresses has them tried at once,
/// each started a moment after the one before (QUIC hears nothing back
/// from an address nobody listens at, so one at a time would wait out the
/// timeout on each).
pub struct QuicConnect {
    address: String,
    timeout: Duration,
    dial: Arc<Dial>,
}

/// What every connection of a [`QuicConnect`] is made with, shared by the
/// threads trying its addresses.
struct Dial {
    /// The client configuration, its server verifier set per connection.
    client: Arc<ClientConfig>,
    verifier: Arc<dyn ServerCertVerifier>,
    /// When the policy keeps resumption: the one configuration and verifier
    /// every connection uses, and the turn connects take so that a refusal
    /// kept is that connection's.
    resuming: Option<Resuming>,
    me: String,
    /// The name the server is dialled by: its certificate's, under roots.
    name: String,
    /// Whether the server's certificate is checked: false at no security.
    proves: bool,
    rt: Arc<Rt>,
}

struct Resuming {
    crypto: Arc<QuicClientConfig>,
    watched: Arc<Watched>,
    turn: Mutex<()>,
}

/// One connection's configuration and the verifier keeping its refusal,
/// and the turn it holds while connecting when the policy keeps
/// resumption.
struct Attempt<'a> {
    crypto: Arc<QuicClientConfig>,
    watched: Arc<Watched>,
    _turn: Option<MutexGuard<'a, ()>>,
}

impl core::fmt::Debug for QuicConnect {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("QuicConnect")
            .field("address", &self.address)
            .field("as", &self.dial.me)
            .finish()
    }
}

fn quic_client(tls: ClientConfig) -> io::Result<Arc<QuicClientConfig>> {
    QuicClientConfig::try_from(Arc::new(tls))
        .map(Arc::new)
        .map_err(|e| invalid(format!("the TLS policy does not fit QUIC: {e}")))
}

impl QuicConnect {
    /// To `address`, as `config` says (the same configuration as
    /// [`TlsConnect`](super::tls::TlsConnect)), within [`CONNECT_TIMEOUT`]
    /// for each address tried.
    pub fn new(address: &str, config: &TlsConfig) -> io::Result<QuicConnect> {
        let name = server_name(host_of(address), config.by_roots())?
            .to_str()
            .into_owned();
        let verifier: Arc<dyn ServerCertVerifier> = config
            .server_trust()?
            .server_cert_verifier_with(config.tls_policy())
            .map_err(|e| invalid(super::tls::words(&e)))?;
        let client = config.client()?;
        // Only a policy keeping resumption needs one verifier for every
        // connection; with none, connects need not take turns.
        let resuming = if config.tls_policy().allow_resumption {
            let watched = Arc::new(Watched {
                inner: Arc::clone(&verifier),
                refusal: Mutex::new(None),
            });
            let mut tls: ClientConfig = (*client).clone();
            tls.dangerous()
                .set_certificate_verifier(Arc::clone(&watched) as Arc<dyn ServerCertVerifier>);
            Some(Resuming {
                crypto: quic_client(tls)?,
                watched,
                turn: Mutex::new(()),
            })
        } else {
            // Checked here, as the shared configuration is: a policy QUIC
            // cannot run fails `new`, not a connect.
            quic_client((*client).clone())?;
            None
        };
        Ok(QuicConnect {
            address: address.to_string(),
            timeout: CONNECT_TIMEOUT,
            dial: Arc::new(Dial {
                client,
                verifier,
                resuming,
                me: config.me(),
                name,
                proves: config.proves_peers(),
                rt: Rt::new()?,
            }),
        })
    }

    /// To `address` at no security: TLS whose certificates nobody checks
    /// (this side presents one it generated, and takes the server's
    /// whatever it is), the link's key proof deciding. A server listening
    /// with [`QuicListen::bind_trusted`] answers it.
    pub fn trusted(address: &str) -> io::Result<QuicConnect> {
        QuicConnect::new(address, &TlsConfig::checking_nobody()?)
    }

    /// Within `timeout` per address tried, the handshake included.
    pub fn timeout(mut self, timeout: Duration) -> QuicConnect {
        self.timeout = timeout;
        self
    }

    /// Tries every address at once, each started [`NEXT_ADDRESS`] after the
    /// one before or as soon as it failed; the first link wins, and a
    /// refusal is the answer. The others are let go as they finish.
    fn race(&self, addrs: Vec<SocketAddr>) -> io::Result<Connection> {
        /// Stops every attempt still in its handshake when the race is over.
        struct Over(Arc<Ending>);
        impl Drop for Over {
            fn drop(&mut self) {
                self.0.end();
            }
        }
        let over = Over(Arc::new(Ending::default()));
        let (tx, rx) = std::sync::mpsc::channel();
        let start = |addr: SocketAddr| {
            let (dial, tx, timeout) = (Arc::clone(&self.dial), tx.clone(), self.timeout);
            let over = Arc::clone(&over.0);
            let started = thread::Builder::new()
                .name("guatiao-remote quic connect".to_string())
                .spawn(move || {
                    let _ = tx.send(dial.connect_to(addr, timeout, Some(&over)));
                });
            started.map(drop)
        };
        let mut left = addrs.into_iter();
        let mut running = 0usize;
        let mut last = io::Error::new(
            io::ErrorKind::TimedOut,
            format!(
                "no address of {} answered within {:?}",
                self.address, self.timeout
            ),
        );
        loop {
            if running == 0 || left.len() > 0 {
                // Start the next now: none is running, or it is time.
                if let Some(addr) = left.next() {
                    match start(addr) {
                        Ok(()) => running += 1,
                        Err(e) => last = e,
                    }
                    if running == 0 {
                        continue;
                    }
                } else if running == 0 {
                    return Err(last);
                }
            }
            let got = if left.len() > 0 {
                match rx.recv_timeout(NEXT_ADDRESS) {
                    Ok(got) => got,
                    Err(_) => continue,
                }
            } else {
                // Each attempt answers within its handshake and its stream's
                // opening, each bounded by the timeout.
                let bound = self.timeout.saturating_mul(2).saturating_add(NEXT_ADDRESS);
                rx.recv_timeout(bound)
                    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, last.to_string()))?
            };
            running -= 1;
            match got {
                Ok(conn) => return Ok(conn),
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied => return Err(e),
                Err(e) => last = e,
            }
        }
    }
}

impl Dial {
    /// What a connection is made with.
    fn crypto(&self) -> io::Result<Attempt<'_>> {
        if let Some(r) = &self.resuming {
            let turn = lock(&r.turn);
            *lock(&r.watched.refusal) = None;
            return Ok(Attempt {
                crypto: Arc::clone(&r.crypto),
                watched: Arc::clone(&r.watched),
                _turn: Some(turn),
            });
        }
        let watched = Arc::new(Watched {
            inner: Arc::clone(&self.verifier),
            refusal: Mutex::new(None),
        });
        let mut tls: ClientConfig = (*self.client).clone();
        tls.dangerous()
            .set_certificate_verifier(Arc::clone(&watched) as Arc<dyn ServerCertVerifier>);
        Ok(Attempt {
            crypto: quic_client(tls)?,
            watched,
            _turn: None,
        })
    }

    /// A connection to `addr`, its handshake given up once `over` ends.
    fn connect_to(
        &self,
        addr: SocketAddr,
        timeout: Duration,
        over: Option<&Ending>,
    ) -> io::Result<Connection> {
        let attempt = self.crypto()?;
        let watched = &attempt.watched;
        let mut config = quinn::ClientConfig::new(Arc::clone(&attempt.crypto) as Arc<_>);
        config.transport_config(transport());
        let handle = self.rt.handle();
        let bind: SocketAddr = if addr.is_ipv4() {
            (core::net::Ipv4Addr::UNSPECIFIED, 0).into()
        } else {
            (core::net::Ipv6Addr::UNSPECIFIED, 0).into()
        };
        // quinn spawns its drivers on the runtime it is called inside.
        let (endpoint, connecting) = {
            let _inside = handle.enter();
            let endpoint = quinn::Endpoint::client(bind)?;
            let connecting = endpoint
                .connect_with(config, addr, &self.name)
                .map_err(invalid)?;
            (endpoint, connecting)
        };
        let handshake = async move {
            match over {
                Some(over) => over.or_ended(connecting).await,
                None => Some(connecting.await),
            }
        };
        let conn = wait_at_most(handle, Some(timeout), handshake)?
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "another address of the name linked first",
                )
            })?
            .map_err(|e| {
            if let Some(r) = *lock(&watched.refusal) {
                return untrusted_server(r, addr, &self.name);
            }
            if certificate_refused(&e) {
                return untrusted_client(&self.me);
            }
            if no_protocol(&e) {
                return no_alpn();
            }
            match (&e, alert(&e)) {
                (ConnectionError::TransportError(t), Some(_)) => {
                    return io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        format!(
                            "the server at {addr} presented a certificate this client refuses: {}",
                            t.reason
                        ),
                    );
                }
                (_, Some(a)) => {
                    return io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        format!(
                            "the server at {addr} refused the TLS handshake ({:?}): it may not trust this client, {}",
                            AlertDescription::from(a),
                            self.me
                        ),
                    );
                }
                _ => {}
            }
            let kind = match &e {
                ConnectionError::TimedOut => io::ErrorKind::TimedOut,
                _ => io::ErrorKind::ConnectionRefused,
            };
            io::Error::new(
                kind,
                format!("the QUIC handshake with the server at {addr} failed: {e}"),
            )
        })?;
        check_alpn(&conn)?;
        let peer = fingerprint(&conn)?;
        let opening = conn.open_bi();
        let (send, recv) =
            wait_at_most(handle, Some(timeout), opening)?.map_err(|e| gone(e, Some(&self.me)))?;
        let shared = Shared {
            conn: OnceLock::new(),
            send: Mutex::new(None),
            peer: OnceLock::new(),
            proves: self.proves,
            client: Some(self.me.clone()),
            heard: AtomicBool::new(false),
            ending: Ending::default(),
            endpoint: Some(endpoint),
            rt: Arc::clone(&self.rt),
        };
        shared.open(conn, send, Some(&peer));
        Ok(connection(shared, None, Some(recv), format!("quic {addr}")))
    }
}

impl Connect for QuicConnect {
    fn connect(&self) -> io::Result<Connection> {
        let addrs: Vec<SocketAddr> = self.address.to_socket_addrs()?.collect();
        // Connects taking turns try one address at a time.
        if addrs.len() > 1 && self.dial.resuming.is_none() {
            return self.race(addrs);
        }
        let mut last = io::Error::new(io::ErrorKind::NotFound, "no address to connect to");
        for addr in addrs {
            match self.dial.connect_to(addr, self.timeout, None) {
                Ok(conn) => return Ok(conn),
                // A refusal is the answer; another address will not change it.
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied => return Err(e),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    fn describe(&self) -> String {
        format!("quic {}", self.address)
    }
}

/// What every connection a listener accepts is built from.
struct Listener {
    /// The listener's TLS configuration, as coqui-peer-tls builds it
    /// under the policy: what each connection's own copies.
    tls: Arc<ServerConfig>,
    /// The client verifier; none at server TLS, which asks no certificate.
    verifier: Option<Arc<dyn ClientCertVerifier>>,
    certificate: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
    acl: Option<Arc<Acl>>,
    /// Whether clients' certificates are checked: false at no security.
    proves: bool,
}

/// A listener at the secure level over QUIC. It admits peers by the
/// policy's access control list, every source when there is none: by
/// address as it accepts, refusing a peer it denies before any handshake;
/// by fingerprint after the handshake, which runs on the serving thread's
/// first read, so a client that stalls holds only that thread.
pub struct QuicListen {
    endpoint: quinn::Endpoint,
    listener: Arc<Listener>,
    rt: Arc<Rt>,
}

impl core::fmt::Debug for QuicListen {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("QuicListen")
            .field("addr", &self.endpoint.local_addr().ok())
            .field("acl", &self.listener.acl.is_some())
            .finish()
    }
}

impl QuicListen {
    /// Binds the UDP `address` (a wildcard included) as `config` says.
    pub fn bind(address: &str, config: &TlsConfig) -> io::Result<QuicListen> {
        let addr = address
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| invalid("no address to bind"))?;
        QuicListen::from_socket(UdpSocket::bind(addr)?, config)
    }

    /// Binds the UDP `address` at no security, admitting the peers `acl`
    /// permits by address: QUIC's TLS with a certificate it generated,
    /// taking any client's, the link's key proof deciding.
    pub fn bind_trusted(address: &str, acl: Acl) -> io::Result<QuicListen> {
        let addr = address
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| invalid("no address to bind"))?;
        QuicListen::from_socket_trusted(UdpSocket::bind(addr)?, acl)
    }

    /// A UDP socket the caller bound itself, at no security.
    pub fn from_socket_trusted(socket: UdpSocket, acl: Acl) -> io::Result<QuicListen> {
        let mut config = TlsConfig::checking_nobody()?;
        let mut policy = config.tls_policy().clone();
        policy.acl = Some(acl);
        config = config.policy(policy);
        QuicListen::from_socket(socket, &config)
    }

    /// A UDP socket the caller bound itself, as `config` says.
    pub fn from_socket(socket: UdpSocket, config: &TlsConfig) -> io::Result<QuicListen> {
        let acl = match config.tls_policy().acl.clone() {
            Some(acl) => Some(Arc::new(canonical_acl(acl)?)),
            None => None,
        };
        let tls = config.server()?;
        let verifier: Option<Arc<dyn ClientCertVerifier>> = if config.is_mutual() {
            Some(
                config
                    .client_trust()?
                    .client_cert_verifier_with(config.tls_policy())
                    .map_err(|e| invalid(super::tls::words(&e)))?,
            )
        } else {
            None
        };
        let identity = config.identity();
        let pem = identity
            .to_pem()
            .map_err(|e| invalid(super::tls::words(&e)))?;
        let key = PrivateKeyDer::from_pem_slice(pem.as_bytes()).map_err(invalid)?;
        let crypto = QuicServerConfig::try_from(Arc::clone(&tls))
            .map_err(|e| invalid(format!("the TLS policy does not fit QUIC: {e}")))?;
        let mut server = quinn::ServerConfig::with_crypto(Arc::new(crypto));
        server.transport_config(transport());
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
        Ok(QuicListen {
            endpoint,
            listener: Arc::new(Listener {
                tls,
                verifier,
                certificate: CertificateDer::from(identity.certificate_der().to_vec()),
                key,
                acl,
                proves: config.proves_peers(),
            }),
            rt,
        })
    }

    /// Where it listens.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.endpoint.local_addr()
    }

    /// The next connection, before its handshake, or a peer the list
    /// denies by address, refused before any handshake.
    pub fn accept(&self) -> io::Result<Accepted> {
        let incoming = loop {
            let accepting = self.endpoint.accept();
            let incoming = wait(self.rt.handle(), accepting).ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotConnected, "the QUIC endpoint is closed")
            })?;
            // A first packet's address may be forged: a Retry proves the
            // client receives at it, as TCP's handshake does, before the
            // list decides or a connection takes a thread.
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
                // A rule on the local address cannot be decided without it;
                // the others need only its port.
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
        let local = local.unwrap_or(bound);
        let shared = Shared {
            conn: OnceLock::new(),
            send: Mutex::new(None),
            peer: OnceLock::new(),
            proves: self.listener.proves,
            client: None,
            heard: AtomicBool::new(false),
            ending: Ending::default(),
            endpoint: Some(self.endpoint.clone()),
            rt: Arc::clone(&self.rt),
        };
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
            format!("quic {source}"),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coqui_peer_tls::Identity;

    fn frame(body: &[u8]) -> Vec<u8> {
        let mut f = (body.len() as u32).to_le_bytes().to_vec();
        f.extend_from_slice(body);
        f
    }

    /// A listener and a client that pin each other, linked: the client's
    /// first frame has run the server's handshake on its first read.
    fn pair() -> (Connection, Connection) {
        let (server_id, client_id) = (Identity::generate().unwrap(), Identity::generate().unwrap());
        let server = TlsConfig::new(server_id.clone()).pins([client_id.fingerprint()]);
        let client = TlsConfig::new(client_id).pins([server_id.fingerprint()]);
        let listen = QuicListen::bind("127.0.0.1:0", &server).unwrap();
        let connect = QuicConnect::new(&listen.local_addr().unwrap().to_string(), &client).unwrap();
        let dialing = thread::spawn(move || {
            let mut c = connect.connect().unwrap();
            c.writer.write_frame(&frame(b"start")).unwrap();
            c
        });
        let Accepted::Connection(mut accepted) = listen.accept().unwrap() else {
            panic!("no access control list: every source proceeds");
        };
        accepted
            .reader
            .set_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut buf = Vec::new();
        assert!(accepted.reader.read_frame(&mut buf, 1 << 10).unwrap());
        assert_eq!(buf, b"start");
        accepted.reader.set_timeout(None).unwrap();
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
        let id = Identity::generate().unwrap();
        let config = TlsConfig::new(id.clone()).pins([id.fingerprint()]);
        let listen = QuicListen::bind("127.0.0.1:0", &config).unwrap();
        let connect = QuicConnect::new(&listen.local_addr().unwrap().to_string(), &config).unwrap();
        let dialing = thread::spawn(move || connect.connect());
        let Accepted::Connection(mut server) = listen.accept().unwrap() else {
            panic!("every source proceeds");
        };
        let e = server.writer.write_frame(&frame(b"early")).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::NotConnected);
        assert_eq!(server.reader.peer_id(), None);
        let datagrams = server.reader.datagrams().unwrap();
        assert_eq!(datagrams.max_size(), None);
        assert_eq!(
            datagrams.send(b"early").unwrap_err().kind(),
            io::ErrorKind::NotConnected
        );
        drop(server);
        let _ = dialing.join();
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
    fn an_end_with_the_stream_finished_reads_as_a_clean_end() {
        let (mut client, mut server) = pair();
        server.writer.write_frame(&frame(b"last")).unwrap();
        server.writer.shutdown();
        let mut buf = Vec::new();
        assert!(client.reader.read_frame(&mut buf, 1 << 10).unwrap());
        assert_eq!(buf, b"last");
        assert!(!client.reader.read_frame(&mut buf, 1 << 10).unwrap());
    }

    #[test]
    fn an_end_without_the_stream_finished_is_an_error_not_an_end() {
        let (mut client, mut server) = pair();
        server.writer.write_frame(&frame(b"heard")).unwrap();
        let mut buf = Vec::new();
        assert!(client.reader.read_frame(&mut buf, 1 << 10).unwrap());
        // Let go unclosed, and the last holder of the listener's runtime.
        drop(server);
        let e = client.reader.read_frame(&mut buf, 1 << 10).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::UnexpectedEof, "{e}");
    }

    #[test]
    fn the_closer_still_delivers_what_was_written() {
        let (mut client, mut server) = pair();
        // A link ending queues its last frame, then calls the closer.
        server.writer.write_frame(&frame(b"goodbye")).unwrap();
        (server.closer.take().unwrap())();
        let mut buf = Vec::new();
        assert!(client.reader.read_frame(&mut buf, 1 << 10).unwrap());
        assert_eq!(buf, b"goodbye");
        assert!(!client.reader.read_frame(&mut buf, 1 << 10).unwrap());
    }

    #[test]
    fn an_unproven_source_holds_no_connection() {
        let id = Identity::generate().unwrap();
        let config = TlsConfig::new(id.clone()).pins([id.fingerprint()]);
        let listen = QuicListen::bind("127.0.0.1:0", &config).unwrap();
        let target = listen.local_addr().unwrap();
        // A client's first packet, caught on its way, then sent again from
        // an address that never answers: as a forged source would.
        let catcher = UdpSocket::bind("127.0.0.1:0").unwrap();
        catcher
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let connect = QuicConnect::new(&catcher.local_addr().unwrap().to_string(), &config)
            .unwrap()
            .timeout(Duration::from_millis(500));
        let dialing = thread::spawn(move || connect.connect());
        let mut first = vec![0u8; 2048];
        let n = catcher.recv(&mut first).unwrap();
        let forger = UdpSocket::bind("127.0.0.1:0").unwrap();
        forger.send_to(&first[..n], target).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(
                listen
                    .accept()
                    .map(|a| matches!(a, Accepted::Connection(_))),
            );
        });
        assert!(
            rx.recv_timeout(Duration::from_secs(1)).is_err(),
            "a connection was accepted from an address that never proved it receives"
        );
        assert!(dialing.join().unwrap().is_err());
    }

    #[test]
    fn datagrams_cross_beside_the_frames() {
        let (client, server) = pair();
        let (cd, sd) = (
            client.reader.datagrams().unwrap(),
            server.reader.datagrams().unwrap(),
        );
        let room = cd.max_size().expect("the peer takes datagrams");
        assert!(room > 1000, "{room}");
        // Loopback loses none in practice; send a few in case one is.
        for _ in 0..5 {
            cd.send(b"sample").unwrap();
        }
        let got = sd.recv(Some(Duration::from_secs(5))).unwrap();
        assert_eq!(got.as_deref(), Some(&b"sample"[..]));
        // Past any UDP payload (the path's room grows as its MTU is found).
        assert_eq!(
            cd.send(&vec![0; 1 << 16]).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        // A wait that runs out is no datagram, not an error.
        while sd.recv(Some(Duration::from_millis(50))).unwrap().is_some() {}
        assert_eq!(sd.recv(Some(Duration::from_millis(50))).unwrap(), None);
    }

    #[test]
    fn a_thread_inside_another_runtime_can_wait() {
        let rt = Rt::new().unwrap();
        let other = Rt::new().unwrap();
        let inner = rt.handle().clone();
        let got = other
            .handle()
            .block_on(async move { wait(&inner, async { 7 }) });
        assert_eq!(got, 7);
    }

    #[test]
    fn a_connection_moved_to_another_address_is_not_followed() {
        let (server_id, client_id) = (Identity::generate().unwrap(), Identity::generate().unwrap());
        let server = TlsConfig::new(server_id.clone()).pins([client_id.fingerprint()]);
        let client = TlsConfig::new(client_id).pins([server_id.fingerprint()]);
        let listen = QuicListen::bind("127.0.0.1:0", &server).unwrap();
        let addr = listen.local_addr().unwrap();
        let connect = QuicConnect::new(&addr.to_string(), &client).unwrap();
        // A client of quinn's own, whose socket the test can change.
        let rt = Rt::new().unwrap();
        let h = rt.handle().clone();
        let (endpoint, connecting) = {
            let _inside = h.enter();
            let endpoint = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).unwrap();
            let crypto = connect.dial.crypto().unwrap().crypto;
            let mut config = quinn::ClientConfig::new(crypto as Arc<_>);
            config.transport_config(transport());
            let connecting = endpoint
                .connect_with(config, addr, coqui_peer_tls::SERVER_NAME)
                .unwrap();
            (endpoint, connecting)
        };
        let serving = thread::spawn(move || {
            let Accepted::Connection(mut server) = listen.accept().unwrap() else {
                panic!("every source proceeds");
            };
            server
                .reader
                .set_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut buf = Vec::new();
            assert!(server.reader.read_frame(&mut buf, 1 << 10).unwrap());
            assert_eq!(buf, b"first");
            server
        });
        let conn = wait(&h, connecting).unwrap();
        let (mut send, _recv) = wait(&h, conn.open_bi()).unwrap();
        wait(&h, send.write_all(&frame(b"first"))).unwrap();
        let mut server = serving.join().unwrap();
        let mut buf = Vec::new();
        {
            let _inside = h.enter();
            endpoint
                .rebind(UdpSocket::bind("127.0.0.1:0").unwrap())
                .unwrap();
        }
        wait(&h, send.write_all(&frame(b"moved"))).unwrap();
        server
            .reader
            .set_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        assert!(
            server.reader.read_frame(&mut buf, 1 << 10).is_err(),
            "a frame from the new address arrived"
        );
    }

    #[test]
    fn connects_through_one_client_do_not_wait_for_each_other() {
        let (server_id, client_id) = (Identity::generate().unwrap(), Identity::generate().unwrap());
        let server = TlsConfig::new(server_id.clone()).pins([client_id.fingerprint()]);
        let client = TlsConfig::new(client_id).pins([server_id.fingerprint()]);
        let listen = QuicListen::bind("127.0.0.1:0", &server).unwrap();
        let connect = Arc::new(
            QuicConnect::new(&listen.local_addr().unwrap().to_string(), &client)
                .unwrap()
                .timeout(Duration::from_secs(5)),
        );
        let dial = |connect: &Arc<QuicConnect>| {
            let connect = Arc::clone(connect);
            thread::spawn(move || {
                let started = Instant::now();
                let linked = connect.connect().map(drop);
                (linked.map_err(|e| e.kind()), started.elapsed())
            })
        };
        // The first connection's handshake waits for the server to read it,
        // which it does only once the second has linked.
        let first = dial(&connect);
        let Accepted::Connection(mut held) = listen.accept().unwrap() else {
            panic!("every source proceeds");
        };
        let second = dial(&connect);
        let Accepted::Connection(mut served) = listen.accept().unwrap() else {
            panic!("every source proceeds");
        };
        let reading = thread::spawn(move || {
            served
                .reader
                .set_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let _ = served.reader.read_frame(&mut Vec::new(), 64);
        });
        let (linked, took) = second.join().unwrap();
        assert_eq!(linked, Ok(()));
        assert!(took < Duration::from_secs(2), "{took:?}");
        held.reader
            .set_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let _ = held.reader.read_frame(&mut Vec::new(), 64);
        assert_eq!(first.join().unwrap().0, Ok(()));
        reading.join().unwrap();
    }

    #[test]
    fn an_address_nobody_answers_does_not_hold_up_the_next() {
        let id = Identity::generate().unwrap();
        let config = TlsConfig::new(id.clone()).pins([id.fingerprint()]);
        let listen = QuicListen::bind("127.0.0.1:0", &config).unwrap();
        // Bound and silent: QUIC hears nothing back from it.
        let silent = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addrs = vec![silent.local_addr().unwrap(), listen.local_addr().unwrap()];
        let connect = QuicConnect::new("unused:0", &config).unwrap();
        let serving = thread::spawn(move || {
            let Accepted::Connection(mut server) = listen.accept().unwrap() else {
                panic!("every source proceeds");
            };
            server
                .reader
                .set_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let _ = server.reader.read_frame(&mut Vec::new(), 64);
        });
        let started = Instant::now();
        let linked = connect.race(addrs).map(drop);
        assert!(linked.is_ok(), "{linked:?}");
        let took = started.elapsed();
        assert!(took < Duration::from_secs(2), "{took:?}");
        serving.join().unwrap();
    }

    #[test]
    fn the_closer_ends_a_handshake_in_flight_at_once() {
        let id = Identity::generate().unwrap();
        let config = TlsConfig::new(id.clone()).pins([id.fingerprint()]);
        let listen = QuicListen::bind("127.0.0.1:0", &config).unwrap();
        // A client that connects and never opens its stream: the server's
        // handshake waits for it.
        let connect = QuicConnect::new(&listen.local_addr().unwrap().to_string(), &config).unwrap();
        let dialing = thread::spawn(move || {
            let c = connect.connect().unwrap();
            thread::sleep(Duration::from_secs(3));
            drop(c);
        });
        let Accepted::Connection(server) = listen.accept().unwrap() else {
            panic!("every source proceeds");
        };
        let Connection {
            mut reader, closer, ..
        } = server;
        reader.set_timeout(Some(Duration::from_secs(10))).unwrap();
        let reading = thread::spawn(move || {
            let started = Instant::now();
            let read = reader.read_frame(&mut Vec::new(), 1 << 10);
            (read.map_err(|e| e.kind()), started.elapsed())
        });
        thread::sleep(Duration::from_millis(300));
        (closer.unwrap())();
        let (read, took) = reading.join().unwrap();
        assert_eq!(read, Err(io::ErrorKind::ConnectionAborted));
        assert!(took < Duration::from_secs(2), "{took:?}");
        dialing.join().unwrap();
    }

    #[test]
    fn a_client_that_never_opens_its_stream_is_bounded_by_the_read_timeout() {
        let id = Identity::generate().unwrap();
        let config = TlsConfig::new(id.clone()).pins([id.fingerprint()]);
        let listen = QuicListen::bind("127.0.0.1:0", &config).unwrap();
        // A client that connects and never opens its stream.
        let connect = QuicConnect::new(&listen.local_addr().unwrap().to_string(), &config).unwrap();
        let dialing = thread::spawn(move || {
            let c = connect.connect().unwrap();
            thread::sleep(Duration::from_secs(3));
            drop(c);
        });
        let Accepted::Connection(mut server) = listen.accept().unwrap() else {
            panic!("every source proceeds");
        };
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
        dialing.join().unwrap();
    }
}
