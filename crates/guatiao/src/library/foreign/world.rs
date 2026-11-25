// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The foreign C library once started: its workers, the calls between the
//! two sides, and the stubs that make a foreign function callable from host
//! code.
//!
//! **One per process.** The foreign C library is process-wide by nature:
//! it takes a signal for itself, shares the program break, and its threads
//! carry its thread pointer. So the started loader is kept in one place
//! every registry and every call into the host can find, which is the one
//! piece of process-wide state this crate holds. A second copy of the crate
//! in one program would start a second C library, which the reserved-signal
//! check refuses by default.
//!
//! # Which side a thread is on
//!
//! The program `ld.so` starts carries a 16-byte thread-local block holding
//! a random mark, which the foreign C library copies into every thread it
//! makes, the library's own included. A thread whose block holds the mark
//! is foreign. That check reads two words near the thread pointer and no
//! thread-local of the host's, so it runs on either side.
//!
//! # Calls
//!
//! Host code calls a foreign function through a stub (`stub_entry`), which
//! saves the call's registers and stack arguments into a [`Frame`] and has
//! [`forward`] make it: on a worker, posted through its mailbox, when the
//! caller is a host thread; directly, when the caller is already foreign.
//! Foreign code reaches host code only through this crate's own entry
//! points (every slot of a kind and every C export go through
//! `exports::guard`, the allocators and the host services likewise), each of
//! which asks [`away`] first and, on a foreign thread, runs its body on a
//! host thread through [`World::home`]: the host thread that posted the
//! call the worker is running, or, for a thread the library made, a host
//! thread kept for such calls.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicPtr, AtomicU32, AtomicUsize, Ordering};
use std::path::PathBuf;
use std::sync::{Condvar, Mutex, PoisonError, RwLock};

use super::bare::{self, Frame, Request, Slot, futex_wait, futex_wake, thread_pointer};
use super::error::Error;
use super::settings::{OpenFlags, SignalSet};
use super::sys;
use super::worker::{Control, SpawnRequest};

/// The started loader, once there is one.
static WORLD: AtomicPtr<World> = AtomicPtr::new(core::ptr::null_mut());

/// The loader, if it was started in this process.
pub(crate) fn world() -> Option<&'static World> {
    let w = WORLD.load(Ordering::Acquire);
    // SAFETY: a non-null pointer is a leaked `World`, published once.
    (!w.is_null()).then(|| unsafe { &*w })
}

/// Publishes the loader; the first one stays.
pub(crate) fn publish(world: &'static World) -> &'static World {
    match WORLD.compare_exchange(
        core::ptr::null_mut(),
        core::ptr::from_ref(world).cast_mut(),
        Ordering::AcqRel,
        Ordering::Acquire,
    ) {
        Ok(_) => world,
        // SAFETY: as in `world`.
        Err(first) => unsafe { &*first },
    }
}

/// The loader, when this thread is one of the foreign C library's.
/// Reads no thread-local; safe to ask on any thread.
#[inline]
pub(crate) fn away() -> Option<&'static World> {
    let w = world()?;
    w.on_foreign_thread().then_some(w)
}

/// What a host thread is running for a foreign one: which mailbox its
/// nested calls go back through.
struct Serving {
    host_tp: usize,
    slot: usize,
}

/// Host threads kept for calls a library makes from threads of its own.
struct Runner {
    job: Mutex<Option<usize>>,
    ready: Condvar,
}

/// The loader's state. Leaked when started; never dropped.
pub(crate) struct World {
    pub(crate) control: &'static Control,
    /// Slot 0 is the thread `ld.so` started on, which only starts workers;
    /// workers are 1 to `active`.
    pub(crate) slots: &'static [Slot],
    locks: Box<[Mutex<()>]>,
    active: AtomicUsize,
    spawning: Mutex<()>,
    next: AtomicUsize,
    pub(crate) forks: u64,
    pub(crate) spin: u32,
    stack_arguments: usize,
    pub(crate) ld_so: PathBuf,
    pub(crate) taken: SignalSet,
    pub(crate) error_capacity: usize,
    pub(crate) declaration_window: usize,
    pub(crate) read_unsized_declaration: bool,
    pub(crate) open_flags: OpenFlags,
    mark: [u64; 2],
    tls_offset: isize,
    serving: Mutex<Vec<Serving>>,
    serving_count: AtomicUsize,
    /// Stray mailboxes waiting for a host thread: a list through their
    /// `next` words, pushed from foreign threads.
    strays: AtomicUsize,
    stray_posts: AtomicU32,
    idle: Mutex<Vec<Arc<Runner>>>,
    runners: AtomicUsize,
    callback_threads: Option<usize>,
    waiting: Mutex<Vec<usize>>,
    /// Every executable segment the foreign linker holds, by start.
    code: RwLock<BTreeMap<usize, usize>>,
    stubs: Mutex<BTreeMap<usize, usize>>,
    /// Tables copied with their foreign slots made callable, by the
    /// original's address and size: the original's bytes, and the copy.
    tables: Mutex<Copies>,
}

// SAFETY: the raw parts are leaked blocks shared with the foreign side
// through atomics; everything else is behind locks.
unsafe impl Sync for World {}
// SAFETY: as above.
unsafe impl Send for World {}

/// Translated table copies, by the original's address and size: the
/// original's bytes when copied, and the copy's address.
type Copies = BTreeMap<(usize, usize), (Vec<u8>, usize)>;

/// What the start hands over.
pub(crate) struct Parts {
    pub(crate) control: &'static Control,
    pub(crate) slots: &'static [Slot],
    pub(crate) workers: usize,
    pub(crate) forks: u64,
    pub(crate) spin: u32,
    pub(crate) stack_arguments: usize,
    pub(crate) ld_so: PathBuf,
    pub(crate) taken: SignalSet,
    pub(crate) error_capacity: usize,
    pub(crate) declaration_window: usize,
    pub(crate) read_unsized_declaration: bool,
    pub(crate) open_flags: OpenFlags,
    pub(crate) mark: [u64; 2],
    pub(crate) tls_offset: isize,
    pub(crate) callback_threads: Option<usize>,
}

impl World {
    pub(crate) fn new(p: Parts) -> World {
        World {
            control: p.control,
            slots: p.slots,
            locks: (0..p.slots.len()).map(|_| Mutex::new(())).collect(),
            active: AtomicUsize::new(p.workers),
            spawning: Mutex::new(()),
            next: AtomicUsize::new(0),
            forks: p.forks,
            spin: p.spin,
            stack_arguments: p.stack_arguments,
            ld_so: p.ld_so,
            taken: p.taken,
            error_capacity: p.error_capacity,
            declaration_window: p.declaration_window,
            read_unsized_declaration: p.read_unsized_declaration,
            open_flags: p.open_flags,
            mark: p.mark,
            tls_offset: p.tls_offset,
            serving: Mutex::new(Vec::new()),
            serving_count: AtomicUsize::new(0),
            strays: AtomicUsize::new(0),
            stray_posts: AtomicU32::new(0),
            idle: Mutex::new(Vec::new()),
            runners: AtomicUsize::new(0),
            callback_threads: p.callback_threads,
            waiting: Mutex::new(Vec::new()),
            code: RwLock::new(BTreeMap::new()),
            stubs: Mutex::new(BTreeMap::new()),
            tables: Mutex::new(BTreeMap::new()),
        }
    }

    /// Whether this thread is the foreign C library's: its thread-local
    /// block holds the mark. TLS-free.
    #[inline]
    pub(crate) fn on_foreign_thread(&self) -> bool {
        let at = thread_pointer().wrapping_add_signed(self.tls_offset) as *const u64;
        // SAFETY: a few bytes from the thread pointer, inside the thread's
        // own block on either side: the foreign C library's static
        // thread-local area, or the host's thread block and thread-locals
        // around its own thread pointer.
        unsafe { at.read_volatile() == self.mark[0] && at.add(1).read_volatile() == self.mark[1] }
    }

    /// The workers started so far.
    pub(crate) fn workers(&self) -> &'static [Slot] {
        let n = self.active.load(Ordering::Acquire);
        &self.slots[1..=n]
    }

    /// Answers `Forked` in a child of the process that started the loader.
    pub(crate) fn check_fork(&self) -> Result<(), Error> {
        if sys::forks_seen() != self.forks {
            Err(Error::Forked)
        } else {
            Ok(())
        }
    }

    // --- host threads calling the foreign side ------------------------------

    /// Runs `job(argument)` on the foreign side and waits for it: on the
    /// foreign thread this host thread is answering a call for, if it is;
    /// else on a free worker, starting one if every worker is busy and
    /// there may be more, else waiting for one.
    ///
    /// # Safety
    ///
    /// `job` is code that may run on a foreign thread (the foreign C
    /// library's world, or this module's TLS-free code), and `argument` is
    /// what it expects, valid until this returns.
    pub(crate) unsafe fn call(
        &'static self,
        job: usize,
        argument: *mut c_void,
    ) -> Result<(), Error> {
        self.check_fork()?;
        let serve = |slot: &Slot, request: *mut Request| self.serve(slot, request);
        if self.serving_count.load(Ordering::Acquire) > 0 {
            let tp = thread_pointer();
            let back = self
                .serving
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .rev()
                .find(|s| s.host_tp == tp)
                .map(|s| s.slot);
            if let Some(slot) = back {
                // SAFETY: the foreign thread of that mailbox waits for this
                // thread's answer and runs what it posts meanwhile.
                unsafe { (*(slot as *const Slot)).host_run(job, argument, self.spin, &serve) };
                return Ok(());
            }
        }
        let index = self.acquire()?;
        let _held = index.1;
        // SAFETY: the lock makes this thread the slot's only poster; the
        // rest is the caller's.
        unsafe { self.slots[index.0].host_run(job, argument, self.spin, &serve) };
        Ok(())
    }

    /// A free worker's index, held.
    fn acquire(&'static self) -> Result<(usize, std::sync::MutexGuard<'static, ()>), Error> {
        loop {
            let n = self.active.load(Ordering::Acquire);
            let first = self.next.fetch_add(1, Ordering::Relaxed) % n;
            for i in 0..n {
                let index = 1 + (first + i) % n;
                if let Ok(held) = self.locks[index].try_lock() {
                    return Ok((index, held));
                }
            }
            if n + 1 < self.slots.len() {
                let _one = self.spawning.lock().unwrap_or_else(PoisonError::into_inner);
                if self.active.load(Ordering::Acquire) == n {
                    self.spawn(n + 1)?;
                    self.active.store(n + 1, Ordering::Release);
                }
                continue;
            }
            let index = 1 + first;
            let held = self.locks[index]
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            return Ok((index, held));
        }
    }

    /// Starts the worker for slot `index`, on the thread `ld.so` started on.
    fn spawn(&'static self, index: usize) -> Result<(), Error> {
        let mut request = SpawnRequest {
            control: self.control,
            index,
            error: 0,
        };
        let _only = self.locks[0].lock().unwrap_or_else(PoisonError::into_inner);
        let serve = |slot: &Slot, request: *mut Request| self.serve(slot, request);
        // SAFETY: `spawn_job` is this module's TLS-free job; the request
        // outlives the call.
        unsafe {
            self.slots[0].host_run(
                bare::spawn_job as *const () as usize,
                (&mut request as *mut SpawnRequest).cast(),
                self.spin,
                &serve,
            )
        };
        if request.error != 0 {
            return Err(Error::Spawn(request.error));
        }
        Ok(())
    }

    /// Runs a request a foreign thread made, on this host thread, with this
    /// thread's own calls back to the foreign side going to that thread.
    fn serve(&self, slot: &Slot, request: *mut Request) {
        let tp = thread_pointer();
        self.serving
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Serving {
                host_tp: tp,
                slot: core::ptr::from_ref(slot) as usize,
            });
        self.serving_count.fetch_add(1, Ordering::AcqRel);
        let ran = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| {
            // SAFETY: the foreign side posted a request of this module's,
            // valid until answered.
            unsafe { ((*request).run)((*request).data) }
        }));
        self.serving_count.fetch_sub(1, Ordering::AcqRel);
        {
            let mut serving = self.serving.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(at) = serving.iter().rposition(|s| s.host_tp == tp) {
                serving.remove(at);
            }
        }
        if ran.is_err() {
            // Nothing can carry a panic back into the foreign caller, which
            // is still waiting for its answer.
            std::eprintln!("guatiao: a call from a foreign thread panicked on the host; aborting");
            std::process::abort();
        }
    }

    // --- foreign threads calling the host -----------------------------------

    /// Runs `f` on a host thread and answers what it returned; called on a
    /// foreign thread (see [`away`]). TLS-free until `f` runs, on the host.
    pub(crate) fn home<R, F: FnOnce() -> R>(&'static self, f: F) -> R {
        struct Data<F, R> {
            f: core::mem::ManuallyDrop<Option<F>>,
            out: Option<R>,
        }
        /// # Safety
        ///
        /// `data` is the `Data<F, R>` of the `home` waiting for it.
        unsafe fn run<R, F: FnOnce() -> R>(data: *mut c_void) {
            // SAFETY: the caller's.
            let d = unsafe { &mut *(data as *mut Data<F, R>) };
            if let Some(f) = d.f.take() {
                d.out = Some(f());
            }
        }
        let mut data = Data {
            f: core::mem::ManuallyDrop::new(Some(f)),
            out: None,
        };
        let mut request = Request {
            run: run::<R, F>,
            data: (&mut data as *mut Data<F, R>).cast(),
        };
        let tp = thread_pointer();
        let mut worker = None;
        for slot in self.workers() {
            if slot.foreign_tp.load(Ordering::Acquire) == tp {
                worker = Some(slot);
                break;
            }
        }
        match worker {
            // SAFETY: this thread is that slot's worker, inside a job its
            // host thread waits for.
            Some(slot) => unsafe { slot.foreign_call(&mut request, self.spin) },
            None => {
                let mailbox = Slot::stray(&mut request);
                self.push_stray(&mailbox);
                // SAFETY: this thread made the mailbox; a host thread answers.
                unsafe { mailbox.foreign_wait(self.spin) };
            }
        }
        match data.out.take() {
            Some(r) => r,
            // SAFETY: the host ran the request before answering it.
            None => unsafe { core::hint::unreachable_unchecked() },
        }
    }

    /// Hands a stray mailbox to the host threads kept for them. TLS-free.
    fn push_stray(&self, mailbox: &Slot) {
        let at = core::ptr::from_ref(mailbox) as usize;
        let mut head = self.strays.load(Ordering::Acquire);
        loop {
            mailbox.next.store(head, Ordering::Relaxed);
            match self
                .strays
                .compare_exchange_weak(head, at, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => break,
                Err(seen) => head = seen,
            }
        }
        self.stray_posts.fetch_add(1, Ordering::SeqCst);
        futex_wake(&self.stray_posts);
    }

    /// The host thread that hands stray mailboxes to runners, for ever.
    pub(crate) fn dispatch(&'static self) -> ! {
        loop {
            let seen = self.stray_posts.load(Ordering::SeqCst);
            let mut head = self.strays.swap(0, Ordering::AcqRel);
            if head == 0 {
                futex_wait(&self.stray_posts, seen);
                continue;
            }
            // Pushed last first; answer the oldest first.
            let mut order = Vec::new();
            while head != 0 {
                order.push(head);
                // SAFETY: a mailbox on a waiting foreign thread's stack.
                head = unsafe { &*(head as *const Slot) }
                    .next
                    .load(Ordering::Acquire);
            }
            for mailbox in order.into_iter().rev() {
                self.assign(mailbox);
            }
        }
    }

    fn assign(&'static self, mailbox: usize) {
        if let Some(runner) = self
            .idle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop()
        {
            *runner.job.lock().unwrap_or_else(PoisonError::into_inner) = Some(mailbox);
            runner.ready.notify_one();
            return;
        }
        let limit = self.callback_threads.unwrap_or(usize::MAX);
        if self.runners.load(Ordering::Acquire) >= limit {
            self.waiting
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(mailbox);
            return;
        }
        self.runners.fetch_add(1, Ordering::AcqRel);
        let runner = Arc::new(Runner {
            job: Mutex::new(Some(mailbox)),
            ready: Condvar::new(),
        });
        let spawned = std::thread::Builder::new()
            .name("guatiao-foreign-callback".into())
            .spawn(move || self.run(runner));
        if spawned.is_err() {
            std::eprintln!("guatiao: no host thread for a foreign thread's call; aborting");
            std::process::abort();
        }
    }

    /// A runner: answers one stray mailbox after another.
    fn run(&'static self, me: Arc<Runner>) -> ! {
        loop {
            let mailbox = {
                let mut job = me.job.lock().unwrap_or_else(PoisonError::into_inner);
                loop {
                    if let Some(m) = job.take() {
                        break m;
                    }
                    job = me.ready.wait(job).unwrap_or_else(PoisonError::into_inner);
                }
            };
            // SAFETY: a mailbox on a waiting foreign thread's stack, alive
            // until answered.
            let slot = unsafe { &*(mailbox as *const Slot) };
            self.serve(slot, slot.request());
            slot.answer();
            let queued = self
                .waiting
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop();
            match queued {
                Some(next) => *me.job.lock().unwrap_or_else(PoisonError::into_inner) = Some(next),
                None => self
                    .idle
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(Arc::clone(&me)),
            }
        }
    }

    // --- foreign functions as host-callable ones -------------------------------

    /// Records every executable segment the foreign linker holds now.
    pub(crate) fn set_code(&self, segments: &[[usize; 4]]) {
        let mut code = self.code.write().unwrap_or_else(PoisonError::into_inner);
        code.clear();
        for s in segments {
            if s[3] & 1 != 0 && s[2] > s[1] {
                code.insert(s[1], s[2]);
            }
        }
    }

    /// Whether `addr` lies in the foreign C library's code, or a library's
    /// it opened.
    pub(crate) fn is_foreign_code(&self, addr: usize) -> bool {
        let code = self.code.read().unwrap_or_else(PoisonError::into_inner);
        code.range(..=addr)
            .next_back()
            .is_some_and(|(_, &end)| addr < end)
    }

    /// A stub for each target, host-callable: made once per target.
    pub(crate) fn stubs(&self, targets: &[usize]) -> Vec<usize> {
        let mut stubs = self.stubs.lock().unwrap_or_else(PoisonError::into_inner);
        let missing: Vec<usize> = {
            let mut m: Vec<usize> = targets
                .iter()
                .copied()
                .filter(|t| !stubs.contains_key(t))
                .collect();
            m.sort_unstable();
            m.dedup();
            m
        };
        if !missing.is_empty() {
            let made = match write_stubs(&missing) {
                Ok(made) => made,
                Err(e) => {
                    std::eprintln!("guatiao: no memory for foreign call stubs ({e}); aborting");
                    std::process::abort();
                }
            };
            for (t, s) in missing.into_iter().zip(made) {
                stubs.insert(t, s);
            }
        }
        targets.iter().map(|t| stubs[t]).collect()
    }

    /// `table`'s `size` bytes, or a copy of them with every word after the
    /// first eight bytes that addresses foreign code replaced by its stub.
    pub(crate) fn table(&self, table: usize, size: usize) -> usize {
        if table == 0 || size <= 8 {
            return table;
        }
        // SAFETY: an object's or a kind's table: `size` readable bytes by
        // the producer's contract, which the caller relies on as well.
        let bytes = unsafe { core::slice::from_raw_parts(table as *const u8, size) };
        let word = size_of::<usize>();
        let words: Vec<(usize, usize)> = (8..size)
            .step_by(word)
            .filter(|at| at + word <= size)
            .map(|at| {
                let mut w = [0u8; 8];
                w.copy_from_slice(&bytes[at..at + word]);
                (at, usize::from_le_bytes(w))
            })
            .filter(|&(_, v)| v != 0 && self.is_foreign_code(v))
            .collect();
        if words.is_empty() {
            return table;
        }
        let mut tables = self.tables.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((seen, copy)) = tables.get(&(table, size))
            && seen.as_slice() == bytes
        {
            return *copy;
        }
        let stubs = self.stubs(&words.iter().map(|&(_, v)| v).collect::<Vec<_>>());
        let mut copy = vec![0u128; size.div_ceil(16)];
        let out = copy.as_mut_ptr().cast::<u8>();
        // SAFETY: the copy holds at least `size` bytes and is its own.
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), out, size) };
        for (&(at, _), stub) in words.iter().zip(stubs) {
            // SAFETY: a word inside the copy.
            unsafe { out.add(at).cast::<usize>().write_unaligned(stub) };
        }
        // Kept for the life of the process: an object holding it may
        // outlive any cache.
        let copy = Box::leak(copy.into_boxed_slice()).as_ptr() as usize;
        tables.insert((table, size), (bytes.to_vec(), copy));
        copy
    }
}

/// Writes one stub per target into fresh executable memory.
fn write_stubs(targets: &[usize]) -> std::io::Result<Vec<usize>> {
    const STUB: usize = 32;
    let page = sys::page_size();
    let len = (targets.len() * STUB).next_multiple_of(page);
    let at = sys::map_anonymous(len, sys::PROT_READ | sys::PROT_WRITE)?;
    let entry = bare::stub_entry as *const () as usize;
    let mut out = Vec::with_capacity(targets.len());
    for (i, &target) in targets.iter().enumerate() {
        let mut code = [0u8; STUB];
        #[cfg(target_arch = "x86_64")]
        {
            // movabs r10, target; movabs r11, entry; jmp r11; int3...
            code.fill(0xcc);
            code[0..2].copy_from_slice(&[0x49, 0xba]);
            code[2..10].copy_from_slice(&target.to_le_bytes());
            code[10..12].copy_from_slice(&[0x49, 0xbb]);
            code[12..20].copy_from_slice(&entry.to_le_bytes());
            code[20..23].copy_from_slice(&[0x41, 0xff, 0xe3]);
        }
        #[cfg(target_arch = "aarch64")]
        {
            // ldr x16, target; ldr x17, entry; br x17; brk #0; target; entry
            code[0..4].copy_from_slice(&0x5800_0090u32.to_le_bytes());
            code[4..8].copy_from_slice(&0x5800_00b1u32.to_le_bytes());
            code[8..12].copy_from_slice(&0xd61f_0220u32.to_le_bytes());
            code[12..16].copy_from_slice(&0xd420_0000u32.to_le_bytes());
            code[16..24].copy_from_slice(&target.to_le_bytes());
            code[24..32].copy_from_slice(&entry.to_le_bytes());
        }
        // SAFETY: inside the mapping just made, which nothing else knows.
        unsafe { core::ptr::copy_nonoverlapping(code.as_ptr(), at.add(i * STUB), STUB) };
        out.push(at as usize + i * STUB);
    }
    bare::sync_code(at as usize, len);
    // SAFETY: the mapping just made; nothing runs from it yet.
    unsafe { sys::protect(at, len, sys::PROT_READ | sys::PROT_EXEC)? };
    Ok(out)
}

/// Where every stub jumps: makes the call its frame holds, on a worker
/// when the caller is a host thread, here when it is already foreign.
///
/// # Safety
///
/// Called by `stub_entry` with the frame it saved.
pub(crate) unsafe extern "C" fn forward(frame: *mut Frame) {
    let w = WORLD.load(Ordering::Acquire);
    if w.is_null() {
        // SAFETY: stubs exist only once the loader started; nothing else
        // to do but make the call.
        unsafe {
            (*frame).stack_len = 0;
            bare::invoke(frame)
        };
        return;
    }
    // SAFETY: as in `world`.
    let w = unsafe { &*w };
    // SAFETY: the frame is the stub's, on its stack.
    unsafe { (*frame).stack_len = w.stack_arguments };
    if w.on_foreign_thread() {
        // SAFETY: the target is foreign code and so is this thread.
        unsafe { bare::invoke(frame) };
        return;
    }
    // SAFETY: `invoke` is TLS-free, the target foreign code, and the frame
    // lives until the call answers.
    if let Err(e) = unsafe { w.call(bare::invoke as *const () as usize, frame.cast()) } {
        std::eprintln!("guatiao: a call into a foreign library could not be made: {e}; aborting");
        std::process::abort();
    }
}
