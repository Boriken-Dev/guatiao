// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The channel rules, checked from the host's side against any echo:
//! a function that starts a channel for a configuration and a sink.
//!
//! Each rule is one function answering what it saw broken, so a test can
//! show a rule failing on a provider that breaks it as well as passing on
//! one that keeps it. Included with `#[path]` by every test that drives an
//! echo, in this workspace or another.
//!
//! A host of a channel never waits inside `receive` for its own `send` to
//! return: a provider whose output depends on its input, an echo, would
//! deadlock once its queue is full. Every sink here takes a message
//! without sending one.

#![allow(
    dead_code,
    reason = "each test including this file uses only part of it"
)]

extern crate alloc;

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::time::Duration;
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Instant;

use guatiao::library::{Object, ProviderError};
use guatiao::{Map, Status};
use guatiao_channel::{Channel, ChannelSink};

/// What starts a channel: the echo kind's `start`, however it is reached.
pub type Start<'a> =
    dyn Fn(&Map, Object<dyn ChannelSink>) -> Result<Object<dyn Channel>, ProviderError> + 'a;

/// How long an awaited call may take: generous, for an engine run under
/// emulation or a peer across a link.
pub const PATIENCE: Duration = Duration::from_secs(30);
/// How long to watch for a call that must not come, or for one that must
/// come promptly.
pub const GRACE: Duration = Duration::from_millis(300);
/// How long a held sink keeps the provider waiting: longer than any
/// timeout a provider might keep for itself.
pub const HOLD: Duration = Duration::from_secs(3);

const GONE: u32 = 12;

/// One call the sink saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Received(Vec<u8>),
    Closed(u32),
}

#[derive(Default)]
struct Log {
    events: Vec<Event>,
    broken: Vec<String>,
    /// `receive` waits while this is set.
    held: bool,
    /// `receive` answers GONE from now on.
    away: bool,
    /// `receive` has answered GONE once.
    answered_gone: bool,
    destroyed: bool,
}

struct Shared {
    log: Mutex<Log>,
    changed: Condvar,
    /// Some call on the sink is running: the next must wait for it.
    in_call: AtomicBool,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Log> {
        self.log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// The host's side of one channel: the sink handed to the provider, seen
/// from the test.
pub struct Probe {
    shared: Arc<Shared>,
    patience: Duration,
}

struct Sink {
    shared: Arc<Shared>,
}

impl ChannelSink for Sink {
    fn receive(&mut self, bytes: &[u8]) -> Result<(), ProviderError> {
        let overlapped = self.shared.in_call.swap(true, Ordering::AcqRel);
        let mut log = self.shared.lock();
        if overlapped {
            log.broken
                .push("receive was called while another call on the sink was running".into());
        }
        if log.events.iter().any(|e| matches!(e, Event::Closed(_))) {
            log.broken.push(format!(
                "receive({:?}) came after closed",
                String::from_utf8_lossy(bytes)
            ));
        }
        if log.answered_gone {
            log.broken.push(format!(
                "receive({:?}) came after the sink answered GONE",
                String::from_utf8_lossy(bytes)
            ));
        }
        while log.held {
            log = self
                .shared
                .changed
                .wait(log)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        let answer = if log.away {
            log.answered_gone = true;
            Err(ProviderError::new(
                Status::GUATIAO_ERR_GONE,
                "the host takes nothing more",
            ))
        } else {
            log.events.push(Event::Received(bytes.to_vec()));
            Ok(())
        };
        drop(log);
        // Long enough for a second caller running alongside to be seen.
        std::thread::sleep(Duration::from_micros(200));
        self.shared.in_call.store(false, Ordering::Release);
        self.shared.changed.notify_all();
        answer
    }

    fn closed(&mut self, status: u32) {
        let overlapped = self.shared.in_call.swap(true, Ordering::AcqRel);
        let mut log = self.shared.lock();
        if overlapped {
            log.broken
                .push("closed was called while another call on the sink was running".into());
        }
        if log.events.iter().any(|e| matches!(e, Event::Closed(_))) {
            log.broken
                .push(format!("closed({status}) came a second time"));
        }
        log.events.push(Event::Closed(status));
        drop(log);
        std::thread::sleep(Duration::from_micros(200));
        self.shared.in_call.store(false, Ordering::Release);
        self.shared.changed.notify_all();
    }
}

impl Drop for Sink {
    fn drop(&mut self) {
        self.shared.lock().destroyed = true;
        self.shared.changed.notify_all();
    }
}

impl Probe {
    /// A probe whose waits give up after `patience`, and the sink to hand
    /// over.
    pub fn new(patience: Duration) -> (Probe, Object<dyn ChannelSink>) {
        let shared = Arc::new(Shared {
            log: Mutex::new(Log::default()),
            changed: Condvar::new(),
            in_call: AtomicBool::new(false),
        });
        let sink = Sink {
            shared: Arc::clone(&shared),
        }
        .into_object();
        (Probe { shared, patience }, sink)
    }

    /// `receive` waits from now until [`release`](Self::release).
    pub fn hold(&self) {
        self.shared.lock().held = true;
    }

    pub fn release(&self) {
        self.shared.lock().held = false;
        self.shared.changed.notify_all();
    }

    /// The host has gone: `receive` answers GONE from now on.
    pub fn go_away(&self) {
        self.shared.lock().away = true;
    }

    /// Waits until `done` holds of the log, or says what it saw instead.
    fn wait_for(&self, what: &str, done: impl Fn(&Log) -> bool) -> Result<(), String> {
        let deadline = Instant::now() + self.patience;
        let mut log = self.shared.lock();
        while !done(&log) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(format!(
                    "timed out waiting for {what}; the sink saw {}",
                    describe(&log.events)
                ));
            }
            log = self
                .shared
                .changed
                .wait_timeout(log, left)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        Ok(())
    }

    /// The messages received once there are `n` of them.
    pub fn received(&self, n: usize) -> Result<Vec<Vec<u8>>, String> {
        self.wait_for(&format!("{n} message(s)"), |log| {
            messages(&log.events).len() >= n || closed(&log.events).is_some()
        })?;
        let got = messages(&self.shared.lock().events);
        if got.len() < n {
            return Err(format!(
                "the stream closed after {} of {n} message(s): {}",
                got.len(),
                describe(&self.shared.lock().events)
            ));
        }
        Ok(got)
    }

    /// The status `closed` was called with.
    pub fn closed(&self) -> Result<u32, String> {
        self.wait_for("closed", |log| closed(&log.events).is_some())?;
        Ok(closed(&self.shared.lock().events).expect("waited for"))
    }

    /// Everything the sink saw, after a grace period in which nothing
    /// more may come, with every rule it saw broken, and the sink let go
    /// once `closed` has been called.
    pub fn settle(&self) -> Result<Vec<Event>, String> {
        std::thread::sleep(GRACE);
        let log = self.shared.lock();
        if let Some(broken) = log.broken.first() {
            return Err(broken.clone());
        }
        if closed(&log.events).is_some() && !log.destroyed {
            return Err(format!(
                "the sink was not destroyed within {GRACE:?} of closed"
            ));
        }
        Ok(log.events.clone())
    }

    /// Whether the provider has let the sink go.
    pub fn destroyed(&self) -> bool {
        self.shared.lock().destroyed
    }
}

fn messages(events: &[Event]) -> Vec<Vec<u8>> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Received(b) => Some(b.clone()),
            Event::Closed(_) => None,
        })
        .collect()
}

fn closed(events: &[Event]) -> Option<u32> {
    events.iter().find_map(|e| match e {
        Event::Closed(s) => Some(*s),
        Event::Received(_) => None,
    })
}

fn describe(events: &[Event]) -> String {
    let shown: Vec<String> = events
        .iter()
        .map(|e| match e {
            Event::Received(b) => format!("{:?}", String::from_utf8_lossy(b)),
            Event::Closed(s) => format!("closed({s})"),
        })
        .collect();
    format!("[{}]", shown.join(", "))
}

fn texts(messages: &[Vec<u8>]) -> Vec<String> {
    messages
        .iter()
        .map(|b| String::from_utf8_lossy(b).into_owned())
        .collect()
}

fn config(pairs: &[(&str, u64)]) -> Map {
    let mut map = Map::new();
    for (key, value) in pairs {
        map.set(key, *value);
    }
    map
}

fn started(
    start: &Start,
    patience: Duration,
    config: &Map,
) -> Result<(Probe, Object<dyn Channel>), String> {
    let (probe, sink) = Probe::new(patience);
    let channel = start(config, sink).map_err(|e| format!("start was refused: {e}"))?;
    Ok((probe, channel))
}

fn send(channel: &dyn Channel, text: &str) -> Result<(), String> {
    channel
        .send(text.as_bytes())
        .map_err(|e| format!("send({text:?}) was refused: {e}"))
}

/// A send after the stream ended answers GONE.
fn refused_gone(channel: &dyn Channel, text: &str) -> Result<(), String> {
    match channel.send(text.as_bytes()) {
        Ok(()) => Err(format!("send({text:?}) after the end was taken")),
        Err(e) if u32::from(e.status) == GONE => Ok(()),
        Err(e) => Err(format!(
            "send({text:?}) after the end answered {e}, not GUATIAO_ERR_GONE"
        )),
    }
}

/// Polls `done` until it holds or `limit` passes.
fn within(limit: Duration, done: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    while !done() {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    true
}

/// Messages come back in the order they were sent, each sender's in its
/// own order when two threads send at once, through one `receive` at a
/// time.
pub fn order_is_kept(start: &Start, patience: Duration) -> Result<(), String> {
    let (probe, channel) = started(start, patience, &Map::new())?;
    let expected: Vec<String> = (0..20).map(|i| format!("m{i}")).collect();
    for text in &expected {
        send(&*channel, text)?;
    }
    let got = texts(&probe.received(20)?);
    if got != expected {
        return Err(format!("sent {expected:?}, received {got:?}"));
    }

    let channel: &dyn Channel = &*channel;
    std::thread::scope(|s| -> Result<(), String> {
        let senders: Vec<_> = ["a", "b"]
            .into_iter()
            .map(|who| {
                s.spawn(move || -> Result<(), String> {
                    for i in 0..30 {
                        send(channel, &format!("{who}{i}"))?;
                    }
                    Ok(())
                })
            })
            .collect();
        for sender in senders {
            sender
                .join()
                .map_err(|_| "a sender panicked".to_string())??;
        }
        Ok(())
    })?;
    let got = texts(&probe.received(80)?);
    for who in ["a", "b"] {
        let theirs: Vec<&String> = got[20..].iter().filter(|t| t.starts_with(who)).collect();
        let sent: Vec<String> = (0..30).map(|i| format!("{who}{i}")).collect();
        if theirs
            .iter()
            .map(|t| t.as_str())
            .ne(sent.iter().map(String::as_str))
        {
            return Err(format!("{who}'s messages arrived as {theirs:?}"));
        }
    }
    channel.close(0);
    probe.closed()?;
    probe.settle().map(drop)
}

/// A sink that takes its time makes the provider wait for several
/// seconds: nothing is refused, dropped or closed meanwhile, and
/// everything arrives once it takes again.
pub fn back_pressure_waits(start: &Start, patience: Duration) -> Result<(), String> {
    let (probe, channel) = started(start, patience, &Map::new())?;
    probe.hold();
    let expected: Vec<String> = (0..24).map(|i| format!("p{i}")).collect();
    let channel: &dyn Channel = &*channel;
    std::thread::scope(|s| -> Result<(), String> {
        let sender = s.spawn(|| -> Result<(), String> {
            for text in &expected {
                send(channel, text)?;
            }
            Ok(())
        });
        // A provider may take every message while the sink holds, or
        // make the sender wait: either way none is refused.
        std::thread::sleep(HOLD);
        let closed_meanwhile = closed(&probe.shared.lock().events);
        probe.release();
        if let Some(status) = closed_meanwhile {
            return Err(format!(
                "the provider closed ({status}) while the sink was taking its time"
            ));
        }
        sender
            .join()
            .map_err(|_| "the sender panicked".to_string())?
    })?;
    let got = texts(&probe.received(expected.len())?);
    if got != expected {
        return Err(format!("sent {expected:?}, received {got:?}"));
    }
    channel.close(0);
    probe.closed()?;
    probe.settle().map(drop)
}

/// The host's `close`: `send` answers GONE, and `closed` comes once,
/// with the host's status, after which the sink hears nothing more.
pub fn host_close(start: &Start, patience: Duration) -> Result<(), String> {
    let (probe, channel) = started(start, patience, &Map::new())?;
    for text in ["1", "2", "3"] {
        send(&*channel, text)?;
    }
    channel.close(5);
    refused_gone(&*channel, "4")?;
    let status = probe.closed()?;
    if status != 5 {
        return Err(format!("closed({status}), not the host's closed(5)"));
    }
    drop(channel);
    let events = probe.settle()?;
    let got = texts(&messages(&events));
    let got: Vec<&str> = got.iter().map(String::as_str).collect();
    if !["1", "2", "3"].starts_with(&got) {
        return Err(format!("received {got:?}, not a prefix of [1, 2, 3]"));
    }
    Ok(())
}

/// Dropping the channel is `close(OK)`.
pub fn dropping_closes(start: &Start, patience: Duration) -> Result<(), String> {
    let (probe, channel) = started(start, patience, &Map::new())?;
    send(&*channel, "1")?;
    probe.received(1)?;
    drop(channel);
    let status = probe.closed()?;
    if status != 0 {
        return Err(format!(
            "dropping the channel closed it with {status}, not 0"
        ));
    }
    probe.settle().map(drop)
}

/// The provider's own close: `closed` once, with its status, and `send`
/// answers GONE from then on.
pub fn provider_close(start: &Start, patience: Duration) -> Result<(), String> {
    let (probe, channel) = started(
        start,
        patience,
        &config(&[("close_after", 2), ("close_status", 7)]),
    )?;
    send(&*channel, "1")?;
    send(&*channel, "2")?;
    let status = probe.closed()?;
    if status != 7 {
        return Err(format!("closed({status}), not the provider's closed(7)"));
    }
    refused_gone(&*channel, "3")?;
    drop(channel);
    let events = probe.settle()?;
    let got = texts(&messages(&events));
    if got != ["1", "2"] {
        return Err(format!("received {got:?}, not [1, 2]"));
    }
    Ok(())
}

/// A sink answering GONE ends the stream: `closed` with GONE, no
/// `receive` after, and `send` answers GONE.
pub fn gone_sink_ends_the_stream(start: &Start, patience: Duration) -> Result<(), String> {
    let (probe, channel) = started(start, patience, &Map::new())?;
    send(&*channel, "1")?;
    probe.received(1)?;
    probe.go_away();
    // Taken by the provider, then refused by the sink.
    send(&*channel, "2")?;
    let status = probe.closed()?;
    if status != GONE {
        return Err(format!(
            "closed({status}) after the sink answered GONE, not closed(GONE)"
        ));
    }
    refused_gone(&*channel, "3")?;
    drop(channel);
    probe.settle().map(drop)
}

/// Messages still queued when the sink answers GONE are not delivered:
/// the provider calls `closed` and nothing else.
pub fn gone_sink_leaves_the_queue_unsent(start: &Start, patience: Duration) -> Result<(), String> {
    let (probe, channel) = started(start, patience, &Map::new())?;
    probe.hold();
    for text in ["1", "2", "3"] {
        send(&*channel, text)?;
    }
    probe.go_away();
    std::thread::sleep(GRACE);
    probe.release();
    let status = probe.closed()?;
    if status != GONE {
        return Err(format!("closed({status}), not closed(GONE)"));
    }
    drop(channel);
    // Any receive after the GONE answer is a break the sink noted.
    probe.settle().map(drop)
}

/// `close` and dropping the channel return at once, though the sink is
/// in the middle of a `receive` that has not returned.
pub fn close_does_not_wait_for_receive(start: &Start, patience: Duration) -> Result<(), String> {
    let (probe, channel) = started(start, patience, &Map::new())?;
    probe.hold();
    send(&*channel, "x")?;
    // Long enough for the provider to be inside `receive`.
    std::thread::sleep(GRACE);
    let done = Arc::new(AtomicBool::new(false));
    let ended = Arc::clone(&done);
    let host = std::thread::spawn(move || {
        channel.close(3);
        drop(channel);
        ended.store(true, Ordering::Release);
    });
    let returned = within(GRACE * 3, || done.load(Ordering::Acquire));
    probe.release();
    host.join()
        .map_err(|_| "the closing thread panicked".to_string())?;
    if !returned {
        return Err(format!(
            "close and drop were still waiting {:?} after the sink was left inside receive",
            GRACE * 3
        ));
    }
    let status = probe.closed()?;
    if status != 3 {
        return Err(format!("closed({status}), not the host's closed(3)"));
    }
    probe.settle().map(drop)
}

/// `close` wakes a `send` waiting for room, which answers GONE; no `send`
/// is taken once `close` has returned.
pub fn close_wakes_a_waiting_send(start: &Start, patience: Duration) -> Result<(), String> {
    let (probe, channel) = started(start, patience, &Map::new())?;
    probe.hold();
    let after_close = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let failure: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let progress = Arc::new(AtomicUsize::new(0));
    let channel: Arc<Object<dyn Channel>> = Arc::new(channel);
    let sender = {
        let (channel, after_close, finished, failure, progress) = (
            Arc::clone(&channel),
            Arc::clone(&after_close),
            Arc::clone(&finished),
            Arc::clone(&failure),
            Arc::clone(&progress),
        );
        std::thread::spawn(move || {
            for i in 0..200 {
                // Read before the call: a send that began once `close` had
                // returned must not be taken.
                let began_after_close = after_close.load(Ordering::Acquire);
                match channel.send(format!("w{i}").as_bytes()) {
                    Ok(()) if began_after_close => {
                        *failure.lock().expect("unpoisoned") =
                            Some(format!("send(w{i}) was taken after close returned"));
                        break;
                    }
                    Ok(()) => {
                        progress.fetch_add(1, Ordering::AcqRel);
                    }
                    Err(e) if u32::from(e.status) == GONE => break,
                    Err(e) => {
                        *failure.lock().expect("unpoisoned") =
                            Some(format!("send(w{i}) answered {e}, not GUATIAO_ERR_GONE"));
                        break;
                    }
                }
            }
            finished.store(true, Ordering::Release);
        })
    };
    // Long enough for the sender to be waiting, if the provider makes it.
    std::thread::sleep(GRACE / 2);
    let before = progress.load(Ordering::Acquire);
    std::thread::sleep(GRACE / 2);
    // No send finished in half a grace period: one is waiting for room.
    let waiting = progress.load(Ordering::Acquire) == before && !finished.load(Ordering::Acquire);
    channel.close(4);
    after_close.store(true, Ordering::Release);
    let woke = within(GRACE * 3, || finished.load(Ordering::Acquire));
    probe.release();
    sender
        .join()
        .map_err(|_| "the sender panicked".to_string())?;
    if let Some(why) = failure.lock().expect("unpoisoned").take() {
        return Err(why);
    }
    if waiting && progress.load(Ordering::Acquire) != before {
        return Err("a send that was waiting for room was taken once close woke it".into());
    }
    if !woke {
        return Err(format!(
            "a send was still waiting {:?} after close returned",
            GRACE * 3
        ));
    }
    let status = probe.closed()?;
    if status != 4 {
        return Err(format!("closed({status}), not the host's closed(4)"));
    }
    drop(channel);
    probe.settle().map(drop)
}

pub type Rule = fn(&Start, Duration) -> Result<(), String>;

/// Every rule, by name.
pub const RULES: [(&str, Rule); 9] = [
    ("order_is_kept", order_is_kept),
    ("back_pressure_waits", back_pressure_waits),
    ("host_close", host_close),
    ("dropping_closes", dropping_closes),
    ("provider_close", provider_close),
    ("gone_sink_ends_the_stream", gone_sink_ends_the_stream),
    (
        "gone_sink_leaves_the_queue_unsent",
        gone_sink_leaves_the_queue_unsent,
    ),
    (
        "close_does_not_wait_for_receive",
        close_does_not_wait_for_receive,
    ),
    ("close_wakes_a_waiting_send", close_wakes_a_waiting_send),
];

/// Runs every rule against `start`, and panics naming each one broken.
pub fn hold_every_rule(who: &str, start: &Start) {
    let broken: Vec<String> = RULES
        .iter()
        .filter_map(|(name, rule)| {
            rule(start, PATIENCE)
                .err()
                .map(|why| format!("  {name}: {why}"))
        })
        .collect();
    assert!(
        broken.is_empty(),
        "{who} broke the channel rules:\n{}",
        broken.join("\n")
    );
}

/// The rules that need a provider thread of its own, which a language
/// whose objects are called on one interpreter thread cannot give: its
/// echo answers inside the `send` that took the message, so a sink left
/// inside `receive` holds the only thread that could take a `close`.
pub const NEEDS_A_THREAD: [&str; 3] = [
    "gone_sink_leaves_the_queue_unsent",
    "close_does_not_wait_for_receive",
    "close_wakes_a_waiting_send",
];

/// Runs every rule an echo with no thread of its own can keep.
pub fn hold_every_inline_rule(who: &str, start: &Start) {
    let broken: Vec<String> = RULES
        .iter()
        .filter(|(name, _)| !NEEDS_A_THREAD.contains(name))
        .filter_map(|(name, rule)| {
            rule(start, PATIENCE)
                .err()
                .map(|why| format!("  {name}: {why}"))
        })
        .collect();
    assert!(
        broken.is_empty(),
        "{who} broke the channel rules:\n{}",
        broken.join("\n")
    );
}
