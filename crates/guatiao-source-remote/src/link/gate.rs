// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The link's writer, and the frames waiting for it.
//!
//! No thread waits for another's write. A frame is queued; a thread that
//! finds the writer free takes it and writes everything queued, in order
//! and in one go where the transport can, until nothing is left. So
//! threads calling at once share a write rather than queueing for the
//! writer one wake-up at a time, and a write blocked on a peer that takes
//! nothing holds up only the thread writing: every other caller is already
//! waiting for its answer, and at its deadline takes back a frame not yet
//! written.

use std::io;
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::transport::FrameWrite;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) struct Gate {
    state: Mutex<State>,
}

struct State {
    /// `None` while a thread writes.
    writer: Option<Box<dyn FrameWrite>>,
    /// Frames for the thread writing, in order, each with the call it
    /// starts (0: none).
    queued: Vec<(u64, Vec<u8>)>,
    /// The last batch's vector, kept for the next.
    spare: Vec<(u64, Vec<u8>)>,
    closed: bool,
}

/// Why a frame was not written.
pub(crate) enum Unsent {
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
                spare: Vec::new(),
                closed: false,
            }),
        }
    }

    /// Sends `frame`, which starts call `call` (0: none): written now if
    /// the writer is free, with whatever else is queued, else left for the
    /// thread writing. Never waits for another thread.
    pub(crate) fn send(&self, call: u64, frame: Vec<u8>) -> Result<(), Unsent> {
        let writer = {
            let mut state = lock(&self.state);
            if state.closed {
                return Err(Unsent::Closed);
            }
            state.queued.push((call, frame));
            match state.writer.take() {
                Some(w) => w,
                None => return Ok(()),
            }
        };
        self.drain(writer)
    }

    /// Takes back call `call`'s frame if it is still queued: `true` when
    /// it never reached the peer.
    pub(crate) fn retract(&self, call: u64) -> bool {
        let mut state = lock(&self.state);
        match state.queued.iter().position(|(c, _)| *c == call) {
            Some(at) => {
                state.queued.remove(at);
                true
            }
            None => false,
        }
    }

    /// Runs `f` on the writer if it is free; `false` when it is not.
    pub(crate) fn try_with(&self, f: impl FnOnce(&mut dyn FrameWrite)) -> bool {
        let Some(mut writer) = lock(&self.state).writer.take() else {
            return false;
        };
        f(&mut *writer);
        // Whatever was queued meanwhile still goes before the writer
        // returns; a failure here is the link ending anyway.
        let _ = self.drain(writer);
        true
    }

    /// Refuses every later frame. What is queued already is still written
    /// by the thread writing, if one is: a GOODBYE queued behind a write in
    /// progress reaches the peer.
    pub(crate) fn close(&self) {
        let mut state = lock(&self.state);
        state.closed = true;
        if state.writer.is_some() {
            state.queued.clear();
        }
    }

    /// Writes what is queued until nothing is, then gives the writer back.
    fn drain(&self, mut writer: Box<dyn FrameWrite>) -> Result<(), Unsent> {
        let mut batch = Vec::new();
        loop {
            let mut state = lock(&self.state);
            batch.clear();
            if state.queued.is_empty() {
                state.spare = batch;
                state.writer = Some(writer);
                return Ok(());
            }
            if batch.capacity() == 0 {
                batch = core::mem::take(&mut state.spare);
            }
            core::mem::swap(&mut batch, &mut state.queued);
            drop(state);
            let frames: Vec<&[u8]> = batch.iter().map(|(_, f)| f.as_slice()).collect();
            let written = writer.write_frames(&frames);
            drop(frames);
            if let Err(e) = written {
                let mut state = lock(&self.state);
                state.queued.clear();
                state.writer = Some(writer);
                return Err(Unsent::Failed(e));
            }
        }
    }
}
