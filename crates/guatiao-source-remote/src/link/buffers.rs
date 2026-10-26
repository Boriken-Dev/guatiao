// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Large buffers a link reuses: the frames it reads and the replies it
//! writes. Freed after every call, a large buffer can go back to the
//! operating system (glibc trims a thread's arena, and maps a large block
//! afresh once its threshold says so), and the next one is then faulted in
//! page by page; kept, its pages stay mapped. A link keeps a few, bounded,
//! until it ends.

use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// Smaller buffers are not kept: an allocator serves them from memory it
/// keeps anyway.
pub(crate) const KEEP_FROM: usize = 64 << 10;

/// The most a link keeps, in bytes of capacity: a 4 MiB call's frame and
/// its reply, with room. The buffer its reader is reading into is apart.
pub(crate) const KEEP_AT_MOST: usize = 16 << 20;

#[derive(Default)]
pub(crate) struct Buffers {
    kept: Mutex<Kept>,
    /// How many are kept, read without the lock: most calls find none.
    count: AtomicUsize,
}

#[derive(Default)]
struct Kept {
    buffers: Vec<Vec<u8>>,
    bytes: usize,
    /// The link has ended: nothing more is kept.
    closed: bool,
}

impl Buffers {
    fn kept(&self) -> MutexGuard<'_, Kept> {
        self.kept.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A kept buffer, empty, or a new one.
    pub(crate) fn take(&self) -> Vec<u8> {
        if self.count.load(Ordering::Acquire) == 0 {
            return Vec::new();
        }
        let mut kept = self.kept();
        match kept.buffers.pop() {
            Some(buffer) => {
                kept.bytes -= buffer.capacity();
                self.count.store(kept.buffers.len(), Ordering::Release);
                buffer
            }
            None => Vec::new(),
        }
    }

    /// Keeps `buffer` for a later [`take`](Buffers::take) when it is large
    /// and there is room; else lets it go.
    pub(crate) fn give(&self, mut buffer: Vec<u8>) {
        let size = buffer.capacity();
        if size < KEEP_FROM {
            return;
        }
        let mut kept = self.kept();
        if kept.closed || kept.bytes + size > KEEP_AT_MOST {
            return;
        }
        buffer.clear();
        kept.bytes += size;
        kept.buffers.push(buffer);
        self.count.store(kept.buffers.len(), Ordering::Release);
    }

    /// Lets every kept buffer go, and keeps none from now on.
    pub(crate) fn close(&self) {
        let dropped = {
            let mut kept = self.kept();
            kept.closed = true;
            kept.bytes = 0;
            self.count.store(0, Ordering::Release);
            core::mem::take(&mut kept.buffers)
        };
        drop(dropped);
    }

    /// The frame read into `buffer`, the reader's own: a small one is
    /// copied out so the reader keeps its room; a large one leaves with
    /// the buffer, which a kept one, if any, replaces.
    pub(crate) fn hand_over(&self, buffer: &mut Vec<u8>) -> Vec<u8> {
        if buffer.len() < KEEP_FROM {
            return buffer.as_slice().to_vec();
        }
        core::mem::replace(buffer, self.take())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_large_buffer_comes_back_empty_with_its_room() {
        let buffers = Buffers::default();
        assert_eq!(buffers.take().capacity(), 0, "none kept yet");
        buffers.give(vec![7u8; KEEP_FROM]);
        let again = buffers.take();
        assert!(again.is_empty());
        assert!(again.capacity() >= KEEP_FROM);
        assert_eq!(buffers.take().capacity(), 0, "taken once");
    }

    #[test]
    fn small_buffers_and_buffers_past_the_bound_are_let_go() {
        let buffers = Buffers::default();
        buffers.give(Vec::with_capacity(KEEP_FROM - 1));
        assert_eq!(buffers.take().capacity(), 0);
        buffers.give(Vec::with_capacity(KEEP_AT_MOST + 1));
        assert_eq!(buffers.take().capacity(), 0);
        let half = KEEP_AT_MOST / 2;
        for _ in 0..3 {
            buffers.give(Vec::with_capacity(half));
        }
        {
            let kept = buffers.kept();
            assert_eq!(kept.buffers.len(), 2, "two halves fill it");
            assert_eq!(
                kept.bytes,
                kept.buffers.iter().map(Vec::capacity).sum::<usize>()
            );
        }
        buffers.take();
        buffers.give(Vec::with_capacity(half));
        assert_eq!(
            buffers.kept().buffers.len(),
            2,
            "room again once one is taken"
        );
    }

    #[test]
    fn a_closed_link_keeps_nothing() {
        let buffers = Buffers::default();
        buffers.give(Vec::with_capacity(KEEP_FROM));
        buffers.close();
        assert_eq!(buffers.take().capacity(), 0);
        buffers.give(Vec::with_capacity(KEEP_FROM));
        assert_eq!(buffers.take().capacity(), 0);
    }

    #[test]
    fn a_small_frame_is_copied_out_and_a_large_one_leaves_with_its_buffer() {
        let buffers = Buffers::default();
        let mut reading = Vec::with_capacity(KEEP_FROM);
        reading.extend_from_slice(b"small");
        let frame = buffers.hand_over(&mut reading);
        assert_eq!(frame, b"small");
        assert!(reading.capacity() >= KEEP_FROM, "the reader keeps its room");

        let kept = Vec::<u8>::with_capacity(2 * KEEP_FROM);
        let kept_at = kept.as_ptr();
        buffers.give(kept);
        reading.resize(KEEP_FROM, 1);
        let reading_at = reading.as_ptr();
        let frame = buffers.hand_over(&mut reading);
        assert_eq!(frame.as_ptr(), reading_at, "moved, not copied");
        assert_eq!(reading.as_ptr(), kept_at, "the kept one replaces it");
    }
}
