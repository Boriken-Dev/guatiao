// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The one file in this module that contains `unsafe`.
//!
//! Everything that reads a descriptor through a raw pointer, everything
//! that opens a library, and the macro that writes a library's entry
//! point, in one place — so the module around it stays provably safe and
//! an auditor's scope is this file.
//!
//! # A library that has been loaded is never unloaded
//!
//! Every tree a library builds records the address of the allocator that
//! made it, the descriptor's text points into the library's read-only
//! data, and a vtable points at its code. Unloading invalidates all three
//! at once, and the first symptom is a free through an unmapped function
//! pointer at teardown — a long way from the mistake.
//!
//! So the handle is **forgotten**, deliberately, and the mapping stays
//! for the life of the process. That is the same answer [`crate::value::Alloc`] states
//! as a rule: an allocator outlives everything built through it.
//!
//! # Loading is an irreversible probe
//!
//! There is no way to ask whether a file exports a symbol without mapping
//! it, and mapping it runs its static initialisers, which may do anything
//! including abort the process. A loader therefore never opens a file the
//! caller did not name or a directory the caller did not curate.

#![allow(non_camel_case_types)]
// The descriptor readers are used by the loader, which is behind a
// feature. A library AUTHOR compiles this file for the macro and the entry
// type alone, so without that feature they are genuinely unused — and
// four `#[cfg]`s on items that are correct either way would be four more
// things to keep in step than saying it once.
#![cfg_attr(not(feature = "load"), allow(dead_code))]

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};

use super::desc::{HostInfo, LibraryInfo, ProviderInfo};
use crate::value::types::{Str, Value};

/// The symbol a library exports, NUL-terminated for the loader.
pub const ENTRY_SYMBOL: &[u8] = b"guatiao_library_entry\0";

/// The entry point's signature.
///
/// Returning null means **"nothing for this host"**, which is an answer
/// rather than a failure: a library that supports one host and is loaded
/// by another says so this way, and the loader reports it as skipped.
pub type EntryFn = unsafe extern "C" fn(*const HostInfo) -> *const LibraryInfo;

/// Writes a library's entry point.
///
/// Give it a function taking `&HostInfo` and answering
/// `Option<&'static LibraryInfo>`; it emits the `extern "C"` symbol a
/// loader looks for, catches a panic rather than letting one cross the
/// boundary, and answers null for "nothing for this host".
///
/// ```ignore
/// fn describe(host: &guatiao::library::HostInfo) -> Option<&'static guatiao::library::LibraryInfo> {
///     // build a descriptor, keep it in a `OnceLock` of your own
/// }
/// guatiao::guatiao_library!(describe);
/// ```
///
/// # Why a macro rather than a documented signature
///
/// So a library author never writes `unsafe` and never writes
/// `no_mangle` — a misspelled symbol name is a library that loads and
/// offers nothing, which is the failure with the least evidence attached
/// to it. The expansion also holds the unwind discipline the rest of this
/// crate's boundaries use: a panic becomes null, and the payload is
/// forgotten rather than dropped, because dropping it can panic again and
/// a second panic in an `extern "C"` body is an abort.
#[macro_export]
macro_rules! guatiao_library {
    ($describe:path) => {
        /// The symbol a `guatiao` host loads this library by.
        ///
        /// # Safety
        ///
        /// Called by a loader with either null or a pointer to a
        /// well-formed host descriptor.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn guatiao_library_entry(
            host: *const $crate::library::HostInfo,
        ) -> *const $crate::library::LibraryInfo {
            // SAFETY: the loader's side of the contract is that `host` is
            // null or points at a host descriptor declaring its own size.
            let info = unsafe { $crate::library::read_host(host) };
            let Some(info) = info else {
                return ::core::ptr::null();
            };
            $crate::library::answer(|| $describe(&info))
        }
    };
}

/// Reads a host descriptor, honouring its declared size.
///
/// # Safety
///
/// `raw` is null, or points at `raw->struct_size` readable, aligned bytes
/// that stay valid for the call. Nothing past `struct_size` is read.
pub unsafe fn read_host(raw: *const HostInfo) -> Option<HostInfo> {
    if raw.is_null() {
        return None;
    }
    // The leading `u32` only, through the raw pointer. Forming `&*raw`
    // first would assert the whole struct is dereferenceable, which is
    // exactly what a caller with a smaller one does not have.
    // SAFETY: the caller guarantees the first four bytes are readable.
    let declared = unsafe { std::ptr::addr_of!((*raw).struct_size).read() } as usize;
    if declared < HostInfo::floor() {
        return None;
    }

    // SAFETY: every read below is a field projection through the raw
    // pointer, and each field lies within `declared` bytes.
    let mut info = unsafe {
        HostInfo {
            struct_size: declared as u32,
            abi_version: std::ptr::addr_of!((*raw).abi_version).read(),
            host_id: std::ptr::addr_of!((*raw).host_id).read(),
            host_version: std::ptr::addr_of!((*raw).host_version).read(),
            alloc: std::ptr::null(),
        }
    };

    // The appended-field guard. `>=` against offset plus size means a
    // declared size covering only PART of the slot reads as absent rather
    // than handing back half a real pointer and half of whatever followed.
    if declared >= HostInfo::alloc_end() {
        // SAFETY: the guard just established the field is present.
        info.alloc = unsafe { std::ptr::addr_of!((*raw).alloc).read() };
    }
    Some(info)
}

/// Runs a library's `describe` and converts the answer to what the entry
/// point returns.
///
/// A panic becomes null, which a loader reports as "declined" rather than
/// as a crash.
pub fn answer(describe: impl FnOnce() -> Option<&'static LibraryInfo>) -> *const LibraryInfo {
    match catch_unwind(AssertUnwindSafe(describe)) {
        Ok(Some(desc)) => desc as *const LibraryInfo,
        Ok(None) => std::ptr::null(),
        Err(payload) => {
            // Dropping a panic payload can panic, and a second panic
            // inside an `extern "C"` body is an abort.
            std::mem::forget(payload);
            std::ptr::null()
        }
    }
}

/// A borrowed `&str` from a view, or `None` when it is not text.
///
/// # Safety
///
/// `s` is a view whose `len` bytes are readable and stay so.
pub(crate) unsafe fn str_of(s: Str) -> Option<&'static str> {
    if s.len == 0 {
        return Some("");
    }
    if s.ptr.is_null() {
        return None;
    }
    // SAFETY: the caller guarantees `len` readable bytes at `ptr`, and the
    // library they are in is never unloaded.
    let bytes = unsafe { std::slice::from_raw_parts(s.ptr, s.len) };
    std::str::from_utf8(bytes).ok()
}

/// One provider, read out of a descriptor once so nothing downstream has
/// to hold a raw pointer to read a name.
#[derive(Debug, Clone)]
pub struct ProviderView {
    /// What sort of thing it is.
    pub kind: String,
    /// Its identifier within that kind.
    pub id: String,
    /// A name to show a person, possibly empty.
    pub display_name: String,
    /// Its configuration schema, or `None`.
    pub config: Option<&'static Value>,
    /// The function table, whose shape the kind defines.
    pub vtable: *const c_void,
    /// The size the library compiled that table at.
    pub vtable_size: usize,
    /// Handed back to every call through the table.
    pub ctx: *mut c_void,
}

/// Reads a library descriptor and every provider in it.
///
/// # Safety
///
/// `raw` came from a library's entry point and points at a descriptor
/// that stays valid for the life of the process, which is what never
/// unloading the library guarantees.
pub(crate) unsafe fn read_library(
    raw: *const LibraryInfo,
) -> Option<(String, String, Vec<ProviderView>)> {
    if raw.is_null() {
        return None;
    }
    // SAFETY: the caller guarantees the leading word is readable.
    let declared = unsafe { std::ptr::addr_of!((*raw).struct_size).read() } as usize;
    if declared < LibraryInfo::floor() {
        return None;
    }

    // SAFETY: each field lies within `declared` bytes.
    let (id, version, providers) = unsafe {
        (
            str_of(std::ptr::addr_of!((*raw).id).read())?.to_string(),
            str_of(std::ptr::addr_of!((*raw).version).read())?.to_string(),
            std::ptr::addr_of!((*raw).providers).read(),
        )
    };

    let mut out = Vec::with_capacity(providers.len);
    if providers.len > 0 && providers.ptr.is_null() {
        return None;
    }
    for i in 0..providers.len {
        // SAFETY: the library declared `len` descriptors at `ptr`.
        let entry = unsafe { providers.ptr.add(i) };
        // SAFETY: forwarded.
        out.push(unsafe { read_provider(entry) }?);
    }
    Some((id, version, out))
}

/// # Safety
///
/// `raw` points at a provider descriptor declaring its own size.
unsafe fn read_provider(raw: *const ProviderInfo) -> Option<ProviderView> {
    // SAFETY: the caller guarantees the leading word is readable.
    let declared = unsafe { std::ptr::addr_of!((*raw).struct_size).read() } as usize;
    if declared < ProviderInfo::floor() {
        return None;
    }
    // SAFETY: each field lies within `declared` bytes.
    unsafe {
        let config = std::ptr::addr_of!((*raw).config).read();
        Some(ProviderView {
            kind: str_of(std::ptr::addr_of!((*raw).kind).read())?.to_string(),
            id: str_of(std::ptr::addr_of!((*raw).id).read())?.to_string(),
            display_name: str_of(std::ptr::addr_of!((*raw).display_name).read())?.to_string(),
            // A descriptor's value lives as long as the library, which is
            // for the life of the process.
            config: config.as_ref(),
            vtable: std::ptr::addr_of!((*raw).vtable).read(),
            vtable_size: std::ptr::addr_of!((*raw).vtable_size).read() as usize,
            ctx: std::ptr::addr_of!((*raw).ctx).read(),
        })
    }
}

/// Opens a library, calls its entry point, and **forgets the handle**.
///
/// Answers `Ok(None)` when the file has no entry symbol or when the
/// library declined this host: both are "not a library for us" rather than
/// failures.
///
/// # Safety
///
/// Mapping a library runs its static initialisers, which may do anything.
/// The caller is responsible for only naming files it is willing to run.
#[cfg(feature = "load")]
pub(crate) unsafe fn open(
    path: &std::path::Path,
    host: &HostInfo,
) -> Result<Option<(String, String, Vec<ProviderView>)>, libloading::Error> {
    // SAFETY: the caller's side of the contract, stated above.
    let library = unsafe { libloading::Library::new(path)? };

    // SAFETY: the symbol either is absent, which is an error we report,
    // or has the signature this crate defines for it.
    let entry = unsafe { library.get::<EntryFn>(ENTRY_SYMBOL) };
    let Ok(entry) = entry else {
        // No entry symbol: not a library. The mapping still stays, because
        // it has already run whatever it was going to run and unmapping
        // buys nothing back.
        std::mem::forget(library);
        return Ok(None);
    };

    // SAFETY: the library's side of the contract is that its entry point
    // reads a host descriptor and returns a descriptor or null.
    let desc = unsafe { entry(host as *const HostInfo) };

    // FORGOTTEN, NOT DROPPED. Everything the library just handed back
    // points into this mapping: the descriptor, its text, its vtables,
    // and the allocator inside any tree it later builds.
    std::mem::forget(library);

    if desc.is_null() {
        return Ok(None);
    }
    // SAFETY: non-null, from the entry point, and the mapping is
    // permanent.
    Ok(unsafe { read_library(desc) })
}

/// Loads one file the caller named, which is the caller's choice to run
/// whatever is in it.
///
/// Safe to CALL, because the decision this wraps is not a memory-safety
/// question the caller could get wrong by accident — it is "am I willing
/// to run this file", which only the caller can answer and which
/// [`crate::library::Registry::load_file`] states in its own
/// documentation. The `unsafe` that answer authorises is confined to this
/// file.
#[cfg(feature = "load")]
pub(crate) fn open_library(
    path: &std::path::Path,
    host: &HostInfo,
) -> Result<Option<(String, String, Vec<ProviderView>)>, libloading::Error> {
    // SAFETY: naming a file is choosing to run it, which is the contract
    // stated on `Registry::load_file` and on `open` below.
    unsafe { open(path, host) }
}
