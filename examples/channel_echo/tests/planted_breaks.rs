// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Each rule fails on an echo that breaks it. The echo here lives in the
//! test, with one fault planted at a time; without one it keeps every
//! rule, as the library's does.

extern crate alloc;

#[path = "rules/mod.rs"]
mod rules;

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use core::time::Duration;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};

use guatiao::library::{Object, ObjectRaw, ProviderError};
use guatiao::value::types::Bytes;
use guatiao::{Map, Status};
use guatiao_channel::{Channel, ChannelSink, ChannelSinkVtable};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fault {
    None,
    /// Delivers the second message after the third.
    Reorder,
    /// Calls `receive` for every message from a thread of its own, at once.
    Overlap,
    /// Answers BUSY when the queue is full instead of waiting.
    Busy,
    /// Takes a send after the host closed.
    TakesAfterClose,
    /// Tells the sink 0 rather than the host's status.
    WrongStatus,
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
}

struct State {
    closed: AtomicBool,
    host_status: AtomicU32,
}

struct Faulty {
    fault: Fault,
    queue: Mutex<Option<SyncSender<Vec<u8>>>>,
    state: Arc<State>,
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
        if self.state.closed.load(Ordering::Acquire) {
            return if lenient { Ok(()) } else { Err(gone()) };
        }
        let queue = self.queue.lock().expect("unpoisoned").clone();
        let Some(queue) = queue else {
            return if lenient { Ok(()) } else { Err(gone()) };
        };
        if self.fault == Fault::Busy {
            return queue.try_send(bytes.to_vec()).map_err(|e| match e {
                TrySendError::Full(_) => ProviderError::new(Status::GUATIAO_ERR_BUSY, "full"),
                TrySendError::Disconnected(_) => gone(),
            });
        }
        match queue.send(bytes.to_vec()) {
            Ok(()) => Ok(()),
            Err(_) if lenient => Ok(()),
            Err(_) => Err(gone()),
        }
    }

    fn close(&self, status: u32) {
        if self.state.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        self.state.host_status.store(status, Ordering::Release);
        let sender = self.queue.lock().expect("unpoisoned").take();
        if self.fault == Fault::DropKeepsGoing {
            core::mem::forget(sender);
        }
    }
}

impl Drop for Faulty {
    fn drop(&mut self) {
        if self.fault == Fault::DropKeepsGoing && !self.state.closed.load(Ordering::Acquire) {
            core::mem::forget(self.queue.lock().expect("unpoisoned").take());
            return;
        }
        self.close(0);
    }
}

/// The sink as its table, for the one fault that calls it from two
/// threads at once, which `Object` rules out.
struct RawSink(ObjectRaw);

// SAFETY: the sink's kind is `Send`; only the overlap fault calls it from
// two threads at once, which is the break this test plants.
unsafe impl Send for RawSink {}
// SAFETY: as above.
unsafe impl Sync for RawSink {}

impl RawSink {
    fn table(&self) -> &ChannelSinkVtable {
        // SAFETY: the table of a live `channel_sink` object.
        unsafe { &*self.0.table.cast::<ChannelSinkVtable>() }
    }
    fn receive(&self, bytes: &[u8]) {
        // SAFETY: a live object's slot, given its own ctx.
        let mut err = ProviderError::none();
        let receive = self.table().receive.expect("a served table has every slot");
        let _ = unsafe { receive(self.0.ctx, Bytes::from(bytes), &mut err) };
    }
    fn finish(self, status: u32) {
        // SAFETY: as above; `destroy` last, once.
        unsafe {
            let table = self.table();
            let _ = table.closed.expect("a served table has every slot")(self.0.ctx, status);
            table.destroy.expect("a served table has every slot")(self.0.ctx);
        }
    }
}

fn deliver(
    fault: Fault,
    mut sink: Object<dyn ChannelSink>,
    taken: Receiver<Vec<u8>>,
    state: &State,
    close_after: Option<u64>,
    close_status: u32,
) {
    if fault == Fault::Overlap {
        let raw = Arc::new(RawSink(sink.into_raw()));
        std::thread::scope(|s| {
            for message in &taken {
                let raw = Arc::clone(&raw);
                s.spawn(move || raw.receive(&message));
            }
        });
        let raw = Arc::into_inner(raw).expect("every caller has finished");
        raw.finish(state.host_status.load(Ordering::Acquire));
        return;
    }
    let mut echoed = 0u64;
    let mut held_back: Option<Vec<u8>> = None;
    let mut answered_error = false;
    let status = loop {
        if close_after.is_some_and(|n| echoed >= n) {
            state.closed.store(true, Ordering::Release);
            break close_status;
        }
        let Ok(message) = taken.recv() else {
            break if fault == Fault::WrongStatus {
                0
            } else {
                state.host_status.load(Ordering::Acquire)
            };
        };
        if fault == Fault::Reorder && echoed == 1 && held_back.is_none() {
            held_back = Some(message);
            continue;
        }
        let mut answer = sink.receive(&message);
        if let Some(late) = held_back.take() {
            answer = answer.and_then(|()| sink.receive(&late));
            echoed += 1;
        }
        if let Err(e) = answer {
            if fault == Fault::IgnoresGone {
                answered_error = true;
                continue;
            }
            state.closed.store(true, Ordering::Release);
            break e.status.into();
        }
        echoed += 1;
    };
    if fault == Fault::IgnoresGone && answered_error {
        // Ended by the host, after ignoring the sink: no `closed` at all.
        return;
    }
    drop(taken);
    sink.closed(status);
    match fault {
        Fault::ClosedTwice => sink.closed(status),
        Fault::ReceivesAfterClosed => {
            let _ = sink.receive(b"late");
        }
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
    let (queue, taken) = sync_channel(4);
    let state = Arc::new(State {
        closed: AtomicBool::new(false),
        host_status: AtomicU32::new(0),
    });
    let shared = Arc::clone(&state);
    std::thread::spawn(move || deliver(fault, sink, taken, &shared, close_after, close_status));
    Ok(Faulty {
        fault,
        queue: Mutex::new(Some(queue)),
        state,
    }
    .into_object())
}

/// The rules a fault must break, and the ones it leaves standing.
fn broken_by(fault: Fault) -> Vec<&'static str> {
    rules::RULES
        .iter()
        .filter(|(_, rule)| rule(&|c: &Map, s| start(fault, c, s)).is_err())
        .map(|(name, _)| *name)
        .collect()
}

#[test]
fn the_echo_without_a_fault_keeps_every_rule() {
    rules::hold_every_rule("the test's echo", &|c: &Map, s| start(Fault::None, c, s));
}

#[test]
fn each_planted_fault_breaks_its_rule() {
    // A fault that never closes would otherwise wait the full patience.
    rules::wait_at_most(Duration::from_secs(3));
    for (fault, rule) in [
        (Fault::Reorder, "order_is_kept"),
        (Fault::Overlap, "order_is_kept"),
        (Fault::Busy, "back_pressure_waits"),
        (Fault::TakesAfterClose, "host_close"),
        (Fault::WrongStatus, "host_close"),
        (Fault::ClosedTwice, "host_close"),
        (Fault::ReceivesAfterClosed, "host_close"),
        (Fault::DropKeepsGoing, "dropping_closes"),
        (Fault::TakesAfterOwnClose, "provider_close"),
        (Fault::IgnoresGone, "gone_sink_ends_the_stream"),
    ] {
        let broken = broken_by(fault);
        assert!(
            broken.contains(&rule),
            "{fault:?} should break {rule}; it broke {broken:?}"
        );
    }
}
