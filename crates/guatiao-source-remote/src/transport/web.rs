// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What the WebSocket and WebTransport transports share: a coqui-web
//! channel as a link's two halves. A channel carries whole messages, so
//! a frame crosses as its body with no length in front; the channel
//! frames its messages itself where its wire needs it (WebTransport's
//! stream).
//!
//! A server's channel is made by its first read, on the thread serving
//! the link, so a peer that connects and stalls holds only that thread.

#![forbid(unsafe_code)]

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::io;
use std::sync::{Mutex, OnceLock, mpsc};
use std::thread;
use std::time::Instant;

use coqui_web::{Channel, Failure};

use super::{Connection, Datagrams, FrameRead, FrameWrite};
use crate::lock;

/// What a server's handshake came to: the channel, and the fingerprint of
/// the certificate the client proved it holds the key of, if one did.
pub(super) type Opened = io::Result<(Arc<dyn Channel>, Option<String>)>;

/// Opens a server's channel: the handshake coqui-web has not run yet.
pub(super) type Open = Box<dyn FnOnce() -> Opened + Send>;

/// A coqui-web failure as the error kind a link tells apart. A rejected
/// certificate is `PermissionDenied`, as a refusal is elsewhere.
pub(super) fn io_error(failure: Failure) -> io::Error {
    use io::ErrorKind::{
        BrokenPipe, ConnectionReset, Interrupted, InvalidData, PermissionDenied, TimedOut,
        Unsupported,
    };
    let kind = match failure {
        Failure::Timeout | Failure::SuspectedMtu => TimedOut,
        Failure::PathLost => ConnectionReset,
        Failure::NoProgress => BrokenPipe,
        Failure::Certificate | Failure::PeerProof | Failure::Permission => PermissionDenied,
        Failure::Protocol => InvalidData,
        Failure::Cancelled => Interrupted,
        // `Failure` is non-exhaustive.
        Failure::Unsupported | _ => Unsupported,
    };
    io::Error::new(kind, failure.to_string())
}

/// The peer's address in a `Debug` rendering of coqui-web's pending
/// connection, for the words a link is named by; `None` when it says none.
#[cfg(feature = "ws")]
pub(super) fn source_of(pending: &impl core::fmt::Debug) -> Option<String> {
    let shown = format!("{pending:?}");
    let rest = shown.split_once("peer: Some(")?.1;
    Some(rest.split_once(')')?.0.to_string())
}

/// What a connection's reader, writer and closer share.
pub(super) struct Shared {
    channel: OnceLock<Arc<dyn Channel>>,
    /// Set by `shutdown` or the closer: the channel closes as it opens,
    /// or at once when it has.
    ended: AtomicBool,
    /// The server's certificate fingerprint a client verified, canonical.
    peer: OnceLock<String>,
    /// Whether TLS runs under the channel.
    encrypted: bool,
    /// Whether the channel carries datagrams beside its messages.
    datagrams: bool,
    /// Wakes the read that waits for a server's handshake, which coqui-web
    /// bounds but cannot interrupt, when the link ends first.
    ///
    /// Also the lock `open` and `end` order themselves by: each sets its
    /// own flag and reads the other's under it, so one of them sees the
    /// other.
    waking: Mutex<Option<mpsc::Sender<Opened>>>,
    /// What the channel's one message bound is made of.
    bounds: Mutex<Bounds>,
}

/// coqui-web bounds a message by one number, both ways. A link has two:
/// what its reader allows, and what its peer takes. The channel gets the
/// larger, and the reader and the writer each keep their own check.
#[derive(Default)]
struct Bounds {
    read: usize,
    send: usize,
    /// What the channel was last told.
    applied: usize,
}

fn aborted() -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionAborted,
        "the link ended before its handshake did",
    )
}

impl Shared {
    pub(super) fn new(encrypted: bool, datagrams: bool) -> Arc<Shared> {
        Arc::new(Shared {
            channel: OnceLock::new(),
            ended: AtomicBool::new(false),
            peer: OnceLock::new(),
            encrypted,
            datagrams,
            waking: Mutex::new(None),
            bounds: Mutex::new(Bounds::default()),
        })
    }

    /// The channel is open: `peer` is the server's fingerprint a client
    /// verified.
    pub(super) fn open(&self, channel: Arc<dyn Channel>, peer: Option<String>) {
        if let Some(peer) = peer {
            let _ = self.peer.set(peer);
        }
        let ended = {
            let _order = lock(&self.waking);
            let _ = self.channel.set(Arc::clone(&channel));
            self.ended.load(Ordering::SeqCst)
        };
        self.apply_bounds(&channel);
        if ended {
            channel.close();
        }
    }

    /// Tells the channel the larger of the read and send bounds, when it
    /// has changed.
    fn bound(&self, read: Option<usize>, send: Option<usize>) {
        {
            let mut bounds = lock(&self.bounds);
            if let Some(read) = read {
                bounds.read = read;
            }
            if let Some(send) = send {
                bounds.send = send;
            }
        }
        if let Some(channel) = self.channel.get() {
            self.apply_bounds(channel);
        }
    }

    fn apply_bounds(&self, channel: &Arc<dyn Channel>) {
        let mut bounds = lock(&self.bounds);
        let wanted = bounds.read.max(bounds.send);
        if wanted != 0 && wanted != bounds.applied {
            channel.set_max_message(wanted);
            bounds.applied = wanted;
        }
    }

    /// The largest frame the peer takes.
    fn send_max(&self) -> usize {
        lock(&self.bounds).send
    }

    /// Ends the channel both ways, however far it has come.
    fn end(&self) {
        let (waking, channel) = {
            let mut order = lock(&self.waking);
            self.ended.store(true, Ordering::SeqCst);
            (order.take(), self.channel.get().cloned())
        };
        if let Some(waking) = waking {
            let _ = waking.send(Err(aborted()));
        }
        if let Some(channel) = channel {
            channel.close();
        }
    }

    fn ended(&self) -> bool {
        self.ended.load(Ordering::SeqCst)
    }

    fn channel(&self) -> io::Result<&Arc<dyn Channel>> {
        self.channel.get().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotConnected,
                "the handshake has not happened: a server's end reads first",
            )
        })
    }
}

struct Reader {
    shared: Arc<Shared>,
    open: Option<Open>,
    timeout: Option<Duration>,
    /// The largest message the channel was last told to take.
    max: usize,
}

impl Reader {
    /// The channel, opened by this first call on a server's end.
    fn channel(&mut self) -> io::Result<Option<Arc<dyn Channel>>> {
        if let Some(open) = self.open.take() {
            // The handshake runs on a thread of its own, so a link that
            // ends meanwhile is not held for the handshake's bound; the
            // thread closes what it opens too late.
            let (tx, rx) = mpsc::channel::<Opened>();
            *lock(&self.shared.waking) = Some(tx.clone());
            if self.shared.ended() {
                return Err(aborted());
            }
            thread::Builder::new()
                .name("guatiao-web-handshake".to_string())
                .spawn(move || {
                    if let Err(mpsc::SendError(Ok((late, _)))) = tx.send(open()) {
                        late.close();
                    }
                })?;
            let opened = rx.recv().map_err(|_| aborted());
            lock(&self.shared.waking).take();
            let (channel, peer) = opened??;
            self.shared.open(Arc::clone(&channel), peer);
        }
        Ok(self.shared.channel.get().cloned())
    }
}

impl FrameRead for Reader {
    fn read_frame(&mut self, buf: &mut Vec<u8>, max: usize) -> io::Result<bool> {
        self.read_frame_in(buf, max, &mut |_| Vec::new())
    }

    fn read_frame_in(
        &mut self,
        buf: &mut Vec<u8>,
        max: usize,
        _room: &mut dyn FnMut(usize) -> Vec<u8>,
    ) -> io::Result<bool> {
        let Some(channel) = self.channel()? else {
            return Ok(false);
        };
        if self.max != max {
            self.shared.bound(Some(max), None);
            self.max = max;
        }
        match channel.receive(self.timeout) {
            Ok(Some(message)) => {
                if message.len() > max {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("a message over the {max} bytes allowed"),
                    ));
                }
                // The message's own buffer is the frame's: no copy.
                *buf = message;
                Ok(true)
            }
            Ok(None) => Ok(false),
            Err(Failure::Protocol) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("a message over the {max} bytes allowed, or one that breaks the wire"),
            )),
            // This side ended the channel: a clean end of the reads.
            Err(_) if self.shared.ended() => Ok(false),
            Err(failure) => Err(io_error(failure)),
        }
    }

    fn set_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.timeout = timeout;
        Ok(())
    }

    fn peer_id(&self) -> Option<String> {
        self.shared.peer.get().cloned()
    }

    fn encrypted(&self) -> bool {
        self.shared.encrypted
    }

    fn datagrams(&self) -> Option<Arc<dyn Datagrams>> {
        self.shared.datagrams.then(|| {
            Arc::new(ChannelDatagrams {
                shared: Arc::clone(&self.shared),
            }) as Arc<dyn Datagrams>
        })
    }
}

struct Writer {
    shared: Arc<Shared>,
    /// How long a write waits for a peer slow to take what is queued.
    timeout: Option<Duration>,
}

/// The body of a frame the link built: the frame without its length.
fn body(frame: &[u8]) -> io::Result<&[u8]> {
    frame
        .get(4..)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "a frame without its length"))
}

impl Writer {
    fn send(&self, message: &[u8]) -> io::Result<()> {
        let channel = self.shared.channel()?;
        let send_max = self.shared.send_max();
        if send_max != 0 && message.len() > send_max {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "a frame of {} bytes, past the {send_max} the peer takes",
                    message.len()
                ),
            ));
        }
        // coqui-web queues a message and answers `NoProgress` when the
        // queue is full instead of waiting: the peer is slow, not gone, so
        // wait for room as a stream's write does, for the link's write
        // timeout.
        let until = self.timeout.map(|t| Instant::now() + t);
        let mut pause = Duration::from_millis(1);
        loop {
            match channel.send(message) {
                Ok(()) => return Ok(()),
                Err(Failure::NoProgress) if !self.shared.ended() => {
                    if until.is_some_and(|until| Instant::now() >= until) {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "the peer took nothing of the queued frames within the write timeout",
                        ));
                    }
                    thread::sleep(pause);
                    pause = (pause * 2).min(Duration::from_millis(20));
                }
                Err(Failure::Protocol) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "a message the channel does not carry: empty, or over its bound",
                    ));
                }
                Err(_) if self.shared.ended() => {
                    return Err(io::Error::new(io::ErrorKind::BrokenPipe, "the link ended"));
                }
                Err(other) => return Err(io_error(other)),
            }
        }
    }
}

impl FrameWrite for Writer {
    fn write_frame(&mut self, frame: &[u8]) -> io::Result<()> {
        self.send(body(frame)?)
    }

    fn write_frame_parts(&mut self, head: &[u8], tail: &[u8]) -> io::Result<()> {
        let head = body(head)?;
        let mut joined = Vec::with_capacity(head.len() + tail.len());
        joined.extend_from_slice(head);
        joined.extend_from_slice(tail);
        self.send(&joined)
    }

    fn write_frames(&mut self, frames: &[&[u8]]) -> io::Result<()> {
        frames.iter().try_for_each(|frame| self.send(body(frame)?))
    }

    fn set_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.timeout = timeout;
        Ok(())
    }

    fn set_max_frame(&mut self, bytes: usize) {
        self.shared.bound(None, Some(bytes));
    }

    fn shutdown(&mut self) {
        self.shared.end();
    }
}

/// A channel's datagrams, once it is open.
struct ChannelDatagrams {
    shared: Arc<Shared>,
}

impl ChannelDatagrams {
    fn inner(&self) -> io::Result<&dyn coqui_web::Datagrams> {
        self.shared.channel()?.datagrams().ok_or_else(|| {
            io::Error::new(io::ErrorKind::Unsupported, "the peer takes no datagrams")
        })
    }
}

impl Datagrams for ChannelDatagrams {
    fn send(&self, payload: &[u8]) -> io::Result<()> {
        let inner = self.inner()?;
        inner.send(payload).map_err(|failure| match failure {
            Failure::Protocol => io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "a datagram of {} bytes is larger than the path takes ({:?})",
                    payload.len(),
                    inner.max_size()
                ),
            ),
            Failure::PathLost => io::Error::new(io::ErrorKind::NotConnected, "the link ended"),
            other => io_error(other),
        })
    }

    fn recv(&self, timeout: Option<Duration>) -> io::Result<Option<Vec<u8>>> {
        match self.inner()?.receive(timeout) {
            Ok(Some(datagram)) => Ok(Some(datagram)),
            Ok(None) => Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "the link ended",
            )),
            Err(Failure::Timeout) => Ok(None),
            Err(other) => Err(io_error(other)),
        }
    }

    fn max_size(&self) -> Option<usize> {
        self.inner().ok()?.max_size()
    }
}

/// The two halves of a link over `shared`'s channel: open already, or made
/// by `open` on the first read.
pub(super) fn connection(shared: Arc<Shared>, open: Option<Open>, peer: String) -> Connection {
    let closing = Arc::clone(&shared);
    Connection {
        reader: Box::new(Reader {
            shared: Arc::clone(&shared),
            open,
            timeout: None,
            max: 0,
        }),
        writer: Box::new(Writer {
            shared,
            timeout: None,
        }),
        peer,
        closer: Some(Box::new(move || closing.end())),
    }
}

/// A refusal queue for a listener whose admission coqui-web applies
/// before it hands a connection over: the sources it turned away, for
/// `Accepted::Refused`.
#[derive(Default)]
pub(super) struct Refused(Mutex<Vec<core::net::SocketAddr>>);

impl Refused {
    /// Most remembered between two accepts; a flood past it is not listed.
    const MOST: usize = 1024;

    pub(super) fn add(&self, source: core::net::SocketAddr) {
        let mut held = lock(&self.0);
        if held.len() < Self::MOST {
            held.push(source);
        }
    }

    pub(super) fn take(&self) -> Option<core::net::SocketAddr> {
        lock(&self.0).pop()
    }
}

#[cfg(feature = "tls")]
pub(super) use verified::{
    Watching, forget_proven, server_config, server_config_proving, take_proven,
};

#[cfg(feature = "tls")]
mod verified {
    use alloc::sync::Arc;
    use core::cell::RefCell;
    use std::io;
    use std::sync::Mutex;

    use coqui_peer_tls::{PeerId, Refusal};
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::server::ResolvesServerCert;
    use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
    use rustls::{DigitallySignedStruct, DistinguishedName, ServerConfig, SignatureScheme};

    use crate::lock;
    use crate::transport::tls::TlsConfig;

    /// A client's server verifier, keeping what one connection's handshake
    /// said: the refusal for the words, and the fingerprint of the
    /// certificate whose key the server proved. One per connection.
    #[derive(Debug)]
    pub(crate) struct Watching {
        inner: Arc<dyn ServerCertVerifier>,
        refusal: Mutex<Option<Refusal>>,
        failed: Mutex<Option<rustls::Error>>,
        proven: Mutex<Option<PeerId>>,
    }

    impl Watching {
        pub(crate) fn new(inner: Arc<dyn ServerCertVerifier>) -> Arc<Watching> {
            Arc::new(Watching {
                inner,
                refusal: Mutex::new(None),
                failed: Mutex::new(None),
                proven: Mutex::new(None),
            })
        }

        /// The connect's refusal of the server's certificate in words
        /// naming what to change, if it refused one: `addr` is the
        /// server's address and `host` the name dialled.
        pub(crate) fn rejection(&self, addr: &str, host: &str) -> Option<io::Error> {
            if let Some(refusal) = *lock(&self.refusal) {
                return Some(crate::transport::tls::untrusted_server(refusal, addr, host));
            }
            lock(&self.failed).as_ref().map(|e| match e {
                rustls::Error::InvalidCertificate(why) => io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "the server at {addr} presented a certificate this client refuses ({why:?})"
                    ),
                ),
                other => io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("the TLS handshake with the server at {addr} failed: {other}"),
                ),
            })
        }

        /// The server's fingerprint, once it proved it holds the key.
        pub(crate) fn proven(&self) -> Option<String> {
            lock(&self.proven).map(|id| id.canonical())
        }

        fn run<T>(&self, out: Result<T, rustls::Error>) -> Result<T, rustls::Error> {
            if let Err(e) = &out {
                if let Some(r) = Refusal::from_rustls(e) {
                    lock(&self.refusal).get_or_insert(r);
                }
                lock(&self.failed).get_or_insert_with(|| e.clone());
            }
            out
        }
    }

    impl ServerCertVerifier for Watching {
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
            let out = self.run(
                self.inner
                    .verify_tls12_signature(message, certificate, signed),
            );
            if out.is_ok() {
                *lock(&self.proven) = Some(PeerId::of(certificate.as_ref()));
            }
            out
        }

        fn verify_tls13_signature(
            &self,
            message: &[u8],
            certificate: &CertificateDer<'_>,
            signed: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            let out = self.run(
                self.inner
                    .verify_tls13_signature(message, certificate, signed),
            );
            if out.is_ok() {
                *lock(&self.proven) = Some(PeerId::of(certificate.as_ref()));
            }
            out
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

    thread_local! {
        /// The fingerprint of the certificate a client proved it holds the
        /// key of, in the handshake this thread is running.
        static PROVEN: RefCell<Option<PeerId>> = const { RefCell::new(None) };
    }

    /// Forgets what an earlier handshake on this thread proved.
    pub(crate) fn forget_proven() {
        PROVEN.with(|proven| proven.take());
    }

    /// What the handshake just run on this thread proved of its client.
    pub(crate) fn take_proven() -> Option<String> {
        PROVEN.with(|proven| proven.take()).map(|id| id.canonical())
    }

    /// A server's client verifier that notes the fingerprint of the
    /// certificate whose key the client proved, on the thread running the
    /// handshake. coqui-web's WebSocket listener runs a handshake on the
    /// thread that calls `Pending::handshake`, one thread to a connection,
    /// and reports no certificate; this is how the link learns who the
    /// client is.
    #[derive(Debug)]
    struct Proving {
        inner: Arc<dyn ClientCertVerifier>,
    }

    impl Proving {
        fn note<T>(
            out: Result<T, rustls::Error>,
            certificate: &CertificateDer<'_>,
        ) -> Result<T, rustls::Error> {
            if out.is_ok() {
                PROVEN.with(|proven| *proven.borrow_mut() = Some(PeerId::of(certificate.as_ref())));
            }
            out
        }
    }

    impl ClientCertVerifier for Proving {
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
            self.inner
                .verify_client_cert(certificate, intermediates, now)
        }

        fn verify_tls12_signature(
            &self,
            message: &[u8],
            certificate: &CertificateDer<'_>,
            signed: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Proving::note(
                self.inner
                    .verify_tls12_signature(message, certificate, signed),
                certificate,
            )
        }

        fn verify_tls13_signature(
            &self,
            message: &[u8],
            certificate: &CertificateDer<'_>,
            signed: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Proving::note(
                self.inner
                    .verify_tls13_signature(message, certificate, signed),
                certificate,
            )
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            self.inner.supported_verify_schemes()
        }

        fn requires_raw_public_keys(&self) -> bool {
            self.inner.requires_raw_public_keys()
        }
    }

    /// The server configuration of a mutual-TLS WebSocket listener: as
    /// `config` says (trust, policy), its clients' certificates noted by
    /// [`take_proven`]. The policy's access control list reads each
    /// connection's addresses from the scope the caller sets around the
    /// handshake (`coqui_peer_tls::with_addresses`).
    pub(crate) fn server_config_proving(
        config: &TlsConfig,
        alpn: &[u8],
    ) -> io::Result<Arc<ServerConfig>> {
        let mut bare = config.tls_policy().clone();
        bare.acl = None;
        let verifier = config
            .client_trust()?
            .client_cert_verifier_with(&bare)
            .map_err(|e| crate::transport::tls::unbuilt(&e))?;
        let verifier = Arc::new(Proving { inner: verifier });
        coqui_peer_tls::server_config_with_verifier(
            config.identity(),
            verifier,
            &[alpn],
            config.tls_policy(),
        )
        .map(|built| Arc::new(built.config))
        .map_err(|e| crate::transport::tls::unbuilt(&e))
    }

    /// The server configuration coqui-web's listeners take: `config`'s
    /// identity (or `resolver`'s certificates), policy and trust, naming
    /// `alpn`. The policy's access control list is left out of the TLS
    /// configuration: coqui-peer-tls reads each connection's addresses
    /// from a scope its own accept helpers set, which coqui-web's
    /// listeners do not, so the listener decides the list by address
    /// before the handshake instead.
    pub(crate) fn server_config(
        config: &TlsConfig,
        alpn: &[u8],
        resolver: Option<Arc<dyn ResolvesServerCert>>,
    ) -> io::Result<Arc<ServerConfig>> {
        let mut policy = config.tls_policy().clone();
        policy.acl = None;
        let built = config.clone().policy(policy).server_naming(alpn)?;
        match resolver {
            None => Ok(built),
            Some(resolver) => {
                let mut tls = ServerConfig::clone(&built);
                tls.cert_resolver = resolver;
                Ok(Arc::new(tls))
            }
        }
    }
}

/// Whether a listener of `config` admits a client from `source` by its
/// address: a rule naming the address a packet came to cannot be decided
/// from the source alone, and is refused, as it is on a QUIC endpoint
/// that cannot tell the local address.
#[cfg(feature = "tls")]
pub(super) fn admits(
    config: &super::tls::TlsConfig,
    source: core::net::SocketAddr,
    bound: core::net::SocketAddr,
) -> bool {
    match &config.tls_policy().acl {
        None => true,
        Some(acl) => {
            !acl.rules.iter().any(|r| r.destination.is_some())
                && config.admits(source, bound).is_ok()
        }
    }
}
