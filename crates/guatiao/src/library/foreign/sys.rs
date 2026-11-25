// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The host's own C library and the kernel, for what the loader needs on
//! host threads: memory, futexes, the signal mask and handlers, the program
//! break and the auxiliary vector. Every function here runs on a host thread.

use core::ffi::{c_int, c_long, c_ulong, c_void};
use core::sync::atomic::AtomicU32;
use core::time::Duration;
use std::io;

unsafe extern "C" {
    fn syscall(number: c_long, ...) -> c_long;
    fn mmap(
        addr: *mut c_void,
        len: usize,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        offset: i64,
    ) -> *mut c_void;
    fn mprotect(addr: *mut c_void, len: usize, prot: c_int) -> c_int;
    fn munmap(addr: *mut c_void, len: usize) -> c_int;
    fn getauxval(kind: c_ulong) -> c_ulong;
    fn pthread_atfork(
        prepare: Option<unsafe extern "C" fn()>,
        parent: Option<unsafe extern "C" fn()>,
        child: Option<unsafe extern "C" fn()>,
    ) -> c_int;
}

#[cfg(target_arch = "x86_64")]
mod nr {
    pub(super) const BRK: i64 = 12;
    pub(super) const RT_SIGACTION: i64 = 13;
    pub(super) const RT_SIGPROCMASK: i64 = 14;
    pub(crate) const CLONE: i64 = 56;
    pub(super) const FUTEX: i64 = 202;
    pub(super) const GETRANDOM: i64 = 318;
}
#[cfg(target_arch = "aarch64")]
mod nr {
    pub(super) const BRK: i64 = 214;
    pub(super) const RT_SIGACTION: i64 = 134;
    pub(super) const RT_SIGPROCMASK: i64 = 135;
    pub(crate) const CLONE: i64 = 220;
    pub(super) const FUTEX: i64 = 98;
    pub(super) const GETRANDOM: i64 = 278;
}
pub(crate) use nr::CLONE as SYS_CLONE;

pub(crate) const PROT_NONE: c_int = 0;
pub(crate) const PROT_READ: c_int = 1;
pub(crate) const PROT_WRITE: c_int = 2;
pub(crate) const PROT_EXEC: c_int = 4;
const MAP_PRIVATE: c_int = 0x02;
const MAP_ANONYMOUS: c_int = 0x20;
const MAP_NORESERVE: c_int = 0x4000;
const MAP_FIXED_NOREPLACE: c_int = 0x10_0000;
const FUTEX_WAIT_PRIVATE: c_int = 128;
const SIG_SETMASK: c_int = 2;
const EEXIST: i32 = 17;
const EINTR: i32 = 4;

pub(crate) const AT_NULL: u64 = 0;
pub(crate) const AT_PHDR: u64 = 3;
pub(crate) const AT_PHENT: u64 = 4;
pub(crate) const AT_PHNUM: u64 = 5;
pub(crate) const AT_PAGESZ: u64 = 6;
pub(crate) const AT_BASE: u64 = 7;
pub(crate) const AT_FLAGS: u64 = 8;
pub(crate) const AT_ENTRY: u64 = 9;
pub(crate) const AT_UID: u64 = 11;
pub(crate) const AT_EUID: u64 = 12;
pub(crate) const AT_GID: u64 = 13;
pub(crate) const AT_EGID: u64 = 14;
pub(crate) const AT_SECURE: u64 = 23;
pub(crate) const AT_RANDOM: u64 = 25;
pub(crate) const AT_EXECFN: u64 = 31;
/// Passed on from the host's own vector when it has them: the CPU's
/// features (which glibc's string functions are chosen by), the platform
/// name, the vDSO, the clock tick, the signal stack minimum and rseq's area.
pub(crate) const PASSED_ON: [u64; 10] = [16, 26, 29, 30, 15, 33, 17, 51, 27, 28];

/// The kernel's page size.
pub(crate) fn page_size() -> usize {
    // SAFETY: getauxval reads the vector the C library saved at start.
    let page = unsafe { getauxval(AT_PAGESZ as c_ulong) } as usize;
    if page.is_power_of_two() && page >= 4096 {
        page
    } else {
        4096
    }
}

/// An entry of the host's auxiliary vector, or `None` when it is 0 (absent
/// entries read as 0).
pub(crate) fn auxv(kind: u64) -> Option<u64> {
    // SAFETY: as in `page_size`.
    let value = unsafe { getauxval(kind as c_ulong) } as u64;
    (value != 0).then_some(value)
}

/// Like [`auxv`], 0 included.
pub(crate) fn auxv_or_zero(kind: u64) -> u64 {
    auxv(kind).unwrap_or(0)
}

/// Fresh anonymous private memory with `prot`, anywhere.
pub(crate) fn map_anonymous(len: usize, prot: c_int) -> io::Result<*mut u8> {
    // SAFETY: a new mapping at an address the kernel picks touches nothing
    // that exists.
    let at = unsafe {
        mmap(
            core::ptr::null_mut(),
            len,
            prot,
            MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE,
            -1,
            0,
        )
    };
    if at as isize == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(at.cast())
    }
}

/// Changes the protection of pages this loader mapped.
///
/// # Safety
///
/// `[at, at + len)` is memory this loader mapped and nothing else relies on
/// its present protection.
pub(crate) unsafe fn protect(at: *mut u8, len: usize, prot: c_int) -> io::Result<()> {
    // SAFETY: the caller's.
    if unsafe { mprotect(at.cast(), len, prot) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Gives pages this loader mapped back.
///
/// # Safety
///
/// `[at, at + len)` is memory this loader mapped and nothing uses.
pub(crate) unsafe fn unmap(at: *mut u8, len: usize) {
    // SAFETY: the caller's.
    unsafe { munmap(at.cast(), len) };
}

/// Sleeps while `word` holds `expected`, up to `timeout`. Returns early on
/// a wake, a change, a signal or the timeout; the caller looks again.
pub(crate) fn futex_wait(word: &AtomicU32, expected: u32, timeout: Option<Duration>) {
    let spec;
    let at = match timeout {
        Some(t) => {
            spec = [
                i64::try_from(t.as_secs()).unwrap_or(i64::MAX),
                i64::from(t.subsec_nanos()),
            ];
            spec.as_ptr()
        }
        None => core::ptr::null(),
    };
    // SAFETY: the word is a live `AtomicU32`; the timespec outlives the call.
    unsafe {
        syscall(
            nr::FUTEX,
            word.as_ptr(),
            FUTEX_WAIT_PRIVATE as c_long,
            expected as c_long,
            at,
            0usize,
            0usize,
        )
    };
}

/// 16 bytes from the kernel's generator, for `AT_RANDOM` (the foreign C
/// library's stack canary and pointer guard).
pub(crate) fn random() -> io::Result<[u8; 16]> {
    let mut bytes = [0u8; 16];
    let mut filled = 0;
    while filled < bytes.len() {
        // SAFETY: writes at most the rest of `bytes`.
        let n = unsafe {
            syscall(
                nr::GETRANDOM,
                bytes[filled..].as_mut_ptr(),
                bytes.len() - filled,
                0usize,
            )
        };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.raw_os_error() == Some(EINTR) {
                continue;
            }
            return Err(e);
        }
        filled += n as usize;
    }
    Ok(bytes)
}

/// Blocks every signal on this thread and returns the mask it had.
pub(crate) fn block_all() -> u64 {
    let all = u64::MAX;
    let mut old = 0u64;
    // SAFETY: two 8-byte kernel signal sets.
    unsafe {
        syscall(
            nr::RT_SIGPROCMASK,
            SIG_SETMASK as c_long,
            &all as *const u64,
            &mut old as *mut u64,
            8usize,
        )
    };
    old
}

/// Sets this thread's signal mask.
pub(crate) fn set_mask(mask: u64) {
    // SAFETY: an 8-byte kernel signal set.
    unsafe {
        syscall(
            nr::RT_SIGPROCMASK,
            SIG_SETMASK as c_long,
            &mask as *const u64,
            core::ptr::null_mut::<u64>(),
            8usize,
        )
    };
}

/// The handler installed for `signal`, as a word: 0 is the default, 1 is
/// ignore, anything else a function.
pub(crate) fn handler(signal: u32) -> usize {
    // The kernel's struct sigaction on both architectures: handler, flags,
    // restorer, mask.
    let mut action = [0u64; 4];
    // SAFETY: a query only; the kernel writes one struct sigaction.
    let r = unsafe {
        syscall(
            nr::RT_SIGACTION,
            c_long::from(signal),
            core::ptr::null::<u64>(),
            action.as_mut_ptr(),
            8usize,
        )
    };
    if r == 0 { action[0] as usize } else { 0 }
}

/// The program break.
pub(crate) fn program_break() -> usize {
    // SAFETY: brk(0) moves nothing and answers where the break is.
    unsafe { syscall(nr::BRK, 0usize) as usize }
}

/// Maps one inaccessible page where the program break ends, so the kernel
/// refuses to grow it and each C library's allocator falls back to `mmap`.
/// A page already there fences it as well.
pub(crate) fn fence_program_break(page: usize) -> io::Result<()> {
    let at = program_break().next_multiple_of(page);
    // SAFETY: MAP_FIXED_NOREPLACE never replaces a mapping; it fails with
    // EEXIST where one is.
    let got = unsafe {
        mmap(
            at as *mut c_void,
            page,
            PROT_NONE,
            MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE,
            -1,
            0,
        )
    };
    if got as isize == -1 {
        let e = io::Error::last_os_error();
        return if e.raw_os_error() == Some(EEXIST) {
            Ok(())
        } else {
            Err(e)
        };
    }
    if got as usize != at {
        // A kernel before 4.17 takes the flag as a hint.
        // SAFETY: the page just mapped, at an address nothing else knows.
        unsafe { munmap(got, page) };
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "the kernel placed the fence elsewhere (MAP_FIXED_NOREPLACE needs Linux 4.17)",
        ));
    }
    Ok(())
}

/// Counts the forks of this process, in the child.
static FORKS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

unsafe extern "C" fn forked() {
    FORKS.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
}

/// How many times this process's ancestors forked to make it, as the host
/// C library's fork handlers counted; registers the handler on first use.
pub(crate) fn forks() -> u64 {
    static REGISTER: std::sync::Once = std::sync::Once::new();
    REGISTER.call_once(|| {
        // SAFETY: `forked` touches one atomic, which a child may do.
        unsafe { pthread_atfork(None, None, Some(forked)) };
    });
    FORKS.load(core::sync::atomic::Ordering::SeqCst)
}

/// Like [`forks`], without registering: what a call compares.
pub(crate) fn forks_seen() -> u64 {
    FORKS.load(core::sync::atomic::Ordering::SeqCst)
}
