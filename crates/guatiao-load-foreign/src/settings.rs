// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a host decides about the loader. Every bound is here, and every
//! check defaults to the strict choice, which the host may weaken; only a
//! value that cannot work is refused.

#![forbid(unsafe_code)]

use alloc::vec::Vec;
use core::ops::BitOr;
use core::time::Duration;
use std::ffi::OsString;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use crate::error::Error;
use crate::helper::Helper;

/// How the loader starts and behaves. Start from [`Settings::default`] and
/// change what the host needs.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Settings {
    /// The dynamic linker to enter. `None`: the interpreter the helper names
    /// (`/lib64/ld-linux-x86-64.so.2` on x86_64, `/lib/ld-linux-aarch64.so.1`
    /// on aarch64, for a helper built on a usual glibc system).
    pub ld_so: Option<PathBuf>,
    /// The helper program. Default: the one built into this crate.
    pub helper: Helper,
    /// How many foreign workers run jobs, the bootstrap thread included; at
    /// least 1. Default 1.
    pub workers: usize,
    /// The bootstrap thread's stack, which becomes the first worker's for
    /// the life of the process. Default 8 MiB, mapped lazily; `ld.so` itself
    /// needs some 8 KiB to start.
    pub bootstrap_stack: usize,
    /// The alignment of the stack pointer `ld.so` is entered with: a power
    /// of two, at least the 16 both ABIs require. Default 16.
    pub stack_alignment: usize,
    /// The stack of each worker after the first. `None`: the foreign C
    /// library's default.
    pub worker_stack: Option<usize>,
    /// How many times a waiting side looks again before it sleeps on the
    /// futex. Default 2000: a quick call answers within the spin, an idle
    /// worker sleeps.
    pub spin: u32,
    /// The signals workers block. Default: every asynchronous signal
    /// ([`SignalSet::asynchronous`]), so a signal meant for the host never
    /// runs its handler on a foreign thread. glibc keeps its own two (32 and
    /// 33) open whatever is asked.
    pub blocked_signals: SignalSet,
    /// The environment the foreign C library starts with. Default: the
    /// host's, without the dynamic linker's own variables.
    pub environment: Environment,
    /// How long to wait for the helper to report ready. `None` waits for
    /// ever. Default 10 s.
    pub start_timeout: Option<Duration>,
    /// How many bytes of a `dlerror` message are kept. Default 1024.
    pub error_capacity: usize,
    /// The most bytes a library's declaration may span. Default 4096, as a
    /// scan reads.
    pub declaration_window: usize,
    /// Read a declaration whose size the library's symbol table does not
    /// give, up to the window. Default false: such a declaration is
    /// refused, since reading a window past its end may leave the image.
    pub read_unsized_declaration: bool,
    /// What [`Foreign::open`](crate::Foreign::open) passes to `dlopen`.
    /// Default `NOW | LOCAL`: every symbol bound before the library's code
    /// runs, none made visible to the next library.
    pub open_flags: OpenFlags,
    /// Start even when the host handles signal 32 or 33, which glibc takes
    /// when it starts. Default false.
    pub allow_reserved_signal_handlers: bool,
    /// Fence the program break before the foreign C library starts, so
    /// neither allocator grows it from a stale idea of where it ends.
    /// Default true.
    pub fence_program_break: bool,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            ld_so: None,
            helper: Helper::Built,
            workers: 1,
            bootstrap_stack: 8 << 20,
            stack_alignment: 16,
            worker_stack: None,
            spin: 2000,
            blocked_signals: SignalSet::asynchronous(),
            environment: Environment::default(),
            start_timeout: Some(Duration::from_secs(10)),
            error_capacity: 1024,
            declaration_window: 4096,
            read_unsized_declaration: false,
            open_flags: OpenFlags::NOW | OpenFlags::LOCAL,
            allow_reserved_signal_handlers: false,
            fence_program_break: true,
        }
    }
}

impl Settings {
    /// Refuses a value that cannot work.
    pub(crate) fn check(&self) -> Result<(), Error> {
        let refuse = |name, reason: &str| {
            Err(Error::Setting {
                name,
                reason: reason.into(),
            })
        };
        if self.workers == 0 {
            return refuse("workers", "every job runs on a worker: at least one");
        }
        if u32::try_from(self.workers).is_err() {
            return refuse("workers", "more than the helper counts (u32)");
        }
        if !self.stack_alignment.is_power_of_two() || self.stack_alignment < 16 {
            return refuse(
                "stack_alignment",
                "a power of two of at least 16, as both ABIs require",
            );
        }
        Ok(())
    }
}

/// The environment the foreign C library starts with. It reads it once, so
/// a later change to the host's environment does not reach it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Environment {
    /// The host's, without the variables that steer the dynamic linker
    /// (`LD_*`, `GLIBC_TUNABLES`), which a static host was never subject
    /// to.
    #[default]
    WithoutLoaderVariables,
    /// The host's as it is.
    Inherit,
    /// Exactly these `NAME=value` entries.
    Exact(Vec<OsString>),
}

impl Environment {
    /// The entries, each with its NUL.
    pub(crate) fn entries(&self) -> Vec<Vec<u8>> {
        let with_nul = |mut bytes: Vec<u8>| {
            bytes.push(0);
            bytes
        };
        let host = |keep: &dyn Fn(&[u8]) -> bool| {
            std::env::vars_os()
                .filter(|(name, _)| keep(name.as_bytes()))
                .map(|(name, value)| {
                    let mut entry = name.as_bytes().to_vec();
                    entry.push(b'=');
                    entry.extend_from_slice(value.as_bytes());
                    with_nul(entry)
                })
                .collect()
        };
        match self {
            Environment::WithoutLoaderVariables => {
                host(&|name| !name.starts_with(b"LD_") && name != b"GLIBC_TUNABLES")
            }
            Environment::Inherit => host(&|_| true),
            Environment::Exact(entries) => entries
                .iter()
                .filter(|e| !e.as_bytes().contains(&0))
                .map(|e| with_nul(e.as_bytes().to_vec()))
                .collect(),
        }
    }
}

/// A set of signals 1 to 64.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct SignalSet(u64);

/// Faults the kernel raises on the thread that caused them, which blocking
/// cannot defer: illegal instruction, trap, bus error, floating point,
/// segmentation fault, bad system call.
const SYNCHRONOUS: [u32; 6] = [4, 5, 7, 8, 11, 31];

impl SignalSet {
    /// No signal.
    pub const EMPTY: SignalSet = SignalSet(0);

    /// Every signal.
    pub const fn all() -> SignalSet {
        SignalSet(u64::MAX)
    }

    /// Every signal but the synchronous faults.
    pub const fn asynchronous() -> SignalSet {
        let mut set = SignalSet::all();
        let mut i = 0;
        while i < SYNCHRONOUS.len() {
            set = set.without(SYNCHRONOUS[i]);
            i += 1;
        }
        set
    }

    /// The set from its bits: signal `n` is bit `n - 1`.
    pub const fn from_bits(bits: u64) -> SignalSet {
        SignalSet(bits)
    }

    /// Signal `n` is bit `n - 1`.
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// With `signal` (1 to 64; another number changes nothing).
    pub const fn with(self, signal: u32) -> SignalSet {
        match bit(signal) {
            Some(b) => SignalSet(self.0 | b),
            None => self,
        }
    }

    /// Without `signal`.
    pub const fn without(self, signal: u32) -> SignalSet {
        match bit(signal) {
            Some(b) => SignalSet(self.0 & !b),
            None => self,
        }
    }

    /// Whether `signal` is in the set.
    pub const fn contains(self, signal: u32) -> bool {
        match bit(signal) {
            Some(b) => self.0 & b != 0,
            None => false,
        }
    }

    /// Whether the set is empty.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The signals in the set, lowest first.
    pub fn iter(self) -> impl Iterator<Item = u32> {
        (1..=64).filter(move |&s| self.contains(s))
    }
}

const fn bit(signal: u32) -> Option<u64> {
    if matches!(signal, 1..=64) {
        Some(1 << (signal - 1))
    } else {
        None
    }
}

/// `dlopen`'s flags, as glibc numbers them on x86_64 and aarch64.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OpenFlags(i32);

impl OpenFlags {
    /// Bind each function on its first call.
    pub const LAZY: OpenFlags = OpenFlags(0x1);
    /// Bind every symbol before `dlopen` returns.
    pub const NOW: OpenFlags = OpenFlags(0x2);
    /// Only answer a library already open.
    pub const NOLOAD: OpenFlags = OpenFlags(0x4);
    /// Prefer the library's own symbols to the global scope's.
    pub const DEEPBIND: OpenFlags = OpenFlags(0x8);
    /// Make the library's symbols visible to libraries opened later.
    pub const GLOBAL: OpenFlags = OpenFlags(0x100);
    /// Keep them to the library (the default when `GLOBAL` is absent).
    pub const LOCAL: OpenFlags = OpenFlags(0);
    /// Never unmap the library, even after `dlclose`.
    pub const NODELETE: OpenFlags = OpenFlags(0x1000);

    /// The flags from `dlopen`'s integer.
    pub const fn from_bits(bits: i32) -> OpenFlags {
        OpenFlags(bits)
    }

    /// `dlopen`'s integer.
    pub const fn bits(self) -> i32 {
        self.0
    }
}

impl BitOr for OpenFlags {
    type Output = OpenFlags;
    fn bitor(self, other: OpenFlags) -> OpenFlags {
        OpenFlags(self.0 | other.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_that_cannot_work_are_refused_and_the_rest_taken() {
        let mut s = Settings::default();
        assert!(s.check().is_ok());
        s.workers = 0;
        assert!(matches!(
            s.check(),
            Err(Error::Setting {
                name: "workers",
                ..
            })
        ));
        s.workers = 1;
        for bad in [0, 8, 24] {
            s.stack_alignment = bad;
            assert!(matches!(
                s.check(),
                Err(Error::Setting {
                    name: "stack_alignment",
                    ..
                })
            ));
        }
        s.stack_alignment = 64;
        s.spin = 0;
        s.error_capacity = 0;
        s.declaration_window = 0;
        assert!(s.check().is_ok());
    }

    #[test]
    fn the_default_blocked_set_leaves_only_the_faults_open() {
        let set = SignalSet::asynchronous();
        for s in SYNCHRONOUS {
            assert!(!set.contains(s));
        }
        assert!(set.contains(2) && set.contains(15) && set.contains(64));
        assert_eq!(set.iter().count(), 64 - SYNCHRONOUS.len());
        assert!(!SignalSet::EMPTY.with(0).with(65).contains(65));
    }
}
