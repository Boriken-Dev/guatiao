// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! An echo serving a channel: every message sent comes back through the
//! host's sink, in order, from a thread this library starts for the
//! channel. It keeps every rule of `guatiao-channel`:
//!
//! - `send` waits for room in a queue of a few messages, so a slow sink
//!   holds the sender back rather than failing it, and `close` wakes a
//!   `send` that is waiting, which then answers `GUATIAO_ERR_GONE`;
//! - one thread calls the sink, in the order the messages were queued;
//! - `close` lets the queue drain, then the thread calls `closed` with
//!   the host's status and lets the sink go; dropping the channel is
//!   `close(0)` and does not wait for the thread;
//! - after `close_after` echoes (the configuration's), the thread closes
//!   with `close_status`, and what was still queued is not delivered;
//! - an error from `receive` ends the stream: `closed` with that status.
//!
//! The library refuses to be unloaded while a thread of its own still
//! runs, which is while a sink is still inside a call, and joins them
//! once none does.
extern crate alloc;

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec::Vec;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

use channel_echo_kind::{Channel, ChannelEcho, ChannelSink};
use guatiao::library::{Object, ProviderError};
use guatiao::{Map, Provider, Status};

/// How many messages wait for the sink before `send` waits too.
const QUEUE: usize = 4;

/// The threads this library started, joined when it is unloaded.
static THREADS: Mutex<Vec<JoinHandle<()>>> = Mutex::new(Vec::new());

/// The library's say in being unloaded: `GUATIAO_ERR_BUSY` while a thread
/// of its own is still running, else every thread is joined and it agrees.
extern "C" fn threads_done() -> Status {
    let mut threads = THREADS.lock().unwrap_or_else(PoisonError::into_inner);
    if threads.iter().any(|t| !t.is_finished()) {
        return Status::GUATIAO_ERR_BUSY;
    }
    for thread in threads.drain(..) {
        let _ = thread.join();
    }
    Status::GUATIAO_OK
}

/// Starts an echoing channel per call.
#[derive(Default, Provider)]
#[provider(ChannelEcho)]
pub struct Echo;

impl ChannelEcho for Echo {
    fn start(
        &self,
        config: &Map,
        sink: Object<dyn ChannelSink>,
    ) -> Result<Object<dyn Channel>, ProviderError> {
        let number = |key: &str| -> Result<Option<u64>, ProviderError> {
            match config.get(key) {
                None => Ok(None),
                Some(v) => u64::try_from(v).map(Some).map_err(|_| {
                    ProviderError::new(
                        Status::GUATIAO_ERR_BAD_VALUE,
                        &format!("`{key}` is not a count"),
                    )
                }),
            }
        };
        let close_after = number("close_after")?;
        let close_status = u32::try_from(number("close_status")?.unwrap_or(0)).map_err(|_| {
            ProviderError::new(
                Status::GUATIAO_ERR_BAD_VALUE,
                "`close_status` is not a status",
            )
        })?;

        let queue = Arc::new(Queue::default());
        let thread_queue = Arc::clone(&queue);
        let thread = std::thread::Builder::new()
            .name("channel_echo".into())
            .spawn(move || deliver(sink, &thread_queue, close_after, close_status))
            .map_err(|_| {
                ProviderError::new(Status::GUATIAO_ERR_INTERNAL, "no thread to echo on")
            })?;
        let mut threads = THREADS.lock().unwrap_or_else(PoisonError::into_inner);
        // Threads that have ended are joined as new ones start.
        let (done, running): (Vec<_>, Vec<_>) = threads.drain(..).partition(|t| t.is_finished());
        *threads = running;
        for finished in done {
            let _ = finished.join();
        }
        threads.push(thread);
        drop(threads);
        Ok(Echoing { queue }.into_object())
    }
}

#[derive(Default)]
struct Inner {
    items: VecDeque<Vec<u8>>,
    /// Either side has ended the stream: `send` answers GONE from here on.
    closed: bool,
    /// The status the host closed with, when it was the host.
    host_status: Option<u32>,
}

/// What the channel and its thread share.
#[derive(Default)]
struct Queue {
    inner: Mutex<Inner>,
    /// Signalled when a message is taken or the stream ends: `send` waits.
    space: Condvar,
    /// Signalled when a message arrives or the host closes: the thread waits.
    arrived: Condvar,
}

impl Queue {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The host's end.
struct Echoing {
    queue: Arc<Queue>,
}

fn gone() -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_GONE, "the channel is closed")
}

impl Channel for Echoing {
    fn send(&self, bytes: &[u8]) -> Result<(), ProviderError> {
        let mut inner = self.queue.lock();
        loop {
            if inner.closed {
                return Err(gone());
            }
            if inner.items.len() < QUEUE {
                break;
            }
            inner = self
                .queue
                .space
                .wait(inner)
                .unwrap_or_else(PoisonError::into_inner);
        }
        inner.items.push_back(bytes.to_vec());
        drop(inner);
        self.queue.arrived.notify_one();
        Ok(())
    }

    fn close(&self, status: u32) {
        let mut inner = self.queue.lock();
        if inner.closed {
            return;
        }
        inner.closed = true;
        inner.host_status = Some(status);
        drop(inner);
        // Wakes a `send` waiting for room, and the thread to drain.
        self.queue.space.notify_all();
        self.queue.arrived.notify_all();
    }
}

impl Drop for Echoing {
    fn drop(&mut self) {
        self.close(Status::GUATIAO_OK.into());
    }
}

/// The channel's thread: the only caller of the sink.
fn deliver(
    mut sink: Object<dyn ChannelSink>,
    queue: &Queue,
    close_after: Option<u64>,
    close_status: u32,
) {
    let mut echoed = 0u64;
    let status = loop {
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
            let status = inner.host_status.unwrap_or(close_status);
            // What was queued and not delivered goes with the stream.
            inner.closed = true;
            inner.items.clear();
            drop(inner);
            queue.space.notify_all();
            break status;
        };
        if let Err(e) = sink.receive(&message) {
            let mut inner = queue.lock();
            inner.closed = true;
            inner.items.clear();
            drop(inner);
            queue.space.notify_all();
            break e.status.into();
        }
        echoed += 1;
    };
    sink.closed(status);
}

guatiao::providers!(
    id = "channel_echo",
    version = env!("CARGO_PKG_VERSION"),
    providers = [Echo],
    unload = threads_done
);
