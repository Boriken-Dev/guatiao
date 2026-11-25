// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Native libraries in a statically linked Linux host (`static-dlopen`).
//!
//! A static program (a musl build, say) has no dynamic linker, so it cannot
//! map a shared library. This module maps the **system's own** `ld.so`,
//! hands it a program written in memory (headers and tables, no code) that
//! names the system C library, enters `ld.so` on a thread of its own, and
//! from then on asks that C library to open libraries and run their code.
//! guatiao can do this where most hosts cannot because nothing the host
//! holds needs a library's allocator: values cross as C shapes and are
//! copied into the caller's allocator, so two C libraries in one process
//! never free each other's memory.
//!
//! The registry uses it by itself: [`Registry::load_file`] and the scans
//! load through it when the host cannot `dlopen` (it is statically
//! linked), and through the platform's loader otherwise;
//! [`Registry::set_dlopen`] chooses either way. Nothing else in a host
//! names it.
//!
//! [`Registry::load_file`]: crate::library::Registry::load_file
//! [`Registry::set_dlopen`]: crate::library::Registry::set_dlopen
//!
//! # Threads
//!
//! Each C library keeps its per-thread state (`errno`, its allocator's
//! cache, thread-locals) behind the thread pointer, so library code must
//! never run on a host thread, nor host code on a foreign one.
//!
//! - **Every piece of library code runs on a foreign worker**, a thread
//!   the foreign C library made: its constructors, `dlopen`, `dlsym`, every
//!   call. Host code calls a library's function through a stub the
//!   registry puts in its place, which hands the call to a worker and
//!   waits (a short spin, then a futex): that hop is the cost of a call. A
//!   call finding every worker busy starts another, up to
//!   [`Settings::max_workers`].
//! - **A call into the host comes back to a host thread.** Every way a
//!   library reaches host code through this crate (a kind's slots, an
//!   object's, the allocators, the host's services, the C exports) asks
//!   first which side it runs on, and from a foreign thread runs on a host
//!   thread: the one whose call the worker is running, which waits in its
//!   call for exactly that; or, from a thread the library started itself,
//!   one of the host threads kept for that ([`Settings::callback_threads`]).
//!   A call back into the library from there goes to the same foreign
//!   thread, however deep.
//! - **The thread `ld.so` starts on** becomes the foreign C library's first
//!   thread; it never returns to host code, is never joined, and only
//!   starts workers.
//! - **Workers block asynchronous signals**, so a signal meant for the host
//!   never runs its handler on a foreign thread.
//!
//! A host function given to a library some other way than through this
//! crate (an allocator or an object table a C host fills in itself) is
//! called on the foreign thread, and must touch no thread-local state.
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
//! - **One per process.** The first start's settings hold for the life of
//!   the process.
//! - **`fork`**: the child has no workers; a call there is
//!   [`Error::Forked`], and one made through a stub ends the child.
//! - **Credentials**: a host `setuid` reaches its own C library's threads,
//!   not the foreign workers, which keep the credentials they started with.
//! - **Exit**: the host's `exit` ends every thread; the foreign C library's
//!   `atexit` handlers and library destructors do not run.
//!
//! The program asks for every C library symbol at the oldest version glibc
//! has on its architecture ([`Settings::program_needs`] reads them back):
//! GLIBC_2.2.5 and GLIBC_2.3.3 (`dladdr1`) on x86_64, GLIBC_2.17 on
//! aarch64. No C compiler or linker is involved, at build time or after.
//!
//! Linux on x86_64 and aarch64 only; elsewhere the module does not exist (a
//! host there can load libraries itself). Entering the system `ld.so` from
//! a static program is the technique of pfalcon's foreign-dlopen (MIT) and
//! of Cosmopolitan libc's `dlopen` (ISC); this module takes no code from
//! either.

pub(crate) mod bare;
mod elf;
mod error;
mod helper;
mod image;
mod settings;
mod stack;
mod start;
mod sys;
mod worker;
pub(crate) mod world;

pub use error::{Error, ImageError};
pub use helper::VersionNeed;
pub use settings::{Environment, OpenFlags, Settings, SignalSet};
pub use start::{Foreign, Job, Library, Symbol};
