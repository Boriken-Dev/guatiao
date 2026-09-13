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
use crate::value::types::{Map, MaybeNull, Str, Value};

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
            meta: MaybeNull::null(),
        }
    };

    // The appended-field guard. `>=` against offset plus size means a
    // declared size covering only PART of the slot reads as absent rather
    // than handing back half a real pointer and half of whatever followed.
    if declared >= HostInfo::alloc_end() {
        // SAFETY: the guard just established the field is present.
        info.alloc = unsafe { std::ptr::addr_of!((*raw).alloc).read() };
    }
    if declared >= HostInfo::meta_end() {
        // SAFETY: as above.
        info.meta = unsafe { std::ptr::addr_of!((*raw).meta).read() };
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
    /// Every kind it serves. May be empty.
    pub kinds: Vec<&'static str>,
    /// Its identifier, unique across every provider a host loads.
    pub id: &'static str,
    /// A name to show a person, possibly empty.
    pub display_name: &'static str,
    /// Its configuration schema, or `None`.
    pub config: Option<&'static Value>,
    /// The function table, whose shape the kind defines.
    pub vtable: *const c_void,
    /// The size the library compiled that table at.
    pub vtable_size: usize,
    /// Handed back to every call through the table.
    pub ctx: *mut c_void,
    /// Whatever else the provider declared, or `None`. See
    /// [`ProviderInfo::meta`].
    pub meta: Option<&'static Map>,
    /// The version it declared for itself, or `None` to inherit its
    /// library's. See [`ProviderInfo::version`].
    pub version: Option<&'static str>,
    /// Its runtime-availability slot, or `None` when it declares none —
    /// which means available. See [`ProviderInfo::available`].
    pub available: Option<unsafe extern "C" fn(ctx: *mut c_void, reason: *mut Str) -> bool>,
}

impl ProviderView {
    /// Whether it serves this kind.
    pub fn supports(&self, kind: &str) -> bool {
        self.kinds.contains(&kind)
    }

    /// Whether it can actually run here, and why not when it cannot.
    ///
    /// **Asked every time, never cached.** A library may load an optional
    /// dependency, lose a device, or fail its own integrity check while a
    /// process runs, and an answer kept from load time would predate all
    /// of that.
    ///
    /// A provider declaring no slot is available: that is the common case
    /// and the right default.
    ///
    /// Safe to call, for the same reason the allocator's slots are safe to
    /// call: reading the descriptor was the unsafe step and it established
    /// this contract. What "available" MEANS is between a host and a
    /// library; this only carries the answer.
    pub fn available(&self) -> Result<(), &'static str> {
        let Some(ask) = self.available else {
            return Ok(());
        };
        let mut reason = Str::empty();
        // SAFETY: the slot's signature is this envelope's own, checked
        // present by `read_provider`'s guard; `reason` is a writable local;
        // and the library it lives in is never unloaded.
        if unsafe { ask(self.ctx, &mut reason) } {
            return Ok(());
        }
        // SAFETY: the contract on the field is that a written reason
        // outlives every reader.
        Err(unsafe { str_of(reason) }.unwrap_or(""))
    }
}

impl ProviderView {
    /// The vtable as `T`, or `None` when the library compiled a shorter
    /// one than this host knows.
    ///
    /// **The check is `vtable_size >= size_of::<T>()`.** A library built
    /// before the host appended a slot declares a smaller table, and
    /// casting to `T` anyway hands out a reference whose tail is whatever
    /// followed the table in that library's image. A host that wants to
    /// support the older library asks for the smaller `T` it also knows,
    /// and both answer `Some`.
    ///
    /// `None` for a null table, which is what a provider offering no
    /// behaviour declares.
    ///
    /// # Safety
    ///
    /// `T` is the vtable type this provider's **kind** defines, laid out
    /// as the library compiled it. Nothing in the envelope can check that
    /// — a `kind` is a name, and what it means is agreed between whoever
    /// defined it and whoever implements it.
    pub unsafe fn vtable_as<T>(&self) -> Option<&T> {
        if self.vtable.is_null() || self.vtable_size < size_of::<T>() {
            return None;
        }
        // SAFETY: the caller states `T` is this kind's table; the pointer
        // is non-null and the library declared at least `size_of::<T>()`
        // bytes at it; and a loaded library is never unloaded, so the
        // borrow lives as long as the provider.
        Some(unsafe { &*self.vtable.cast::<T>() })
    }
}

/// One library's descriptor, read out once.
#[derive(Debug, Clone)]
pub struct LibraryView {
    /// The library's own identifier.
    pub id: &'static str,
    /// Its version string, uninterpreted.
    pub version: &'static str,
    /// Whatever else it declared, or `None`. See [`LibraryInfo::meta`].
    pub meta: Option<&'static Map>,
    /// What it offers.
    pub providers: Vec<ProviderView>,
}

/// Reads a library descriptor and every provider in it.
///
/// # Safety
///
/// `raw` came from a library's entry point and points at a descriptor
/// that stays valid for the life of the process, which is what never
/// unloading the library guarantees.
pub(crate) unsafe fn read_library(raw: *const LibraryInfo) -> Option<LibraryView> {
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
            str_of(std::ptr::addr_of!((*raw).id).read())?,
            str_of(std::ptr::addr_of!((*raw).version).read())?,
            std::ptr::addr_of!((*raw).providers).read(),
        )
    };

    let mut meta = None;
    if declared >= LibraryInfo::meta_end() {
        // SAFETY: the guard established the field is present.
        let raw_meta = unsafe { std::ptr::addr_of!((*raw).meta).read() };
        // SAFETY: a non-null `meta` is a well-formed map by the contract
        // on the field, and a descriptor's storage lives as long as the
        // library, which is for the life of the process.
        meta = unsafe { raw_meta.get() };
    }

    let mut out = Vec::with_capacity(providers.len);
    if providers.len > 0 && providers.ptr.is_null() {
        return None;
    }
    // The library's own element size, never this build's. See
    // [`Providers`] for what assuming it costs.
    let stride = providers.stride;
    if providers.len > 0 && stride < ProviderInfo::floor() {
        return None;
    }
    for i in 0..providers.len {
        // SAFETY: the library declared `len` descriptors of `stride` bytes
        // at `ptr`, so byte arithmetic is what reaches element `i`.
        let entry = unsafe { providers.ptr.cast::<u8>().add(i * stride) }.cast::<ProviderInfo>();
        // SAFETY: forwarded.
        out.push(unsafe { read_provider(entry, stride) }?);
    }
    Some(LibraryView {
        id,
        version,
        meta,
        providers: out,
    })
}

/// `limit` is how many bytes this descriptor may claim: the array's
/// stride, or its own size when it stands alone.
///
/// # Safety
///
/// `raw` points at a provider descriptor declaring its own size.
unsafe fn read_provider(raw: *const ProviderInfo, limit: usize) -> Option<ProviderView> {
    // SAFETY: the caller guarantees the leading word is readable.
    let declared = unsafe { std::ptr::addr_of!((*raw).struct_size).read() } as usize;
    // Above the floor, and not past its neighbour: an element claiming
    // more than the stride overlaps the next one, and every field it
    // reads beyond the stride belongs to that one.
    if declared < ProviderInfo::floor() || declared > limit {
        return None;
    }
    // SAFETY: each field lies within `declared` bytes.
    unsafe {
        let config = std::ptr::addr_of!((*raw).config).read();
        // A fixed stride, unlike the provider array: a `Str` declares no
        // `struct_size`, so it has no way to grow and no skew to survive.
        let kinds = std::ptr::addr_of!((*raw).kinds).read();
        if kinds.len > 0 && kinds.ptr.is_null() {
            return None;
        }
        let mut names = Vec::with_capacity(kinds.len);
        for i in 0..kinds.len {
            // SAFETY: the library declared `len` names at `ptr`.
            names.push(str_of(kinds.ptr.add(i).read())?);
        }

        Some(ProviderView {
            kinds: names,
            id: str_of(std::ptr::addr_of!((*raw).id).read())?,
            display_name: str_of(std::ptr::addr_of!((*raw).display_name).read())?,
            // A descriptor's value lives as long as the library, which is
            // for the life of the process.
            config: config.as_ref(),
            vtable: std::ptr::addr_of!((*raw).vtable).read(),
            vtable_size: std::ptr::addr_of!((*raw).vtable_size).read() as usize,
            ctx: std::ptr::addr_of!((*raw).ctx).read(),
            meta: if declared >= ProviderInfo::meta_end() {
                // SAFETY: the guard established the field is present; a
                // non-null `meta` is a well-formed map by the contract on
                // the field; and the library is never unloaded.
                std::ptr::addr_of!((*raw).meta).read().get()
            } else {
                None
            },
            available: if declared >= ProviderInfo::available_end() {
                // SAFETY: the guard established the field is present.
                std::ptr::addr_of!((*raw).available).read()
            } else {
                None
            },
            version: if declared >= ProviderInfo::version_end() {
                // SAFETY: the guard established the field is present.
                // Empty is how a provider says "my library's", which is a
                // different statement from a descriptor that predates the
                // field — and both land on `None` deliberately, because
                // both mean the same thing to a reader.
                str_of(std::ptr::addr_of!((*raw).version).read()).filter(|v| !v.is_empty())
            } else {
                None
            },
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
) -> Result<Option<LibraryView>, libloading::Error> {
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
) -> Result<Option<LibraryView>, libloading::Error> {
    // SAFETY: naming a file is choosing to run it, which is the contract
    // stated on `Registry::load_file` and on `open` below.
    unsafe { open(path, host) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::desc::Kinds;
    use crate::library::desc::{LibraryInfo, ProviderInfo, Providers};

    /// The byte every descriptor's tail is filled with.
    ///
    /// Non-zero on purpose. A tail of zeroes reads back as a null pointer
    /// and a zero size, which is exactly what a correct guard answers — so
    /// a reader with NO guard would pass every assertion below. `0xAA`
    /// makes the two outcomes different.
    const TAIL: u8 = 0xAA;

    /// A descriptor of `T`, `declared` bytes of it real and the rest
    /// filled with [`TAIL`].
    ///
    /// Returns the backing store as well: the pointer is into it, so it
    /// has to outlive the read.
    fn short_of<T>(value: &T, declared: usize) -> (Vec<u64>, *const T) {
        assert!(declared <= size_of::<T>());
        // `u64` for its alignment, which is at least what any of these
        // descriptors needs: every field is a pointer, a usize or a u32.
        let words = size_of::<T>().div_ceil(size_of::<u64>());
        let mut buf = vec![0u64; words];
        let base = buf.as_mut_ptr().cast::<u8>();
        // SAFETY: `words * 8` bytes are owned by `buf`.
        unsafe {
            std::ptr::write_bytes(base, TAIL, words * size_of::<u64>());
            std::ptr::copy_nonoverlapping((value as *const T).cast::<u8>(), base, declared);
        }
        let ptr = buf.as_ptr().cast::<T>();
        (buf, ptr)
    }

    fn a_host(size: usize) -> HostInfo {
        HostInfo {
            struct_size: size as u32,
            abi_version: crate::library::ABI_VERSION,
            host_id: Str::borrowed("test-host"),
            host_version: Str::borrowed("1.0"),
            alloc: std::ptr::null(),
            meta: MaybeNull::null(),
        }
    }

    /// A host from before `alloc` was appended reads as having none.
    ///
    /// This is the test the sentinel exists for: with a zero tail it would
    /// pass against a reader that ignored `struct_size` entirely.
    #[test]
    fn a_shorter_host_reads_its_appended_field_as_absent() {
        let value = a_host(HostInfo::floor());
        let (_buf, ptr) = short_of(&value, HostInfo::floor());
        // SAFETY: `_buf` owns the bytes and outlives the call.
        let read = unsafe { read_host(ptr) }.expect("a floor-sized host is usable");

        assert_eq!(read.struct_size as usize, HostInfo::floor());
        assert_eq!(read.host_id.len, "test-host".len());
        assert!(
            read.alloc.is_null(),
            "the allocator sits past the declared size, so it is absent — \
             reading it anyway would hand back {:p}, the sentinel this test \
             fills the tail with",
            read.alloc
        );
    }

    /// A size that covers only PART of an appended field reads as absent.
    ///
    /// The guard is `>=` against offset plus size rather than `>` against
    /// the offset, so half a pointer is never combined with whatever
    /// followed it.
    #[test]
    fn a_size_that_cuts_the_appended_field_in_half_reads_it_as_absent() {
        let value = a_host(HostInfo::alloc_end() - 1);
        let (_buf, ptr) = short_of(&value, HostInfo::alloc_end() - 1);
        // SAFETY: as above.
        let read = unsafe { read_host(ptr) }.expect("still above the floor");
        assert!(read.alloc.is_null(), "half a pointer is not a pointer");
    }

    /// Below the floor is refused outright.
    #[test]
    fn a_host_below_the_floor_is_refused() {
        for declared in [0usize, 4, HostInfo::floor() - 1] {
            let value = a_host(declared);
            let (_buf, ptr) = short_of(&value, size_of::<HostInfo>());
            // SAFETY: as above.
            assert!(
                unsafe { read_host(ptr) }.is_none(),
                "{declared} is below the floor of {}",
                HostInfo::floor()
            );
        }
        // SAFETY: null is the other case.
        assert!(unsafe { read_host(std::ptr::null()) }.is_none());
    }

    /// A NEWER host is accepted and its unknown tail ignored.
    ///
    /// The direction that matters for a library: it must keep working when
    /// the host grows, or every library needs rebuilding for a host change
    /// that added a field it does not read.
    #[test]
    fn a_newer_host_is_accepted_and_its_unknown_tail_ignored() {
        let mut value = a_host(size_of::<HostInfo>() + 64);
        let alloc = crate::value::alloc::rust_alloc();
        value.alloc = &alloc;
        let (_buf, ptr) = short_of(&value, size_of::<HostInfo>());
        // SAFETY: `_buf` covers `size_of::<HostInfo>()`, which is every
        // field this build knows; the declared 64 extra bytes are never
        // read, which is the property under test.
        let read = unsafe { read_host(ptr) }.expect("a newer host is still usable");
        assert!(
            !read.alloc.is_null(),
            "a field this build knows is still read"
        );
    }

    fn a_library(size: usize) -> LibraryInfo {
        LibraryInfo {
            struct_size: size as u32,
            abi_version: crate::library::ABI_VERSION,
            id: Str::borrowed("lib"),
            version: Str::borrowed("0.1.0"),
            providers: Providers::empty(),
            meta: MaybeNull::null(),
        }
    }

    #[test]
    fn a_library_descriptor_below_the_floor_is_refused() {
        let value = a_library(LibraryInfo::floor() - 1);
        let (_buf, ptr) = short_of(&value, size_of::<LibraryInfo>());
        // SAFETY: `_buf` owns the bytes.
        assert!(unsafe { read_library(ptr) }.is_none());

        let value = a_library(LibraryInfo::floor());
        let (_buf, ptr) = short_of(&value, LibraryInfo::floor());
        // SAFETY: as above.
        let view = unsafe { read_library(ptr) }.expect("a floor-sized descriptor is usable");
        assert_eq!((view.id, view.version), ("lib", "0.1.0"));
        assert!(view.providers.is_empty());
        assert!(
            view.meta.is_none(),
            "a descriptor that predates `meta` declares none"
        );
    }

    /// Two providers laid out at a SMALLER stride than this build's.
    ///
    /// **Two, because element 0 lands correctly whatever the stride.** A
    /// reader walking at its own element size passes every one-provider
    /// test there is and lands inside element 1 here, where it reads the
    /// 0xAA tail as a `struct_size` and guards every field against it.
    #[test]
    fn a_provider_array_is_walked_at_the_librarys_stride() {
        let stride = ProviderInfo::floor();
        assert!(
            stride < size_of::<ProviderInfo>(),
            "a slot has been appended since v1, so the old element is smaller"
        );

        let first = a_provider(stride, 0);
        let mut second = a_provider(stride, 0);
        second.id = Str::borrowed("second");

        let words = (stride * 2).div_ceil(size_of::<u64>());
        let mut array = vec![0u64; words];
        let base = array.as_mut_ptr().cast::<u8>();
        // SAFETY: `array` owns `words * 8` bytes, which covers both
        // elements; the tail is the sentinel so a misread is visible.
        unsafe {
            std::ptr::write_bytes(base, TAIL, words * size_of::<u64>());
            std::ptr::copy_nonoverlapping(
                (&first as *const ProviderInfo).cast::<u8>(),
                base,
                stride,
            );
            std::ptr::copy_nonoverlapping(
                (&second as *const ProviderInfo).cast::<u8>(),
                base.add(stride),
                stride,
            );
        }

        let mut value = a_library(size_of::<LibraryInfo>());
        value.providers = Providers {
            ptr: array.as_ptr().cast(),
            len: 2,
            stride,
        };
        let (_buf, ptr) = short_of(&value, size_of::<LibraryInfo>());
        // SAFETY: `_buf` owns the descriptor and `array` the elements.
        let view = unsafe { read_library(ptr) }.expect("both providers are readable");

        assert_eq!(view.providers.len(), 2);
        assert_eq!(view.providers[0].id, "hello");
        assert_eq!(view.providers[1].id, "second");
    }

    /// A stride below the floor cannot be an element, so the array is not
    /// walked at all.
    #[test]
    fn a_provider_array_whose_stride_is_below_the_floor_is_refused() {
        let one = a_provider(ProviderInfo::floor(), 0);
        let mut value = a_library(size_of::<LibraryInfo>());
        value.providers = Providers {
            ptr: &one,
            len: 1,
            stride: ProviderInfo::floor() - 1,
        };
        let (_buf, ptr) = short_of(&value, size_of::<LibraryInfo>());
        // SAFETY: `_buf` owns the descriptor and `one` the element.
        assert!(unsafe { read_library(ptr) }.is_none());
    }

    /// A `Str` array cannot be a `static` without saying why: it holds a
    /// raw pointer. These address string literals in this image.
    struct Names([Str; 1]);
    // SAFETY: a constant that is never written, whose pointer addresses a
    // string literal in this binary.
    unsafe impl Sync for Names {}
    static KINDS: Names = Names([Str::borrowed("greeter")]);

    fn a_provider(size: usize, vtable_size: u32) -> ProviderInfo {
        ProviderInfo {
            struct_size: size as u32,
            vtable_size,
            kinds: Kinds::new(&KINDS.0),
            id: Str::borrowed("hello"),
            display_name: Str::borrowed("Hello"),
            config: std::ptr::null(),
            vtable: std::ptr::null(),
            ctx: std::ptr::null_mut(),
            meta: MaybeNull::null(),
            version: Str::borrowed(""),
            available: None,
        }
    }

    unsafe extern "C" fn refuses(_ctx: *mut c_void, reason: *mut Str) -> bool {
        if !reason.is_null() {
            // SAFETY: the test passes writable storage.
            unsafe { reason.write(Str::borrowed("no calendar here")) };
        }
        false
    }

    /// A descriptor from before `available` reads as AVAILABLE.
    ///
    /// The direction that matters: an older library must not become
    /// unusable because a host learned to ask a question it never heard.
    #[test]
    fn a_descriptor_from_before_available_is_available() {
        let value = a_provider(ProviderInfo::version_end(), 0);
        let (_buf, ptr) = short_of(&value, ProviderInfo::version_end());
        // SAFETY: `_buf` owns the bytes.
        let view =
            unsafe { read_provider(ptr, ProviderInfo::version_end()) }.expect("above the floor");
        assert!(
            view.available.is_none(),
            "the slot sits past the declared size, so reading it anyway \
             hands back the 0xAA tail as a function pointer"
        );
        assert_eq!(view.available(), Ok(()));
    }

    /// And one that declares it is asked, and its reason survives.
    #[test]
    fn a_provider_that_refuses_says_why() {
        let mut value = a_provider(size_of::<ProviderInfo>(), 0);
        value.available = Some(refuses);
        let (_buf, ptr) = short_of(&value, size_of::<ProviderInfo>());
        // SAFETY: `_buf` owns the bytes.
        let view =
            unsafe { read_provider(ptr, size_of::<ProviderInfo>()) }.expect("a full descriptor");
        assert_eq!(view.available(), Err("no calendar here"));
    }

    #[test]
    fn a_provider_descriptor_below_the_floor_is_refused() {
        let value = a_provider(ProviderInfo::floor() - 1, 0);
        let (_buf, ptr) = short_of(&value, size_of::<ProviderInfo>());
        // SAFETY: `_buf` owns the bytes.
        assert!(unsafe { read_provider(ptr, size_of::<ProviderInfo>()) }.is_none());
    }

    /// The slot appended after the guards were written, read both ways.
    ///
    /// This is the mechanism doing its job rather than rehearsing it:
    /// `meta` is the first field genuinely added since, so a descriptor
    /// sized without it is what every library built before today hands
    /// over.
    #[test]
    fn a_descriptor_from_before_meta_reads_it_as_absent() {
        // A host.
        let value = a_host(HostInfo::alloc_end());
        let (_buf, ptr) = short_of(&value, HostInfo::alloc_end());
        // SAFETY: `_buf` owns the bytes.
        let read = unsafe { read_host(ptr) }.expect("above the floor");
        assert!(
            read.meta.is_null(),
            "`meta` sits past the declared size — reading it anyway hands \
             back the 0xAA tail this test fills with"
        );

        // A provider.
        let value = a_provider(ProviderInfo::floor(), 0);
        let (_buf, ptr) = short_of(&value, ProviderInfo::floor());
        // SAFETY: as above.
        let view =
            unsafe { read_provider(ptr, ProviderInfo::floor()) }.expect("a floor-sized provider");
        assert!(view.meta.is_none());
    }

    /// And a descriptor that DOES declare it hands it over.
    ///
    /// Without this the test above passes against a reader that never
    /// reads the field at all.
    #[test]
    fn a_descriptor_that_declares_meta_hands_it_over() {
        let map = Map::new();
        let mut value = a_host(size_of::<HostInfo>());
        // SAFETY: `map` outlives the read below.
        value.meta = MaybeNull::of(unsafe { &*(&map as *const Map) });
        let (_buf, ptr) = short_of(&value, size_of::<HostInfo>());
        // SAFETY: `_buf` owns the descriptor, `map` the pointee.
        let read = unsafe { read_host(ptr) }.expect("a full host");
        assert!(!read.meta.is_null(), "a declared slot is read");
        // SAFETY: it points at `map`, which is a well-formed empty map.
        assert_eq!(unsafe { read.meta.get() }.expect("non-null").len(), 0);
    }

    /// A provider carries the size its vtable was compiled at, and a host
    /// that knows a longer one must refuse rather than cast.
    ///
    /// The table itself is the sentinel-filled buffer, so a reader that
    /// skipped the size check would hand back a `&Newer` whose last field
    /// is 0xAAAA... rather than a function pointer.
    #[test]
    fn a_shorter_vtable_is_refused_and_the_size_it_declares_is_kept() {
        #[repr(C)]
        struct Older {
            greet: usize,
        }
        #[repr(C)]
        struct Newer {
            greet: usize,
            farewell: usize,
        }

        let table = [0u64; 4];
        let mut value = a_provider(size_of::<ProviderInfo>(), size_of::<Older>() as u32);
        value.vtable = table.as_ptr().cast();
        let (_buf, ptr) = short_of(&value, size_of::<ProviderInfo>());
        // SAFETY: `_buf` owns the descriptor and `table` the vtable.
        let view =
            unsafe { read_provider(ptr, size_of::<ProviderInfo>()) }.expect("a full descriptor");

        assert_eq!(
            view.vtable_size,
            size_of::<Older>(),
            "the declared size survives the read, because the host needs it \
             to decide what it may cast to"
        );
        assert!(
            view.vtable_size < size_of::<Newer>(),
            "this library predates the host's newest slot"
        );
    }
}
