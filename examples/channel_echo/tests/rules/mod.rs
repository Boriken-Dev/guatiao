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

#![allow(
    dead_code,
    reason = "each test including this file uses only part of it"
)]

extern crate alloc;

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use core::time::Duration;
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Instant;

use guatiao::library::{Object, ProviderError};
use guatiao::{Map, Status};
use guatiao_channel::{Channel, ChannelSink};

/// What starts a channel: the echo kind's `start`, however it is reached.
pub type Start<'a> =
    dyn Fn(&Map, Object<dyn ChannelSink>) -> Result<Object<dyn Channel>, ProviderError> + 'a;

/// How long an awaited call may take, in milliseconds: generous, for an
/// engine run under emulation or a peer across a link.
static WAIT_MS: AtomicU64 = AtomicU64::new(30_000);

/// Shortens how long a wait lasts, for a test expecting one to run out.
pub fn wait_at_most(limit: Duration) {
    WAIT_MS.store(
        u64::try_from(limit.as_millis()).unwrap_or(u64::MAX),
        Ordering::Relaxed,
    );
}
/// How long to watch for a call that must not come.
const GRACE: Duration = Duration::from_millis(300);
/// How long a held sink keeps the provider waiting.
const HOLD: Duration = Duration::from_millis(500);

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
    in_receive: AtomicBool,
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
}

struct Sink {
    shared: Arc<Shared>,
}

impl ChannelSink for Sink {
    fn receive(&mut self, bytes: &[u8]) -> Result<(), ProviderError> {
        let overlapped = self.shared.in_receive.swap(true, Ordering::AcqRel);
        let mut log = self.shared.lock();
        if overlapped {
            log.broken
                .push("receive was called while another receive was running".into());
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
        self.shared.in_receive.store(false, Ordering::Release);
        self.shared.changed.notify_all();
        answer
    }

    fn closed(&mut self, status: u32) {
        let mut log = self.shared.lock();
        if log.events.iter().any(|e| matches!(e, Event::Closed(_))) {
            log.broken
                .push(format!("closed({status}) came a second time"));
        }
        log.events.push(Event::Closed(status));
        drop(log);
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
    /// A probe, and the sink to hand over.
    pub fn new() -> (Probe, Object<dyn ChannelSink>) {
        let shared = Arc::new(Shared {
            log: Mutex::new(Log::default()),
            changed: Condvar::new(),
            in_receive: AtomicBool::new(false),
        });
        let sink = Sink {
            shared: Arc::clone(&shared),
        }
        .into_object();
        (Probe { shared }, sink)
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
        let deadline = Instant::now() + Duration::from_millis(WAIT_MS.load(Ordering::Relaxed));
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
    /// more may come, with every rule it saw broken.
    pub fn settle(&self) -> Result<Vec<Event>, String> {
        std::thread::sleep(GRACE);
        let log = self.shared.lock();
        if let Some(broken) = log.broken.first() {
            return Err(broken.clone());
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

fn started(start: &Start, config: &Map) -> Result<(Probe, Object<dyn Channel>), String> {
    let (probe, sink) = Probe::new();
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

/// Messages come back in the order they were sent, each sender's in its
/// own order when two threads send at once, through one `receive` at a
/// time.
pub fn order_is_kept(start: &Start) -> Result<(), String> {
    let (probe, channel) = started(start, &Map::new())?;
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

/// A sink that takes its time makes the provider wait: nothing is
/// refused, dropped or closed meanwhile, and everything arrives once it
/// takes again.
pub fn back_pressure_waits(start: &Start) -> Result<(), String> {
    let (probe, channel) = started(start, &Map::new())?;
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
pub fn host_close(start: &Start) -> Result<(), String> {
    let (probe, channel) = started(start, &Map::new())?;
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
pub fn dropping_closes(start: &Start) -> Result<(), String> {
    let (probe, channel) = started(start, &Map::new())?;
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
pub fn provider_close(start: &Start) -> Result<(), String> {
    let (probe, channel) = started(start, &config(&[("close_after", 2), ("close_status", 7)]))?;
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
pub fn gone_sink_ends_the_stream(start: &Start) -> Result<(), String> {
    let (probe, channel) = started(start, &Map::new())?;
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

/// Every rule, by name.
pub type Rule = fn(&Start) -> Result<(), String>;

pub const RULES: [(&str, Rule); 6] = [
    ("order_is_kept", order_is_kept),
    ("back_pressure_waits", back_pressure_waits),
    ("host_close", host_close),
    ("dropping_closes", dropping_closes),
    ("provider_close", provider_close),
    ("gone_sink_ends_the_stream", gone_sink_ends_the_stream),
];

/// Runs every rule against `start`, and panics naming each one broken.
pub fn hold_every_rule(who: &str, start: &Start) {
    let broken: Vec<String> = RULES
        .iter()
        .filter_map(|(name, rule)| rule(start).err().map(|why| format!("  {name}: {why}")))
        .collect();
    assert!(
        broken.is_empty(),
        "{who} broke the channel rules:\n{}",
        broken.join("\n")
    );
}
