// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Each rule fails on an echo that breaks it. The echo here lives in the
//! test, with one fault planted at a time; without one it keeps every
//! rule, as the library's does.

extern crate alloc;

#[path = "rules/mod.rs"]
mod rules;

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use core::time::Duration;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

use guatiao::library::{Object, ObjectRaw, ProviderError};
use guatiao::value::types::{Bytes, Str};
use guatiao::{Map, Status};
use guatiao_channel::{Channel, ChannelSink, ChannelSinkVtable};

const QUEUE: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fault {
    None,
    /// Delivers the second message after the third.
    Reorder,
    /// Calls `receive` for every message from a thread of its own, at once.
    Overlap,
    /// Answers BUSY when the queue is full instead of waiting.
    Busy,
    /// Gives up on a full queue after a second, answering BUSY.
    SendTimesOut,
    /// Takes a send after the host closed.
    TakesAfterClose,
    /// Tells the sink 0 rather than the host's status.
    WrongStatus,
    /// Tells the sink an empty reason rather than the host's.
    DropsReason,
    /// Calls `closed` twice.
    ClosedTwice,
    /// Calls `receive` once more after `closed`.
    ReceivesAfterClosed,
    /// Dropping the channel ends nothing.
    DropKeepsGoing,
    /// Takes a send after closing the stream itself.
    TakesAfterOwnClose,
    /// Keeps calling `receive` after it answered GONE, and never closes.
    IgnoresGone,
    /// Delivers what is queued after the sink answered GONE, and only
    /// then calls `closed`.
    DrainsAfterGone,
    /// `close` calls `closed` on the host's thread, whatever the sink is
    /// in the middle of.
    ClosesOnHostThread,
    /// Dropping the channel waits for its thread, which waits on the sink.
    DropJoinsReader,
    /// `close` leaves a `send` waiting for room asleep.
    CloseLeavesSendBlocked,
}

#[derive(Default)]
struct Inner {
    items: VecDeque<Vec<u8>>,
    closed: bool,
    host_close: Option<(u32, String)>,
}

#[derive(Default)]
struct Queue {
    inner: Mutex<Inner>,
    space: Condvar,
    arrived: Condvar,
}

impl Queue {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

struct Faulty {
    fault: Fault,
    queue: Arc<Queue>,
    reader: Mutex<Option<JoinHandle<()>>>,
    raw: Option<Arc<RawSink>>,
}

fn gone() -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_GONE, "closed")
}

impl Channel for Faulty {
    fn send(&self, bytes: &[u8]) -> Result<(), ProviderError> {
        let lenient = matches!(
            self.fault,
            Fault::TakesAfterClose | Fault::TakesAfterOwnClose
        );
        let mut inner = self.queue.lock();
        loop {
            if inner.closed {
                return if lenient { Ok(()) } else { Err(gone()) };
            }
            if inner.items.len() < QUEUE {
                break;
            }
            match self.fault {
                Fault::Busy => {
                    return Err(ProviderError::new(Status::GUATIAO_ERR_BUSY, "full"));
                }
                Fault::SendTimesOut => {
                    let (woken, timeout) = self
                        .queue
                        .space
                        .wait_timeout(inner, Duration::from_secs(1))
                        .unwrap_or_else(PoisonError::into_inner);
                    inner = woken;
                    if timeout.timed_out() && inner.items.len() >= QUEUE {
                        return Err(ProviderError::new(Status::GUATIAO_ERR_BUSY, "timed out"));
                    }
                }
                _ => {
                    inner = self
                        .queue
                        .space
                        .wait(inner)
                        .unwrap_or_else(PoisonError::into_inner);
                }
            }
        }
        inner.items.push_back(bytes.to_vec());
        drop(inner);
        self.queue.arrived.notify_one();
        Ok(())
    }

    fn close(&self, status: u32, reason: &str) {
        let mut inner = self.queue.lock();
        if inner.closed {
            return;
        }
        inner.closed = true;
        inner.host_close = Some((status, reason.to_owned()));
        drop(inner);
        if self.fault != Fault::CloseLeavesSendBlocked {
            self.queue.space.notify_all();
        }
        self.queue.arrived.notify_all();
        if let (Fault::ClosesOnHostThread, Some(raw)) = (self.fault, &self.raw) {
            raw.closed(status, reason);
        }
    }
}

impl Drop for Faulty {
    fn drop(&mut self) {
        if self.fault == Fault::DropKeepsGoing {
            return;
        }
        self.close(0, "");
        if self.fault == Fault::DropJoinsReader {
            let reader = self
                .reader
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some(reader) = reader {
                let _ = reader.join();
            }
        }
    }
}

/// The sink as its table, for the faults that call it from two threads at
/// once, which `Object` rules out.
///
/// Those faults make two live `&mut` borrows of one sink, which is
/// undefined behaviour by Rust's rules however well the sink in this
/// test copes, so they are not run under Miri.
struct RawSink(ObjectRaw);

// SAFETY: the sink's kind is `Send`; the faults that share it between
// threads are the break each test plants.
unsafe impl Send for RawSink {}
// SAFETY: as above.
unsafe impl Sync for RawSink {}

impl RawSink {
    fn table(&self) -> &ChannelSinkVtable {
        // SAFETY: the table of a live `channel_sink` object.
        unsafe { &*self.0.table.cast::<ChannelSinkVtable>() }
    }
    fn receive(&self, bytes: &[u8]) -> Result<(), ProviderError> {
        let mut err = ProviderError::none();
        let receive = self.table().receive.expect("a served table has every slot");
        // SAFETY: a live object's slot, given its own ctx.
        let status = unsafe { receive(self.0.ctx, Bytes::from(bytes), &mut err) };
        if status == Status::GUATIAO_OK {
            Ok(())
        } else {
            Err(ProviderError::new(status, err.message()))
        }
    }
    fn closed(&self, status: u32, reason: &str) {
        let closed = self.table().closed.expect("a served table has every slot");
        // SAFETY: a live object's slot, given its own ctx.
        let _ = unsafe { closed(self.0.ctx, status, Str::from(reason)) };
    }
}

impl Drop for RawSink {
    fn drop(&mut self) {
        let destroy = self.table().destroy.expect("a served table has every slot");
        // SAFETY: the last owner lets the object go, once.
        unsafe { destroy(self.0.ctx) };
    }
}

/// The sink as the reader holds it.
enum Held {
    Owned(Object<dyn ChannelSink>),
    Raw(Arc<RawSink>),
}

impl Held {
    fn closed(&mut self, status: u32, reason: &str) {
        match self {
            Held::Owned(sink) => sink.closed(status, reason),
            Held::Raw(raw) => raw.closed(status, reason),
        }
    }
}

fn deliver(
    fault: Fault,
    mut held: Held,
    queue: &Queue,
    close_after: Option<u64>,
    close_status: u32,
    close_reason: String,
) {
    let mut echoed = 0u64;
    let mut held_back: Option<Vec<u8>> = None;
    let mut refused: Option<u32> = None;
    let (status, reason) = std::thread::scope(|scope| {
        let mut receive = |bytes: &[u8]| -> Result<(), ProviderError> {
            match (&mut held, fault) {
                (Held::Raw(raw), Fault::Overlap) => {
                    let (raw, bytes) = (Arc::clone(raw), bytes.to_vec());
                    scope.spawn(move || {
                        let _ = raw.receive(&bytes);
                    });
                    Ok(())
                }
                (Held::Raw(raw), _) => raw.receive(bytes),
                (Held::Owned(sink), _) => sink.receive(bytes),
            }
        };
        loop {
            let message = {
                let mut inner = queue.lock();
                loop {
                    if close_after.is_some_and(|n| echoed >= n) {
                        break None;
                    }
                    if let Some(message) = inner.items.pop_front() {
                        queue.space.notify_all();
                        break Some(message);
                    }
                    if inner.closed {
                        break None;
                    }
                    inner = queue
                        .arrived
                        .wait(inner)
                        .unwrap_or_else(PoisonError::into_inner);
                }
            };
            let Some(message) = message else {
                let mut inner = queue.lock();
                let host = inner.host_close.take();
                inner.closed = true;
                inner.items.clear();
                drop(inner);
                queue.space.notify_all();
                break match (fault, host) {
                    (Fault::WrongStatus, Some((_, reason))) => (0, reason),
                    (Fault::DropsReason, Some((status, _))) => (status, String::new()),
                    (_, Some(closing)) => closing,
                    (_, None) => (close_status, close_reason.clone()),
                };
            };
            if fault == Fault::Reorder && echoed == 1 && held_back.is_none() {
                held_back = Some(message);
                continue;
            }
            let mut answer = receive(&message);
            if let Some(late) = held_back.take() {
                answer = answer.and_then(|()| receive(&late));
                echoed += 1;
            }
            if let Err(e) = answer {
                let status: u32 = e.status.into();
                match fault {
                    Fault::IgnoresGone => {
                        refused = Some(status);
                        continue;
                    }
                    Fault::DrainsAfterGone => {
                        let rest: Vec<Vec<u8>> = queue.lock().items.drain(..).collect();
                        for message in rest {
                            let _ = receive(&message);
                        }
                    }
                    _ => {}
                }
                let mut inner = queue.lock();
                inner.closed = true;
                inner.items.clear();
                drop(inner);
                queue.space.notify_all();
                break (status, String::new());
            }
            echoed += 1;
        }
    });
    if fault == Fault::IgnoresGone && refused.is_some() {
        // Ended by the host, after ignoring the sink: no `closed` at all.
        return;
    }
    if fault == Fault::ClosesOnHostThread {
        // `closed` was the host's thread's doing.
        return;
    }
    held.closed(status, &reason);
    match fault {
        Fault::ClosedTwice => held.closed(status, &reason),
        Fault::ReceivesAfterClosed => match &mut held {
            Held::Owned(sink) => {
                let _ = sink.receive(b"late");
            }
            Held::Raw(raw) => {
                let _ = raw.receive(b"late");
            }
        },
        _ => {}
    }
}

fn start(
    fault: Fault,
    config: &Map,
    sink: Object<dyn ChannelSink>,
) -> Result<Object<dyn Channel>, ProviderError> {
    let number = |key: &str| config.get(key).and_then(|v| u64::try_from(v).ok());
    let close_after = number("close_after");
    let close_status = number("close_status").map_or(0, |n| u32::try_from(n).unwrap_or(0));
    let close_reason = config
        .get("close_reason")
        .and_then(|v| <&str>::try_from(v).ok())
        .unwrap_or_default()
        .to_owned();
    let queue = Arc::new(Queue::default());
    let (held, raw) = if matches!(fault, Fault::Overlap | Fault::ClosesOnHostThread) {
        let raw = Arc::new(RawSink(sink.into_raw()));
        (Held::Raw(Arc::clone(&raw)), Some(raw))
    } else {
        (Held::Owned(sink), None)
    };
    let shared = Arc::clone(&queue);
    let reader = std::thread::spawn(move || {
        deliver(
            fault,
            held,
            &shared,
            close_after,
            close_status,
            close_reason,
        );
    });
    Ok(Faulty {
        fault,
        queue,
        reader: Mutex::new(Some(reader)),
        raw,
    }
    .into_object())
}

/// The rules a fault must break, and the ones it leaves standing.
fn broken_by(fault: Fault) -> Vec<&'static str> {
    // Patient enough for the sink's holds, short of a fault that never
    // answers.
    let patience = Duration::from_secs(8);
    rules::RULES
        .iter()
        .filter(|(_, rule)| rule(&|c: &Map, s| start(fault, c, s), patience).is_err())
        .map(|(name, _)| *name)
        .collect()
}

fn each_breaks(faults: &[(Fault, &str)]) {
    std::thread::scope(|scope| {
        let runs: Vec<_> = faults
            .iter()
            .map(|(fault, rule)| scope.spawn(move || (*fault, *rule, broken_by(*fault))))
            .collect();
        for run in runs {
            let (fault, rule, broken) = run.join().expect("a fault's run");
            assert!(
                broken.contains(&rule),
                "{fault:?} should break {rule}; it broke {broken:?}"
            );
        }
    });
}

#[test]
fn the_echo_without_a_fault_keeps_every_rule() {
    rules::hold_every_rule("the test's echo", &|c: &Map, s| start(Fault::None, c, s));
}

#[test]
fn each_planted_fault_breaks_its_rule() {
    each_breaks(&[
        (Fault::Reorder, "order_is_kept"),
        (Fault::Busy, "back_pressure_waits"),
        (Fault::SendTimesOut, "back_pressure_waits"),
        (Fault::TakesAfterClose, "host_close"),
        (Fault::WrongStatus, "host_close"),
        (Fault::DropsReason, "host_close"),
        (Fault::ClosedTwice, "host_close"),
        (Fault::ReceivesAfterClosed, "host_close"),
        (Fault::DropKeepsGoing, "dropping_closes"),
        (Fault::TakesAfterOwnClose, "provider_close"),
        (Fault::IgnoresGone, "gone_sink_ends_the_stream"),
        (Fault::DrainsAfterGone, "gone_sink_leaves_the_queue_unsent"),
        (Fault::DropJoinsReader, "close_does_not_wait_for_receive"),
        (Fault::CloseLeavesSendBlocked, "close_wakes_a_waiting_send"),
    ]);
}

/// These two call the sink from two threads at once through its table.
#[test]
#[cfg_attr(miri, ignore = "two live `&mut` borrows of one sink, on purpose")]
fn each_planted_concurrent_fault_breaks_its_rule() {
    each_breaks(&[
        (Fault::Overlap, "order_is_kept"),
        (Fault::ClosesOnHostThread, "close_does_not_wait_for_receive"),
    ]);
}
