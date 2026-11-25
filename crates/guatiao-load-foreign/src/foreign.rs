// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Starting the foreign C library, and asking it for libraries and symbols.

use alloc::boxed::Box;
use alloc::ffi::CString;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use guatiao::library::{DECLARES_SYMBOL, ENTRY_SYMBOL};

use crate::elf::{ET_DYN, Elf};
use crate::error::{Error, ImageError};
use crate::helper::BUILT_NAME;
use crate::settings::{OpenFlags, Settings, SignalSet};
use crate::stack::Stack;
use crate::worker::{
    Control, DL_CLOSE, DL_OPEN, DL_SYMBOL, DlRequest, FAIL_LAYOUT, READY, STARTING, Slot,
};
use crate::{image, sys};

/// The signals glibc takes for itself when it starts: its cancellation and
/// its set-id broadcast.
const RESERVED: [u32; 2] = [32, 33];

/// A job: a function of the foreign C library's world taking one pointer.
pub type Job = unsafe extern "C" fn(*mut c_void);

/// The system's C library, started inside this process, and its workers.
///
/// Cloning shares the same workers. Dropping the last handle leaves them
/// parked: they belong to the foreign C library, which cannot let its
/// threads go without ending the process, so the workers, the images and
/// their memory last as long as the process does.
#[derive(Clone)]
pub struct Foreign {
    inner: Arc<Inner>,
}

struct Inner {
    control: &'static Control,
    slots: &'static [Slot],
    locks: Box<[Mutex<()>]>,
    next: AtomicUsize,
    forks: u64,
    spin: u32,
    ld_so: PathBuf,
    taken: SignalSet,
    error_capacity: usize,
    declaration_window: usize,
    read_unsized_declaration: bool,
    open_flags: OpenFlags,
}

impl core::fmt::Debug for Foreign {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Foreign")
            .field("ld_so", &self.inner.ld_so)
            .field("workers", &self.inner.slots.len())
            .finish_non_exhaustive()
    }
}

fn read_image(path: &Path) -> Result<Vec<u8>, ImageError> {
    std::fs::read(path).map_err(ImageError::Io)
}

impl Foreign {
    /// Maps the dynamic linker and the helper, enters the linker on a
    /// thread of its own, and waits for the helper to report its workers
    /// ready.
    ///
    /// Nothing runs before every file has been read and checked, so a
    /// missing or unusable `ld.so` or helper is an error with the host
    /// untouched. Once the linker is entered it is the system's: should it
    /// fail to find the C library, it prints why and ends the process, as
    /// it would for any program.
    pub fn start(settings: &Settings) -> Result<Foreign, Error> {
        settings.check()?;
        let page = sys::page_size();

        let (helper_bytes, helper_path) = settings.helper.bytes()?;
        let helper_error = |reason| Error::Helper {
            path: helper_path.map(Path::to_path_buf),
            reason,
        };
        let helper = Elf::parse(&helper_bytes).map_err(helper_error)?;
        if helper.kind != ET_DYN {
            return Err(helper_error(ImageError::Type(helper.kind)));
        }
        let phdr = helper.phdr_vaddr().map_err(helper_error)?;
        let ld_path = match &settings.ld_so {
            Some(path) => path.clone(),
            None => match helper.interpreter().map_err(helper_error)? {
                Some(path) => PathBuf::from(OsStr::from_bytes(path)),
                None => return Err(helper_error(ImageError::NoInterpreter)),
            },
        };
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

        let helper_at = image::map(&helper, page).map_err(Error::Memory)?;
        let ld_at = image::map(&ld, page).map_err(Error::Memory)?;

        let slots: &'static [Slot] =
            Box::leak((0..settings.workers).map(|_| Slot::new()).collect());
        let control: &'static Control = Box::leak(Box::new(Control::new(
            slots,
            settings.spin,
            settings.blocked_signals.bits(),
            settings.worker_stack.unwrap_or(0) as u64,
        )));

        let name = match helper_path {
            Some(path) => path.as_os_str().as_bytes().to_vec(),
            None => BUILT_NAME.as_bytes().to_vec(),
        };
        let mut args = Vec::with_capacity(2);
        args.push(name);
        args.push(alloc::format!("{:x}", control as *const Control as usize).into_bytes());
        for a in &mut args {
            a.retain(|&b| b != 0);
            a.push(0);
        }
        let mut auxv = alloc::vec![
            (sys::AT_PHDR, (helper_at.bias as u64).wrapping_add(phdr)),
            (sys::AT_PHENT, u64::from(helper.phentsize)),
            (sys::AT_PHNUM, u64::from(helper.phnum)),
            (sys::AT_PAGESZ, page as u64),
            (sys::AT_BASE, ld_at.bias as u64),
            (sys::AT_FLAGS, 0),
            (
                sys::AT_ENTRY,
                (helper_at.bias as u64).wrapping_add(helper.entry)
            ),
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
        let random = sys::random().map_err(Error::Memory)?;
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
        // blocked until the helper sets the workers' mask.
        let mask = sys::block_all();
        // SAFETY: `sp` is the top of a stack built for `ld.so`, and `entry`
        // is `ld.so`'s entry point in the image just mapped.
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
                code => Error::Spawn(code),
            });
        }

        let mut taken = SignalSet::EMPTY;
        for (i, &handler) in before.iter().enumerate() {
            let signal = i as u32 + 1;
            if sys::handler(signal) != handler {
                taken = taken.with(signal);
            }
        }
        Ok(Foreign {
            inner: Arc::new(Inner {
                control,
                slots,
                locks: (0..slots.len()).map(|_| Mutex::new(())).collect(),
                next: AtomicUsize::new(0),
                forks,
                spin: settings.spin,
                ld_so: ld_path,
                taken,
                error_capacity: settings.error_capacity,
                declaration_window: settings.declaration_window,
                read_unsized_declaration: settings.read_unsized_declaration,
                open_flags: settings.open_flags,
            }),
        })
    }

    /// The dynamic linker that was entered.
    pub fn ld_so(&self) -> &Path {
        &self.inner.ld_so
    }

    /// How many workers run jobs.
    pub fn workers(&self) -> usize {
        self.inner.slots.len()
    }

    /// The kernel thread id of each worker, the bootstrap thread first.
    pub fn worker_ids(&self) -> Vec<i32> {
        self.inner
            .slots
            .iter()
            .map(|s| s.tid.load(Ordering::Acquire))
            .collect()
    }

    /// The signals whose handlers changed while the foreign C library
    /// started (glibc's set-id signal, 33, for one).
    pub fn signals_taken(&self) -> SignalSet {
        self.inner.taken
    }

    fn check_fork(&self) -> Result<(), Error> {
        if sys::forks_seen() != self.inner.forks {
            Err(Error::Forked)
        } else {
            Ok(())
        }
    }

    /// Runs `job(argument)` on worker `index` and waits for it.
    ///
    /// # Safety
    ///
    /// `job` is code of the foreign C library's world (a symbol of a library
    /// opened here, or of the C library itself) and `argument` is what it
    /// expects, valid until this returns. Host code must not run as a job:
    /// it would find the foreign C library's thread state where its own
    /// should be.
    pub unsafe fn call_on(
        &self,
        index: usize,
        job: Job,
        argument: *mut c_void,
    ) -> Result<(), Error> {
        self.check_fork()?;
        let inner = &self.inner;
        let (Some(slot), Some(lock)) = (inner.slots.get(index), inner.locks.get(index)) else {
            return Err(Error::NoWorker {
                index,
                workers: inner.slots.len(),
            });
        };
        let _only = lock.lock().unwrap_or_else(PoisonError::into_inner);
        // SAFETY: the lock makes this the slot's only poster; the rest is
        // the caller's.
        unsafe { slot.run(job as usize, argument, inner.spin) };
        Ok(())
    }

    /// Runs `job(argument)` on a free worker, or waits for one, and waits
    /// for it.
    ///
    /// # Safety
    ///
    /// As for [`Foreign::call_on`].
    pub unsafe fn call(&self, job: Job, argument: *mut c_void) -> Result<(), Error> {
        // SAFETY: the caller's.
        unsafe { self.call_any(job as usize, argument) }
    }

    /// # Safety
    ///
    /// As for [`Foreign::call_on`], with `job` an address.
    unsafe fn call_any(&self, job: usize, argument: *mut c_void) -> Result<(), Error> {
        self.check_fork()?;
        let inner = &self.inner;
        let n = inner.slots.len();
        let first = inner.next.fetch_add(1, Ordering::Relaxed) % n;
        for i in 0..n {
            let index = (first + i) % n;
            if let Ok(_only) = inner.locks[index].try_lock() {
                // SAFETY: the lock makes this the slot's only poster.
                unsafe { inner.slots[index].run(job, argument, inner.spin) };
                return Ok(());
            }
        }
        let _only = inner.locks[first]
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // SAFETY: as above.
        unsafe { inner.slots[first].run(job, argument, inner.spin) };
        Ok(())
    }

    /// Runs a request for the dynamic linker on a worker, with room for
    /// its error message.
    fn dl(&self, request: &mut DlRequest) -> Result<Option<String>, Error> {
        let mut message: Vec<u8> = Vec::with_capacity(self.inner.error_capacity);
        request.error = message.as_mut_ptr();
        request.error_cap = message.capacity() as u64;
        let job = self.inner.control.dl.load(Ordering::Acquire);
        // SAFETY: `gt_dl` is the helper's own job; the request and its
        // buffers outlive the call, which waits for it.
        unsafe { self.call_any(job, (request as *mut DlRequest).cast())? };
        if request.status == 0 {
            return Ok(None);
        }
        let len = (request.error_len as usize).min(message.capacity());
        // SAFETY: the helper wrote `len` bytes, at most the capacity.
        unsafe { message.set_len(len) };
        Ok(Some(String::from_utf8_lossy(&message).into_owned()))
    }

    /// Opens a library with the settings' flags, on a worker: its
    /// constructors run there.
    pub fn open(&self, name: impl AsRef<OsStr>) -> Result<Library, Error> {
        self.open_with(name, self.inner.open_flags)
    }

    /// Opens a library with `flags`, on a worker.
    pub fn open_with(&self, name: impl AsRef<OsStr>, flags: OpenFlags) -> Result<Library, Error> {
        let name = name.as_ref().to_os_string();
        let text = CString::new(name.as_bytes()).map_err(|_| Error::Name(name.clone()))?;
        let mut request = DlRequest::new(DL_OPEN);
        request.flags = flags.bits();
        request.name = text.as_ptr().cast();
        if let Some(message) = self.dl(&mut request)? {
            return Err(Error::Open { name, message });
        }
        Ok(Library {
            foreign: self.clone(),
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
    /// The address, in the library's image.
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
        let mut request = DlRequest::new(DL_SYMBOL);
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

    /// What the library declares (`guatiao_declares`), copied out of its
    /// image up to and including the empty string that ends it; `None` when
    /// it declares nothing. guatiao's `Declared::parse` reads it.
    pub fn declaration(&self) -> Result<Option<Vec<u8>>, Error> {
        let name = core::ffi::CStr::from_bytes_with_nul(DECLARES_SYMBOL)
            .expect("guatiao's declaration symbol ends in its NUL");
        let Some(symbol) = self.lookup(name)? else {
            return Ok(None);
        };
        let inner = &self.foreign.inner;
        let window = inner.declaration_window;
        let len = match symbol.size() {
            Some(size) if size > window as u64 => {
                return Err(Error::DeclarationTooLarge { size, window });
            }
            Some(size) => size as usize,
            None if inner.read_unsized_declaration => window,
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
        let mut request = DlRequest::new(DL_CLOSE);
        request.handle = self.handle as *mut c_void;
        match self.foreign.dl(&mut request)? {
            None => Ok(()),
            Some(message) => Err(Error::Close { message }),
        }
    }
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
