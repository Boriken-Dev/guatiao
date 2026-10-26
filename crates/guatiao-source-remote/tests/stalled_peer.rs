// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A peer that takes nothing in holds up no call past its deadline: a call
//! waiting for the link's writer gives up when its time is up, and a call
//! that timed out does not wait to send its CANCEL.

extern crate alloc;

mod common;

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::time::Duration;
use std::io;
use std::sync::mpsc;
use std::time::Instant;

use guatiao::Status;
use guatiao::library::ProviderError;
use guatiao_source_remote::transport::{Connect, Connection, FrameRead, FrameWrite, memory};
use guatiao_source_remote::{Link, LinkOptions, RemoteSource, Server, Trust};

#[derive(Default)]
struct Valve {
    /// The host's writes wait, as on a full ring nobody drains.
    choked: AtomicBool,
    /// The server reads nothing more.
    stalled: AtomicBool,
    /// Frames the stalled server holds unread.
    held: AtomicUsize,
    /// Host writes waiting at the choke.
    choking: AtomicUsize,
}

fn wait_while(flag: &AtomicBool) {
    while flag.load(Ordering::Acquire) {
        std::thread::sleep(Duration::from_millis(1));
    }
}

struct StallRead(Box<dyn FrameRead>, Arc<Valve>);

impl FrameRead for StallRead {
    fn read_frame(&mut self, buf: &mut Vec<u8>, max: usize) -> io::Result<bool> {
        // Also after: a read already waiting when the valve closes holds
        // what it gets.
        let got = self.0.read_frame(buf, max);
        if self.1.stalled.load(Ordering::Acquire) {
            self.1.held.fetch_add(1, Ordering::AcqRel);
            wait_while(&self.1.stalled);
        }
        got
    }

    fn set_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.0.set_timeout(timeout)
    }
}

/// A writer that cannot bound its writes: it ignores `set_timeout`.
struct ChokeWrite(Box<dyn FrameWrite>, Arc<Valve>);

impl FrameWrite for ChokeWrite {
    fn write_frame(&mut self, frame: &[u8]) -> io::Result<()> {
        if self.1.choked.load(Ordering::Acquire) {
            self.1.choking.fetch_add(1, Ordering::AcqRel);
            wait_while(&self.1.choked);
        }
        self.0.write_frame(frame)
    }

    fn shutdown(&mut self) {
        self.0.shutdown();
    }
}

struct Choked {
    server: Server,
    valve: Arc<Valve>,
}

impl Connect for Choked {
    fn connect(&self) -> io::Result<Connection> {
        let (host, device) = memory::pair();
        let device = Connection {
            reader: Box::new(StallRead(device.reader, Arc::clone(&self.valve))),
            ..device
        };
        let server = self.server.clone();
        std::thread::spawn(move || {
            let _ = server.serve(device);
        });
        Ok(Connection {
            writer: Box::new(ChokeWrite(host.writer, Arc::clone(&self.valve))),
            ..host
        })
    }

    fn describe(&self) -> String {
        "memory, choked".to_string()
    }
}

/// Pings on a thread of its own: its outcome and how long it took.
fn ping(link: &Arc<Link>) -> mpsc::Receiver<(Result<Duration, ProviderError>, Duration)> {
    let (tx, rx) = mpsc::channel();
    let link = Arc::clone(link);
    std::thread::spawn(move || {
        let started = Instant::now();
        let outcome = link.ping();
        let _ = tx.send((outcome, started.elapsed()));
    });
    rx
}

#[test]
fn a_write_the_peer_never_takes_holds_up_no_other_call() {
    let valve = Arc::new(Valve::default());
    let server = common::server(LinkOptions::new(Trust::Local));
    let source = RemoteSource::new(
        Box::new(Choked {
            server,
            valve: Arc::clone(&valve),
        }),
        LinkOptions::new(Trust::Local).timeout(Some(Duration::from_millis(300))),
    );
    let link = source.link().expect("the link is set up");
    link.ping().expect("the server answers");

    valve.stalled.store(true, Ordering::Release);
    // Sent and never answered: it times out, and cancels.
    let sent = ping(&link);
    common::eventually("the server holds the call unread", || {
        valve.held.load(Ordering::Acquire) == 1
    });
    valve.choked.store(true, Ordering::Release);
    // Holds the writer: its frame never goes.
    let holder = ping(&link);
    common::eventually("a write waits at the choke", || {
        valve.choking.load(Ordering::Acquire) == 1
    });
    // Waits for the writer.
    let waiting = ping(&link);

    let bound = Duration::from_secs(5);
    let sent = sent.recv_timeout(bound);
    let waiting = waiting.recv_timeout(bound);
    valve.choked.store(false, Ordering::Release);
    valve.stalled.store(false, Ordering::Release);
    let holder = holder.recv_timeout(bound);

    for (what, got) in [("the timed-out call", sent), ("the waiting call", waiting)] {
        let (outcome, took) = got.unwrap_or_else(|_| panic!("{what} never returned"));
        let e = outcome.expect_err(what);
        assert_eq!(
            e.status,
            Status::GUATIAO_ERR_GONE,
            "{what}: {}",
            e.message()
        );
        assert!(took < Duration::from_secs(2), "{what} took {took:?}");
    }
    assert!(
        holder.is_ok(),
        "the call holding the writer returns once it is free"
    );
    link.ping()
        .expect("the link carries on once the peer reads again");
}
