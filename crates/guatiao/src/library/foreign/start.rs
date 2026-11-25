// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Starting the foreign C library, and asking it for libraries and symbols.

use alloc::boxed::Box;
use alloc::ffi::CString;
use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::Ordering;
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use super::bare::{self, Slot};
use super::elf::{ET_DYN, Elf};
use super::error::{Error, ImageError};
use super::helper;
use super::settings::{OpenFlags, Settings, SignalSet};
use super::stack::Stack;
use super::worker::{
    Control, DL_CLOSE, DL_OPEN, DL_SYMBOL, DlRequest, FAIL_LAYOUT, FAIL_TLS, Import, PhdrRequest,
    READY, STARTING,
};
use super::world::{self, Parts, World};
use super::{image, sys};
use crate::library::{DECLARES_SYMBOL, ENTRY_SYMBOL};

/// The signals glibc takes for itself when it starts: its cancellation and
/// its set-id broadcast.
const RESERVED: [u32; 2] = [32, 33];

/// One start at a time: the loader is one per process.
static STARTING_ONE: Mutex<()> = Mutex::new(());

/// A job: a function of the foreign C library's world taking one pointer.
pub type Job = unsafe extern "C" fn(*mut c_void);

/// The system's C library, started inside this process, and its workers.
///
/// There is one per process: [`Foreign::start`] starts it the first time
/// and answers the same one after. Its workers, the images and their memory
/// last as long as the process does: they belong to the foreign C library,
/// which cannot let its threads go without ending the process.
#[derive(Clone, Copy)]
pub struct Foreign {
    world: &'static World,
}

impl core::fmt::Debug for Foreign {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Foreign")
            .field("ld_so", &self.world.ld_so)
            .field("workers", &self.world.workers().len())
            .finish_non_exhaustive()
    }
}

fn read_image(path: &Path) -> Result<Vec<u8>, ImageError> {
    std::fs::read(path).map_err(ImageError::Io)
}

impl Foreign {
    /// The process's foreign C library: started with `settings` the first
    /// time, and the one already started (whatever its settings) after.
    ///
    /// Nothing runs before the dynamic linker has been read and checked,
    /// so a missing or unusable `ld.so` is an error with the host
    /// untouched. Once the linker is entered it is the system's: should it
    /// fail to find the C library, it prints why and ends the process, as
    /// it would for any program.
    pub fn start(settings: &Settings) -> Result<Foreign, Error> {
        if let Some(world) = world::world() {
            return Ok(Foreign { world });
        }
        let _one = STARTING_ONE.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(world) = world::world() {
            return Ok(Foreign { world });
        }
        let world = start(settings)?;
        Ok(Foreign {
            world: world::publish(world),
        })
    }

    /// The process's foreign C library, if it was started.
    pub fn get() -> Option<Foreign> {
        world::world().map(|world| Foreign { world })
    }

    /// The dynamic linker that was entered.
    pub fn ld_so(&self) -> &Path {
        &self.world.ld_so
    }

    /// How many workers there are now.
    pub fn workers(&self) -> usize {
        self.world.workers().len()
    }

    /// The kernel thread id of each worker, and last that of the thread
    /// `ld.so` started on, which only starts workers.
    pub fn thread_ids(&self) -> Vec<i32> {
        self.world
            .workers()
            .iter()
            .chain(self.world.slots.first())
            .map(|s| s.tid.load(Ordering::Acquire))
            .collect()
    }

    /// The signals whose handlers changed while the foreign C library
    /// started (glibc's set-id signal, 33, for one).
    pub fn signals_taken(&self) -> SignalSet {
        self.world.taken
    }

    /// Whether the calling thread is one of the foreign C library's.
    pub fn on_foreign_thread(&self) -> bool {
        self.world.on_foreign_thread()
    }

    /// Runs `job(argument)` on a worker and waits for it. A call into the
    /// host the job makes meanwhile runs on this thread.
    ///
    /// # Safety
    ///
    /// `job` is code of the foreign C library's world (a symbol of a library
    /// opened here, or of the C library itself) and `argument` is what it
    /// expects, valid until this returns.
    pub unsafe fn call(&self, job: Job, argument: *mut c_void) -> Result<(), Error> {
        // SAFETY: the caller's.
        unsafe { self.world.call(job as usize, argument) }
    }

    /// `address`, callable from host code: itself when it is not the
    /// foreign C library's code, or a library's it opened; else a stub that
    /// makes each call on a worker (or directly, when the caller is a
    /// foreign thread already).
    pub fn callable(&self, address: usize) -> usize {
        if address == 0 || !self.world.is_foreign_code(address) {
            return address;
        }
        self.world.stubs(&[address])[0]
    }

    /// Runs a request for the dynamic linker on a worker, with room for
    /// its error message.
    fn dl(&self, request: &mut DlRequest) -> Result<Option<String>, Error> {
        let mut message: Vec<u8> = Vec::with_capacity(self.world.error_capacity);
        request.error = message.as_mut_ptr();
        request.error_cap = message.capacity() as u64;
        // SAFETY: `dl_job` is this crate's TLS-free job; the request and
        // its buffers outlive the call, which waits for it.
        unsafe {
            self.world.call(
                bare::dl_job as *const () as usize,
                (request as *mut DlRequest).cast(),
            )?
        };
        if request.status == 0 {
            return Ok(None);
        }
        let len = (request.error_len as usize).min(message.capacity());
        // SAFETY: the job wrote `len` bytes, at most the capacity.
        unsafe { message.set_len(len) };
        Ok(Some(String::from_utf8_lossy(&message).into_owned()))
    }

    /// Every loaded segment the foreign linker holds, as `[object, start,
    /// end, flags]`; records the executable ones as foreign code.
    pub(crate) fn segments(&self) -> Result<Vec<[usize; 4]>, Error> {
        let mut capacity = 256;
        loop {
            let mut buffer = vec![[0usize; 4]; capacity];
            let mut request = PhdrRequest {
                control: self.world.control,
                buffer: buffer.as_mut_ptr(),
                capacity,
                found: 0,
                objects: 0,
            };
            // SAFETY: `phdr_job` is TLS-free; the buffer outlives the call.
            unsafe {
                self.world.call(
                    bare::phdr_job as *const () as usize,
                    (&mut request as *mut PhdrRequest).cast(),
                )?
            };
            if request.found <= capacity {
                buffer.truncate(request.found);
                self.world.set_code(&buffer);
                return Ok(buffer);
            }
            capacity = request.found + 64;
        }
    }

    /// Opens a library with the settings' flags, on a worker: its
    /// constructors run there.
    pub fn open(&self, name: impl AsRef<OsStr>) -> Result<Library, Error> {
        self.open_with(name, self.world.open_flags)
    }

    /// Opens a library with `flags`, on a worker.
    pub fn open_with(&self, name: impl AsRef<OsStr>, flags: OpenFlags) -> Result<Library, Error> {
        let name = name.as_ref().to_os_string();
        let text = CString::new(name.as_bytes()).map_err(|_| Error::Name(name.clone()))?;
        let mut request = DlRequest::new(self.world.control, DL_OPEN);
        request.flags = flags.bits();
        request.name = text.as_ptr().cast();
        if let Some(message) = self.dl(&mut request)? {
            return Err(Error::Open { name, message });
        }
        self.segments()?;
        Ok(Library {
            foreign: *self,
            handle: request.handle as usize,
            name,
        })
    }
}

/// A library the foreign dynamic linker opened.
///
/// Dropping it leaves the library open, as dropping a registry leaves its
/// libraries mapped; [`Library::close`] closes it.
#[derive(Debug)]
pub struct Library {
    foreign: Foreign,
    handle: usize,
    name: OsString,
}

/// An address a library exports, and its size from the library's symbol
/// table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Symbol {
    address: usize,
    size: u64,
}

impl Symbol {
    /// The address, in the library's image: foreign code, which host code
    /// calls only through [`Foreign::callable`].
    pub fn address(&self) -> *mut c_void {
        self.address as *mut c_void
    }

    /// The size the symbol table gives, `None` when it gives none.
    pub fn size(&self) -> Option<u64> {
        (self.size != 0).then_some(self.size)
    }

    /// The symbol as a job for [`Foreign::call`].
    ///
    /// # Safety
    ///
    /// The symbol is a function taking one pointer and returning nothing.
    pub unsafe fn as_job(&self) -> Job {
        // SAFETY: the caller's; the address is non-null.
        unsafe { core::mem::transmute::<usize, Job>(self.address) }
    }
}

/// Where one opened object lies: its whole span and its executable ranges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Placement {
    pub(crate) span: (usize, usize),
    pub(crate) code: Vec<(usize, usize)>,
}

impl Library {
    /// The name it was opened by.
    pub fn name(&self) -> &OsStr {
        &self.name
    }

    /// `dlopen`'s handle, for jobs that take one.
    pub fn handle(&self) -> *mut c_void {
        self.handle as *mut c_void
    }

    /// The loader it was opened through.
    pub fn foreign(&self) -> &Foreign {
        &self.foreign
    }

    /// Looks `name` up in the library and what it links, on a worker.
    /// `None` when it is not there, or is null.
    pub fn symbol(&self, name: &str) -> Result<Option<Symbol>, Error> {
        let text = CString::new(name).map_err(|_| Error::Name(name.into()))?;
        self.lookup(&text)
    }

    fn lookup(&self, name: &core::ffi::CStr) -> Result<Option<Symbol>, Error> {
        let mut request = DlRequest::new(self.foreign.world.control, DL_SYMBOL);
        request.handle = self.handle as *mut c_void;
        request.name = name.as_ptr().cast();
        if self.foreign.dl(&mut request)?.is_some() || request.address.is_null() {
            return Ok(None);
        }
        Ok(Some(Symbol {
            address: request.address as usize,
            size: request.size,
        }))
    }

    /// The guatiao entry point, `None` when the library exports none (it is
    /// not a guatiao library).
    pub fn entry(&self) -> Result<Option<Symbol>, Error> {
        let name = core::ffi::CStr::from_bytes_with_nul(ENTRY_SYMBOL)
            .expect("guatiao's entry symbol ends in its NUL");
        self.lookup(name)
    }

    /// Where the object holding `address` lies, from the foreign linker's
    /// record of its segments.
    pub(crate) fn placement(&self, address: usize) -> Result<Option<Placement>, Error> {
        let segments = self.foreign.segments()?;
        let Some(object) = segments
            .iter()
            .find(|s| s[1] <= address && address < s[2])
            .map(|s| s[0])
        else {
            return Ok(None);
        };
        let own: Vec<&[usize; 4]> = segments.iter().filter(|s| s[0] == object).collect();
        let start = own.iter().map(|s| s[1]).min().unwrap_or(0);
        let end = own.iter().map(|s| s[2]).max().unwrap_or(0);
        let code = own
            .iter()
            .filter(|s| s[3] & 1 != 0)
            .map(|s| (s[1], s[2]))
            .collect();
        Ok(Some(Placement {
            span: (start, end),
            code,
        }))
    }

    /// What the library declares (`guatiao_declares`), copied out of its
    /// image up to and including the empty string that ends it; `None` when
    /// it declares nothing. guatiao's `Declared::parse` reads it.
    pub fn declaration(&self) -> Result<Option<Vec<u8>>, Error> {
        let name = core::ffi::CStr::from_bytes_with_nul(DECLARES_SYMBOL)
            .expect("guatiao's declaration symbol ends in its NUL");
        let Some(symbol) = self.lookup(name)? else {
            return Ok(None);
        };
        let world = self.foreign.world;
        let window = world.declaration_window;
        let len = match symbol.size() {
            Some(size) if size > window as u64 => {
                return Err(Error::DeclarationTooLarge { size, window });
            }
            Some(size) => size as usize,
            None if world.read_unsized_declaration => window,
            None => return Err(Error::DeclarationSizeUnknown),
        };
        // SAFETY: the library's own symbol table says the object spans
        // `len` bytes at this address (or the host chose to read the window
        // without that), and the library stays open while `self` lives.
        let bytes =
            unsafe { core::slice::from_raw_parts(symbol.address as *const u8, len) }.to_vec();
        let mut at = 0;
        loop {
            match bytes[at..].iter().position(|&b| b == 0) {
                Some(0) => return Ok(Some(bytes[..=at].to_vec())),
                Some(end) => at += end + 1,
                None => return Err(Error::DeclarationUnterminated { read: len }),
            }
            if at >= bytes.len() {
                return Err(Error::DeclarationUnterminated { read: len });
            }
        }
    }

    /// Closes the library on a worker; its destructors run there. Symbols
    /// taken from it are dangling afterwards.
    pub fn close(self) -> Result<(), Error> {
        let mut request = DlRequest::new(self.foreign.world.control, DL_CLOSE);
        request.handle = self.handle as *mut c_void;
        let answer = self.foreign.dl(&mut request)?;
        self.foreign.segments()?;
        match answer {
            None => Ok(()),
            Some(message) => Err(Error::Close { message }),
        }
    }
}

/// Maps the dynamic linker and the program written for it, enters the
/// linker on a thread of its own, and waits for the program to report its
/// workers ready.
fn start(settings: &Settings) -> Result<&'static World, Error> {
    settings.check()?;
    let page = sys::page_size();

    let ld_path = settings.interpreter();
    let ld_error = |reason| Error::LdSo {
        path: ld_path.clone(),
        reason,
    };
    let ld_bytes = read_image(&ld_path).map_err(ld_error)?;
    let ld = Elf::parse(&ld_bytes).map_err(ld_error)?;
    if ld.kind != ET_DYN {
        return Err(ld_error(ImageError::Type(ld.kind)));
    }
    if ld.interpreter().map_err(ld_error)?.is_some() {
        return Err(ld_error(ImageError::HasInterpreter));
    }

    let random = sys::random().map_err(Error::Memory)?;
    let mark_bytes = sys::random().map_err(Error::Memory)?;
    let mut mark = [0u64; 2];
    for (i, m) in mark.iter_mut().enumerate() {
        let mut w = [0u8; 8];
        w.copy_from_slice(&mark_bytes[i * 8..i * 8 + 8]);
        *m = u64::from_le_bytes(w);
    }
    let written = helper::write(ld_path.as_os_str().as_bytes(), &settings.needed, mark);
    let program = Elf::parse(&written.bytes).map_err(|reason| Error::Program { reason })?;
    let phdr = program
        .phdr_vaddr()
        .map_err(|reason| Error::Program { reason })?;

    let before: Vec<usize> = (1..=64).map(sys::handler).collect();
    if !settings.allow_reserved_signal_handlers {
        for signal in RESERVED {
            if before[signal as usize - 1] != 0 {
                return Err(Error::SignalInUse { signal });
            }
        }
    }
    let forks = sys::forks();
    if settings.fence_program_break {
        sys::fence_program_break(page).map_err(Error::Fence)?;
    }

    let program_at = image::map(&program, page).map_err(Error::Memory)?;
    let ld_at = image::map(&ld, page).map_err(Error::Memory)?;

    let slots: &'static [Slot] = Box::leak(
        (0..=settings.max_workers)
            .map(|_| Slot::new())
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );
    let mut block = Control::new(
        slots,
        settings.workers,
        settings.spin,
        settings.blocked_signals.bits(),
        settings.worker_stack.unwrap_or(0) as u64,
        mark,
    );
    for (i, slot) in written.slots.iter().enumerate() {
        block.import_at[i] = program_at.bias.wrapping_add(*slot as usize);
    }
    let control: &'static Control = Box::leak(Box::new(block));
    for slot in slots {
        // A worker finds the block through its slot.
        slot.next
            .store(core::ptr::from_ref(control) as usize, Ordering::Relaxed);
    }

    let mut args = Vec::with_capacity(2);
    args.push(b"guatiao-foreign".to_vec());
    args.push(alloc::format!("{:x}", core::ptr::from_ref(control) as usize).into_bytes());
    for a in &mut args {
        a.push(0);
    }
    let mut auxv = alloc::vec![
        (sys::AT_PHDR, (program_at.bias as u64).wrapping_add(phdr)),
        (sys::AT_PHENT, u64::from(program.phentsize)),
        (sys::AT_PHNUM, u64::from(program.phnum)),
        (sys::AT_PAGESZ, page as u64),
        (sys::AT_BASE, ld_at.bias as u64),
        (sys::AT_FLAGS, 0),
        (sys::AT_ENTRY, bare::entry as *const () as usize as u64),
    ];
    for kind in [
        sys::AT_UID,
        sys::AT_EUID,
        sys::AT_GID,
        sys::AT_EGID,
        sys::AT_SECURE,
    ] {
        auxv.push((kind, sys::auxv_or_zero(kind)));
    }
    for kind in sys::PASSED_ON {
        if let Some(value) = sys::auxv(kind) {
            auxv.push((kind, value));
        }
    }
    let stack = Stack::new(settings.bootstrap_stack, page)?;
    let sp = stack.build(
        settings.stack_alignment,
        &args,
        &settings.environment.entries(),
        &auxv,
        random,
    )?;

    let entry = ld_at.bias.wrapping_add(ld.entry as usize);
    // The new thread starts with every signal blocked and keeps them
    // blocked until the program sets the workers' mask.
    let mask = sys::block_all();
    // SAFETY: `sp` is the top of a stack built for `ld.so`, and `entry` is
    // `ld.so`'s entry point in the image just mapped.
    let tid = unsafe { enter(sp, entry) };
    sys::set_mask(mask);
    if tid < 0 {
        return Err(Error::Thread(std::io::Error::from_raw_os_error(
            -tid as i32,
        )));
    }

    let deadline = settings.start_timeout.map(|t| (t, Instant::now() + t));
    let state = loop {
        let state = control.ready.load(Ordering::Acquire);
        if state != STARTING {
            break state;
        }
        let left = match deadline {
            Some((timeout, at)) => {
                let now = Instant::now();
                if now >= at {
                    return Err(Error::StartTimedOut(timeout));
                }
                Some(at - now)
            }
            None => None,
        };
        sys::futex_wait(&control.ready, STARTING, left);
    };
    if state != READY {
        return Err(match control.failure.load(Ordering::Acquire) {
            FAIL_LAYOUT => Error::Layout,
            FAIL_TLS => Error::NoMark,
            code => Error::Spawn(code),
        });
    }
    if control.function(Import::Dlopen) == 0 {
        return Err(Error::Layout);
    }

    let mut taken = SignalSet::EMPTY;
    for (i, &handler) in before.iter().enumerate() {
        let signal = i as u32 + 1;
        if sys::handler(signal) != handler {
            taken = taken.with(signal);
        }
    }
    let world: &'static World = Box::leak(Box::new(World::new(Parts {
        control,
        slots,
        workers: settings.workers,
        forks,
        spin: settings.spin,
        stack_arguments: settings.stack_arguments,
        ld_so: ld_path,
        taken,
        error_capacity: settings.error_capacity,
        declaration_window: settings.declaration_window,
        read_unsized_declaration: settings.read_unsized_declaration,
        open_flags: settings.open_flags,
        mark,
        tls_offset: control.tls_offset.load(Ordering::Acquire),
        callback_threads: settings.callback_threads,
    })));
    std::thread::Builder::new()
        .name("guatiao-foreign-dispatch".into())
        .spawn(move || world.dispatch())
        .map_err(Error::Thread)?;
    Foreign { world }.segments()?;
    Ok(world)
}

/// Starts a thread on the stack at `sp` that jumps to `entry` as a new
/// program's first thread would: no thread pointer of its own yet, the
/// finaliser argument zero. The thread shares everything with the process
/// and never returns into this code. Answers its thread id, or `-errno`.
///
/// # Safety
///
/// `sp` points at a start-up block on a stack nothing else uses, and
/// `entry` at a dynamic linker's entry point mapped executable.
#[cfg(target_arch = "x86_64")]
unsafe fn enter(sp: usize, entry: usize) -> isize {
    // CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD |
    // CLONE_SYSVSEM: a thread of this process, with no thread pointer set.
    const FLAGS: usize = 0x100 | 0x200 | 0x400 | 0x800 | 0x1_0000 | 0x4_0000;
    let ret: isize;
    // SAFETY: the parent returns from the syscall with the clobbers named;
    // the child runs on `sp` and leaves through `jmp`, never back here.
    unsafe {
        core::arch::asm!(
            "syscall",
            "test rax, rax",
            "jnz 2f",
            "xor ebp, ebp",
            "jmp r9",
            "2:",
            inlateout("rax") sys::SYS_CLONE as isize => ret,
            in("rdi") FLAGS,
            in("rsi") sp,
            in("rdx") 0usize,
            in("r10") 0usize,
            in("r8") 0usize,
            in("r9") entry,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

/// As the x86_64 one.
///
/// # Safety
///
/// As the x86_64 one.
#[cfg(target_arch = "aarch64")]
unsafe fn enter(sp: usize, entry: usize) -> isize {
    const FLAGS: usize = 0x100 | 0x200 | 0x400 | 0x800 | 0x1_0000 | 0x4_0000;
    let ret: isize;
    // SAFETY: as the x86_64 one; the child returns from `svc` with x0 = 0.
    unsafe {
        core::arch::asm!(
            "svc #0",
            "cbnz x0, 2f",
            "mov x29, xzr",
            "mov x30, xzr",
            "br x9",
            "2:",
            inlateout("x0") FLAGS as isize => ret,
            in("x1") sp,
            in("x2") 0usize,
            in("x3") 0usize,
            in("x4") 0usize,
            in("x8") sys::SYS_CLONE,
            in("x9") entry,
            options(nostack),
        );
    }
    ret
}
