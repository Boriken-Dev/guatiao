// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What can go wrong, each case naming what it was about.

#![forbid(unsafe_code)]

use alloc::string::String;
use core::fmt;
use core::time::Duration;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;

/// Why the foreign loader could not start, open, look up or close.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The dynamic linker could not be used: its path, and why.
    LdSo {
        /// The path read.
        path: PathBuf,
        /// What was wrong with it.
        reason: ImageError,
    },
    /// The program written for `ld.so` does not read back: a defect of
    /// this crate, reported rather than entered.
    Program {
        /// What was wrong with it.
        reason: ImageError,
    },
    /// A setting holds a value that cannot work.
    Setting {
        /// The field of [`Settings`](super::Settings).
        name: &'static str,
        /// Why the value cannot work.
        reason: String,
    },
    /// The host already handles a signal the foreign C library takes for
    /// itself when it starts.
    SignalInUse {
        /// The signal number.
        signal: u32,
    },
    /// The program break could not be fenced.
    Fence(io::Error),
    /// Memory for the images, the stack or the workers' mailboxes could not
    /// be had.
    Memory(io::Error),
    /// The thread `ld.so` starts on could not be created.
    Thread(io::Error),
    /// The foreign C library did not report ready in time; its thread
    /// stays parked.
    StartTimedOut(Duration),
    /// The program `ld.so` started did not accept the control block it was
    /// given, or found no relocated imports.
    Layout,
    /// The program found no thread-local mark near its thread pointer, so
    /// the host could not tell the foreign C library's threads from its
    /// own.
    NoMark,
    /// A foreign worker could not be created: the foreign C library's error
    /// number.
    Spawn(i32),
    /// The process forked since the loader started; its workers are in the
    /// parent.
    Forked,
    /// A name holds a NUL byte.
    Name(OsString),
    /// The foreign dynamic linker did not open a library.
    Open {
        /// What was asked for.
        name: OsString,
        /// `dlerror`'s text, cut at the settings' error capacity.
        message: String,
    },
    /// The foreign dynamic linker did not close a library.
    Close {
        /// `dlerror`'s text, cut at the settings' error capacity.
        message: String,
    },
    /// A library declares more than the settings' declaration window.
    DeclarationTooLarge {
        /// The declaration's size, from the library's symbol table.
        size: u64,
        /// The window.
        window: usize,
    },
    /// The library's symbol table gives no size for its declaration, and
    /// the settings do not read one without.
    DeclarationSizeUnknown,
    /// A declaration has no terminating empty string within what was read.
    DeclarationUnterminated {
        /// How many bytes were read.
        read: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::LdSo { path, reason } => {
                write!(f, "the dynamic linker {}: {reason}", path.display())
            }
            Error::Program { reason } => write!(f, "the program written for ld.so: {reason}"),
            Error::Setting { name, reason } => write!(f, "setting `{name}`: {reason}"),
            Error::SignalInUse { signal } => write!(
                f,
                "the host handles signal {signal}, which the foreign C library takes when it starts"
            ),
            Error::Fence(e) => write!(f, "the program break could not be fenced: {e}"),
            Error::Memory(e) => write!(f, "memory for the foreign loader: {e}"),
            Error::Thread(e) => write!(f, "the thread for the dynamic linker: {e}"),
            Error::StartTimedOut(t) => {
                write!(f, "the foreign C library was not ready after {t:?}")
            }
            Error::Layout => f.write_str(
                "the program ld.so started refused the control block or found no imports",
            ),
            Error::NoMark => f.write_str(
                "the foreign C library's threads carry no thread-local mark the host can find",
            ),
            Error::Spawn(code) => write!(
                f,
                "a foreign worker could not be created: {}",
                io::Error::from_raw_os_error(*code)
            ),
            Error::Forked => f.write_str(
                "the process forked since the foreign loader started; its workers are in the parent",
            ),
            Error::Name(name) => write!(f, "{} holds a NUL byte", name.to_string_lossy()),
            Error::Open { name, message } => {
                write!(f, "{} did not open: {message}", name.to_string_lossy())
            }
            Error::Close { message } => write!(f, "the library did not close: {message}"),
            Error::DeclarationTooLarge { size, window } => write!(
                f,
                "the declaration is {size} bytes, more than the window of {window}"
            ),
            Error::DeclarationSizeUnknown => f.write_str(
                "the library's symbol table gives no size for its declaration",
            ),
            Error::DeclarationUnterminated { read } => {
                write!(f, "the declaration does not end within {read} bytes")
            }
        }
    }
}

impl core::error::Error for Error {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Error::LdSo { reason, .. } | Error::Program { reason } => Some(reason),
            Error::Fence(e) | Error::Memory(e) | Error::Thread(e) => Some(e),
            _ => None,
        }
    }
}

/// Why a file could not be mapped and entered.
#[derive(Debug)]
#[non_exhaustive]
pub enum ImageError {
    /// It could not be read.
    Io(io::Error),
    /// It is not an ELF file.
    NotElf,
    /// It is not a 64-bit little-endian ELF file.
    Class,
    /// It is for another machine (`e_machine` values).
    Machine {
        /// What the file says.
        found: u16,
        /// What this build runs on.
        expected: u16,
    },
    /// It is not position-independent (`ET_DYN`): the `e_type` found.
    Type(u16),
    /// A dynamic linker that names an interpreter of its own.
    HasInterpreter,
    /// Its headers do not describe a file this loader can map.
    Malformed(&'static str),
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImageError::Io(e) => write!(f, "{e}"),
            ImageError::NotElf => f.write_str("not an ELF file"),
            ImageError::Class => f.write_str("not a 64-bit little-endian ELF file"),
            ImageError::Machine { found, expected } => write!(
                f,
                "built for machine {found}, and this program runs on {expected}"
            ),
            ImageError::Type(t) => write!(f, "ELF type {t}, not position-independent (3)"),
            ImageError::HasInterpreter => f.write_str(
                "names an interpreter of its own: it is a program, not a dynamic linker",
            ),
            ImageError::Malformed(what) => write!(f, "malformed: {what}"),
        }
    }
}

impl core::error::Error for ImageError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            ImageError::Io(e) => Some(e),
            _ => None,
        }
    }
}
