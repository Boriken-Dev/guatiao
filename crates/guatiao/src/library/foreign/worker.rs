// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The memory the host and the program `ld.so` starts share: a control
//! block, one mailbox per worker, and the requests the loader's own jobs
//! take. Both sides are this crate's code, so the layouts are Rust's.

use core::ffi::c_void;
use core::sync::atomic::{AtomicI32, AtomicIsize, AtomicU32, AtomicUsize, Ordering};

use super::bare::{Slot, futex_wake};

pub(crate) const STARTING: u32 = 0;
pub(crate) const READY: u32 = 1;
pub(crate) const FAILED: u32 = 2;
/// `failure` when the program was handed a block it cannot read.
pub(crate) const FAIL_LAYOUT: i32 = -1;
/// `failure` when the image's thread-local mark was not found near the
/// thread pointer.
pub(crate) const FAIL_TLS: i32 = -2;

/// What the program takes from the foreign C library, each through a
/// relocation `ld.so` fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub(crate) enum Import {
    LibcStartMain,
    Dlopen,
    Dlsym,
    Dlerror,
    Dlclose,
    Dladdr1,
    DlIteratePhdr,
    PthreadCreate,
    PthreadAttrInit,
    PthreadAttrSetstacksize,
    PthreadAttrDestroy,
    PthreadSigmask,
}

impl Import {
    pub(crate) const ALL: [Import; 12] = [
        Import::LibcStartMain,
        Import::Dlopen,
        Import::Dlsym,
        Import::Dlerror,
        Import::Dlclose,
        Import::Dladdr1,
        Import::DlIteratePhdr,
        Import::PthreadCreate,
        Import::PthreadAttrInit,
        Import::PthreadAttrSetstacksize,
        Import::PthreadAttrDestroy,
        Import::PthreadSigmask,
    ];

    /// The symbol's name.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Import::LibcStartMain => "__libc_start_main",
            Import::Dlopen => "dlopen",
            Import::Dlsym => "dlsym",
            Import::Dlerror => "dlerror",
            Import::Dlclose => "dlclose",
            Import::Dladdr1 => "dladdr1",
            Import::DlIteratePhdr => "dl_iterate_phdr",
            Import::PthreadCreate => "pthread_create",
            Import::PthreadAttrInit => "pthread_attr_init",
            Import::PthreadAttrSetstacksize => "pthread_attr_setstacksize",
            Import::PthreadAttrDestroy => "pthread_attr_destroy",
            Import::PthreadSigmask => "pthread_sigmask",
        }
    }
}

const IMPORTS: usize = Import::ALL.len();

/// The control block. Written by the host before the program starts,
/// read by it after; `ready`, `failure`, `started`, `fns` and `tls_offset`
/// are the program's answers.
#[repr(C)]
pub(crate) struct Control {
    pub(crate) size: u32,
    pub(crate) ready: AtomicU32,
    pub(crate) failure: AtomicI32,
    pub(crate) started: AtomicU32,
    pub(crate) spin: u32,
    workers: u32,
    /// How many workers the program starts before it reports ready.
    pub(crate) initial: u32,
    pub(crate) blocked: u64,
    pub(crate) worker_stack: u64,
    slots: *const Slot,
    /// Where `ld.so` writes each import, in the mapped image.
    pub(crate) import_at: [usize; IMPORTS],
    /// Each import, copied out once the C library started.
    fns: [AtomicUsize; IMPORTS],
    /// What the image's thread-local block starts with.
    pub(crate) mark: [u64; 2],
    /// Where that block lies from a foreign thread's thread pointer.
    pub(crate) tls_offset: AtomicIsize,
}

// SAFETY: every field the program writes is atomic; the rest is written
// once, before the program exists.
unsafe impl Sync for Control {}
// SAFETY: as above; the block is leaked and never moves.
unsafe impl Send for Control {}

impl Control {
    pub(crate) fn new(
        slots: &'static [Slot],
        initial: usize,
        spin: u32,
        blocked: u64,
        worker_stack: u64,
        mark: [u64; 2],
    ) -> Control {
        Control {
            size: size_of::<Control>() as u32,
            ready: AtomicU32::new(STARTING),
            failure: AtomicI32::new(0),
            started: AtomicU32::new(0),
            spin,
            workers: slots.len() as u32,
            initial: initial as u32,
            blocked,
            worker_stack,
            slots: slots.as_ptr(),
            import_at: [0; IMPORTS],
            fns: [const { AtomicUsize::new(0) }; IMPORTS],
            mark,
            tls_offset: AtomicIsize::new(0),
        }
    }

    /// The workers' mailboxes.
    pub(crate) fn slots(&self) -> &'static [Slot] {
        // SAFETY: a leaked array of `workers` slots, set at construction.
        unsafe { core::slice::from_raw_parts(self.slots, self.workers as usize) }
    }

    /// Reports a start that cannot go on.
    pub(crate) fn fail(&self, failure: i32) {
        self.failure.store(failure, Ordering::SeqCst);
        self.ready.store(FAILED, Ordering::SeqCst);
        futex_wake(&self.ready);
    }

    /// What `ld.so` wrote for `import`.
    ///
    /// # Safety
    ///
    /// `ld.so` has relocated the image.
    pub(crate) unsafe fn import(&self, import: Import) -> usize {
        let at = self.import_at[import as usize];
        if at == 0 {
            return 0;
        }
        // SAFETY: an aligned word of the mapped image, filled by ld.so.
        unsafe { (at as *const usize).read_volatile() }
    }

    /// Copies every import out of the image.
    ///
    /// # Safety
    ///
    /// As [`Control::import`].
    pub(crate) unsafe fn copy_imports(&self) {
        for import in Import::ALL {
            // SAFETY: forwarded.
            let f = unsafe { self.import(import) };
            self.fns[import as usize].store(f, Ordering::Release);
        }
    }

    /// An import, once copied.
    pub(crate) fn function(&self, import: Import) -> usize {
        self.fns[import as usize].load(Ordering::Acquire)
    }
}

/// A request for the foreign dynamic linker, run by [`super::bare::dl_job`].
#[repr(C)]
pub(crate) struct DlRequest {
    pub(crate) control: *const Control,
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
}

pub(crate) const DL_OPEN: u32 = 1;
pub(crate) const DL_SYMBOL: u32 = 2;
pub(crate) const DL_CLOSE: u32 = 3;

impl DlRequest {
    pub(crate) fn new(control: &Control, op: u32) -> DlRequest {
        DlRequest {
            control,
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
        }
    }
}

/// A walk of every object the foreign linker holds, run by
/// [`super::bare::phdr_job`]: each loaded segment as `[object, start, end,
/// flags]`.
#[repr(C)]
pub(crate) struct PhdrRequest {
    pub(crate) control: *const Control,
    pub(crate) buffer: *mut [usize; 4],
    pub(crate) capacity: usize,
    /// How many segments there were, which may exceed `capacity`.
    pub(crate) found: usize,
    pub(crate) objects: usize,
}

/// One more worker, for slot `index`, run by [`super::bare::spawn_job`].
#[repr(C)]
pub(crate) struct SpawnRequest {
    pub(crate) control: *const Control,
    pub(crate) index: usize,
    /// The C library's error number, 0 when the worker runs.
    pub(crate) error: i32,
}
