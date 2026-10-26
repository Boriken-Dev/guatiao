// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A call refused before it runs lets go what it handed over: the server
//! releases the host's object, so the host drops it, however early the
//! refusal came (busy, made during a call that returned, a library closed
//! under it).

extern crate alloc;

mod common;

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::io;
use std::sync::Mutex;

use common::{Ear, Memory, eventually, host, server};
use greeter_kind::{Greeter, Listener, Wide};
use guatiao::value::convert::{FromValue, TryAsRef};
use guatiao::{Alloc, Map, Status, Value};
use guatiao_source_remote::protocol::Frame;
use guatiao_source_remote::transport::{Connect, Connection, FrameRead, FrameWrite};
use guatiao_source_remote::{LinkOptions, RemoteSource, Server, Trust};

/// The tests count one listener's drops each; one at a time keeps the
/// server's pool to the test that set it up.
static SERIAL: Mutex<()> = Mutex::new(());

/// How the host's next CALL is changed on its way out.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Change {
    /// Names a parent call the server never made.
    Parent,
    /// Goes after a CLOSE of the library it calls into.
    Closed,
}

#[derive(Default)]
struct Tamper {
    /// The change the next CALL gets, once.
    next: Mutex<Option<Change>>,
    /// The library and stamp the host opened.
    opened: Mutex<Option<(String, String)>>,
    /// Whether the host is told the server runs more calls than it does.
    more_calls: AtomicBool,
}

struct TamperWrite(Box<dyn FrameWrite>, Arc<Tamper>);

impl FrameWrite for TamperWrite {
    fn write_frame(&mut self, frame: &[u8]) -> io::Result<()> {
        let decoded = Frame::decode(&frame[4..], Alloc::rust()).expect("the host's frame");
        match decoded {
            Frame::Open { library, stamp, .. } => {
                *self.1.opened.lock().unwrap() = Some((library.to_string(), stamp.to_string()));
            }
            Frame::Call {
                call,
                parent,
                target,
                slot,
                args,
            } => match self.1.next.lock().unwrap().take() {
                Some(Change::Parent) => {
                    let mut out = Vec::new();
                    Frame::Call {
                        call,
                        parent: parent + 9_999,
                        target,
                        slot,
                        args,
                    }
                    .encode(&mut out, usize::MAX)
                    .unwrap();
                    return self.0.write_frame(&out);
                }
                Some(Change::Closed) => {
                    let (library, stamp) = self.1.opened.lock().unwrap().clone().unwrap();
                    let mut close = Vec::new();
                    Frame::Close {
                        library: &library,
                        stamp: &stamp,
                    }
                    .encode(&mut close, usize::MAX)
                    .unwrap();
                    self.0.write_frame(&close)?;
                }
                None => {}
            },
            _ => {}
        }
        self.0.write_frame(frame)
    }

    fn shutdown(&mut self) {
        self.0.shutdown();
    }
}

struct TamperRead(Box<dyn FrameRead>, Arc<Tamper>);

impl FrameRead for TamperRead {
    fn read_frame(&mut self, buf: &mut Vec<u8>, max: usize) -> io::Result<bool> {
        let got = self.0.read_frame(buf, max)?;
        if !got || !self.1.more_calls.load(Ordering::Acquire) {
            return Ok(got);
        }
        let changed = match Frame::decode(buf, Alloc::rust()) {
            Ok(Frame::Welcome {
                nonce,
                proof,
                options,
            }) => {
                let max_frame = TryAsRef::<Map>::try_as_ref(&options)
                    .and_then(|m| m.get("max_frame"))
                    .and_then(|v| u64::from_value(v).ok())
                    .unwrap();
                let mut offer = Map::new();
                offer.set("max_frame", max_frame);
                offer.set("max_calls", 8u64);
                let mut out = Vec::new();
                Frame::Welcome {
                    nonce,
                    proof,
                    options: Value::from(offer),
                }
                .encode(&mut out, usize::MAX)
                .unwrap();
                Some(out[4..].to_vec())
            }
            _ => None,
        };
        if let Some(body) = changed {
            *buf = body;
        }
        Ok(true)
    }

    fn set_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.0.set_timeout(timeout)
    }
}

/// Connects to a server in this process through a [`Tamper`].
struct Tampered(Server, Arc<Tamper>);

impl Connect for Tampered {
    fn connect(&self) -> io::Result<Connection> {
        let host = Memory(self.0.clone()).connect()?;
        Ok(Connection {
            reader: Box::new(TamperRead(host.reader, Arc::clone(&self.1))),
            writer: Box::new(TamperWrite(host.writer, Arc::clone(&self.1))),
            ..host
        })
    }

    fn describe(&self) -> String {
        "memory, tampered".to_string()
    }
}

/// A host of a server running at most `max_calls` calls at once.
fn tampered(max_calls: usize, tamper: &Arc<Tamper>) -> (RemoteSource, guatiao::library::Registry) {
    let server = server(LinkOptions::new(Trust::Local).max_calls(max_calls));
    let source = RemoteSource::new(
        Box::new(Tampered(server, Arc::clone(tamper))),
        LinkOptions::new(Trust::Local).timeout(Some(Duration::from_secs(5))),
    );
    let registry = host(&source);
    (source, registry)
}

/// Hands an `Ear` to `start` and answers the status of the refusal, once
/// the host has dropped the `Ear` the server let go.
fn refused_start(greeter: &dyn Greeter, source: &RemoteSource) -> (Status, String) {
    let ear = Ear::new();
    let dropped = Arc::clone(&ear.dropped);
    let Err(e) = greeter.start(ear.into_object()) else {
        panic!("the call is refused");
    };
    eventually("the server lets go of the listener handed for it", || {
        *dropped.lock().unwrap() == 1
    });
    assert_eq!(source.objects_out(), 0);
    (e.status, e.message().to_string())
}

#[test]
fn a_call_made_during_a_call_that_returned_lets_go_what_it_handed() {
    let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let tamper = Arc::new(Tamper::default());
    let (source, registry) = tampered(4, &tamper);
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    *tamper.next.lock().unwrap() = Some(Change::Parent);
    let (status, words) = refused_start(&*greeter, &source);
    assert_eq!(status, Status::GUATIAO_ERR_GONE, "{words}");
    assert!(words.contains("has returned"), "{words}");
}

#[test]
fn a_call_into_a_library_closed_under_it_lets_go_what_it_handed() {
    let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let tamper = Arc::new(Tamper::default());
    let (source, registry) = tampered(4, &tamper);
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    *tamper.next.lock().unwrap() = Some(Change::Closed);
    let (status, words) = refused_start(&*greeter, &source);
    assert_eq!(status, Status::GUATIAO_ERR_NOT_FOUND, "{words}");
}

#[test]
fn a_call_the_server_is_too_busy_for_lets_go_what_it_handed() {
    let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let tamper = Arc::new(Tamper::default());
    // The host is told the server runs eight calls; it runs one.
    tamper.more_calls.store(true, Ordering::Release);
    let (source, registry) = tampered(1, &tamper);
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    let wide = registry
        .offers::<dyn Wide>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    // The server's one call runs for a while, so the next is refused.
    let busy = std::thread::spawn(move || wide.echo_i64(1_000 + 1_500));
    std::thread::sleep(Duration::from_millis(300));
    let (status, words) = refused_start(&*greeter, &source);
    assert_eq!(status, Status::GUATIAO_ERR_BUSY, "{words}");
    assert_eq!(busy.join().unwrap().unwrap(), 2_500);
}
