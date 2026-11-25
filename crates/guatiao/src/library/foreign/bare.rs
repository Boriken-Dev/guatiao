// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Code that runs on the foreign C library's threads as well as the
//! host's: the program `ld.so` enters, the workers' loops, the jobs the
//! loader posts, and both halves of a call between the two sides.
//!
//! Everything here reads no thread-local, sets no `errno` and never
//! panics: on a foreign thread the thread pointer is the foreign C
//! library's, so the host's thread-locals (Rust's and its C library's,
//! `errno` among them) would be read from the wrong memory. Syscalls are
//! made with the instruction, arithmetic wraps, memory is read through
//! pointers checked by hand, and nothing allocates.

use core::ffi::{c_char, c_int, c_void};
use core::sync::atomic::{AtomicI32, AtomicU32, AtomicUsize, Ordering};

use super::worker::{
    Control, DL_CLOSE, DL_OPEN, DL_SYMBOL, DlRequest, FAIL_LAYOUT, FAIL_TLS, Import, PhdrRequest,
    READY, SpawnRequest,
};

// --- the kernel, without the host's C library ------------------------------

#[cfg(target_arch = "x86_64")]
mod nr {
    pub(super) const FUTEX: usize = 202;
    pub(super) const GETTID: usize = 186;
}
#[cfg(target_arch = "aarch64")]
mod nr {
    pub(super) const FUTEX: usize = 98;
    pub(super) const GETTID: usize = 178;
}

const FUTEX_WAIT_PRIVATE: usize = 128;
const FUTEX_WAKE_PRIVATE: usize = 129;

/// A syscall of up to four arguments; answers the kernel's word (`-errno`
/// on failure), and touches no `errno`.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn syscall4(n: usize, a: usize, b: usize, c: usize, d: usize) -> isize {
    let ret: isize;
    // SAFETY: the caller's; the kernel clobbers rcx and r11.
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") n as isize => ret,
            in("rdi") a,
            in("rsi") b,
            in("rdx") c,
            in("r10") d,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn syscall4(n: usize, a: usize, b: usize, c: usize, d: usize) -> isize {
    let ret: isize;
    // SAFETY: the caller's.
    unsafe {
        core::arch::asm!(
            "svc #0",
            inlateout("x0") a as isize => ret,
            in("x1") b,
            in("x2") c,
            in("x3") d,
            in("x8") n,
            options(nostack),
        );
    }
    ret
}

/// Sleeps while `word` holds `expected`; returns on a wake, a change or a
/// signal, and the caller looks again.
pub(crate) fn futex_wait(word: &AtomicU32, expected: u32) {
    // SAFETY: the word is a live atomic; no timeout.
    unsafe {
        syscall4(
            nr::FUTEX,
            word.as_ptr() as usize,
            FUTEX_WAIT_PRIVATE,
            expected as usize,
            0,
        )
    };
}

/// Wakes every thread sleeping on `word`.
pub(crate) fn futex_wake(word: &AtomicU32) {
    // SAFETY: the word is a live atomic.
    unsafe {
        syscall4(
            nr::FUTEX,
            word.as_ptr() as usize,
            FUTEX_WAKE_PRIVATE,
            i32::MAX as usize,
            0,
        )
    };
}

/// This thread's kernel id.
pub(crate) fn gettid() -> i32 {
    // SAFETY: gettid takes nothing and cannot fail.
    unsafe { syscall4(nr::GETTID, 0, 0, 0, 0) as i32 }
}

/// This thread's thread pointer: the C library's own record of the thread,
/// whichever C library made it.
#[inline(always)]
pub(crate) fn thread_pointer() -> usize {
    let tp: usize;
    // SAFETY: reads a register (x86_64: the self pointer both C libraries
    // keep at `fs:0`).
    unsafe {
        #[cfg(target_arch = "x86_64")]
        core::arch::asm!("mov {}, fs:0", out(reg) tp, options(nostack, readonly, preserves_flags));
        #[cfg(target_arch = "aarch64")]
        core::arch::asm!("mrs {}, tpidr_el0", out(reg) tp, options(nomem, nostack, preserves_flags));
    }
    tp
}

fn relax() {
    core::hint::spin_loop();
}

// --- the mailbox ------------------------------------------------------------

/// Nothing posted yet.
pub(crate) const IDLE: u32 = 0;
/// The host posted a job for the foreign side.
pub(crate) const JOB: u32 = 1;
/// The foreign side finished the job.
pub(crate) const DONE: u32 = 2;
/// The foreign side asks the host to run a request.
pub(crate) const CALL: u32 = 3;
/// The host ran the request.
pub(crate) const BACK: u32 = 4;

/// What one side asks the host to run: a function of the host's and its
/// argument.
#[repr(C)]
pub(crate) struct Request {
    pub(crate) run: unsafe fn(*mut c_void),
    pub(crate) data: *mut c_void,
}

/// One pairing of a foreign thread and a host thread: a worker's mailbox,
/// or the one a foreign thread the library made sets up on its own stack
/// for one call into the host.
///
/// The two sides take turns: the host posts a job (`JOB`) and waits for
/// `DONE`; while it runs, the foreign side may post a request (`CALL`) and
/// wait for `BACK`, and the host, running it, may post a job again. Each
/// side waits only for the answer to its own last post and serves what the
/// other posts meanwhile, so a call into the host may call back into the
/// library, as deep as the two like, always on the same pair of threads.
#[repr(C)]
pub(crate) struct Slot {
    pub(crate) state: AtomicU32,
    foreign_sleeps: AtomicU32,
    host_sleeps: AtomicU32,
    pub(crate) tid: AtomicI32,
    /// The foreign thread's thread pointer.
    pub(crate) foreign_tp: AtomicUsize,
    job: AtomicUsize,
    argument: AtomicUsize,
    request: AtomicUsize,
    /// The next mailbox waiting for a host thread (strays only).
    pub(crate) next: AtomicUsize,
}

impl Slot {
    pub(crate) const fn new() -> Slot {
        Slot {
            state: AtomicU32::new(IDLE),
            foreign_sleeps: AtomicU32::new(0),
            host_sleeps: AtomicU32::new(0),
            tid: AtomicI32::new(0),
            foreign_tp: AtomicUsize::new(0),
            job: AtomicUsize::new(0),
            argument: AtomicUsize::new(0),
            request: AtomicUsize::new(0),
            next: AtomicUsize::new(0),
        }
    }

    fn set(&self, state: u32, other_sleeps: &AtomicU32) {
        self.state.store(state, Ordering::SeqCst);
        if other_sleeps.load(Ordering::SeqCst) != 0 {
            futex_wake(&self.state);
        }
    }

    /// Waits until the state is one of `a` or `b`, spinning `spin` times
    /// before sleeping, and answers which.
    fn wait_for(&self, a: u32, b: u32, spin: u32, sleeps: &AtomicU32) -> u32 {
        let mut left = spin;
        loop {
            let seen = self.state.load(Ordering::Acquire);
            if seen == a || seen == b {
                return seen;
            }
            if left > 0 {
                left -= 1;
                relax();
                continue;
            }
            sleeps.store(1, Ordering::SeqCst);
            let seen = self.state.load(Ordering::SeqCst);
            if seen != a && seen != b {
                futex_wait(&self.state, seen);
            }
            sleeps.store(0, Ordering::SeqCst);
        }
    }

    /// The host's half: posts `job(argument)` and waits for it, running on
    /// this thread every request the foreign side makes meanwhile through
    /// `serve`.
    ///
    /// # Safety
    ///
    /// The caller is the host thread paired with this slot for the call;
    /// `job` is a function of either world that is safe to run on the
    /// foreign side (code of the foreign C library, or code of this module),
    /// and `argument` is valid for it until this returns.
    pub(crate) unsafe fn host_run(
        &self,
        job: usize,
        argument: *mut c_void,
        spin: u32,
        serve: &dyn Fn(&Slot, *mut Request),
    ) {
        self.job.store(job, Ordering::Relaxed);
        self.argument.store(argument as usize, Ordering::Relaxed);
        self.set(JOB, &self.foreign_sleeps);
        loop {
            if self.wait_for(DONE, CALL, spin, &self.host_sleeps) == DONE {
                return;
            }
            let request = self.request.load(Ordering::Acquire) as *mut Request;
            serve(self, request);
            self.set(BACK, &self.foreign_sleeps);
        }
    }

    /// Runs the posted job on this (foreign) thread and answers it.
    ///
    /// # Safety
    ///
    /// The state is `JOB`, posted by `host_run`.
    unsafe fn run_posted(&self) {
        let job = self.job.load(Ordering::Acquire);
        let argument = self.argument.load(Ordering::Acquire) as *mut c_void;
        // SAFETY: `host_run`'s caller vouched for the job and its argument.
        let job: unsafe extern "C" fn(*mut c_void) = unsafe { core::mem::transmute(job) };
        // SAFETY: as above.
        unsafe { job(argument) };
        self.set(DONE, &self.host_sleeps);
    }

    /// The foreign side's half of a request: posts it and waits for the
    /// host, running every job the host posts meanwhile.
    ///
    /// # Safety
    ///
    /// The caller is the foreign thread paired with this slot, and the
    /// request is valid until this returns.
    pub(crate) unsafe fn foreign_call(&self, request: *mut Request, spin: u32) {
        self.request.store(request as usize, Ordering::Release);
        self.set(CALL, &self.host_sleeps);
        // SAFETY: the request is posted; the caller is the paired thread.
        unsafe { self.foreign_wait(spin) };
    }

    /// Waits for `BACK`, running jobs meanwhile.
    ///
    /// # Safety
    ///
    /// As [`Slot::foreign_call`], with the request already posted.
    pub(crate) unsafe fn foreign_wait(&self, spin: u32) {
        while self.wait_for(BACK, JOB, spin, &self.foreign_sleeps) == JOB {
            // SAFETY: the state is JOB.
            unsafe { self.run_posted() };
        }
    }

    /// The request a stray mailbox carries.
    pub(crate) fn request(&self) -> *mut Request {
        self.request.load(Ordering::Acquire) as *mut Request
    }

    /// The host's answer to a stray mailbox's request: posted once the host
    /// ran it.
    pub(crate) fn answer(&self) {
        self.set(BACK, &self.foreign_sleeps);
    }

    /// A mailbox for one request of a foreign thread no worker pairs: `CALL`
    /// from the start.
    pub(crate) fn stray(request: *mut Request) -> Slot {
        let slot = Slot::new();
        slot.state.store(CALL, Ordering::Relaxed);
        slot.request.store(request as usize, Ordering::Relaxed);
        slot.foreign_tp.store(thread_pointer(), Ordering::Relaxed);
        slot
    }

    /// A worker's loop: runs each job posted, for ever.
    ///
    /// # Safety
    ///
    /// The caller is this slot's worker.
    unsafe fn serve(&self, spin: u32) -> ! {
        loop {
            let mut left = spin;
            loop {
                let seen = self.state.load(Ordering::Acquire);
                if seen == JOB {
                    break;
                }
                if left > 0 {
                    left -= 1;
                    relax();
                    continue;
                }
                self.foreign_sleeps.store(1, Ordering::SeqCst);
                let seen = self.state.load(Ordering::SeqCst);
                if seen != JOB {
                    futex_wait(&self.state, seen);
                }
                self.foreign_sleeps.store(0, Ordering::SeqCst);
            }
            // SAFETY: the state is JOB.
            unsafe { self.run_posted() };
        }
    }
}

// --- the program ld.so enters ---------------------------------------------

/// Reads the control block's address from `argv[1]`, as hex.
///
/// # Safety
///
/// `argv` holds `argc` strings.
unsafe fn control_of(argc: usize, argv: *const *const u8) -> Option<&'static Control> {
    if argc < 2 {
        return None;
    }
    // SAFETY: argv[1] exists, a NUL-terminated string the loader wrote.
    let mut text = unsafe { *argv.add(1) };
    if text.is_null() {
        return None;
    }
    let mut value: usize = 0;
    let mut digits = 0;
    loop {
        // SAFETY: within the string, which ends in NUL.
        let c = unsafe { *text };
        let digit = match c {
            0 => break,
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            _ => return None,
        };
        value = value.wrapping_mul(16).wrapping_add(digit as usize);
        digits += 1;
        // SAFETY: the next byte is in the string or its NUL.
        text = unsafe { text.add(1) };
    }
    // SAFETY: the loader put the address of a leaked, live control block
    // here, and a value of no digits is no block.
    (digits > 0 && value != 0).then(|| unsafe { &*(value as *const Control) })
}

/// Parks the thread for good: what a bootstrap that cannot report does.
fn park() -> ! {
    let never = AtomicU32::new(0);
    loop {
        futex_wait(&never, 0);
    }
}

/// `ld.so`'s jump target once it has relocated the helper image: the stack
/// pointer at `argc`, and the finaliser `ld.so` hands a program's start.
/// Calls the C library's `__libc_start_main`, as a program's own start
/// would.
#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
pub(crate) unsafe extern "C" fn entry() {
    core::arch::naked_asm!(
        "xor ebp, ebp",
        "mov rdi, rsp",
        "mov rsi, rdx",
        "and rsp, -16",
        "call {boot}",
        "ud2",
        boot = sym boot,
    )
}

#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
pub(crate) unsafe extern "C" fn entry() {
    core::arch::naked_asm!(
        "mov x29, #0",
        "mov x30, #0",
        "mov x1, x0",
        "mov x0, sp",
        "bl {boot}",
        "brk #0",
        boot = sym boot,
    )
}

type StartMain = unsafe extern "C" fn(
    main: unsafe extern "C" fn(c_int, *mut *mut c_char, *mut *mut c_char) -> c_int,
    argc: c_int,
    argv: *mut *mut c_char,
    init: usize,
    fini: usize,
    rtld_fini: usize,
    stack_end: *mut c_void,
) -> c_int;

/// # Safety
///
/// Entered from [`entry`] on the stack `ld.so` was started on.
unsafe extern "C" fn boot(sp: *mut usize, rtld_fini: usize) -> ! {
    // SAFETY: the start-up block the loader built: argc, then argv.
    let argc = unsafe { *sp };
    // SAFETY: as above.
    let argv = unsafe { sp.add(1) } as *mut *mut c_char;
    // SAFETY: as above.
    let Some(control) = (unsafe { control_of(argc, argv as *const *const u8) }) else {
        park()
    };
    if control.size != size_of::<Control>() as u32 {
        control.fail(FAIL_LAYOUT);
        park();
    }
    // SAFETY: ld.so filled each import's slot before jumping here.
    let start = unsafe { control.import(Import::LibcStartMain) };
    if start == 0 {
        control.fail(FAIL_LAYOUT);
        park();
    }
    // SAFETY: `__libc_start_main` of the C library ld.so just started.
    let start: StartMain = unsafe { core::mem::transmute(start) };
    // SAFETY: what a program's own start passes; `main` never returns.
    unsafe { start(main, argc as c_int, argv, 0, 0, rtld_fini, sp.cast()) };
    park()
}

type PthreadCreate = unsafe extern "C" fn(
    *mut usize,
    *const c_void,
    unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    *mut c_void,
) -> c_int;
type AttrInit = unsafe extern "C" fn(*mut c_void) -> c_int;
type AttrStack = unsafe extern "C" fn(*mut c_void, usize) -> c_int;
type SigMask = unsafe extern "C" fn(c_int, *const c_void, *mut c_void) -> c_int;

/// The program's `main`, on the thread ld.so started on, with the C library
/// initialised: starts the other workers, finds where the image's
/// thread-local mark lies, reports ready, and serves the first mailbox.
unsafe extern "C" fn main(argc: c_int, argv: *mut *mut c_char, _env: *mut *mut c_char) -> c_int {
    // SAFETY: the same argv `boot` read.
    let Some(control) = (unsafe { control_of(argc as usize, argv as *const *const u8) }) else {
        park()
    };
    // SAFETY: the import slots were filled before `boot`.
    unsafe { control.copy_imports() };

    // The mark the image's thread-local block starts with: somewhere near
    // the thread pointer, at the same offset on every thread of this C
    // library.
    let tp = thread_pointer();
    let mut found = None;
    // Outward from the thread pointer, so the first match is found having
    // read only what lies between: the program's block is the first in
    // the static area, a few words from the thread pointer on either
    // architecture.
    let mut step: isize = 0;
    while step <= 512 {
        for offset in [-step, step] {
            let at = tp.wrapping_add_signed(offset) as *const u64;
            // SAFETY: between the thread pointer and the program's block,
            // inside the C library's thread block for this thread.
            let words = unsafe { [at.read(), at.add(1).read()] };
            if words == control.mark {
                found = Some(offset);
                break;
            }
        }
        if found.is_some() {
            break;
        }
        step += 8;
    }
    let Some(offset) = found else {
        control.fail(FAIL_TLS);
        park()
    };
    control.tls_offset.store(offset, Ordering::Release);

    // Every worker inherits the mask of the thread that creates it.
    let mut set = [0u64; 16];
    set[0] = control.blocked;
    // SAFETY: the C library's pthread_sigmask on a sigset_t (128 bytes).
    let mask: SigMask = unsafe { core::mem::transmute(control.function(Import::PthreadSigmask)) };
    // SAFETY: as above.
    unsafe { mask(2, set.as_ptr().cast(), core::ptr::null_mut()) };

    let slots = control.slots();
    let first = &slots[0];
    first.tid.store(gettid(), Ordering::SeqCst);
    first.foreign_tp.store(tp, Ordering::SeqCst);
    let initial = control.initial as usize;
    let mut i = 1;
    while i <= initial {
        // SAFETY: the C library is started and its imports copied.
        let err = unsafe { spawn(control, i) };
        if err != 0 {
            control.fail(err);
            park();
        }
        i += 1;
    }
    loop {
        let started = control.started.load(Ordering::SeqCst);
        if started as usize == initial {
            break;
        }
        futex_wait(&control.started, started);
    }
    control.ready.store(READY, Ordering::SeqCst);
    futex_wake(&control.ready);
    // This thread runs no library code: it starts workers when the host
    // asks for more.
    // SAFETY: this thread is the first slot's.
    unsafe { first.serve(control.spin) }
}

/// Starts the worker of slot `index`; answers the C library's error number.
///
/// # Safety
///
/// On a foreign thread, the imports copied, `index` a slot of the block.
unsafe fn spawn(control: &Control, index: usize) -> c_int {
    let Some(slot) = control.slots().get(index) else {
        return 22;
    };
    // SAFETY: the C library's own functions, through their pinned symbols.
    let (create, init, stack, destroy): (PthreadCreate, AttrInit, AttrStack, AttrInit) = unsafe {
        (
            core::mem::transmute::<usize, PthreadCreate>(control.function(Import::PthreadCreate)),
            core::mem::transmute::<usize, AttrInit>(control.function(Import::PthreadAttrInit)),
            core::mem::transmute::<usize, AttrStack>(
                control.function(Import::PthreadAttrSetstacksize),
            ),
            core::mem::transmute::<usize, AttrInit>(control.function(Import::PthreadAttrDestroy)),
        )
    };
    // pthread_attr_t is 56 bytes on x86_64 and 64 on aarch64.
    let mut attr = [0u64; 16];
    let mut thread = 0usize;
    let attr_at = attr.as_mut_ptr().cast::<c_void>();
    // SAFETY: an attribute block larger than the C library's.
    let mut err = unsafe { init(attr_at) };
    if err == 0 && control.worker_stack != 0 {
        // SAFETY: as above.
        err = unsafe { stack(attr_at, control.worker_stack as usize) };
    }
    if err == 0 {
        // SAFETY: the slot outlives the process; `worker` is TLS-free.
        err = unsafe {
            create(
                &mut thread,
                attr_at,
                worker,
                core::ptr::from_ref(slot).cast_mut().cast(),
            )
        };
    }
    // SAFETY: as above.
    unsafe { destroy(attr_at) };
    err
}

/// Starts one more worker and waits until it runs, on the thread `ld.so`
/// started on.
///
/// # Safety
///
/// `argument` is a [`SpawnRequest`] alive until the job is answered.
pub(crate) unsafe extern "C" fn spawn_job(argument: *mut c_void) {
    // SAFETY: the caller's.
    let r = unsafe { &mut *(argument as *mut SpawnRequest) };
    // SAFETY: the request names the block it was posted through.
    let control = unsafe { &*r.control };
    // SAFETY: this is the first slot's thread.
    r.error = unsafe { spawn(control, r.index) };
    if r.error != 0 {
        return;
    }
    let Some(slot) = control.slots().get(r.index) else {
        return;
    };
    loop {
        let started = control.started.load(Ordering::SeqCst);
        if slot.tid.load(Ordering::SeqCst) != 0 {
            break;
        }
        futex_wait(&control.started, started);
    }
}

/// A worker after the first.
unsafe extern "C" fn worker(argument: *mut c_void) -> *mut c_void {
    // SAFETY: `main` passes a slot of the leaked array.
    let slot = unsafe { &*(argument as *const Slot) };
    slot.foreign_tp.store(thread_pointer(), Ordering::SeqCst);
    slot.tid.store(gettid(), Ordering::SeqCst);
    // SAFETY: the control block is leaked and its address was in argv.
    let control = unsafe { &*slot_control(slot) };
    control.started.fetch_add(1, Ordering::SeqCst);
    futex_wake(&control.started);
    // SAFETY: this thread is the slot's worker.
    unsafe { slot.serve(control.spin) }
}

/// The control block a worker's slot belongs to: kept in the slot's `next`
/// word, which only stray mailboxes otherwise use.
fn slot_control(slot: &Slot) -> *const Control {
    slot.next.load(Ordering::Acquire) as *const Control
}

// --- jobs ---------------------------------------------------------------------

type DlOpen = unsafe extern "C" fn(*const c_char, c_int) -> *mut c_void;
type DlSym = unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void;
type DlError = unsafe extern "C" fn() -> *const c_char;
type DlClose = unsafe extern "C" fn(*mut c_void) -> c_int;
type DlAddr1 =
    unsafe extern "C" fn(*const c_void, *mut [usize; 4], *mut *const c_void, c_int) -> c_int;

fn note(r: &mut DlRequest, message: *const c_char) {
    r.status = 1;
    let mut n = 0usize;
    if !message.is_null() && !r.error.is_null() {
        loop {
            if n as u64 >= r.error_cap {
                break;
            }
            // SAFETY: `dlerror`'s string, NUL-terminated.
            let c = unsafe { *message.cast::<u8>().add(n) };
            if c == 0 {
                break;
            }
            // SAFETY: within the host's buffer of `error_cap` bytes.
            unsafe { *r.error.add(n) = c };
            n += 1;
        }
    }
    r.error_len = n as u64;
}

/// The dynamic linker's requests, run on a worker.
///
/// # Safety
///
/// `argument` is a [`DlRequest`] whose strings and buffer live until the
/// job is answered.
pub(crate) unsafe extern "C" fn dl_job(argument: *mut c_void) {
    // SAFETY: the caller's.
    let r = unsafe { &mut *(argument as *mut DlRequest) };
    // SAFETY: the request names the control block it was posted through.
    let control = unsafe { &*r.control };
    r.status = 0;
    r.error_len = 0;
    // SAFETY: the C library's functions, filled by ld.so.
    let (open, sym, error, close, addr1): (DlOpen, DlSym, DlError, DlClose, DlAddr1) = unsafe {
        (
            core::mem::transmute::<usize, DlOpen>(control.function(Import::Dlopen)),
            core::mem::transmute::<usize, DlSym>(control.function(Import::Dlsym)),
            core::mem::transmute::<usize, DlError>(control.function(Import::Dlerror)),
            core::mem::transmute::<usize, DlClose>(control.function(Import::Dlclose)),
            core::mem::transmute::<usize, DlAddr1>(control.function(Import::Dladdr1)),
        )
    };
    // SAFETY: each call is the C library's, with the request's arguments.
    unsafe {
        match r.op {
            DL_OPEN => {
                error();
                r.handle = open(r.name.cast(), r.flags);
                if r.handle.is_null() {
                    note(r, error());
                }
            }
            DL_SYMBOL => {
                error();
                r.address = sym(r.handle, r.name.cast());
                r.size = 0;
                let message = error();
                if !message.is_null() {
                    note(r, message);
                    return;
                }
                let mut info = [0usize; 4];
                let mut symbol: *const c_void = core::ptr::null();
                // RTLD_DL_SYMENT: the symbol's own entry, whose `st_size`
                // (offset 16 in an Elf64_Sym) bounds what is read.
                if !r.address.is_null()
                    && addr1(r.address, &mut info, &mut symbol, 1) != 0
                    && !symbol.is_null()
                {
                    r.size = symbol.cast::<u8>().add(16).cast::<u64>().read_unaligned();
                }
            }
            DL_CLOSE => {
                if close(r.handle) != 0 {
                    note(r, error());
                }
            }
            _ => note(r, c"unknown request".as_ptr()),
        }
    }
}

/// `dl_iterate_phdr`'s view of one object: its load bias, name and program
/// headers.
#[repr(C)]
struct PhdrInfo {
    addr: usize,
    name: *const c_char,
    phdr: *const u8,
    phnum: u16,
}

type Iterate = unsafe extern "C" fn(
    unsafe extern "C" fn(*const PhdrInfo, usize, *mut c_void) -> c_int,
    *mut c_void,
) -> c_int;

/// Each loaded segment of every object the foreign linker holds, recorded
/// into the request's buffer, run on a worker.
///
/// # Safety
///
/// `argument` is a [`PhdrRequest`] whose buffer lives until the job is
/// answered.
pub(crate) unsafe extern "C" fn phdr_job(argument: *mut c_void) {
    // SAFETY: the caller's.
    let r = unsafe { &mut *(argument as *mut PhdrRequest) };
    r.found = 0;
    r.objects = 0;
    // SAFETY: as in `dl_job`.
    let iterate: Iterate =
        unsafe { core::mem::transmute((*r.control).function(Import::DlIteratePhdr)) };
    // SAFETY: the C library's walk, with this module's callback.
    unsafe { iterate(each_object, argument) };
}

unsafe extern "C" fn each_object(info: *const PhdrInfo, _size: usize, data: *mut c_void) -> c_int {
    // SAFETY: `phdr_job`'s request, and the C library's record.
    let (r, info) = unsafe { (&mut *(data as *mut PhdrRequest), &*info) };
    let object = r.objects;
    r.objects = r.objects.wrapping_add(1);
    let mut i = 0u16;
    while i < info.phnum {
        // An Elf64_Phdr: type, flags, offset, vaddr, paddr, filesz, memsz.
        // SAFETY: the object's own program headers, `phnum` of them.
        let (kind, flags, vaddr, memsz) = unsafe {
            let p = info.phdr.add(i as usize * 56);
            (
                p.cast::<u32>().read_unaligned(),
                p.add(4).cast::<u32>().read_unaligned(),
                p.add(16).cast::<u64>().read_unaligned(),
                p.add(40).cast::<u64>().read_unaligned(),
            )
        };
        i += 1;
        if kind != 1 {
            continue;
        }
        if r.found < r.capacity {
            let start = info.addr.wrapping_add(vaddr as usize);
            // SAFETY: within the host's buffer of `capacity` entries.
            unsafe {
                r.buffer.add(r.found).write([
                    object,
                    start,
                    start.wrapping_add(memsz as usize),
                    flags as usize,
                ])
            };
        }
        r.found = r.found.wrapping_add(1);
    }
    0
}

// --- a call made through a stub --------------------------------------------

/// The registers a call carries, and a window of the caller's stack: what
/// a stub saves so the call can be made again on another thread.
///
/// x86_64 keeps `rdi rsi rdx rcx r8 r9 rax` in `gp`, `xmm0..7` in `fp`,
/// and answers in `rax rdx` and `xmm0 xmm1`; aarch64 `x0..x8`, `q0..q7`,
/// and answers in `x0 x1` and `q0..q3`.
#[repr(C, align(16))]
pub(crate) struct Frame {
    /// What to call.
    pub(crate) target: usize,
    /// The caller's arguments on the stack, as the callee would see them.
    pub(crate) stack: usize,
    /// How many bytes of them to copy.
    pub(crate) stack_len: usize,
    pub(crate) gp: [u64; 9],
    pub(crate) fp: [[u64; 2]; 8],
    pub(crate) ret_gp: [u64; 2],
    pub(crate) ret_fp: [[u64; 2]; 4],
}

/// Makes the call a frame describes on this thread: loads the argument
/// registers, copies the stack window below the stack pointer, calls, and
/// stores the answer registers.
///
/// # Safety
///
/// `frame` is a frame a stub saved, its target code of this thread's world,
/// and `stack_len` a multiple of 16.
#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
pub(crate) unsafe extern "C" fn invoke(frame: *mut Frame) {
    core::arch::naked_asm!(
        "push rbp",
        "mov rbp, rsp",
        "push rbx",
        "push r12",
        "mov rbx, rdi",
        "mov rcx, [rbx + 16]",
        "sub rsp, rcx",
        "and rsp, -16",
        "mov rdi, rsp",
        "mov rsi, [rbx + 8]",
        "rep movsb",
        "movdqu xmm0, [rbx + 96]",
        "movdqu xmm1, [rbx + 112]",
        "movdqu xmm2, [rbx + 128]",
        "movdqu xmm3, [rbx + 144]",
        "movdqu xmm4, [rbx + 160]",
        "movdqu xmm5, [rbx + 176]",
        "movdqu xmm6, [rbx + 192]",
        "movdqu xmm7, [rbx + 208]",
        "mov rdi, [rbx + 24]",
        "mov rsi, [rbx + 32]",
        "mov rdx, [rbx + 40]",
        "mov rcx, [rbx + 48]",
        "mov r8, [rbx + 56]",
        "mov r9, [rbx + 64]",
        "mov rax, [rbx + 72]",
        "call qword ptr [rbx]",
        "mov [rbx + 224], rax",
        "mov [rbx + 232], rdx",
        "movdqu [rbx + 240], xmm0",
        "movdqu [rbx + 256], xmm1",
        "lea rsp, [rbp - 16]",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "ret",
    )
}

#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
pub(crate) unsafe extern "C" fn invoke(frame: *mut Frame) {
    core::arch::naked_asm!(
        "stp x29, x30, [sp, #-32]!",
        "mov x29, sp",
        "str x19, [sp, #16]",
        "mov x19, x0",
        "ldr x9, [x19, #16]",
        "mov x10, sp",
        "sub x10, x10, x9",
        "and x10, x10, #-16",
        "mov sp, x10",
        "ldr x11, [x19, #8]",
        "mov x12, sp",
        "cbz x9, 2f",
        "1:",
        "ldr x13, [x11], #8",
        "str x13, [x12], #8",
        "subs x9, x9, #8",
        "b.hi 1b",
        "2:",
        "ldp q0, q1, [x19, #96]",
        "ldp q2, q3, [x19, #128]",
        "ldp q4, q5, [x19, #160]",
        "ldp q6, q7, [x19, #192]",
        "ldp x0, x1, [x19, #24]",
        "ldp x2, x3, [x19, #40]",
        "ldp x4, x5, [x19, #56]",
        "ldp x6, x7, [x19, #72]",
        "ldr x8, [x19, #88]",
        "ldr x16, [x19]",
        "blr x16",
        "stp x0, x1, [x19, #224]",
        "stp q0, q1, [x19, #240]",
        "stp q2, q3, [x19, #272]",
        "mov sp, x29",
        "ldr x19, [sp, #16]",
        "ldp x29, x30, [sp], #32",
        "ret",
    )
}

/// Where a stub jumps, with its target in `r10` (x86_64) or `x16`
/// (aarch64): saves the call into a [`Frame`] on this stack, has
/// [`super::world::forward`] make it, and returns its answer.
#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
pub(crate) unsafe extern "C" fn stub_entry() {
    core::arch::naked_asm!(
        "push rbp",
        "mov rbp, rsp",
        "sub rsp, 304",
        "mov [rsp], r10",
        "lea r11, [rbp + 16]",
        "mov [rsp + 8], r11",
        "mov [rsp + 24], rdi",
        "mov [rsp + 32], rsi",
        "mov [rsp + 40], rdx",
        "mov [rsp + 48], rcx",
        "mov [rsp + 56], r8",
        "mov [rsp + 64], r9",
        "mov [rsp + 72], rax",
        "movdqu [rsp + 96], xmm0",
        "movdqu [rsp + 112], xmm1",
        "movdqu [rsp + 128], xmm2",
        "movdqu [rsp + 144], xmm3",
        "movdqu [rsp + 160], xmm4",
        "movdqu [rsp + 176], xmm5",
        "movdqu [rsp + 192], xmm6",
        "movdqu [rsp + 208], xmm7",
        "mov rdi, rsp",
        "call {forward}",
        "mov rax, [rsp + 224]",
        "mov rdx, [rsp + 232]",
        "movdqu xmm0, [rsp + 240]",
        "movdqu xmm1, [rsp + 256]",
        "leave",
        "ret",
        forward = sym super::world::forward,
    )
}

#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
pub(crate) unsafe extern "C" fn stub_entry() {
    core::arch::naked_asm!(
        "stp x29, x30, [sp, #-16]!",
        "mov x29, sp",
        "sub sp, sp, #304",
        "str x16, [sp]",
        "add x9, x29, #16",
        "str x9, [sp, #8]",
        "stp x0, x1, [sp, #24]",
        "stp x2, x3, [sp, #40]",
        "stp x4, x5, [sp, #56]",
        "stp x6, x7, [sp, #72]",
        "str x8, [sp, #88]",
        "stp q0, q1, [sp, #96]",
        "stp q2, q3, [sp, #128]",
        "stp q4, q5, [sp, #160]",
        "stp q6, q7, [sp, #192]",
        "mov x0, sp",
        "bl {forward}",
        "ldp x0, x1, [sp, #224]",
        "ldp q0, q1, [sp, #240]",
        "ldp q2, q3, [sp, #272]",
        "mov sp, x29",
        "ldp x29, x30, [sp], #16",
        "ret",
        forward = sym super::world::forward,
    )
}

/// Makes the code just written to `[at, at + len)` visible to instruction
/// fetch, which aarch64 does not do by itself.
#[cfg(target_arch = "aarch64")]
pub(crate) fn sync_code(at: usize, len: usize) {
    let ctr: u64;
    // SAFETY: reads the cache type register, which EL0 may.
    unsafe { core::arch::asm!("mrs {}, ctr_el0", out(reg) ctr, options(nomem, nostack)) };
    let dline = 4usize << ((ctr >> 16) & 0xf);
    let iline = 4usize << (ctr & 0xf);
    let end = at.wrapping_add(len);
    let mut p = at & !(dline - 1);
    while p < end {
        // SAFETY: cleans a line of memory this process just wrote.
        unsafe { core::arch::asm!("dc cvau, {}", in(reg) p, options(nostack)) };
        p += dline;
    }
    // SAFETY: barriers.
    unsafe { core::arch::asm!("dsb ish", options(nostack)) };
    let mut p = at & !(iline - 1);
    while p < end {
        // SAFETY: invalidates the instruction cache for that line.
        unsafe { core::arch::asm!("ic ivau, {}", in(reg) p, options(nostack)) };
        p += iline;
    }
    // SAFETY: barriers.
    unsafe { core::arch::asm!("dsb ish", "isb", options(nostack)) };
}

/// x86_64 keeps instruction fetch coherent with stores.
#[cfg(target_arch = "x86_64")]
pub(crate) fn sync_code(_at: usize, _len: usize) {}

/// The size and offsets [`Frame`]'s users rely on.
const _: () = {
    assert!(size_of::<Frame>() == 304);
    assert!(core::mem::offset_of!(Frame, gp) == 24);
    assert!(core::mem::offset_of!(Frame, fp) == 96);
    assert!(core::mem::offset_of!(Frame, ret_gp) == 224);
    assert!(core::mem::offset_of!(Frame, ret_fp) == 240);
};
