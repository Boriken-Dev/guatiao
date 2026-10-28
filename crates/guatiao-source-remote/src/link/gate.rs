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
//!
//! A write that panics fails as a transport error does: the thread writing
//! hears `Failed` naming the panic and ends the link, so no caller is left
//! waiting on a writer that never comes back.

use alloc::sync::Arc;
use core::panic::AssertUnwindSafe;
use std::io;
use std::panic::catch_unwind;
use std::sync::Mutex;

use super::buffers::Buffers;
use crate::lock;
use crate::transport::FrameWrite;

pub(crate) struct Gate {
    state: Mutex<State>,
    /// What a written frame's buffers go back to; shared with the pool,
    /// which lets them go when its threads all leave idle.
    buffers: Arc<Buffers>,
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
            buffers: Arc::default(),
        }
    }

    /// The link's large buffers: a frame's go back here once written.
    pub(crate) fn buffers(&self) -> &Arc<Buffers> {
        &self.buffers
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
        if let Err(e) = guarded(|| writer.write_frame_parts(&head, tail)) {
            return Err(self.failed(writer, e));
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

    /// A write failed part way: nothing more goes through this writer, which
    /// stays for the link's end to shut down.
    fn failed(&self, writer: Box<dyn FrameWrite>, e: io::Error) -> Unsent {
        let mut state = lock(&self.state);
        state.closed = true;
        state.queued.clear();
        state.writer = Some(writer);
        Unsent::Failed(e)
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
            if let Err(e) = guarded(|| write_batch(&mut *writer, &batch)) {
                return Err(self.failed(writer, e));
            }
            for written in batch.drain(..) {
                self.buffers.give(written.head);
                if let Some((tail, _)) = written.tail {
                    self.buffers.give(tail);
                }
            }
        }
    }
}

/// Runs a write, a panic in it answered as the transport failing: the
/// writer would otherwise unwind out of the gate and never come back.
fn guarded(write: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
    catch_unwind(AssertUnwindSafe(write)).unwrap_or_else(|payload| {
        Err(io::Error::other(format!(
            "the writer panicked: {}",
            super::panic_words(&*payload)
        )))
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::sync::Arc;

    /// One call on the writer, with the bytes it was given.
    #[derive(Debug, PartialEq)]
    enum Op {
        Frames(Vec<Vec<u8>>),
        Parts(Vec<u8>, Vec<u8>),
    }

    /// A writer that records each call it gets.
    struct Recording(Arc<Mutex<Vec<Op>>>);

    impl FrameWrite for Recording {
        fn write_frame(&mut self, frame: &[u8]) -> io::Result<()> {
            lock(&self.0).push(Op::Frames(vec![frame.to_vec()]));
            Ok(())
        }

        fn write_frame_parts(&mut self, head: &[u8], tail: &[u8]) -> io::Result<()> {
            lock(&self.0).push(Op::Parts(head.to_vec(), tail.to_vec()));
            Ok(())
        }

        fn write_frames(&mut self, frames: &[&[u8]]) -> io::Result<()> {
            lock(&self.0).push(Op::Frames(frames.iter().map(|f| f.to_vec()).collect()));
            Ok(())
        }
    }

    fn gate() -> (Gate, Arc<Mutex<Vec<Op>>>) {
        let ops = Arc::new(Mutex::new(Vec::new()));
        (Gate::new(Box::new(Recording(Arc::clone(&ops)))), ops)
    }

    fn bytes(s: &str) -> Vec<u8> {
        s.as_bytes().to_vec()
    }

    #[test]
    fn a_free_writer_writes_a_frames_parts_from_where_they_lie() {
        let (gate, ops) = gate();
        assert!(gate.send_parts(1, bytes("head"), b"tail").is_ok());
        assert_eq!(*lock(&ops), [Op::Parts(bytes("head"), bytes("tail"))]);
        assert!(!gate.retract(1), "it went out");
    }

    #[test]
    fn frames_queued_behind_a_busy_writer_go_out_in_order_whole_runs_together() {
        let (gate, ops) = gate();
        let busy = gate.try_with(|_| {
            for sent in [
                gate.send(1, bytes("a")),
                gate.send_with_tail(2, bytes("h2"), bytes("..t2"), 2),
                gate.send(3, bytes("b")),
                gate.send(4, bytes("c")),
                // Copied behind its head while the writer is busy.
                gate.send_parts(5, bytes("h5"), b"t5"),
                gate.send_with_tail(6, bytes("h6"), bytes("t6"), 0),
            ] {
                assert!(sent.is_ok());
            }
            assert!(lock(&ops).is_empty(), "nothing goes out while busy");
        });
        assert!(busy);
        assert_eq!(
            *lock(&ops),
            [
                Op::Frames(vec![bytes("a")]),
                Op::Parts(bytes("h2"), bytes("t2")),
                Op::Frames(vec![bytes("b"), bytes("c"), bytes("h5t5")]),
                Op::Parts(bytes("h6"), bytes("t6")),
            ]
        );
    }

    #[test]
    fn a_large_frame_once_written_leaves_its_buffers_for_reuse() {
        let (gate, ops) = gate();
        let big = super::super::buffers::KEEP_FROM;
        let busy = gate.try_with(|_| {
            assert!(gate.send(1, vec![1; big]).is_ok());
            assert!(gate.send_with_tail(2, bytes("h"), vec![2; big], 0).is_ok());
        });
        assert!(busy);
        assert_eq!(lock(&ops).len(), 2);
        for _ in 0..2 {
            let again = gate.buffers().take();
            assert!(again.is_empty() && again.capacity() >= big);
        }
        assert_eq!(gate.buffers().take().capacity(), 0);
    }

    #[test]
    fn a_frame_with_a_tail_queued_behind_a_busy_writer_can_be_taken_back() {
        let (gate, ops) = gate();
        gate.try_with(|_| {
            assert!(gate.send_parts(7, bytes("h7"), b"t7").is_ok());
            assert!(gate.send_with_tail(8, bytes("h8"), bytes("t8"), 0).is_ok());
            assert!(gate.send(9, bytes("c9")).is_ok());
            assert!(gate.retract(7));
            assert!(!gate.retract(7), "taken back once");
            assert!(gate.retract(8));
        });
        assert_eq!(*lock(&ops), [Op::Frames(vec![bytes("c9")])]);
    }
}
