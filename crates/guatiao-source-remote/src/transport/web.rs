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
//! the link, so a peer that connects and stalls holds only that thread;
//! the closer cancels a handshake still under way.

#![forbid(unsafe_code)]

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::time::Duration;
use std::io;
use std::sync::{Mutex, OnceLock};

use coqui_web::{Cancel, Channel, Failure};

use super::{Connection, Datagrams, FrameRead, FrameWrite};
use crate::lock;

/// Opens a server's channel: the handshake coqui-web has not run yet.
pub(super) type Open = Box<dyn FnOnce() -> io::Result<Arc<dyn Channel>> + Send>;

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

fn aborted() -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionAborted,
        "the link ended before its handshake did",
    )
}

/// What a connection's reader, writer and closer share.
pub(super) struct Shared {
    channel: OnceLock<Arc<dyn Channel>>,
    /// Set by `shutdown` or the closer: the channel closes as it opens,
    /// or at once when it has.
    ended: AtomicBool,
    /// Held by `open` and `end` while each sets its own flag and reads the
    /// other's, so one of them sees the other; it keeps the cancel of a
    /// server's handshake until the channel is open.
    order: Mutex<Option<Cancel>>,
    /// Whether the peer's certificate, as the channel reports it, names
    /// the peer: false at no security, where nothing is checked.
    #[cfg_attr(not(feature = "tls"), allow(dead_code))]
    names_peer: bool,
    /// Whether TLS runs under the channel.
    encrypted: bool,
    /// Whether the channel carries datagrams beside its messages.
    datagrams: bool,
    /// The largest message the link's reader allows and its peer takes,
    /// told to the channel once it is open (0: not yet known).
    receive: AtomicUsize,
    send: AtomicUsize,
}

impl Shared {
    pub(super) fn new(encrypted: bool, datagrams: bool, names_peer: bool) -> Arc<Shared> {
        Arc::new(Shared {
            channel: OnceLock::new(),
            ended: AtomicBool::new(false),
            order: Mutex::new(None),
            names_peer,
            encrypted,
            datagrams,
            receive: AtomicUsize::new(0),
            send: AtomicUsize::new(0),
        })
    }

    /// Keeps the cancel of a handshake that has not run, for `end`.
    fn on_cancel(&self, cancel: Cancel) {
        let ended = {
            let mut order = lock(&self.order);
            if self.ended.load(Ordering::SeqCst) {
                true
            } else {
                *order = Some(cancel.clone());
                false
            }
        };
        if ended {
            cancel.cancel();
        }
    }

    /// The channel is open.
    pub(super) fn open(&self, channel: Arc<dyn Channel>) {
        let ended = {
            let mut order = lock(&self.order);
            order.take();
            let _ = self.channel.set(Arc::clone(&channel));
            self.ended.load(Ordering::SeqCst)
        };
        self.apply(&channel);
        if ended {
            channel.close();
        }
    }

    /// Tells the channel what the link's reader allows and what its peer
    /// takes: its bounds on what it receives and on what it sends.
    fn bound(&self, receive: Option<usize>, send: Option<usize>) {
        if let Some(receive) = receive {
            self.receive.store(receive, Ordering::SeqCst);
        }
        if let Some(send) = send {
            self.send.store(send, Ordering::SeqCst);
        }
        if let Some(channel) = self.channel.get() {
            self.apply(channel);
        }
    }

    fn apply(&self, channel: &Arc<dyn Channel>) {
        let receive = self.receive.load(Ordering::SeqCst);
        if receive != 0 {
            channel.set_max_receive(receive);
        }
        let send = self.send.load(Ordering::SeqCst);
        if send != 0 {
            channel.set_max_send(send);
        }
    }

    /// The largest frame the peer takes (0: not yet known).
    fn send_max(&self) -> usize {
        self.send.load(Ordering::SeqCst)
    }

    /// The fingerprint of the certificate the peer presented and this
    /// side's TLS accepted, if it names the peer.
    fn peer_id(&self) -> Option<String> {
        #[cfg(feature = "tls")]
        if self.names_peer {
            let chain = self.channel.get()?.peer_certificates()?;
            return Some(coqui_peer_tls::PeerId::of(chain.first()?).canonical());
        }
        None
    }

    /// Ends the channel both ways, however far it has come, a handshake
    /// under way included.
    fn end(&self) {
        let (cancel, channel) = {
            let mut order = lock(&self.order);
            self.ended.store(true, Ordering::SeqCst);
            (order.take(), self.channel.get().cloned())
        };
        if let Some(cancel) = cancel {
            cancel.cancel();
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
    /// Whether the link told the reader its bound (`set_max_frame`): the
    /// channel then reads under it, and each read's `max` only bounds the
    /// frame it returns.
    fixed: bool,
}

impl Reader {
    /// The channel, opened by this first call on a server's end.
    fn channel(&mut self) -> io::Result<Option<Arc<dyn Channel>>> {
        if let Some(open) = self.open.take() {
            if self.shared.ended() {
                return Err(aborted());
            }
            match open() {
                Ok(channel) => self.shared.open(channel),
                // The closer cancelled it.
                Err(_) if self.shared.ended() => return Err(aborted()),
                Err(e) => return Err(e),
            }
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
        if !self.fixed && self.max != max {
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

    fn set_max_frame(&mut self, bytes: usize) {
        self.shared.bound(Some(bytes), None);
        self.max = bytes;
        self.fixed = true;
    }

    fn peer_id(&self) -> Option<String> {
        self.shared.peer_id()
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
        // coqui-web waits for room in its send queue for the link's write
        // timeout, as a stream's write waits for the peer.
        channel
            .send_timeout(message, self.timeout)
            .map_err(|failure| match failure {
                Failure::Protocol => io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "a message the channel does not carry: empty, or over its bound",
                ),
                Failure::Timeout => io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the peer took nothing of the queued frames within the write timeout",
                ),
                _ if self.shared.ended() => {
                    io::Error::new(io::ErrorKind::BrokenPipe, "the link ended")
                }
                other => io_error(other),
            })
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

/// Refuses a client the listener has decided against after the
/// handshake: the link's own refusal frame (`BAD_VALUE`, `why` in it), as
/// the link would send it, then the channel closed with it flushed. The
/// client reads it as the reply it waits for.
#[cfg_attr(not(feature = "webtransport"), allow(dead_code))]
pub(super) fn refuse(channel: &dyn Channel, why: &str) {
    let frame = crate::protocol::Frame::Goodbye {
        status: guatiao::Status::GUATIAO_ERR_BAD_VALUE.into(),
        message: why,
    };
    let mut out = Vec::new();
    if frame.encode(&mut out, usize::MAX).is_ok()
        && let Some(body) = out.get(4..)
    {
        let _ = channel.send(body);
    }
    channel.close();
}

/// The two halves of a link over `shared`'s channel: open already
/// (`open` none), or made by `open` on the first read, which the closer
/// can cancel by `cancel`.
pub(super) fn connection(
    shared: Arc<Shared>,
    open: Option<(Open, Cancel)>,
    peer: String,
) -> Connection {
    let closing = Arc::clone(&shared);
    let open = open.map(|(open, cancel)| {
        shared.on_cancel(cancel);
        open
    });
    Connection {
        reader: Box::new(Reader {
            shared: Arc::clone(&shared),
            open,
            timeout: None,
            max: 0,
            fixed: false,
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
#[cfg_attr(not(feature = "webtransport"), allow(unused_imports))]
pub(super) use verified::{Watching, canonical_list, list_permits, server_config};

#[cfg(feature = "tls")]
mod verified {
    use alloc::sync::Arc;
    use core::net::SocketAddr;
    use std::io;
    use std::sync::Mutex;

    use coqui_net_acl::{Acl, Outcome};
    use coqui_peer_tls::{Digest, PeerId, Refusal};
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::server::ResolvesServerCert;
    use rustls::{DigitallySignedStruct, DistinguishedName, ServerConfig, SignatureScheme};

    use crate::lock;
    use crate::transport::tls::TlsConfig;

    /// A client's server verifier, keeping what one connection's handshake
    /// said, for the words of a refusal. One per connection.
    #[derive(Debug)]
    pub(crate) struct Watching {
        inner: Arc<dyn ServerCertVerifier>,
        refusal: Mutex<Option<Refusal>>,
        failed: Mutex<Option<rustls::Error>>,
    }

    impl Watching {
        pub(crate) fn new(inner: Arc<dyn ServerCertVerifier>) -> Arc<Watching> {
            Arc::new(Watching {
                inner,
                refusal: Mutex::new(None),
                failed: Mutex::new(None),
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

    /// The server configuration coqui-web's listeners take: `config`'s
    /// identity (or `resolver`'s certificates), policy and trust, naming
    /// `alpn`.
    ///
    /// `scoped` says the handshake runs on a thread the caller gives the
    /// connection's addresses (`coqui_peer_tls::with_addresses`), so the
    /// policy's access control list can stay in the TLS at mutual TLS: a
    /// WebSocket listener. Otherwise the list is left out of it, because
    /// coqui-peer-tls would find no addresses and refuse every client, and
    /// the listener decides its address rules in coqui-web's admission. A
    /// WebTransport listener takes a configuration per connection instead
    /// (`ServerConfigs`), and this one only as the fail-closed fallback.
    pub(crate) fn server_config(
        config: &TlsConfig,
        alpn: &[u8],
        resolver: Option<Arc<dyn ResolvesServerCert>>,
        scoped: bool,
    ) -> io::Result<Arc<ServerConfig>> {
        let mut policy = config.tls_policy().clone();
        let mutual = config.is_mutual() && config.proves_peers();
        if !(scoped && mutual) {
            policy.acl = None;
        }
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

    /// The policy's list with its fingerprints in canonical form, checked
    /// once: a rule naming something that is not a SHA-256 fingerprint is
    /// refused here, as a listener is made. `None` where there is no list.
    #[cfg_attr(not(feature = "webtransport"), allow(dead_code))]
    pub(crate) fn canonical_list(config: &TlsConfig) -> io::Result<Option<Arc<Acl>>> {
        let Some(acl) = &config.tls_policy().acl else {
            return Ok(None);
        };
        let mut acl = acl.clone();
        acl.canonicalize_fingerprints(|index, text| {
            text.parse::<PeerId>()
                .ok()
                .filter(|id| id.digest() == Digest::Sha256)
                .map(|id| id.canonical())
                .ok_or_else(|| {
                    format!("acl rule {index}: the fingerprint is not a SHA-256 peer id")
                })
        })
        .map_err(crate::transport::tls::invalid)?;
        Ok(Some(Arc::new(acl)))
    }

    /// Whether the list permits the client that presented `chain` (its own
    /// certificate first), from `source` to `local`: the fingerprint rules
    /// of the list, decided after a handshake that had no addresses to
    /// decide them by.
    #[cfg_attr(not(feature = "webtransport"), allow(dead_code))]
    pub(crate) fn list_permits(
        acl: &Acl,
        source: SocketAddr,
        local: SocketAddr,
        chain: Option<&[Vec<u8>]>,
    ) -> io::Result<()> {
        let denied = |why: String| io::Error::new(io::ErrorKind::PermissionDenied, why);
        let Some(certificate) = chain.and_then(<[Vec<u8>]>::first) else {
            return Err(denied(
                "the client presented no certificate to decide the list by".to_string(),
            ));
        };
        let decision = acl.decide_peer(source, local, &PeerId::of(certificate).canonical());
        match decision.outcome {
            Outcome::Permit => Ok(()),
            _ => Err(denied(format!(
                "the server's access control list refuses this client's certificate (rule {})",
                decision
                    .rule_index
                    .map_or_else(|| "its default".to_string(), |i| i.to_string())
            ))),
        }
    }
}

/// Whether any rule of `acl` names the address or the port a packet came
/// to, which a listener bound to a wildcard cannot know.
pub(super) fn names_local(acl: &coqui_net_acl::Acl) -> bool {
    acl.rules
        .iter()
        .any(|r| r.destination.is_some() || r.port.is_some())
}

/// Whether a listener of `config` admits a client from `source` by its
/// address. A rule naming the address a packet came to (or the port) is
/// decided by the address the listener is bound to; when that is a
/// wildcard it cannot be, and the rule refuses the client, as it does on
/// a QUIC endpoint that cannot tell the local address.
#[cfg(feature = "tls")]
pub(super) fn admits(
    config: &super::tls::TlsConfig,
    source: core::net::SocketAddr,
    bound: core::net::SocketAddr,
) -> bool {
    match &config.tls_policy().acl {
        None => true,
        Some(acl) => {
            (!names_local(acl) || !bound.ip().is_unspecified())
                && config.admits(source, bound).is_ok()
        }
    }
}
