// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The link's writer, held by one thread at a time for one frame.
//!
//! A thread waits for the writer at most until its call's deadline, and a
//! frame nobody waits on (CANCEL, RELEASE, a refusal) never waits at all:
//! it is queued, and whoever holds the writer sends it before giving the
//! writer back. So a write blocked on a peer that takes nothing holds up
//! no other call past its deadline.

use std::io;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use crate::transport::FrameWrite;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) struct Gate {
    state: Mutex<State>,
    freed: Condvar,
}

struct State {
    /// `None` while a thread holds it.
    writer: Option<Box<dyn FrameWrite>>,
    /// Frames queued for whoever holds the writer, in order.
    queued: Vec<Vec<u8>>,
    /// Threads waiting for the writer: only then is giving it back a
    /// wake-up.
    waiting: usize,
    closed: bool,
}

/// Why a frame was not written.
pub(crate) enum Unsent {
    /// The deadline passed before the writer was free: nothing was
    /// written, the link is as it was.
    Late,
    /// The link has ended.
    Closed,
    /// The transport failed: the link must end.
    Failed(io::Error),
}

impl Gate {
    pub(crate) fn new(writer: Box<dyn FrameWrite>) -> Gate {
        Gate {
            state: Mutex::new(State {
                writer: Some(writer),
                queued: Vec::new(),
                waiting: 0,
                closed: false,
            }),
            freed: Condvar::new(),
        }
    }

    /// Writes `frame`, waiting for the writer at most until `deadline`.
    pub(crate) fn send(&self, frame: &[u8], deadline: Option<Instant>) -> Result<(), Unsent> {
        let mut writer = self.claim(deadline)?;
        let written = writer.write_frame(frame);
        self.give_back(writer, written)
    }

    /// Writes `frame` now if the writer is free, else queues it for the
    /// thread holding it. Never waits for another thread.
    pub(crate) fn post(&self, frame: Vec<u8>) -> Result<(), Unsent> {
        let mut writer = {
            let mut state = lock(&self.state);
            if state.closed {
                return Err(Unsent::Closed);
            }
            match state.writer.take() {
                Some(w) => w,
                None => {
                    state.queued.push(frame);
                    return Ok(());
                }
            }
        };
        let written = writer.write_frame(&frame);
        self.give_back(writer, written)
    }

    /// Runs `f` on the writer if it is free; `false` when it is not.
    pub(crate) fn try_with(&self, f: impl FnOnce(&mut dyn FrameWrite)) -> bool {
        let Some(mut writer) = lock(&self.state).writer.take() else {
            return false;
        };
        f(&mut *writer);
        // Whatever was queued meanwhile still goes before the writer
        // returns; a failure here is the link ending anyway.
        let _ = self.give_back(writer, Ok(()));
        true
    }

    /// Refuses every later frame and wakes every waiter.
    pub(crate) fn close(&self) {
        let mut state = lock(&self.state);
        state.closed = true;
        state.queued.clear();
        if state.waiting > 0 {
            self.freed.notify_all();
        }
    }

    fn claim(&self, deadline: Option<Instant>) -> Result<Box<dyn FrameWrite>, Unsent> {
        let mut state = lock(&self.state);
        loop {
            if state.closed {
                return Err(Unsent::Closed);
            }
            if let Some(w) = state.writer.take() {
                return Ok(w);
            }
            state.waiting += 1;
            state = match deadline {
                None => self
                    .freed
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner),
                Some(d) => {
                    let now = Instant::now();
                    if now >= d {
                        state.waiting -= 1;
                        return Err(Unsent::Late);
                    }
                    self.freed
                        .wait_timeout(state, d - now)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0
                }
            };
            state.waiting -= 1;
        }
    }

    /// Sends what was queued while `writer` was held, then gives it back.
    fn give_back(
        &self,
        mut writer: Box<dyn FrameWrite>,
        mut written: io::Result<()>,
    ) -> Result<(), Unsent> {
        loop {
            let mut state = lock(&self.state);
            if written.is_err() || state.queued.is_empty() || state.closed {
                state.queued.clear();
                state.writer = Some(writer);
                if state.waiting > 0 {
                    self.freed.notify_one();
                }
                return written.map_err(Unsent::Failed);
            }
            let queued = core::mem::take(&mut state.queued);
            drop(state);
            for frame in &queued {
                written = writer.write_frame(frame);
                if written.is_err() {
                    break;
                }
            }
        }
    }
}
