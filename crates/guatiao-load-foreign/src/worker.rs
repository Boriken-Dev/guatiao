// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The memory the host and the helper share: a control block and one
//! mailbox per worker, laid out as `helper/helper.c` declares them. A host
//! thread posts a job in a mailbox and waits for it; the worker runs it and
//! answers. Both sides spin a while, then sleep on the mailbox's futex,
//! each telling the other it sleeps so a wake is only sent when needed.

use core::ffi::c_void;
use core::sync::atomic::{AtomicI32, AtomicU32, AtomicUsize, Ordering};

use crate::sys;

pub(crate) const IDLE: u32 = 0;
pub(crate) const POSTED: u32 = 1;
pub(crate) const DONE: u32 = 2;

pub(crate) const STARTING: u32 = 0;
pub(crate) const READY: u32 = 1;
/// `failure` when the helper refuses the control block; otherwise it holds
/// the error number of the worker that could not be created.
pub(crate) const FAIL_LAYOUT: i32 = -1;

/// One worker's mailbox (`struct gt_slot`).
#[repr(C)]
pub(crate) struct Slot {
    state: AtomicU32,
    worker_sleeps: AtomicU32,
    host_sleeps: AtomicU32,
    pub(crate) tid: AtomicI32,
    function: AtomicUsize,
    argument: AtomicUsize,
}

impl Slot {
    pub(crate) fn new() -> Slot {
        Slot {
            state: AtomicU32::new(IDLE),
            worker_sleeps: AtomicU32::new(0),
            host_sleeps: AtomicU32::new(0),
            tid: AtomicI32::new(0),
            function: AtomicUsize::new(0),
            argument: AtomicUsize::new(0),
        }
    }

    /// Runs `function(argument)` on this slot's worker and waits for it.
    ///
    /// # Safety
    ///
    /// The caller is this slot's only poster until it returns; `function`
    /// is code of the foreign C library's world taking one pointer, and
    /// `argument` is valid for it until it returns.
    pub(crate) unsafe fn run(&self, function: usize, argument: *mut c_void, spin: u32) {
        self.function.store(function, Ordering::Relaxed);
        self.argument.store(argument as usize, Ordering::Relaxed);
        self.state.store(POSTED, Ordering::SeqCst);
        if self.worker_sleeps.load(Ordering::SeqCst) != 0 {
            sys::futex_wake(&self.state);
        }
        let mut left = spin;
        loop {
            if self.state.load(Ordering::Acquire) == DONE {
                break;
            }
            if left > 0 {
                left -= 1;
                core::hint::spin_loop();
                continue;
            }
            self.host_sleeps.store(1, Ordering::SeqCst);
            if self.state.load(Ordering::SeqCst) == POSTED {
                sys::futex_wait(&self.state, POSTED, None);
            }
            self.host_sleeps.store(0, Ordering::SeqCst);
        }
        self.state.store(IDLE, Ordering::Relaxed);
    }
}

/// The control block (`struct gt_control`). `ready` and `failure` come
/// first so a helper of another layout can still say it refuses this one.
#[repr(C)]
pub(crate) struct Control {
    size: u32,
    pub(crate) ready: AtomicU32,
    pub(crate) failure: AtomicI32,
    slot_size: u32,
    workers: u32,
    spin: u32,
    blocked: u64,
    worker_stack: u64,
    slots: *const Slot,
    started: AtomicU32,
    pad: u32,
    pub(crate) dl: AtomicUsize,
}

// SAFETY: every field the helper writes is atomic; the rest is written
// once, before the bootstrap thread exists.
unsafe impl Sync for Control {}
// SAFETY: as above; the block is leaked and never moves.
unsafe impl Send for Control {}

impl Control {
    pub(crate) fn new(
        slots: &'static [Slot],
        spin: u32,
        blocked: u64,
        worker_stack: u64,
    ) -> Control {
        Control {
            size: size_of::<Control>() as u32,
            ready: AtomicU32::new(STARTING),
            failure: AtomicI32::new(0),
            slot_size: size_of::<Slot>() as u32,
            workers: slots.len() as u32,
            spin,
            blocked,
            worker_stack,
            slots: slots.as_ptr(),
            started: AtomicU32::new(0),
            pad: 0,
            dl: AtomicUsize::new(0),
        }
    }
}

/// A request for the foreign dynamic linker (`struct gt_dl`), run as a job
/// by the helper's `gt_dl`.
#[repr(C)]
pub(crate) struct DlRequest {
    pub(crate) op: u32,
    pub(crate) flags: i32,
    pub(crate) handle: *mut c_void,
    pub(crate) name: *const u8,
    pub(crate) address: *mut c_void,
    pub(crate) size: u64,
    pub(crate) error: *mut u8,
    pub(crate) error_cap: u64,
    pub(crate) error_len: u64,
    pub(crate) status: i32,
    pad: u32,
}

pub(crate) const DL_OPEN: u32 = 1;
pub(crate) const DL_SYMBOL: u32 = 2;
pub(crate) const DL_CLOSE: u32 = 3;

impl DlRequest {
    pub(crate) fn new(op: u32) -> DlRequest {
        DlRequest {
            op,
            flags: 0,
            handle: core::ptr::null_mut(),
            name: core::ptr::null(),
            address: core::ptr::null_mut(),
            size: 0,
            error: core::ptr::null_mut(),
            error_cap: 0,
            error_len: 0,
            status: 0,
            pad: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sizes `helper/helper.c` checks against.
    #[test]
    fn the_shared_layouts_match_the_helper() {
        assert_eq!(size_of::<Slot>(), 32);
        assert_eq!(size_of::<Control>(), 64);
        assert_eq!(size_of::<DlRequest>(), 72);
        assert_eq!(core::mem::offset_of!(Control, ready), 4);
        assert_eq!(core::mem::offset_of!(Control, slots), 40);
        assert_eq!(core::mem::offset_of!(Control, dl), 56);
        assert_eq!(core::mem::offset_of!(DlRequest, status), 64);
    }
}
