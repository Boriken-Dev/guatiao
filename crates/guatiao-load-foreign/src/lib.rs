// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Native guatiao libraries in a statically linked Linux host.
//!
//! A static program (a musl build, say) has no dynamic linker, so it cannot
//! map a shared library. This crate maps the **system's own** `ld.so` and a
//! small helper program built against the system C library, enters `ld.so`
//! on a thread of its own, and from then on asks that C library to open
//! libraries and run their code. guatiao can do this where most hosts cannot
//! because nothing the host holds needs a library's allocator: values cross
//! as C shapes and are copied into the caller's allocator, so two C libraries
//! in one process never free each other's memory.
//!
//! # Threads
//!
//! Each C library keeps its per-thread state (`errno`, its allocator's
//! cache, thread-locals) behind the thread pointer, so library code must
//! never run on a host thread, nor host code on a foreign one. Every piece
//! of library code (its constructors, `dlopen`, `dlsym`, every call) runs on
//! a **foreign worker**: a thread the foreign C library made. The thread
//! `ld.so` starts on becomes the first worker; it never returns to host code
//! and is never joined. The host hands a worker a job and waits for it (a
//! short spin, then a futex), which is the whole cost of a call. Workers
//! block asynchronous signals, so a signal meant for the host never runs its
//! handler on a foreign thread.
//!
//! ```ignore
//! use guatiao_load_foreign::{Foreign, Settings};
//! let foreign = Foreign::start(&Settings::default())?;
//! let library = foreign.open("/opt/plugins/libgreeter.so")?;
//! let declared = library.declaration()?;          // `guatiao_declares`, copied
//! let entry = library.entry()?;                   // `guatiao_library_entry`
//! ```
//!
//! # What the process shares, and what it does not
//!
//! - **The program break**: both C libraries' allocators would grow it, each
//!   from its own idea of where it ends, and shrink the other's heap. By
//!   default the loader fences it (a page mapped at its end) so both fall
//!   back to `mmap`; [`Settings::fence_program_break`].
//! - **Signal handlers are process-wide.** glibc installs its handler on
//!   signal 33 when it starts (glibc's set-id broadcast, musl's
//!   cancellation); the loader refuses to start when the host already
//!   handles 32 or 33 ([`Settings::allow_reserved_signal_handlers`]) and
//!   reports what changed ([`Foreign::signals_taken`]).
//! - **`fork`**: the child has no workers; a call there is
//!   [`Error::Forked`].
//! - **Credentials**: a host `setuid` reaches its own C library's threads,
//!   not the foreign workers, which keep the credentials they started with.
//! - **Exit**: the host's `exit` ends every thread; the foreign C library's
//!   `atexit` handlers and library destructors do not run.
//!
//! The helper's C library symbols are pinned to the oldest versions glibc
//! has on each architecture ([`Helper::needs`] reads them back): GLIBC_2.2.5
//! and GLIBC_2.3.3 (`dladdr1`) on x86_64, GLIBC_2.17 on aarch64.
//!
//! Linux on x86_64 and aarch64 only; elsewhere the crate is empty (a host
//! there can load libraries itself). Entering the system `ld.so` from a
//! static program is the technique of pfalcon's foreign-dlopen (MIT) and of
//! Cosmopolitan libc's `dlopen` (ISC); this crate takes no code from either.

// Off Linux the crate is empty, and the links above name nothing.
#![cfg_attr(
    not(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )),
    allow(unused, rustdoc::broken_intra_doc_links)
)]

extern crate alloc;

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod elf;
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod error;
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod foreign;
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod helper;
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod image;
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod settings;
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod stack;
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod sys;
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod worker;

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
pub use error::{Error, ImageError};
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
pub use foreign::{Foreign, Job, Library, Symbol};
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
pub use helper::{Helper, VersionNeed};
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
pub use settings::{Environment, OpenFlags, Settings, SignalSet};
