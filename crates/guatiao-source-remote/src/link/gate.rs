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
    /// Frames for the thread writing, in order.
    queued: Vec<Queued>,
    /// The last batch's vector, kept for the next.
    spare: Vec<Queued>,
    closed: bool,
}

/// A frame waiting for the writer.
struct Queued {
    /// The call it starts (0: none).
    call: u64,
    head: Vec<u8>,
    /// The frame's last bytes, kept out of `head`: a vector and where in it
    /// they start.
    tail: Option<(Vec<u8>, usize)>,
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
        self.queue(Queued {
            call,
            head: frame,
            tail: None,
        })
    }

    /// Sends a frame whose last bytes are `tail[from..]`, as
    /// [`send`](Gate::send) does: a large answer, encoded where it lies.
    pub(crate) fn send_with_tail(
        &self,
        call: u64,
        head: Vec<u8>,
        tail: Vec<u8>,
        from: usize,
    ) -> Result<(), Unsent> {
        self.queue(Queued {
            call,
            head,
            tail: Some((tail, from)),
        })
    }

    fn queue(&self, frame: Queued) -> Result<(), Unsent> {
        let writer = {
            let mut state = lock(&self.state);
            if state.closed {
                return Err(Unsent::Closed);
            }
            state.queued.push(frame);
            match state.writer.take() {
                Some(w) => w,
                None => return Ok(()),
            }
        };
        self.drain(writer)
    }

    /// Sends a frame whose last bytes are `tail`, a large argument kept
    /// out of `head`: written from where it lies if the writer is free,
    /// else copied behind `head` and queued as [`send`](Gate::send) does.
    pub(crate) fn send_parts(
        &self,
        call: u64,
        mut head: Vec<u8>,
        tail: &[u8],
    ) -> Result<(), Unsent> {
        let free = {
            let mut state = lock(&self.state);
            if state.closed {
                return Err(Unsent::Closed);
            }
            state.writer.take()
        };
        let Some(mut writer) = free else {
            // Copied outside the lock: the thread writing takes its next
            // batch meanwhile.
            head.extend_from_slice(tail);
            return self.send(call, head);
        };
        if let Err(e) = writer.write_frame_parts(&head, tail) {
            let mut state = lock(&self.state);
            state.queued.clear();
            state.writer = Some(writer);
            return Err(Unsent::Failed(e));
        }
        self.drain(writer)
    }

    /// Takes back call `call`'s frame if it is still queued: `true` when
    /// it never reached the peer.
    pub(crate) fn retract(&self, call: u64) -> bool {
        let mut state = lock(&self.state);
        match state.queued.iter().position(|q| q.call == call) {
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
            if let Err(e) = write_batch(&mut *writer, &batch) {
                let mut state = lock(&self.state);
                state.queued.clear();
                state.writer = Some(writer);
                return Err(Unsent::Failed(e));
            }
        }
    }
}

/// Writes `batch` in order: runs of whole frames together, a frame with a
/// tail as its two parts.
fn write_batch(writer: &mut dyn FrameWrite, batch: &[Queued]) -> io::Result<()> {
    let mut whole: Vec<&[u8]> = Vec::with_capacity(batch.len());
    for q in batch {
        match &q.tail {
            None => whole.push(&q.head),
            Some((tail, from)) => {
                if !whole.is_empty() {
                    writer.write_frames(&whole)?;
                    whole.clear();
                }
                writer.write_frame_parts(&q.head, &tail[*from..])?;
            }
        }
    }
    if whole.is_empty() {
        Ok(())
    } else {
        writer.write_frames(&whole)
    }
}
