// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a host decides about the loader. Every bound is here, and every
//! check defaults to the strict choice, which the host may weaken; only a
//! value that cannot work is refused.

#![forbid(unsafe_code)]

use alloc::string::String;
use alloc::vec::Vec;
use core::ops::BitOr;
use core::time::Duration;
use std::ffi::OsString;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use super::elf::Elf;
use super::error::Error;
use super::helper::{self, DEFAULT_LD_SO, DEFAULT_NEEDED, VersionNeed};

/// How the loader starts and behaves. Start from [`Settings::default`] and
/// change what the host needs.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Settings {
    /// The dynamic linker to enter, and the interpreter the program it
    /// starts names. `None`: the architecture's usual one
    /// (`/lib64/ld-linux-x86-64.so.2` on x86_64, `/lib/ld-linux-aarch64.so.1`
    /// on aarch64).
    pub ld_so: Option<PathBuf>,
    /// The libraries the program `ld.so` starts names, in order. Default
    /// `libdl.so.2`, `libpthread.so.0`, `libc.so.6`: before glibc 2.34 the
    /// first two hold `dlopen` and `pthread_create`.
    pub needed: Vec<String>,
    /// How many workers run jobs from the start; at least 1. Default 2.
    pub workers: usize,
    /// How many there may be: a call finding every worker busy starts
    /// another until there are this many, then waits for one. Default 256.
    pub max_workers: usize,
    /// The stack of the thread `ld.so` starts on, which then starts the
    /// workers for the life of the process. Default 8 MiB, mapped lazily;
    /// `ld.so` itself needs some 8 KiB to start.
    pub bootstrap_stack: usize,
    /// The alignment of the stack pointer `ld.so` is entered with: a power
    /// of two, at least the 16 both ABIs require. Default 16.
    pub stack_alignment: usize,
    /// Each worker's stack. `None`: the foreign C library's default.
    pub worker_stack: Option<usize>,
    /// How many times a waiting side looks again before it sleeps on the
    /// futex. Default 2000: a quick call answers within the spin, an idle
    /// worker sleeps.
    pub spin: u32,
    /// How many bytes of a caller's stack a call carries to the other
    /// side: the arguments past those passed in registers. A multiple of
    /// 16. Default 256, room for 32 words of arguments on the stack.
    pub stack_arguments: usize,
    /// How many host threads may run calls the library makes into the host
    /// from threads of its own, at once. `None`: as many as there are such
    /// calls waiting. Default `None`.
    pub callback_threads: Option<usize>,
    /// The signals workers block. Default: every asynchronous signal
    /// ([`SignalSet::asynchronous`]), so a signal meant for the host never
    /// runs its handler on a foreign thread. glibc keeps its own two (32 and
    /// 33) open whatever is asked.
    pub blocked_signals: SignalSet,
    /// The environment the foreign C library starts with. Default: the
    /// host's, without the dynamic linker's own variables.
    pub environment: Environment,
    /// How long to wait for the foreign C library to report ready. `None`
    /// waits for ever. Default 10 s.
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
    /// What [`Foreign::open`](super::Foreign::open) passes to `dlopen`.
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
            needed: DEFAULT_NEEDED.iter().map(|&n| n.into()).collect(),
            workers: 2,
            max_workers: 256,
            bootstrap_stack: 8 << 20,
            stack_alignment: 16,
            worker_stack: None,
            spin: 2000,
            stack_arguments: 256,
            callback_threads: None,
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
        if self.max_workers < self.workers {
            return refuse("max_workers", "fewer than the workers started");
        }
        if u32::try_from(self.max_workers).is_err() {
            return refuse("max_workers", "more than the loader counts (u32)");
        }
        if !self.stack_alignment.is_power_of_two() || self.stack_alignment < 16 {
            return refuse(
                "stack_alignment",
                "a power of two of at least 16, as both ABIs require",
            );
        }
        if !self.stack_arguments.is_multiple_of(16) {
            return refuse(
                "stack_arguments",
                "a multiple of 16, so the stack stays aligned",
            );
        }
        if self.callback_threads == Some(0) {
            return refuse(
                "callback_threads",
                "a call from a library's own thread needs a host thread: at least one",
            );
        }
        if self.needed.iter().any(|n| n.is_empty() || n.contains('\0')) {
            return refuse("needed", "a library name is empty or holds a NUL byte");
        }
        Ok(())
    }

    /// The program `ld.so` would start with these settings, as bytes: what
    /// [`Foreign::start`](super::Foreign::start) maps, here with a
    /// thread-local mark of zeroes.
    pub fn program(&self) -> Vec<u8> {
        helper::write(
            self.interpreter().as_os_str().as_bytes(),
            &self.needed,
            [0, 0],
        )
        .bytes
    }

    /// The C library versions that program needs. The newest of them is
    /// the oldest C library it runs on.
    pub fn program_needs(&self) -> Result<Vec<VersionNeed>, Error> {
        let bytes = self.program();
        Elf::parse(&bytes)
            .and_then(|e| e.version_needs())
            .map_err(|reason| Error::Program { reason })
    }

    /// The dynamic linker these settings enter.
    pub fn interpreter(&self) -> PathBuf {
        self.ld_so
            .clone()
            .unwrap_or_else(|| PathBuf::from(DEFAULT_LD_SO))
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

/// cbindgen:ignore
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

/// cbindgen:ignore
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
        s.workers = 2;
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
        s.stack_arguments = 24;
        assert!(s.check().is_err());
        s.stack_arguments = 0;
        s.max_workers = 1;
        assert!(s.check().is_err());
        s.max_workers = 2;
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
