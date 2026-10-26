// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A link: one connection, either end of which may call the other while
//! a call of its own is open.
//!
//! One reader thread per link takes every frame off the transport and
//! routes it: a reply to the thread waiting for it; a call made during one
//! of this side's calls (its parent) to the thread waiting for that call,
//! which runs it and goes on waiting; any other call to the link's pool.
//! A frame is queued for the link's writer and written by whichever thread
//! finds it free, with every frame queued meanwhile: no thread waits for
//! another's write, and a call that times out takes back a frame not yet
//! written. A sender keeps at most the peer's `max_calls` top-level calls
//! in flight; nested calls are not counted, since they run on threads
//! already waiting, so the reader never stops reading and a reply can
//! always arrive.
//!
//! Objects cross as ids in their holder's numbering: a handed or answered
//! object until its handle on the far side is dropped (RELEASE), a lent one
//! until the call it was lent for returns. When the link ends, every call
//! waiting answers `GONE` and every object held for the peer is dropped.

#![forbid(unsafe_code)]

mod buffers;
mod gate;
mod handshake;
mod pool;
mod proxies;
#[cfg(test)]
mod tests;
mod waiting;

pub(crate) use proxies::Proxies;

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use core::cell::RefCell;
use core::panic::AssertUnwindSafe;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use core::time::Duration;
use std::io;
use std::panic::catch_unwind;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Mutex, MutexGuard, PoisonError, RwLock};
use std::thread::{self, ThreadId};
use std::time::Instant;

use guatiao::library::ProviderError;
use guatiao::library::kind::{
    Answer, Arg, ArgType, Caller, KindDescription, LentHandle, LentObject, Pass, RawObject,
    SlotDescription,
};
use guatiao::{Alloc, Status, Value};
use guatiao_bridge::Kinds;
use guatiao_libffi::Libffi;

use crate::protocol::args::ObjectRef;
use crate::protocol::{
    self, Decoded, Frame, HANDSHAKE_MAX_FRAME, ProtocolError, Target, Trust, decode_answer,
    decode_args,
};
use crate::transport::{Connection, Datagrams, FrameRead, FrameWrite};
use gate::{Gate, Unsent};
use handshake::Offer;
use pool::{Permits, Pool};
use waiting::Waiting;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn gone(why: &str) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_GONE, why)
}

fn bad(e: ProtocolError) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, &e.to_string())
}

/// How a link is set up. The key and the token never reach a log or a
/// `Debug`.
#[derive(Clone)]
pub struct LinkOptions {
    trust: Trust,
    key: Option<Arc<[u8]>>,
    token: Option<Arc<str>>,
    /// `token`, as a server compares a client's with it.
    required: Option<handshake::Required>,
    max_frame: usize,
    max_calls: usize,
    timeout: Option<Duration>,
    handshake_timeout: Duration,
    spin: Duration,
}

impl core::fmt::Debug for LinkOptions {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LinkOptions")
            .field("trust", &self.trust)
            .field("key", &self.key.as_ref().map(|_| "<set>"))
            .field("token", &self.token.as_ref().map(|_| "<set>"))
            .field("max_frame", &self.max_frame)
            .field("max_calls", &self.max_calls)
            .field("timeout", &self.timeout)
            .field("spin", &self.spin)
            .finish()
    }
}

/// The largest frame a link carries unless told otherwise: 64 MiB.
pub const DEFAULT_MAX_FRAME: usize = 64 << 20;
/// Calls the peer may have in flight at once unless told otherwise.
pub const DEFAULT_MAX_CALLS: usize = 64;
/// How long a call waits for its answer unless told otherwise.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// The longest a waiting thread polls before it sleeps unless told
/// otherwise.
pub const DEFAULT_SPIN: Duration = Duration::from_micros(200);

impl LinkOptions {
    /// A link at `trust`, with the defaults: frames up to 64 MiB, 64 calls
    /// in flight, calls answered within 30 seconds, a handshake within 10.
    pub fn new(trust: Trust) -> LinkOptions {
        LinkOptions {
            trust,
            key: None,
            token: None,
            required: None,
            max_frame: DEFAULT_MAX_FRAME,
            max_calls: DEFAULT_MAX_CALLS,
            timeout: Some(DEFAULT_TIMEOUT),
            handshake_timeout: Duration::from_secs(10),
            spin: DEFAULT_SPIN,
        }
    }

    /// The trust level.
    pub fn trust(&self) -> Trust {
        self.trust
    }

    /// The pre-shared key the `trusted` level proves both ways; at server
    /// TLS, the key a server asks its clients to prove (it sends a proof of
    /// its own) and a client proves when asked.
    pub fn key(mut self, key: &[u8]) -> LinkOptions {
        self.key = Some(Arc::from(key));
        self
    }

    /// The token a client presents (at server TLS or mutual TLS), or a
    /// server requires of every client.
    pub fn token(mut self, token: &str) -> LinkOptions {
        self.token = Some(Arc::from(token));
        self.required = Some(handshake::Required::new(token));
        self
    }

    /// The largest frame this side reads, which bounds what the peer may
    /// send it.
    pub fn max_frame(mut self, bytes: usize) -> LinkOptions {
        self.max_frame = bytes.clamp(HANDSHAKE_MAX_FRAME, u32::MAX as usize);
        self
    }

    /// How many of the peer's calls this side runs at once; more are
    /// refused `GUATIAO_ERR_BUSY`.
    pub fn max_calls(mut self, calls: usize) -> LinkOptions {
        self.max_calls = calls.max(1);
        self
    }

    /// How long a call waits for its answer before it is cancelled and
    /// answers `GUATIAO_ERR_GONE`, and how long a write waits for a peer
    /// that takes nothing in before the link ends; `None` waits as long as
    /// the link lives.
    pub fn timeout(mut self, timeout: Option<Duration>) -> LinkOptions {
        self.timeout = timeout;
        self
    }

    /// How long the handshake may take.
    pub fn handshake_timeout(mut self, timeout: Duration) -> LinkOptions {
        self.handshake_timeout = timeout;
        self
    }

    /// The longest a thread waiting for an answer, or an idle pool thread,
    /// polls before it sleeps. It polls just past the typical wait it has
    /// seen on the link (an answer's round trip, the gap between the peer's
    /// calls), at least 50 µs, and not at all when that is longer than this
    /// or as many threads poll as there are cores. Polling spends a core and saves the
    /// wake-up a sleeping thread costs; zero never polls.
    pub fn spin(mut self, spin: Duration) -> LinkOptions {
        self.spin = spin;
        self
    }
}

/// Why a link could not be set up.
#[derive(Debug)]
#[non_exhaustive]
pub enum LinkError {
    /// The transport failed.
    Io(io::Error),
    /// The peer sent something that is not this protocol.
    Protocol(ProtocolError),
    /// The peer refused the link, with its status and words.
    Refused {
        /// Why, as a status.
        status: Status,
        /// Why, in words.
        why: String,
    },
    /// The peer failed this side's trust check, or this build cannot run
    /// the level asked.
    Trust(String),
}

impl core::fmt::Display for LinkError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            LinkError::Io(e) => write!(f, "the link failed: {e}"),
            LinkError::Protocol(e) => write!(f, "the peer is not speaking this protocol: {e}"),
            LinkError::Refused { why, .. } => write!(f, "the peer refused the link: {why}"),
            LinkError::Trust(why) => f.write_str(why),
        }
    }
}

impl core::error::Error for LinkError {}

impl From<io::Error> for LinkError {
    /// A transport answers `PermissionDenied` when one end's trust check
    /// refused the other (a certificate, an access control list): a trust
    /// failure, not a lost link.
    fn from(e: io::Error) -> LinkError {
        if e.kind() == io::ErrorKind::PermissionDenied {
            return LinkError::Trust(e.to_string());
        }
        LinkError::Io(e)
    }
}

impl From<ProtocolError> for LinkError {
    fn from(e: ProtocolError) -> LinkError {
        LinkError::Protocol(e)
    }
}

impl From<LinkError> for ProviderError {
    fn from(e: LinkError) -> ProviderError {
        let status = match &e {
            LinkError::Io(_) => Status::GUATIAO_ERR_GONE,
            LinkError::Protocol(_) => Status::GUATIAO_ERR_BAD_VALUE,
            LinkError::Refused { status, .. } => *status,
            LinkError::Trust(_) => Status::GUATIAO_ERR_BAD_VALUE,
        };
        ProviderError::new(status, &e.to_string())
    }
}

/// What a provider table on this side answers through.
pub(crate) trait Table: Send + Sync {
    fn kind(&self) -> &Arc<KindDescription>;
    fn call(&self, slot: usize, args: Vec<Arg<'_>>) -> Result<Answer, ProviderError>;
}

/// What one side serves of its own: its catalog, the libraries a peer
/// opens, their tables.
pub(crate) trait Handler: Send + Sync + 'static {
    fn list(&self, link: &Arc<Inner>) -> Result<Value, ProviderError> {
        let _ = link;
        Err(ProviderError::new(
            Status::GUATIAO_ERR_UNSUPPORTED,
            "this side serves no providers",
        ))
    }

    fn open(&self, link: &Arc<Inner>, library: &str, stamp: &str) -> Result<(), ProviderError> {
        let _ = (link, library, stamp);
        Err(ProviderError::new(
            Status::GUATIAO_ERR_UNSUPPORTED,
            "this side serves no providers",
        ))
    }

    fn close(&self, link: &Arc<Inner>, library: &str, stamp: &str) {
        let _ = (link, library, stamp);
    }

    fn table(&self, link: &Arc<Inner>, id: u64) -> Option<Arc<dyn Table>> {
        let _ = (link, id);
        None
    }

    /// The kind of table `id`, which this side named in a catalog though
    /// `link` may not have it open: how a call refused there finds the
    /// objects it handed over.
    fn table_kind(&self, link: &Arc<Inner>, id: u64) -> Option<Arc<KindDescription>> {
        let _ = (link, id);
        None
    }

    fn ended(&self, link: &Arc<Inner>) {
        let _ = link;
    }
}

/// A side that serves nothing: a host's end of a link.
pub(crate) struct Nothing;

impl Handler for Nothing {}

/// An object this side holds for the peer.
enum Export {
    /// Handed over or answered: held until the peer releases it.
    Owned {
        object: Mutex<Option<RawObject>>,
        /// The thread inside a call of it, so a call it makes on itself is
        /// refused rather than waited on forever.
        inside: Mutex<Option<ThreadId>>,
        caller: Arc<Caller>,
        /// What keeps the object's library loaded.
        _keep: Option<Arc<dyn Table>>,
    },
    /// Lent for one of this side's calls.
    Lent {
        handle: LentHandle,
        caller: Arc<Caller>,
    },
}

impl Export {
    fn kind(&self) -> &Arc<KindDescription> {
        match self {
            Export::Owned { caller, .. } | Export::Lent { caller, .. } => caller.kind(),
        }
    }
}

/// What makes a call once its lent objects are shared: their positions
/// and the ids they were lent under.
type Exchange<'f> = &'f mut dyn FnMut(&[(usize, u64)]) -> Result<Answer, ProviderError>;

/// What reaches a thread waiting for a call.
enum Inbound {
    Reply(Vec<u8>),
    Call(Vec<u8>),
    Gone(String),
}

struct State {
    alive: bool,
    why: String,
    /// This side's calls in flight.
    calls: BTreeMap<u64, Mailbox>,
    /// Calls this side stopped waiting for, until their answer arrives.
    cancelled: BTreeMap<u64, SlotDescription>,
    /// The peer's calls running here, and whether they were cancelled.
    running: BTreeMap<u64, Arc<AtomicBool>>,
    exports: BTreeMap<u64, Arc<Export>>,
    next_export: u64,
}

/// How many cancelled calls are remembered for their late answers.
const CANCELLED: usize = 4096;

/// How many of the peer's calls may wait for one of this side's calls to
/// run them; more are refused `BUSY`, so a peer cannot grow the queue.
const MAX_QUEUED: usize = 64;

/// How deep calls made during calls may nest on one thread before one is
/// refused `BUSY` rather than run on a stack that may not hold it: each
/// level stacks a wait, a call through a table and a callback, tens of
/// kilobytes in a debug build, and a Windows main thread has 1 MiB.
const MAX_DEPTH: usize = 16;

/// Where a call in flight hears its reply and the calls made during it.
#[derive(Clone)]
struct Mailbox {
    tx: mpsc::Sender<Inbound>,
    /// Calls made during it, sent and not yet run.
    queued: Arc<AtomicUsize>,
}

impl Mailbox {
    fn new() -> (Mailbox, mpsc::Receiver<Inbound>) {
        let (tx, rx) = mpsc::channel();
        (
            Mailbox {
                tx,
                queued: Arc::new(AtomicUsize::new(0)),
            },
            rx,
        )
    }
}

pub(crate) struct Inner {
    gate: Gate,
    closer: Option<Box<dyn Fn() + Send + Sync>>,
    state: Mutex<State>,
    /// What this side reads.
    read_max: usize,
    /// What the peer reads.
    send_max: usize,
    timeout: Option<Duration>,
    /// How long a thread waiting for an answer polls.
    answers: Waiting,
    permits: Permits,
    pool: Arc<Pool>,
    handler: Arc<dyn Handler>,
    kinds: RwLock<Arc<Kinds>>,
    callers: Mutex<BTreeMap<(String, u32), Arc<Caller>>>,
    proxies: Proxies,
    next_call: AtomicU64,
    peer: String,
    /// The identity the transport proved, `sha256:<hex>`.
    peer_id: Option<String>,
    /// Datagrams beside the frames, when the transport has them.
    datagrams: Option<Arc<dyn Datagrams>>,
    trust: Trust,
}

std::thread_local! {
    /// The peer calls each thread is running, innermost last, by link.
    static RUNNING: RefCell<Vec<(usize, u64)>> = const { RefCell::new(Vec::new()) };
}

/// Marks a call of the peer's as running on this thread.
struct Running;

impl Running {
    fn enter(link: usize, call: u64) -> Running {
        RUNNING.with(|r| r.borrow_mut().push((link, call)));
        Running
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.with(|r| r.borrow_mut().pop());
    }
}

/// The peer's call this thread is running for `link`, innermost, or 0.
fn parent_on(link: usize) -> u64 {
    RUNNING.with(|r| {
        r.borrow()
            .iter()
            .rev()
            .find(|(l, _)| *l == link)
            .map_or(0, |(_, c)| *c)
    })
}

/// One end of a connection, set up and running.
pub struct Link {
    inner: Arc<Inner>,
}

impl core::fmt::Debug for Link {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Link")
            .field("peer", &self.inner.peer)
            .field("trust", &self.inner.trust)
            .field("alive", &self.alive())
            .finish()
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.inner.close_from_here();
    }
}

fn read_handshake(reader: &mut dyn FrameRead, buf: &mut Vec<u8>) -> Result<(), LinkError> {
    if !reader.read_frame(buf, HANDSHAKE_MAX_FRAME)? {
        return Err(LinkError::Io(io::ErrorKind::UnexpectedEof.into()));
    }
    Ok(())
}

fn write_frame(writer: &mut dyn FrameWrite, frame: &Frame<'_>) -> Result<(), LinkError> {
    let mut out = Vec::new();
    frame.encode(&mut out, usize::MAX)?;
    writer.write_frame(&out)?;
    Ok(())
}

fn refuse_link(writer: &mut dyn FrameWrite, status: Status, why: &str) -> LinkError {
    let _ = write_frame(
        writer,
        &Frame::Goodbye {
            status: status.into(),
            message: why,
        },
    );
    writer.shutdown();
    LinkError::Refused {
        status,
        why: why.to_string(),
    }
}

/// Everything a link starts with besides its connection.
pub(crate) struct Setup {
    pub(crate) handler: Arc<dyn Handler>,
    pub(crate) proxies: Proxies,
    pub(crate) kinds: Arc<Kinds>,
    pub(crate) name: String,
}

/// Whether this side's handshake proves the pre-shared key: always at the
/// trusted level, and at server TLS when a key is given (a server's
/// WELCOME then carries a proof, which asks the client for its own).
fn proves_key(options: &LinkOptions) -> bool {
    match options.trust {
        Trust::Trusted => true,
        Trust::ServerTls => options.key.is_some(),
        _ => false,
    }
}

#[cfg(feature = "tcp")]
fn need_key(options: &LinkOptions) -> Result<&[u8], LinkError> {
    options
        .key
        .as_deref()
        .ok_or_else(|| LinkError::Trust("the trusted level needs a pre-shared key".to_string()))
}

impl Link {
    /// Sets up the client's end over `conn`: HELLO, WELCOME and, at the
    /// trusted level, both proofs.
    pub(crate) fn client(
        conn: Connection,
        options: &LinkOptions,
        setup: Setup,
    ) -> Result<Link, LinkError> {
        let Connection {
            mut reader,
            mut writer,
            peer,
            closer,
        } = conn;
        if options.trust == Trust::Trusted && cfg!(not(feature = "tcp")) {
            return Err(LinkError::Trust(
                "the trusted level needs guatiao-source-remote's `tcp` feature".to_string(),
            ));
        }
        if options.trust == Trust::Secure && reader.peer_id().is_none() {
            writer.shutdown();
            return Err(LinkError::Trust(
                "the secure level runs over a transport that proves the peer, `tls://`".to_string(),
            ));
        }
        if options.trust == Trust::ServerTls {
            if reader.peer_id().is_none() {
                writer.shutdown();
                return Err(LinkError::Trust(
                    "server TLS runs over a transport that proves the server by its certificate"
                        .to_string(),
                ));
            }
            if options.key.is_none() && options.token.is_none() {
                writer.shutdown();
                return Err(LinkError::Trust(
                    "at server TLS the client proves itself by a key or a token: name one"
                        .to_string(),
                ));
            }
        }
        reader.set_timeout(Some(options.handshake_timeout))?;
        let ours = Offer {
            max_frame: options.max_frame,
            max_calls: options.max_calls,
            token: options.token.as_deref().map(str::to_string),
        };
        let nonce = handshake::nonce();
        write_frame(
            &mut *writer,
            &Frame::Hello {
                trust: options.trust,
                nonce,
                options: ours.to_value(),
            },
        )?;
        let mut buf = Vec::new();
        read_handshake(&mut *reader, &mut buf)?;
        let (theirs, server_nonce, proof) = match Frame::decode(&buf, Alloc::rust())? {
            Frame::Welcome {
                nonce: server_nonce,
                proof,
                options: offered,
            } => (
                Offer::from_value(&offered, &ours),
                server_nonce,
                proof.to_vec(),
            ),
            Frame::Goodbye { status, message } => {
                return Err(LinkError::Refused {
                    status: Status::from(status),
                    why: message.to_string(),
                });
            }
            _ => {
                return Err(LinkError::Protocol(ProtocolError::Field {
                    at: 0,
                    why: "a WELCOME",
                }));
            }
        };
        // At server TLS a WELCOME with a proof asks for the key, one
        // without asks none (a token, if required, went with the HELLO).
        let asked = match options.trust {
            Trust::Trusted => true,
            Trust::ServerTls => !proof.is_empty(),
            _ => false,
        };
        if asked && cfg!(not(feature = "tcp")) {
            writer.shutdown();
            return Err(LinkError::Trust(
                "the key proof needs guatiao-source-remote's `tcp` feature".to_string(),
            ));
        }
        #[cfg(feature = "tcp")]
        if asked {
            let key = options.key.as_deref().ok_or_else(|| {
                writer.shutdown();
                LinkError::Trust(if options.trust == Trust::Trusted {
                    "the trusted level needs a pre-shared key".to_string()
                } else {
                    "the server proves clients by a pre-shared key, and this client has none"
                        .to_string()
                })
            })?;
            if !handshake::proves(key, handshake::SERVER, &nonce, &server_nonce, &proof) {
                writer.shutdown();
                return Err(LinkError::Trust(
                    "the server does not hold this link's key".to_string(),
                ));
            }
            let mine = handshake::proof(key, handshake::CLIENT, &nonce, &server_nonce);
            write_frame(&mut *writer, &Frame::Proof { proof: &mine })?;
        }
        let _ = (server_nonce, proof);
        reader.set_timeout(None)?;
        writer.set_timeout(options.timeout)?;
        Ok(Link::start(
            reader, writer, closer, peer, options, &theirs, setup,
        ))
    }

    /// Sets up the server's end over `conn`, refusing a client that asks
    /// another trust level or fails its check.
    pub(crate) fn server(
        conn: Connection,
        options: &LinkOptions,
        setup: Setup,
    ) -> Result<Link, LinkError> {
        let Connection {
            mut reader,
            mut writer,
            peer,
            closer,
        } = conn;
        reader.set_timeout(Some(options.handshake_timeout))?;
        let mut buf = Vec::new();
        read_handshake(&mut *reader, &mut buf)?;
        let ours = Offer {
            max_frame: options.max_frame,
            max_calls: options.max_calls,
            token: None,
        };
        let (client_nonce, theirs) = match Frame::decode(&buf, Alloc::rust()) {
            Ok(Frame::Hello {
                trust,
                nonce,
                options: offered,
            }) => {
                if trust != options.trust {
                    return Err(refuse_link(
                        &mut *writer,
                        Status::GUATIAO_ERR_UNSUPPORTED,
                        &format!(
                            "this listener serves the {} level, not {}",
                            options.trust.name(),
                            trust.name()
                        ),
                    ));
                }
                if trust == Trust::Secure && reader.peer_id().is_none() {
                    return Err(refuse_link(
                        &mut *writer,
                        Status::GUATIAO_ERR_UNSUPPORTED,
                        "the secure level runs over a transport that proves the peer, and this one proves none",
                    ));
                }
                if trust == Trust::ServerTls && !reader.encrypted() {
                    return Err(refuse_link(
                        &mut *writer,
                        Status::GUATIAO_ERR_UNSUPPORTED,
                        "server TLS runs over a transport that encrypts and proves the server, and this one does neither",
                    ));
                }
                if trust == Trust::ServerTls && options.key.is_none() && options.token.is_none() {
                    return Err(refuse_link(
                        &mut *writer,
                        Status::GUATIAO_ERR_UNSUPPORTED,
                        "this server proves clients at server TLS by a key or a token, and has neither",
                    ));
                }
                (nonce, Offer::from_value(&offered, &ours))
            }
            Ok(_) => {
                return Err(refuse_link(
                    &mut *writer,
                    Status::GUATIAO_ERR_BAD_VALUE,
                    "a link starts with HELLO",
                ));
            }
            Err(e) => {
                return Err(refuse_link(
                    &mut *writer,
                    Status::GUATIAO_ERR_BAD_VALUE,
                    &e.to_string(),
                ));
            }
        };
        if let Some(required) = &options.required {
            let given = theirs.token.as_deref().unwrap_or_default();
            if !required.is(given.as_bytes()) {
                return Err(refuse_link(
                    &mut *writer,
                    Status::GUATIAO_ERR_BAD_VALUE,
                    "the client's token is not this server's",
                ));
            }
        }
        let nonce = handshake::nonce();
        let proof = if proves_key(options) {
            #[cfg(feature = "tcp")]
            {
                handshake::proof(need_key(options)?, handshake::SERVER, &client_nonce, &nonce)
            }
            #[cfg(not(feature = "tcp"))]
            {
                return Err(refuse_link(
                    &mut *writer,
                    Status::GUATIAO_ERR_UNSUPPORTED,
                    "this server cannot run the trusted level",
                ));
            }
        } else {
            Vec::new()
        };
        write_frame(
            &mut *writer,
            &Frame::Welcome {
                nonce,
                proof: &proof,
                options: ours.to_value(),
            },
        )?;
        #[cfg(feature = "tcp")]
        if proves_key(options) {
            read_handshake(&mut *reader, &mut buf)?;
            let proved = match Frame::decode(&buf, Alloc::rust()) {
                Ok(Frame::Proof { proof }) => handshake::proves(
                    need_key(options)?,
                    handshake::CLIENT,
                    &client_nonce,
                    &nonce,
                    proof,
                ),
                _ => false,
            };
            if !proved {
                return Err(refuse_link(
                    &mut *writer,
                    Status::GUATIAO_ERR_BAD_VALUE,
                    "the client does not hold this link's key",
                ));
            }
        }
        let _ = client_nonce;
        reader.set_timeout(None)?;
        writer.set_timeout(options.timeout)?;
        Ok(Link::start(
            reader, writer, closer, peer, options, &theirs, setup,
        ))
    }

    fn start(
        reader: Box<dyn FrameRead>,
        writer: Box<dyn FrameWrite>,
        closer: Option<Box<dyn Fn() + Send + Sync>>,
        peer: String,
        options: &LinkOptions,
        theirs: &Offer,
        setup: Setup,
    ) -> Link {
        let peer_id = reader.peer_id();
        let datagrams = reader.datagrams();
        let inner = Arc::new(Inner {
            gate: Gate::new(writer),
            closer,
            state: Mutex::new(State {
                alive: true,
                why: String::new(),
                calls: BTreeMap::new(),
                cancelled: BTreeMap::new(),
                running: BTreeMap::new(),
                exports: BTreeMap::new(),
                next_export: 0,
            }),
            read_max: options.max_frame,
            send_max: theirs.max_frame,
            timeout: options.timeout,
            permits: Permits::new(theirs.max_calls),
            pool: Arc::new(Pool::new(options.max_calls, &setup.name, options.spin)),
            answers: Waiting::new(options.spin),
            handler: setup.handler,
            kinds: RwLock::new(setup.kinds),
            callers: Mutex::new(BTreeMap::new()),
            proxies: setup.proxies,
            next_call: AtomicU64::new(1),
            peer,
            peer_id,
            datagrams,
            trust: options.trust,
        });
        let reading = Arc::clone(&inner);
        let started = thread::Builder::new()
            .name(format!("{} reader", setup.name))
            .spawn(move || read_loop(reading, reader));
        if started.is_err() {
            inner.end("the link's reader thread did not start");
        }
        Link { inner }
    }

    /// Whether the link still runs.
    pub fn alive(&self) -> bool {
        lock(&self.inner.state).alive
    }

    /// Why the link ended, once it has.
    pub fn why(&self) -> String {
        lock(&self.inner.state).why.clone()
    }

    /// The peer, in words.
    pub fn peer(&self) -> &str {
        &self.inner.peer
    }

    /// The peer's identity as the transport proved it, `sha256:<hex>` (its
    /// TLS certificate's fingerprint); `None` below the secure level.
    pub fn peer_id(&self) -> Option<&str> {
        self.inner.peer_id.as_deref()
    }

    /// Datagrams beside the link's calls, when its transport has them
    /// (`quic://`): each delivered whole or not at all, in any order.
    pub fn datagrams(&self) -> Option<Arc<dyn Datagrams>> {
        self.inner.datagrams.clone()
    }

    /// The trust level it runs at.
    pub fn trust(&self) -> Trust {
        self.inner.trust
    }

    /// Asks the peer's catalog.
    pub fn list(&self) -> Result<Value, ProviderError> {
        let reply = self
            .inner
            .request(|call, out| Frame::List { call }.encode(out, usize::MAX).map(|_| ()))?;
        let Frame::Reply { body, .. } = Frame::decode(&reply, Alloc::rust()).map_err(bad)? else {
            return Err(bad(ProtocolError::Field {
                at: 0,
                why: "a REPLY",
            }));
        };
        protocol::catalog_of(body, Alloc::rust()).map_err(bad)
    }

    /// Opens a library the peer listed: it holds its providers for this
    /// link until [`close`](Link::close).
    pub fn open(&self, library: &str, stamp: &str) -> Result<(), ProviderError> {
        self.inner
            .request(|call, out| {
                Frame::Open {
                    call,
                    library,
                    stamp,
                }
                .encode(out, usize::MAX)
                .map(|_| ())
            })
            .map(|_| ())
    }

    /// Lets a library opened on the peer go.
    pub fn close(&self, library: &str, stamp: &str) {
        let mut out = Vec::new();
        if (Frame::Close { library, stamp })
            .encode(&mut out, usize::MAX)
            .is_ok()
        {
            self.inner.post(out);
        }
    }

    /// The kinds the link knows: the peer's catalog's on a client, the
    /// server's own on a server.
    pub fn kinds(&self) -> Arc<Kinds> {
        Arc::clone(
            &self
                .inner
                .kinds
                .read()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }

    pub(crate) fn set_kinds(&self, kinds: Arc<Kinds>) {
        self.inner.set_kinds(kinds);
    }

    pub(crate) fn inner(&self) -> &Arc<Inner> {
        &self.inner
    }

    /// Ends the link: calls waiting answer `GONE`, objects held for the
    /// peer are dropped, and the peer is told.
    pub fn end(&self) {
        self.inner.close_from_here();
    }

    /// One round trip with nothing in it: how long the link takes.
    pub fn ping(&self) -> Result<Duration, ProviderError> {
        let started = Instant::now();
        self.inner
            .request(|call, out| Frame::Ping { n: call }.encode(out, usize::MAX))?;
        let took = started.elapsed();
        self.inner.answers.saw(took);
        Ok(took)
    }
}

fn read_loop(inner: Arc<Inner>, mut reader: Box<dyn FrameRead>) {
    let mut buf = Vec::new();
    let why = loop {
        match reader.read_frame(&mut buf, inner.read_max) {
            Ok(true) => {
                let frame = inner.gate.buffers().hand_over(&mut buf);
                if let Err(why) = inner.route(frame) {
                    break why;
                }
            }
            Ok(false) => break "the peer closed the link".to_string(),
            Err(e) => break format!("the link failed: {e}"),
        }
        if !lock(&inner.state).alive {
            break String::new();
        }
    };
    inner.end(&why);
}

/// What a call is made of on the receiving side.
enum Callee {
    Table(Arc<dyn Table>),
    Export(Arc<Export>),
}

impl Callee {
    fn kind(&self) -> &Arc<KindDescription> {
        match self {
            Callee::Table(t) => t.kind(),
            Callee::Export(e) => e.kind(),
        }
    }
}

impl Inner {
    fn key(self: &Arc<Self>) -> usize {
        Arc::as_ptr(self).addr()
    }

    fn why(&self) -> String {
        lock(&self.state).why.clone()
    }

    fn alive_or_gone(&self) -> Result<(), ProviderError> {
        let state = lock(&self.state);
        if state.alive {
            Ok(())
        } else {
            Err(gone(&state.why))
        }
    }

    /// Ends the link from this side: GOODBYE first, written now or by the
    /// thread writing, since on a transport nothing here can wake (a named
    /// pipe, and a shared-memory link's bell on Windows) only the peer
    /// closing its end ends a read blocked here.
    pub(crate) fn close_from_here(self: &Arc<Self>) {
        let why = "this side closed the link";
        if lock(&self.state).alive {
            let mut out = Vec::new();
            if (Frame::Goodbye {
                status: 0,
                message: why,
            })
            .encode(&mut out, usize::MAX)
            .is_ok()
            {
                let _ = self.gate.send(0, out);
            }
        }
        self.end(why);
    }

    /// Ends the link once, whoever notices first.
    pub(crate) fn end(self: &Arc<Self>, why: &str) {
        let (calls, exports) = {
            let mut state = lock(&self.state);
            if !state.alive {
                return;
            }
            state.alive = false;
            state.why = if why.is_empty() {
                "the link ended".to_string()
            } else {
                why.to_string()
            };
            state.cancelled.clear();
            for cancelled in state.running.values() {
                cancelled.store(true, Ordering::Release);
            }
            (
                core::mem::take(&mut state.calls),
                core::mem::take(&mut state.exports),
            )
        };
        let why = self.why();
        for (_, waiting) in calls {
            let _ = waiting.tx.send(Inbound::Gone(why.clone()));
        }
        self.permits.wake_all();
        self.gate.close();
        self.gate.buffers().close();
        if let Some(close) = &self.closer {
            close();
        }
        self.gate.try_with(|w| w.shutdown());
        // Every object held for the peer goes here, on this thread.
        drop(exports);
        self.handler.ended(self);
    }

    fn write(self: &Arc<Self>, frame: Vec<u8>) -> Result<(), ProviderError> {
        let sent = self.gate.send(0, frame);
        self.unsent(sent)
    }

    /// Sends call `id`'s frame: written now, or by the thread writing.
    fn write_call(self: &Arc<Self>, id: u64, frame: Vec<u8>) -> Result<(), ProviderError> {
        let sent = self.gate.send(id, frame);
        self.unsent(sent)
    }

    /// Sends call `id`'s frame, `tail` its last bytes, kept out of `head`.
    fn write_call_parts(
        self: &Arc<Self>,
        id: u64,
        head: Vec<u8>,
        tail: &[u8],
    ) -> Result<(), ProviderError> {
        let sent = self.gate.send_parts(id, head, tail);
        self.unsent(sent)
    }

    /// Sends a frame whose last bytes are `tail[from..]`.
    fn write_with_tail(
        self: &Arc<Self>,
        head: Vec<u8>,
        tail: Vec<u8>,
        from: usize,
    ) -> Result<(), ProviderError> {
        let sent = self.gate.send_with_tail(0, head, tail, from);
        self.unsent(sent)
    }

    /// Sends a frame nobody waits on.
    fn post(self: &Arc<Self>, frame: Vec<u8>) {
        let posted = self.gate.send(0, frame);
        let _ = self.unsent(posted);
    }

    fn unsent(self: &Arc<Self>, sent: Result<(), Unsent>) -> Result<(), ProviderError> {
        match sent {
            Ok(()) => Ok(()),
            Err(Unsent::Failed(e)) => {
                let why = format!("the link failed: {e}");
                self.end(&why);
                Err(gone(&why))
            }
            Err(Unsent::Closed) => Err(gone(&self.why())),
        }
    }

    pub(crate) fn release(self: &Arc<Self>, remote: u64) {
        if !lock(&self.state).alive {
            return;
        }
        let mut out = Vec::new();
        if (Frame::Release { object: remote })
            .encode(&mut out, usize::MAX)
            .is_ok()
        {
            self.post(out);
        }
    }

    pub(crate) fn set_kinds(&self, kinds: Arc<Kinds>) {
        *self.kinds.write().unwrap_or_else(PoisonError::into_inner) = kinds;
    }

    fn kind(&self, name: &str) -> Result<Arc<KindDescription>, ProviderError> {
        self.kinds
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .cloned()
            .ok_or_else(|| {
                ProviderError::new(
                    Status::GUATIAO_ERR_NOT_FOUND,
                    &format!("the kind `{name}` is not described on this link"),
                )
            })
    }

    fn caller(&self, kind: &Arc<KindDescription>) -> Result<Arc<Caller>, ProviderError> {
        let key = (kind.name().to_string(), kind.floor_hash());
        let mut callers = lock(&self.callers);
        if let Some(c) = callers.get(&key) {
            return Ok(Arc::clone(c));
        }
        let caller =
            Arc::new(Caller::new(Arc::clone(kind), &Libffi).map_err(|e| {
                ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, &e.to_string())
            })?);
        callers.insert(key, Arc::clone(&caller));
        Ok(caller)
    }

    fn export(self: &Arc<Self>, export: Export) -> Result<u64, ProviderError> {
        let mut state = lock(&self.state);
        if !state.alive {
            return Err(gone(&state.why));
        }
        state.next_export += 1;
        let id = state.next_export;
        state.exports.insert(id, Arc::new(export));
        Ok(id)
    }

    fn export_owned(
        self: &Arc<Self>,
        object: RawObject,
        kind: &Arc<KindDescription>,
        keep: Option<Arc<dyn Table>>,
    ) -> Result<u64, ProviderError> {
        let caller = self.caller(kind)?;
        self.export(Export::Owned {
            object: Mutex::new(Some(object)),
            inside: Mutex::new(None),
            caller,
            _keep: keep,
        })
    }

    fn unexport(&self, id: u64) {
        let export = lock(&self.state).exports.remove(&id);
        drop(export);
    }

    /// An object of this side's the peer gives back as itself.
    fn take_export(&self, id: u64) -> Result<RawObject, ProviderError> {
        let export = lock(&self.state)
            .exports
            .remove(&id)
            .ok_or_else(|| gone("the object has been let go"))?;
        match &*export {
            Export::Owned { object, .. } => lock(object)
                .take()
                .ok_or_else(|| gone("the object has been let go")),
            Export::Lent { .. } => Err(ProviderError::new(
                Status::GUATIAO_ERR_WRONG_KIND,
                "an object lent for a call cannot be handed over",
            )),
        }
    }

    /// The kind of what `target` names, when this side knows it.
    fn kind_of(self: &Arc<Self>, target: Target) -> Option<Arc<KindDescription>> {
        match target {
            Target::Table(id) => match self.handler.table(self, id) {
                Some(table) => Some(Arc::clone(table.kind())),
                None => self.handler.table_kind(self, id),
            },
            Target::Object(id) => lock(&self.state)
                .exports
                .get(&id)
                .map(|e| Arc::clone(e.kind())),
        }
    }

    /// Lets go what a call of the peer's handed over when it is refused
    /// before running: the peer's objects are released and this side's
    /// own, given back, dropped; a lent one ends with the call on the
    /// peer's side. Arguments whose slot this side does not know (a table
    /// it never named, an object already let go, a slot past its kind's)
    /// cannot be read, and what they handed over waits for the link's end.
    fn let_go_handed(self: &Arc<Self>, target: Target, slot: u64, args: &[u8]) {
        let found = catch_unwind(AssertUnwindSafe(|| {
            let kind = self.kind_of(target)?;
            let desc = kind.slots().get(usize::try_from(slot).ok()?)?.clone();
            decode_args(&desc, args, Alloc::rust())
                .ok()
                .map(|decoded| (desc, decoded))
        }));
        let Ok(Some((desc, decoded))) = found else {
            return;
        };
        for (d, a) in desc.args().iter().zip(&decoded) {
            if let (ArgType::Object { pass, .. }, Decoded::Object(r)) = (d.ty(), a)
                && *pass != Pass::Lent
            {
                self.let_go(*r);
            }
        }
    }

    /// Lets go one object handed over in a call that will not take it. One
    /// of this side's is dropped with the last call still inside it, so
    /// the reader never waits for that call.
    fn let_go(self: &Arc<Self>, object: ObjectRef) {
        match object {
            ObjectRef::Senders(id) => self.release(id),
            ObjectRef::Receivers(id) => {
                let export = {
                    let mut state = lock(&self.state);
                    match state.exports.get(&id).map(|e| &**e) {
                        Some(Export::Owned { .. }) => state.exports.remove(&id),
                        _ => None,
                    }
                };
                drop(export);
            }
        }
    }

    /// [`let_go_handed`](Inner::let_go_handed) for a CALL frame's body.
    fn let_go_frame(self: &Arc<Self>, frame: &[u8]) {
        if let Ok(Frame::Call {
            target, slot, args, ..
        }) = Frame::decode(frame, Alloc::rust())
        {
            self.let_go_handed(target, slot, args);
        }
    }

    fn next_call(&self) -> u64 {
        self.next_call.fetch_add(1, Ordering::Relaxed)
    }

    /// Sends a frame `build` writes for a new call id, and waits for its
    /// REPLY, running the calls made during it.
    fn request(
        self: &Arc<Self>,
        build: impl FnOnce(u64, &mut Vec<u8>) -> Result<(), ProtocolError>,
    ) -> Result<Vec<u8>, ProviderError> {
        let deadline = self.timeout.map(|t| Instant::now() + t);
        let id = self.next_call();
        let (mailbox, rx) = Mailbox::new();
        let queued = Arc::clone(&mailbox.queued);
        {
            let mut state = lock(&self.state);
            if !state.alive {
                return Err(gone(&state.why));
            }
            state.calls.insert(id, mailbox);
        }
        let mut frame = Vec::with_capacity(32);
        if let Err(e) = build(id, &mut frame) {
            lock(&self.state).calls.remove(&id);
            return Err(bad(e));
        }
        self.write_call(id, frame)?;
        let reply = self.wait(id, &rx, &queued, deadline, None)?;
        let Frame::Reply { status, body, .. } =
            Frame::decode(&reply, Alloc::rust()).map_err(bad)?
        else {
            return Err(bad(ProtocolError::Field {
                at: 0,
                why: "a REPLY",
            }));
        };
        if status != 0 {
            let words = protocol::refusal_words(body).unwrap_or("the peer refused");
            return Err(ProviderError::new(Status::from(status), words));
        }
        Ok(reply)
    }

    /// Waits for call `id`'s reply, running what the peer calls during it.
    fn wait(
        self: &Arc<Self>,
        id: u64,
        rx: &mpsc::Receiver<Inbound>,
        queued: &AtomicUsize,
        deadline: Option<Instant>,
        slot: Option<&SlotDescription>,
    ) -> Result<Vec<u8>, ProviderError> {
        let outcome = loop {
            let (spin, polling) = self.answers.poll();
            let polled = pool::spin_recv(rx, spin);
            drop(polling);
            let next = match polled {
                Some(v) => Ok(v),
                None => match deadline {
                    None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
                    Some(d) => rx.recv_timeout(d.saturating_duration_since(Instant::now())),
                },
            };
            match next {
                Ok(Inbound::Reply(frame)) => break Ok(frame),
                Ok(Inbound::Call(frame)) => {
                    queued.fetch_sub(1, Ordering::AcqRel);
                    self.run_inbound(frame);
                }
                Ok(Inbound::Gone(why)) => break Err(gone(&why)),
                Err(RecvTimeoutError::Disconnected) => break Err(gone(&self.why())),
                Err(RecvTimeoutError::Timeout) => {
                    self.cancel(id, slot);
                    break Err(gone(&format!(
                        "no answer within {:?}: the call was cancelled",
                        self.timeout.unwrap_or_default()
                    )));
                }
            }
        };
        lock(&self.state).calls.remove(&id);
        outcome
    }

    fn cancel(self: &Arc<Self>, id: u64, slot: Option<&SlotDescription>) {
        {
            let mut state = lock(&self.state);
            state.calls.remove(&id);
            if let Some(slot) = slot {
                if state.cancelled.len() >= CANCELLED {
                    state.cancelled.pop_first();
                }
                state.cancelled.insert(id, slot.clone());
            }
        }
        if self.gate.retract(id) {
            // It never reached the peer.
            return;
        }
        let mut out = Vec::new();
        if (Frame::Cancel { call: id })
            .encode(&mut out, usize::MAX)
            .is_ok()
        {
            self.post(out);
        }
    }

    /// A late answer to a cancelled call: an object in it is let go.
    fn settle_cancelled(self: &Arc<Self>, slot: &SlotDescription, frame: &[u8]) {
        let Ok(Frame::Reply {
            status: 0, body, ..
        }) = Frame::decode(frame, Alloc::rust())
        else {
            return;
        };
        if !matches!(
            slot.answer(),
            Some(guatiao::library::kind::AnswerType::Object { .. })
        ) {
            return;
        }
        let mut r = protocol::codec::Reader::new(body);
        if let Ok(code) = r.uint() {
            match ObjectRef::from_code(code) {
                ObjectRef::Senders(id) => self.release(id),
                ObjectRef::Receivers(id) => {
                    let _ = self.take_export(id);
                }
            }
        }
    }

    /// Routes one frame the reader took off the transport. An error ends
    /// the link with its words.
    fn route(self: &Arc<Self>, frame: Vec<u8>) -> Result<(), String> {
        let decoded = match Frame::decode(&frame, Alloc::rust()) {
            Ok(f) => f,
            Err(e) => {
                let why = format!("the peer sent a malformed frame: {e}");
                self.goodbye(Status::GUATIAO_ERR_BAD_VALUE, &why);
                return Err(why);
            }
        };
        match decoded {
            Frame::Reply { call, .. } => {
                let mut state = lock(&self.state);
                if let Some(waiting) = state.calls.get(&call).cloned() {
                    drop(state);
                    let _ = waiting.tx.send(Inbound::Reply(frame));
                } else if let Some(slot) = state.cancelled.remove(&call) {
                    drop(state);
                    self.settle_cancelled(&slot, &frame);
                }
            }
            Frame::Pong { n } => {
                // A ping is a request: its pong answers the call it named.
                let waiting = lock(&self.state).calls.get(&n).cloned();
                if let Some(waiting) = waiting {
                    let mut reply = Vec::new();
                    protocol::refusal(&mut reply, n, 0, "");
                    let _ = waiting.tx.send(Inbound::Reply(reply[4..].to_vec()));
                }
            }
            Frame::Call {
                call,
                parent,
                target,
                slot,
                args,
            } if parent != 0 => {
                let waiting = lock(&self.state).calls.get(&parent).cloned();
                if let Some(w) = &waiting
                    && w.queued.load(Ordering::Acquire) >= MAX_QUEUED
                {
                    self.let_go_handed(target, slot, args);
                    self.refuse(
                        call,
                        Status::GUATIAO_ERR_BUSY,
                        "too many calls wait for the call they were made during",
                    );
                    return Ok(());
                }
                let refused = match waiting {
                    None => Some(frame),
                    Some(w) => {
                        w.queued.fetch_add(1, Ordering::AcqRel);
                        match w.tx.send(Inbound::Call(frame)) {
                            Ok(()) => None,
                            Err(mpsc::SendError(Inbound::Call(frame))) => Some(frame),
                            Err(_) => None,
                        }
                    }
                };
                if let Some(frame) = refused {
                    self.let_go_frame(&frame);
                    self.refuse(
                        call,
                        Status::GUATIAO_ERR_GONE,
                        "the call it was made during has returned",
                    );
                }
            }
            Frame::Call { call, .. } => {
                let job = Unrun {
                    link: Arc::clone(self),
                    frame: Some(frame),
                };
                if let Err(busy) = self.pool.run(Box::new(move || job.run())) {
                    let why = busy.words("the peer is running as many calls as it allows");
                    self.refuse(call, Status::GUATIAO_ERR_BUSY, &why);
                }
            }
            Frame::List { call } => {
                let inner = Arc::clone(self);
                let job = Box::new(move || inner.answering(call, || inner.answer_list(call)));
                if let Err(busy) = self.pool.run(job) {
                    self.refuse(
                        call,
                        Status::GUATIAO_ERR_BUSY,
                        &busy.words("the peer is busy"),
                    );
                }
            }
            Frame::Open { call, .. } => {
                let inner = Arc::clone(self);
                let job =
                    Box::new(move || inner.answering(call, || inner.answer_open(call, &frame)));
                if let Err(busy) = self.pool.run(job) {
                    self.refuse(
                        call,
                        Status::GUATIAO_ERR_BUSY,
                        &busy.words("the peer is busy"),
                    );
                }
            }
            Frame::Close { library, stamp } => self.handler.close(self, library, stamp),
            Frame::Cancel { call } => {
                if let Some(c) = lock(&self.state).running.get(&call) {
                    c.store(true, Ordering::Release);
                }
            }
            Frame::Release { object } => self.unexport(object),
            Frame::Ping { n } => {
                let mut out = Vec::new();
                if (Frame::Pong { n }).encode(&mut out, usize::MAX).is_ok() {
                    self.post(out);
                }
            }
            Frame::Goodbye { message, .. } => {
                return Err(format!("the peer ended the link: {message}"));
            }
            _ => {
                let why = "a handshake frame after the handshake".to_string();
                self.goodbye(Status::GUATIAO_ERR_BAD_VALUE, &why);
                return Err(why);
            }
        }
        Ok(())
    }

    fn goodbye(self: &Arc<Self>, status: Status, why: &str) {
        let mut out = Vec::new();
        if (Frame::Goodbye {
            status: status.into(),
            message: why,
        })
        .encode(&mut out, usize::MAX)
        .is_ok()
        {
            self.post(out);
        }
    }

    /// Runs `answer`, which answers the peer's `call`; if it panics, answers
    /// INTERNAL, so the peer does not wait out its timeout for an answer
    /// that will never come.
    fn answering(self: &Arc<Self>, call: u64, answer: impl FnOnce()) {
        if let Err(payload) = catch_unwind(AssertUnwindSafe(answer)) {
            let running = lock(&self.state).running.remove(&call);
            // A cancelled call is answered nothing, panic or not.
            if running.is_some_and(|c| c.load(Ordering::Acquire)) {
                return;
            }
            let what = payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("no message");
            let why = format!("the call panicked on the peer: {what}");
            self.refuse(call, Status::GUATIAO_ERR_INTERNAL, &why);
        }
    }

    fn refuse(self: &Arc<Self>, call: u64, status: Status, why: &str) {
        let mut out = Vec::new();
        protocol::refusal(&mut out, call, status.into(), why);
        self.post(out);
    }

    fn answer_list(self: &Arc<Self>, call: u64) {
        let mut out = Vec::new();
        match self.handler.list(self) {
            Ok(catalog) => {
                let at = protocol::start_reply(&mut out, call, 0);
                let built = protocol::put_catalog(&mut out, &catalog)
                    .and_then(|()| protocol::finish(&mut out, at, self.send_max));
                if let Err(e) = built {
                    out.clear();
                    protocol::refusal(
                        &mut out,
                        call,
                        Status::GUATIAO_ERR_BAD_VALUE.into(),
                        &e.to_string(),
                    );
                }
            }
            Err(e) => protocol::refusal(&mut out, call, e.status.into(), e.message()),
        }
        let _ = self.write(out);
    }

    fn answer_open(self: &Arc<Self>, call: u64, frame: &[u8]) {
        let Ok(Frame::Open { library, stamp, .. }) = Frame::decode(frame, Alloc::rust()) else {
            return;
        };
        let mut out = Vec::new();
        match self.handler.open(self, library, stamp) {
            Ok(()) => protocol::refusal(&mut out, call, 0, ""),
            Err(e) => protocol::refusal(&mut out, call, e.status.into(), e.message()),
        }
        let _ = self.write(out);
    }

    /// Runs one of the peer's calls on this thread and answers it.
    fn run_inbound(self: &Arc<Self>, frame: Vec<u8>) {
        let Ok(Frame::Call {
            call,
            target,
            slot,
            args,
            ..
        }) = Frame::decode(&frame, Alloc::rust())
        else {
            return;
        };
        if RUNNING.with(|r| r.borrow().len()) >= MAX_DEPTH {
            self.let_go_handed(target, slot, args);
            self.refuse(
                call,
                Status::GUATIAO_ERR_BUSY,
                "calls made during calls nest too deep on this thread",
            );
            return;
        }
        self.answering(call, || {
            let cancelled = Arc::new(AtomicBool::new(false));
            lock(&self.state)
                .running
                .insert(call, Arc::clone(&cancelled));
            let mut out = self.gate.buffers().take();
            out.reserve(128);
            let mut tail = None;
            let served = {
                let _running = Running::enter(self.key(), call);
                self.serve(call, target, slot, args, &cancelled, &mut out, &mut tail)
            };
            lock(&self.state).running.remove(&call);
            match served {
                Ok(true) => {
                    let _ = match tail {
                        Some((tail, from)) => self.write_with_tail(out, tail, from),
                        None => self.write(out),
                    };
                }
                Ok(false) => {}
                Err(e) => {
                    if !cancelled.load(Ordering::Acquire) {
                        out.clear();
                        protocol::refusal(&mut out, call, e.status.into(), e.message());
                        let _ = self.write(out);
                    }
                }
            }
        });
        self.gate.buffers().give(frame);
    }

    fn callee(self: &Arc<Self>, target: Target) -> Result<Callee, ProviderError> {
        match target {
            Target::Table(id) => self
                .handler
                .table(self, id)
                .map(Callee::Table)
                .ok_or_else(|| {
                    ProviderError::new(
                        Status::GUATIAO_ERR_NOT_FOUND,
                        "no such table on this link: the library is not open, or has gone",
                    )
                }),
            Target::Object(id) => lock(&self.state)
                .exports
                .get(&id)
                .cloned()
                .map(Callee::Export)
                .ok_or_else(|| gone("the object has been let go")),
        }
    }

    /// Calls what `target` names with the arguments in `args`, and builds
    /// the REPLY in `out`, a large answer's encoding in `tail` (a vector and
    /// where in it the bytes start). `Ok(false)`: cancelled, nothing to send.
    #[allow(clippy::too_many_arguments)]
    fn serve(
        self: &Arc<Self>,
        call: u64,
        target: Target,
        slot: u64,
        args: &[u8],
        cancelled: &AtomicBool,
        out: &mut Vec<u8>,
        tail: &mut Option<(Vec<u8>, usize)>,
    ) -> Result<bool, ProviderError> {
        let callee = self.callee(target).inspect_err(|_| {
            self.let_go_handed(target, slot, args);
        })?;
        let kind = Arc::clone(callee.kind());
        let slot = usize::try_from(slot).unwrap_or(usize::MAX);
        let desc = kind.slots().get(slot).ok_or_else(|| {
            ProviderError::new(Status::GUATIAO_ERR_NOT_FOUND, "the kind has no such slot")
        })?;
        let mut decoded = decode_args(desc, args, Alloc::rust()).map_err(bad)?;

        // Objects first: handles on the peer's, or this side's own back.
        // Every one handed over is taken before a refusal is answered, so
        // none is left held for a call that never ran.
        let n = decoded.len();
        let mut lent_handles: Vec<Option<RawObject>> = (0..n).map(|_| None).collect();
        let mut taken: Vec<Option<RawObject>> = (0..n).map(|_| None).collect();
        let mut own_lent: Vec<Option<Arc<Export>>> = (0..n).map(|_| None).collect();
        let mut refused = None;
        for (i, (d, a)) in desc.args().iter().zip(&decoded).enumerate() {
            let (
                ArgType::Object {
                    kind: object_kind,
                    pass,
                },
                Decoded::Object(r),
            ) = (d.ty(), a)
            else {
                continue;
            };
            if refused.is_some() {
                if *pass != Pass::Lent {
                    self.let_go(*r);
                }
                continue;
            }
            let got = match (*pass, *r) {
                (Pass::Lent, ObjectRef::Senders(id)) => self
                    .kind(object_kind)
                    .and_then(|k| self.proxies.object(&k, self, id, true))
                    .map(|o| lent_handles[i] = Some(o)),
                (Pass::Lent, ObjectRef::Receivers(id)) => lock(&self.state)
                    .exports
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| gone("the object has been let go"))
                    .map(|e| own_lent[i] = Some(e)),
                (_, ObjectRef::Senders(id)) => self
                    .kind(object_kind)
                    .and_then(|k| self.proxies.object(&k, self, id, false))
                    .map(|o| taken[i] = Some(o))
                    .inspect_err(|_| self.release(id)),
                (_, ObjectRef::Receivers(id)) => self.take_export(id).map(|o| taken[i] = Some(o)),
            };
            if let Err(e) = got {
                refused = Some(e);
            }
        }
        if let Some(e) = refused {
            return Err(e);
        }
        let mut guards = Vec::new();
        for e in own_lent.iter().flatten() {
            match &**e {
                Export::Owned { object, inside, .. } => {
                    guards.push((enter(object, inside)?, inside));
                }
                Export::Lent { .. } => {
                    return Err(ProviderError::new(
                        Status::GUATIAO_ERR_UNSUPPORTED,
                        "an object lent to the peer cannot be lent back to this side",
                    ));
                }
            }
        }
        let mut guard_at = guards.iter();
        let mut own: Vec<Option<LentObject<'_>>> = Vec::with_capacity(n);
        for e in &own_lent {
            own.push(match e {
                Some(_) => guard_at
                    .next()
                    .and_then(|(g, _)| g.as_ref())
                    .map(RawObject::lend),
                None => None,
            });
        }

        let mut call_args: Vec<Arg<'_>> = Vec::with_capacity(n);
        for (i, (d, a)) in desc.args().iter().zip(decoded.iter_mut()).enumerate() {
            let pass = match d.ty() {
                ArgType::Tree { pass, .. } | ArgType::Object { pass, .. } => Some(*pass),
                _ => None,
            };
            call_args.push(match a {
                Decoded::Bool(b) => Arg::Bool(*b),
                Decoded::Int(n) => Arg::Int(*n),
                Decoded::Uint(n) => Arg::Uint(*n),
                Decoded::Float(f) => Arg::Float(*f),
                Decoded::Str(s) => Arg::Str(s),
                Decoded::Bytes(b) => Arg::Bytes(b),
                Decoded::Buffer(b) => Arg::Buffer(b.as_mut_slice()),
                Decoded::Value(None) => Arg::Value(None),
                Decoded::Value(Some(v)) => match pass {
                    Some(Pass::Taken) => Arg::TakenValue(core::mem::replace(v, Value::null())),
                    Some(Pass::Lent) => Arg::LentValue(v),
                    _ => Arg::Value(Some(&*v)),
                },
                Decoded::Map(m) => match pass {
                    Some(Pass::Taken) => Arg::TakenMap(core::mem::take(m)),
                    Some(Pass::Lent) => Arg::LentMap(m),
                    _ => Arg::Map(&*m),
                },
                Decoded::List(l) => match pass {
                    Some(Pass::Taken) => Arg::TakenList(core::mem::take(l)),
                    Some(Pass::Lent) => Arg::LentList(l),
                    _ => Arg::List(&*l),
                },
                Decoded::Object(_) => {
                    if let Some(o) = taken[i].take() {
                        Arg::Object(o)
                    } else if let Some(h) = &lent_handles[i] {
                        Arg::LentObject(h.lend())
                    } else if let Some(o) = own[i] {
                        Arg::LentObject(o)
                    } else {
                        return Err(gone("the object has been let go"));
                    }
                }
            });
        }

        let answer = match &callee {
            Callee::Table(t) => t.call(slot, call_args),
            Callee::Export(e) => call_export(e, slot, call_args),
        };
        drop(own);
        for (g, inside) in guards {
            drop(g);
            *lock(inside) = None;
        }
        drop(lent_handles);
        let answer = answer?;
        if cancelled.load(Ordering::Acquire) {
            // Nobody waits: an object it answered is dropped here.
            return Ok(false);
        }
        let keep = match &callee {
            Callee::Table(t) => Some(Arc::clone(t)),
            Callee::Export(e) => match &**e {
                Export::Owned { _keep, .. } => _keep.clone(),
                Export::Lent { .. } => None,
            },
        };
        let at = protocol::start_reply(out, call, 0);
        let mut failed = None;
        let large = protocol::args::encode_answer_tail(desc, answer, out, &mut |object| {
            let kind = match desc.answer() {
                Some(guatiao::library::kind::AnswerType::Object { kind }) => self.kind(kind),
                _ => Err(ProviderError::new(
                    Status::GUATIAO_ERR_WRONG_KIND,
                    "not an object",
                )),
            };
            match kind.and_then(|k| self.export_owned(object, &k, keep.clone())) {
                Ok(id) => ObjectRef::Senders(id),
                Err(e) => {
                    failed = Some(e);
                    ObjectRef::Senders(0)
                }
            }
        })
        .map_err(bad)?;
        if let Some(e) = failed {
            return Err(e);
        }
        if let Some(from) = large {
            // A head of its own: the answer's bytes stay where they were encoded.
            let len = out.len() - from;
            let mut head = out[..from].to_vec();
            protocol::codec::put_uint(&mut head, len as u64);
            protocol::finish_with(&mut head, at, len, self.send_max).map_err(bad)?;
            *tail = Some((core::mem::replace(out, head), from));
            return Ok(true);
        }
        protocol::args::encode_writebacks(desc, &decoded, out).map_err(bad)?;
        protocol::finish(out, at, self.send_max).map_err(bad)?;
        Ok(true)
    }

    /// Calls slot `slot` of `kind` on `target`, over the link: what every
    /// table this side built for the peer's providers and objects runs.
    pub(crate) fn call(
        self: &Arc<Self>,
        target: Target,
        kind: &KindDescription,
        slot: usize,
        mut args: Vec<Arg<'_>>,
    ) -> Result<Answer, ProviderError> {
        let desc = kind.slots().get(slot).ok_or_else(|| {
            ProviderError::new(Status::GUATIAO_ERR_NOT_FOUND, "the kind has no such slot")
        })?;
        self.alive_or_gone()?;
        let parent = parent_on(self.key());
        let deadline = self.timeout.map(|t| Instant::now() + t);
        let _permit = if parent == 0 {
            Some(self.permits.acquire(deadline).ok_or_else(|| {
                ProviderError::new(
                    Status::GUATIAO_ERR_BUSY,
                    "the peer's limit of calls in flight was not freed in time",
                )
            })?)
        } else {
            None
        };

        // Objects: a handed one is held for the peer from now on; one of
        // the peer's own goes back as itself; a lent one is shared below.
        let mut refs: Vec<Option<ObjectRef>> = vec![None; args.len()];
        let mut handed = Vec::new();
        let mut lent = Vec::new();
        for (i, (arg, d)) in args.iter_mut().zip(desc.args()).enumerate() {
            let ArgType::Object {
                kind: object_kind, ..
            } = d.ty()
            else {
                continue;
            };
            match arg {
                Arg::Object(_) => {
                    let Arg::Object(object) = core::mem::replace(arg, Arg::Bool(false)) else {
                        unreachable!("matched above");
                    };
                    refs[i] = Some(match self.proxies.remote_of(object.as_raw(), self) {
                        Some(remote) => {
                            self.proxies.forget(object);
                            ObjectRef::Receivers(remote)
                        }
                        None => {
                            let id = self
                                .export_owned(object, &self.kind(object_kind)?, None)
                                .inspect_err(|_| {
                                    for id in &handed {
                                        self.unexport(*id);
                                    }
                                })?;
                            handed.push(id);
                            ObjectRef::Senders(id)
                        }
                    });
                }
                Arg::LentObject(object) => match self.proxies.remote_of(object.as_raw(), self) {
                    Some(remote) => refs[i] = Some(ObjectRef::Receivers(remote)),
                    None => lent.push((i, *object, object_kind.clone())),
                },
                _ => {}
            }
        }
        self.lending(lent, &mut Vec::new(), &mut |shared| {
            let mut refs = refs.clone();
            for (i, id) in shared {
                refs[*i] = Some(ObjectRef::Senders(*id));
            }
            self.exchange(
                target, desc, slot, parent, &mut args, &refs, deadline, &handed,
            )
        })
    }

    /// Lends each object for the call `f` makes, as an object of this
    /// side's the peer may call until `f` returns.
    fn lending(
        self: &Arc<Self>,
        mut lent: Vec<(usize, LentObject<'_>, String)>,
        shared: &mut Vec<(usize, u64)>,
        f: Exchange<'_>,
    ) -> Result<Answer, ProviderError> {
        let Some((i, object, kind)) = lent.pop() else {
            return f(shared);
        };
        object.share(|handle| {
            let caller = self.kind(&kind).and_then(|k| self.caller(&k))?;
            let id = self.export(Export::Lent { handle, caller })?;
            shared.push((i, id));
            let outcome = self.lending(lent, shared, f);
            self.unexport(id);
            outcome
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn exchange(
        self: &Arc<Self>,
        target: Target,
        desc: &SlotDescription,
        slot: usize,
        parent: u64,
        args: &mut [Arg<'_>],
        refs: &[Option<ObjectRef>],
        deadline: Option<Instant>,
        handed: &[u64],
    ) -> Result<Answer, ProviderError> {
        let id = self.next_call();
        let (mailbox, rx) = Mailbox::new();
        let queued = Arc::clone(&mailbox.queued);
        {
            let mut state = lock(&self.state);
            if !state.alive {
                return Err(gone(&state.why));
            }
            state.calls.insert(id, mailbox);
        }
        let mut frame = Vec::with_capacity(64);
        let at = protocol::start_call(&mut frame, id, parent, target, slot as u64);
        let built =
            protocol::args::encode_args_tail(desc, args, refs, &mut frame).and_then(|tail| {
                let more = tail.map_or(0, <[u8]>::len);
                protocol::finish_with(&mut frame, at, more, self.send_max).map(|()| tail)
            });
        let tail = match built {
            Ok(tail) => tail,
            Err(e) => {
                lock(&self.state).calls.remove(&id);
                // Never sent: what was handed for it is dropped here.
                for id in handed {
                    self.unexport(*id);
                }
                return Err(bad(e));
            }
        };
        let sent = Instant::now();
        let written = match tail {
            Some(tail) => self.write_call_parts(id, frame, tail),
            None => self.write_call(id, frame),
        };
        if let Err(e) = written {
            // Never sent: what was handed for it is dropped here.
            for id in handed {
                self.unexport(*id);
            }
            return Err(e);
        }
        let reply = self.wait(id, &rx, &queued, deadline, Some(desc))?;
        self.answers.saw(sent.elapsed());
        let Frame::Reply { status, body, .. } =
            Frame::decode(&reply, Alloc::rust()).map_err(bad)?
        else {
            return Err(bad(ProtocolError::Field {
                at: 0,
                why: "a REPLY",
            }));
        };
        if status != 0 {
            let words = protocol::refusal_words(body).unwrap_or("the peer refused");
            return Err(ProviderError::new(Status::from(status), words));
        }
        let answer = decode_answer(desc, body, args, Alloc::rust(), &mut |kind, r| match r {
            ObjectRef::Senders(remote) => {
                let kind = self
                    .kind(kind)
                    .map_err(|e| ProtocolError::Value(e.message().to_string()))?;
                self.proxies
                    .object(&kind, self, remote, false)
                    .map_err(|e| ProtocolError::Value(e.message().to_string()))
            }
            ObjectRef::Receivers(own) => self
                .take_export(own)
                .map_err(|e| ProtocolError::Value(e.message().to_string())),
        })
        .map_err(bad);
        // The answer holds copies of what it read: the reply's buffer is free.
        self.gate.buffers().give(reply);
        answer
    }
}

/// A call of the peer's on its way to a pool thread. Dropped unrun, when
/// the pool refuses it, it lets go what the call handed over.
struct Unrun {
    link: Arc<Inner>,
    frame: Option<Vec<u8>>,
}

impl Unrun {
    fn run(mut self) {
        if let Some(frame) = self.frame.take() {
            self.link.run_inbound(frame);
        }
    }
}

impl Drop for Unrun {
    fn drop(&mut self) {
        if let Some(frame) = self.frame.take() {
            self.link.let_go_frame(&frame);
        }
    }
}

/// Locks an object held for the peer for one call, refusing a call it
/// makes on itself from inside one.
fn enter<'a>(
    object: &'a Mutex<Option<RawObject>>,
    inside: &Mutex<Option<ThreadId>>,
) -> Result<MutexGuard<'a, Option<RawObject>>, ProviderError> {
    let me = thread::current().id();
    if *lock(inside) == Some(me) {
        return Err(ProviderError::new(
            Status::GUATIAO_ERR_BUSY,
            "the object is inside a call already, on this thread",
        ));
    }
    let guard = lock(object);
    *lock(inside) = Some(me);
    Ok(guard)
}

fn call_export(export: &Export, slot: usize, args: Vec<Arg<'_>>) -> Result<Answer, ProviderError> {
    match export {
        Export::Owned {
            object,
            inside,
            caller,
            ..
        } => {
            let mut guard = enter(object, inside)?;
            let answer = match guard.as_mut() {
                Some(o) => o.call(caller, slot, args),
                None => Err(gone("the object has been let go")),
            };
            drop(guard);
            *lock(inside) = None;
            answer
        }
        Export::Lent { handle, caller } => handle.call(caller, slot, args),
    }
}
