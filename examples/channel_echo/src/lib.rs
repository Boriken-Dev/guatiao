// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! An echo serving a channel: every message sent comes back through the
//! host's sink, in order, from a thread this library starts for the
//! channel. It keeps every rule of `guatiao-channel`:
//!
//! - `send` waits for room in a queue of a few messages, so a slow sink
//!   holds the sender back rather than failing it;
//! - one thread calls the sink, in the order the messages were queued;
//! - `close` lets the queue drain, then the thread calls `closed` with the
//!   host's status and lets the sink go; dropping the channel is
//!   `close(0)`;
//! - after `close_after` echoes (the configuration's), the thread closes
//!   with `close_status`;
//! - an error from `receive` ends the stream: `closed` with that status.
//!
//! Every way the thread stops drops the queue's receiving end, so a
//! `send` waiting on it, and every `send` after, answers
//! `GUATIAO_ERR_GONE`.
extern crate alloc;

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

use channel_echo_kind::{Channel, ChannelEcho, ChannelSink};
use guatiao::library::{Object, ProviderError};
use guatiao::{Map, Provider, Status};

/// How many messages wait for the sink before `send` waits too.
const QUEUE: usize = 4;

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

        let (queue, taken) = sync_channel(QUEUE);
        let state = Arc::new(State {
            closed: AtomicBool::new(false),
            host_status: AtomicU32::new(0),
        });
        let thread_state = Arc::clone(&state);
        std::thread::Builder::new()
            .name("channel_echo".into())
            .spawn(move || deliver(sink, taken, &thread_state, close_after, close_status))
            .map_err(|_| {
                ProviderError::new(Status::GUATIAO_ERR_INTERNAL, "no thread to echo on")
            })?;
        Ok(Echoing {
            queue: Mutex::new(Some(queue)),
            state,
        }
        .into_object())
    }
}

/// What the channel and its thread share.
struct State {
    /// Either side has closed: `send` answers GONE from here on.
    closed: AtomicBool,
    /// The status the host closed with, read once the queue is drained.
    host_status: AtomicU32,
}

/// The host's end: the queue's sending half until the host closes.
struct Echoing {
    queue: Mutex<Option<SyncSender<Vec<u8>>>>,
    state: Arc<State>,
}

fn gone() -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_GONE, "the channel is closed")
}

impl Channel for Echoing {
    fn send(&self, bytes: &[u8]) -> Result<(), ProviderError> {
        if self.state.closed.load(Ordering::Acquire) {
            return Err(gone());
        }
        // A clone, so a send waiting for room does not hold the lock
        // `close` needs.
        let queue = self
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .ok_or_else(gone)?;
        queue.send(bytes.to_vec()).map_err(|_| gone())
    }

    fn close(&self, status: u32) {
        if self.state.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        self.state.host_status.store(status, Ordering::Release);
        // The thread sees the queue end once it is drained, and any
        // waiting send's clone is gone.
        drop(
            self.queue
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take(),
        );
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
    taken: Receiver<Vec<u8>>,
    state: &State,
    close_after: Option<u64>,
    close_status: u32,
) {
    let mut echoed = 0u64;
    let status = loop {
        if close_after.is_some_and(|n| echoed >= n) {
            state.closed.store(true, Ordering::Release);
            break close_status;
        }
        let Ok(message) = taken.recv() else {
            break state.host_status.load(Ordering::Acquire);
        };
        if let Err(e) = sink.receive(&message) {
            state.closed.store(true, Ordering::Release);
            break e.status.into();
        }
        echoed += 1;
    };
    // Ends every send waiting for room before the sink hears the end.
    drop(taken);
    sink.closed(status);
}

guatiao::providers!(Echo);
