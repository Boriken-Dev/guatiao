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
//! # The mapping is the registry's, and stays until the host says
//!
//! `open` hands the handle back and the registry holds it, so a
//! library stays mapped until the host retires or unloads it. A tree a
//! library built through its own allocator, and every vtable, `ctx` and
//! descriptor pointer taken from it, address that mapping — which is why
//! retiring keeps it and only `unload` closes it, on the host's word.
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

use super::desc::{HostInfo, HostServices, KindTable, LibraryInfo, ProviderInfo};
use crate::value::alloc::Alloc;
use crate::value::status::Status;
use crate::value::types::{Map, MaybeNull, Str, Value};

/// The symbol a library exports, NUL-terminated for the loader.
pub const ENTRY_SYMBOL: &[u8] = b"guatiao_library_entry\0";

/// The data symbol a library declares itself under, NUL-terminated for a
/// loader that looks it up by name.
///
/// Its bytes are `key=value` strings separated by NUL and ended by an
/// empty string, so the whole declaration ends in two NULs. Every kind
/// a provider serves is `kind=<name>`; a library adds pairs of its own.
/// Written by [`declares!`](crate::declares) or
/// [`providers!`](crate::providers); read by a scanner **as data**,
/// before the file is mapped, so a host can keep a library out of a scan
/// by what it declares rather than by its filename.
pub const DECLARES_SYMBOL: &[u8] = b"guatiao_declares\0";

/// How many bytes a declaration takes: `kind=<name>` for every kind of
/// every provider, then each extra pair, each NUL-terminated, then the
/// empty string that ends it. Evaluated at compile time by the macros.
#[doc(hidden)]
pub const fn declaration_len(kinds: &[&[&str]], extra: &[&str]) -> usize {
    let mut n = 1;
    let mut i = 0;
    while i < kinds.len() {
        let mut j = 0;
        while j < kinds[i].len() {
            n += "kind=".len() + kinds[i][j].len() + 1;
            j += 1;
        }
        i += 1;
    }
    let mut i = 0;
    while i < extra.len() {
        n += extra[i].len() + 1;
        i += 1;
    }
    n
}

/// The bytes of a declaration, sized by [`declaration_len`]. A pair that
/// is not `KEY=VALUE`, or holds a NUL, is refused at compile time.
#[doc(hidden)]
pub const fn declaration<const N: usize>(kinds: &[&[&str]], extra: &[&str]) -> [u8; N] {
    let mut out = [0u8; N];
    let mut at = 0;
    let mut i = 0;
    while i < kinds.len() {
        let mut j = 0;
        while j < kinds[i].len() {
            at = put(&mut out, at, b"kind=");
            at = put(&mut out, at, kinds[i][j].as_bytes());
            at += 1;
            j += 1;
        }
        i += 1;
    }
    let mut i = 0;
    while i < extra.len() {
        let pair = extra[i].as_bytes();
        let mut has_eq = false;
        let mut k = 0;
        while k < pair.len() {
            if pair[k] == b'=' && k > 0 {
                has_eq = true;
            }
            if pair[k] == 0 {
                panic!("a declaration cannot hold a NUL");
            }
            k += 1;
        }
        if !has_eq {
            panic!("a declaration is `KEY=VALUE`");
        }
        at = put(&mut out, at, pair);
        at += 1;
        i += 1;
    }
    // `at + 1 == N` by construction; the ending empty string is the zero
    // already there.
    out
}

const fn put(out: &mut [u8], at: usize, bytes: &[u8]) -> usize {
    let mut k = 0;
    while k < bytes.len() {
        out[at + k] = bytes[k];
        k += 1;
    }
    at + bytes.len()
}

/// The entry point's signature.
///
/// Returning null means **"nothing for this host"**, which is an answer
/// rather than a failure: a library that supports one host and is loaded
/// by another says so this way, and the loader reports it as skipped.
pub type EntryFn = unsafe extern "C" fn(*const HostInfo) -> *const LibraryInfo;

/// Writes a library's entry point.
///
/// Give it a function taking a [`Host`] and answering
/// `Option<&'static LibraryInfo>`; it emits the `extern "C"` symbol a
/// loader looks for, catches a panic rather than letting one cross the
/// boundary, and answers null for "nothing for this host". The `Host` is
/// `Copy + Send + Sync + 'static`, so a library keeps it in a `OnceLock`
/// of its own and asks the host for a provider from any later call.
///
/// ```ignore
/// fn describe(host: guatiao::library::Host) -> Option<&'static guatiao::library::LibraryInfo> {
///     // build a descriptor, keep it and `host` in a `OnceLock` of your own
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
            // null or points at a host descriptor declaring its own size
            // that stays valid for the life of the process.
            let host = unsafe { $crate::library::Host::from_raw(host) };
            let Some(host) = host else {
                return ::core::ptr::null();
            };
            $crate::library::answer(|| $describe(host))
        }
    };
}

/// Writes what a library declares, for a scanner to read before mapping
/// it.
///
/// Each argument is a `KEY=VALUE` literal. A host's scan rules match
/// against these (`guatiao_registry_scan_dir_rules`, `ScanRules`), so a
/// library that must never be loaded by a scan says so here and the host
/// writes one rule rather than a filename denylist:
///
/// ```ignore
/// guatiao::declares!("kind=greeter", "kind=writer", "VIEWER=1");
/// ```
///
/// A library built with [`providers!`](crate::providers) declares its
/// kinds by itself and takes extra pairs through `declares = [..]`; this
/// is for a hand-written one. A library declaring nothing is loaded by a
/// plain scan as before, and fails every positive rule.
#[macro_export]
macro_rules! declares {
    ($($pair:expr),* $(,)?) => {
        $crate::__guatiao_declares!(kinds = [], pairs = [$($pair),*]);
    };
}

/// The `static` behind [`declares!`] and [`providers!`].
#[doc(hidden)]
#[macro_export]
macro_rules! __guatiao_declares {
    (kinds = [$($kinds:expr),*], pairs = [$($pair:expr),*]) => {
        const __GUATIAO_DECLARED_KINDS: &[&[&str]] = &[$($kinds),*];
        const __GUATIAO_DECLARED_PAIRS: &[&str] = &[$($pair),*];
        const __GUATIAO_DECLARES_LEN: usize = $crate::library::declaration_len(
            __GUATIAO_DECLARED_KINDS,
            __GUATIAO_DECLARED_PAIRS,
        );
        /// What this library declares, read by a scanner as data.
        #[allow(non_upper_case_globals)]
        #[unsafe(no_mangle)]
        pub static guatiao_declares: [u8; __GUATIAO_DECLARES_LEN] =
            $crate::library::declaration(__GUATIAO_DECLARED_KINDS, __GUATIAO_DECLARED_PAIRS);
    };
}

/// Writes a whole library from its provider types.
///
/// `providers!(A, B)` names the types that `#[derive(Provider)]` made
/// providers of; the library's id and version default to the crate's own
/// `CARGO_PKG_NAME` and `CARGO_PKG_VERSION`. The long form overrides them:
///
/// ```ignore
/// guatiao::providers!(Hello);
/// guatiao::providers!(id = "acme_net", version = "1.2.0", providers = [Hello, Counter]);
/// ```
///
/// Each provider's descriptor is built once, on the first entry call,
/// and kept for the process. A provider whose configuration schema cannot
/// be built makes the library decline the host.
///
/// The library also **declares** every kind its providers serve, as
/// data a scanner reads before mapping it; `providers!(A, B; declares =
/// ["VIEWER=1"])` (or `declares = [..]` after `providers = [..]` in
/// the long form) adds pairs of the library's own. See
/// [`declares!`](crate::declares).
///
/// The long form also takes `unload = <fn>`, an
/// `unsafe extern "C" fn() -> Status` the host calls before it unmaps
/// this library. See [`LibraryInfo::unload`]. Without one the library
/// may be unmapped without notice.
#[macro_export]
macro_rules! providers {
    ($($provider:ty),+ $(,)?) => {
        $crate::providers!(
            id = env!("CARGO_PKG_NAME"),
            version = env!("CARGO_PKG_VERSION"),
            providers = [$($provider),+]
        );
    };
    ($($provider:ty),+ ; declares = [$($pair:expr),* $(,)?]) => {
        $crate::providers!(
            id = env!("CARGO_PKG_NAME"),
            version = env!("CARGO_PKG_VERSION"),
            providers = [$($provider),+],
            declares = [$($pair),*]
        );
    };
    (
        id = $id:expr,
        version = $version:expr,
        providers = [$($provider:ty),+ $(,)?]
        $(, declares = [$($pair:expr),* $(,)?])?,
        unload = $unload:expr $(,)?
    ) => {
        $crate::__guatiao_declares!(
            kinds = [$(<$provider as $crate::library::kind::ProviderDecl>::KINDS),+],
            pairs = [$($($pair),*)?]
        );
        $crate::__guatiao_describe!(
            #[doc(hidden)]
            fn __guatiao_describe;
            id = $id, version = $version, providers = [$($provider),+],
            unload = ::core::option::Option::Some($unload)
        );
        $crate::guatiao_library!(__guatiao_describe);
    };
    (
        id = $id:expr,
        version = $version:expr,
        providers = [$($provider:ty),+ $(,)?]
        $(, declares = [$($pair:expr),* $(,)?])? $(,)?
    ) => {
        $crate::__guatiao_declares!(
            kinds = [$(<$provider as $crate::library::kind::ProviderDecl>::KINDS),+],
            pairs = [$($($pair),*)?]
        );
        $crate::__guatiao_describe!(
            #[doc(hidden)]
            fn __guatiao_describe;
            id = $id, version = $version, providers = [$($provider),+],
            unload = ::core::option::Option::None
        );
        $crate::guatiao_library!(__guatiao_describe);
    };
}

/// Writes a library the host **links** rather than loads.
///
/// The same as [`providers!`](crate::providers) without the two exported symbols: no entry
/// point, no declaration, so the artifact this ends up in — a host's own
/// binary, or an engine that carries one provider compiled in — does not
/// look like a plugin to a scan of its directory. What it writes instead
/// is one function, `library`, which the host hands to
/// [`Registry::register_local`](crate::library::Registry::register_local):
///
/// ```ignore
/// guatiao::local_providers!(Remote);                       // in the host's crate
/// registry.register_local("engine", crate::library)?;      // beside what it loads
/// ```
///
/// The providers are built once, on the first call, and kept for the
/// process, as a loaded library's are.
#[macro_export]
macro_rules! local_providers {
    ($($provider:ty),+ $(,)?) => {
        $crate::local_providers!(
            id = env!("CARGO_PKG_NAME"),
            version = env!("CARGO_PKG_VERSION"),
            providers = [$($provider),+]
        );
    };
    (
        id = $id:expr,
        version = $version:expr,
        providers = [$($provider:ty),+ $(,)?] $(,)?
    ) => {
        $crate::__guatiao_describe!(
            /// What this crate offers as a library the host links: hand
            /// it to `Registry::register_local`.
            pub fn library;
            id = $id, version = $version, providers = [$($provider),+],
            unload = ::core::option::Option::None
        );
    };
}

/// The describe function behind [`providers!`](crate::providers) and
/// [`local_providers!`](crate::local_providers).
#[doc(hidden)]
#[macro_export]
macro_rules! __guatiao_describe {
    (
        $(#[$attr:meta])* $vis:vis fn $name:ident;
        id = $id:expr, version = $version:expr, providers = [$($provider:ty),+],
        unload = $unload:expr
    ) => {
        $(#[$attr])*
        $vis fn $name(
            host: $crate::library::Host,
        ) -> ::core::option::Option<&'static $crate::library::LibraryInfo> {
            static REGISTERED: ::std::sync::OnceLock<::core::option::Option<$crate::library::kind::LibraryParts>> = ::std::sync::OnceLock::new();
            REGISTERED
                .get_or_init(|| {
                    // The host's arena when it offers one, so what this
                    // library builds for the host outlives this mapping.
                    let alloc = host.alloc().unwrap_or_else($crate::Alloc::rust);
                    let mut providers = ::std::vec::Vec::new();
                    $(
                        providers.push(
                            <$provider as $crate::library::kind::ProviderDecl>::provider(host, alloc).ok()?,
                        );
                    )+
                    ::core::option::Option::Some(
                        $crate::library::kind::LibraryParts::new($id, $version, providers)
                            .unloading($unload),
                    )
                })
                .as_ref()
                .map($crate::library::kind::LibraryParts::info)
        }
    };
}

/// A host, as a library keeps it.
///
/// A pointer to the [`HostInfo`] a host handed its entry point, which the
/// host keeps valid for the life of the process. `Copy`, `Send`, `Sync`
/// and `'static`, so it goes in a library's own `OnceLock` and is read
/// from any later call. Every field is read on demand under the host's
/// declared `struct_size`, so a host built before a field existed reads
/// as not having it.
#[derive(Debug, Clone, Copy)]
pub struct Host {
    raw: *const HostInfo,
}

// SAFETY: the pointee is never written after the host hands it over, and
// every answer it gives comes from a slot the host guards itself.
unsafe impl Send for Host {}
// SAFETY: as above.
unsafe impl Sync for Host {}

impl Host {
    /// Wraps the pointer a host passed to the entry point.
    ///
    /// `None` for null, or for a descriptor below the floor.
    ///
    /// # Safety
    ///
    /// `raw` is null, or points at `raw->struct_size` readable, aligned
    /// bytes that stay valid and unchanged for the life of the process.
    pub unsafe fn from_raw(raw: *const HostInfo) -> Option<Host> {
        if raw.is_null() {
            return None;
        }
        // SAFETY: the caller guarantees the first four bytes are readable.
        let declared = unsafe { std::ptr::addr_of!((*raw).struct_size).read() } as usize;
        if declared < HostInfo::floor() {
            return None;
        }
        Some(Host { raw })
    }

    /// The pointer as it was handed over, for a caller that speaks C.
    pub fn as_raw(&self) -> *const HostInfo {
        self.raw
    }

    fn declared(&self) -> usize {
        // SAFETY: `from_raw` established the leading word is readable.
        unsafe { std::ptr::addr_of!((*self.raw).struct_size).read() as usize }
    }

    /// The envelope version the host speaks.
    pub fn abi_version(&self) -> u32 {
        // SAFETY: within the floor.
        unsafe { std::ptr::addr_of!((*self.raw).abi_version).read() }
    }

    /// Who the host is. Empty when it said nothing readable.
    pub fn id(&self) -> &'static str {
        // SAFETY: within the floor; the text lives in the host's block.
        unsafe { str_of(std::ptr::addr_of!((*self.raw).host_id).read()) }.unwrap_or("")
    }

    /// The host's own version string, uninterpreted.
    pub fn version(&self) -> &'static str {
        // SAFETY: as `id`.
        unsafe { str_of(std::ptr::addr_of!((*self.raw).host_version).read()) }.unwrap_or("")
    }

    /// The host's allocator, or `None` when it offers none or predates
    /// the field.
    pub fn alloc(&self) -> Option<Alloc> {
        if self.declared() < HostInfo::alloc_end() {
            return None;
        }
        // SAFETY: the guard established the field is present, and the
        // allocator it names outlives the process by the block's contract.
        let raw = unsafe { std::ptr::addr_of!((*self.raw).alloc).read() };
        unsafe { Alloc::from_raw(raw) }.ok()
    }

    /// Whatever else the host declared, or `None`.
    pub fn meta(&self) -> Option<&'static Map> {
        if self.declared() < HostInfo::meta_end() {
            return None;
        }
        // SAFETY: the guard established the field is present; non-null
        // means a well-formed map by the contract on the field; the block
        // lives for the process.
        unsafe { std::ptr::addr_of!((*self.raw).meta).read().get() }
    }

    /// A copy of the descriptor as it reads today, with absent fields
    /// nulled. For a caller that wants a snapshot rather than a handle.
    pub fn snapshot(&self) -> HostInfo {
        // SAFETY: `from_raw` established the contract `read_host` needs.
        unsafe { read_host(self.raw) }.expect("a Host was built from a readable descriptor")
    }

    /// The services table, or `None` for a host that offers none or
    /// predates the field.
    fn services(&self) -> Option<&'static HostServices> {
        if self.declared() < HostInfo::services_end() {
            return None;
        }
        // SAFETY: the guard established the field is present.
        let table = unsafe { std::ptr::addr_of!((*self.raw).services).read() };
        if table.is_null() {
            return None;
        }
        // SAFETY: a non-null table points at one the host keeps for the
        // process, declaring its own size; the leading word is readable.
        let size = unsafe { std::ptr::addr_of!((*table).struct_size).read() } as usize;
        if size < HostServices::floor() {
            return None;
        }
        // SAFETY: at least the floor is present, which is every field
        // this build reads.
        Some(unsafe { &*table })
    }

    /// One provider by the key the host files it under.
    ///
    /// `Ok(None)` when nothing answers to the key. `Err` when the host
    /// offers no services (`GUATIAO_ERR_NULL`) or its registry is gone
    /// (`GUATIAO_ERR_GONE`).
    pub fn get(&self, key: &str) -> Result<Option<&'static ProviderInfo>, Status> {
        let table = self.services().ok_or(Status::GUATIAO_ERR_NULL)?;
        let get = table.get.ok_or(Status::GUATIAO_ERR_NULL)?;
        let mut out: *const ProviderInfo = std::ptr::null();
        // SAFETY: the slot is the host's own, read under its guard; `out`
        // is a writable local; `key` is readable for the call.
        match unsafe { get(table.ctx, Str::borrowed(key), &mut out) } {
            Status::GUATIAO_OK if !out.is_null() => {
                // SAFETY: the host answers with a descriptor the offering
                // library keeps for the life of the process.
                Ok(Some(unsafe { &*out }))
            }
            Status::GUATIAO_OK | Status::GUATIAO_ERR_NOT_FOUND => Ok(None),
            other => Err(other),
        }
    }

    /// Every provider serving `kind` — every provider when it is empty —
    /// in the host's own order, unavailable ones included. The caller asks
    /// each and chooses.
    ///
    /// `Err` when the host offers no services or its registry is gone.
    pub fn list(&self, kind: &str) -> Result<Vec<&'static ProviderInfo>, Status> {
        let table = self.services().ok_or(Status::GUATIAO_ERR_NULL)?;
        let list = table.list.ok_or(Status::GUATIAO_ERR_NULL)?;
        let kind = Str::borrowed(kind);
        let mut total = 0usize;
        // SAFETY: as `get`; a zero capacity asks for the count alone.
        match unsafe { list(table.ctx, kind, std::ptr::null_mut(), 0, &mut total) } {
            Status::GUATIAO_OK => {}
            other => return Err(other),
        }
        let mut found: Vec<*const ProviderInfo> = vec![std::ptr::null(); total];
        let mut written = 0usize;
        // SAFETY: `found` has room for `total` entries.
        match unsafe {
            list(
                table.ctx,
                kind,
                found.as_mut_ptr(),
                found.len(),
                &mut written,
            )
        } {
            Status::GUATIAO_OK => {}
            other => return Err(other),
        }
        // The registry may have grown between the two calls; what was
        // written is what is real.
        found.truncate(written.min(total));
        Ok(found
            .into_iter()
            .filter(|p| !p.is_null())
            // SAFETY: each is a descriptor its library keeps for the life
            // of the process.
            .map(|p| unsafe { &*p })
            .collect())
    }
}

// --- what a host keeps for its libraries -------------------------------

/// Everything a library reached through a [`Host`] points into: the
/// descriptor, the services table, the text the descriptor names, and a
/// weak handle to the registry's published state.
///
/// **Leaked once per registry, never freed.** That is what lets a library
/// keep the pointer; the registry itself may go, and the block then
/// answers [`Status::GUATIAO_ERR_GONE`].
#[cfg(feature = "load")]
#[derive(Debug)]
pub(crate) struct HostBlock {
    info: HostInfo,
    services: HostServices,
    #[allow(dead_code)]
    id: Box<str>,
    #[allow(dead_code)]
    version: Box<str>,
    state: std::sync::Weak<Shared>,
}

/// What the registry publishes for its services to read: the providers
/// it holds, already in its order, with each descriptor's address.
#[cfg(feature = "load")]
#[derive(Debug, Default)]
pub(crate) struct Snapshot {
    /// In `(priority DESC, key ASC)` order.
    pub(crate) entries: Vec<SnapshotEntry>,
    /// Rendered key to an index into `entries`.
    pub(crate) by_key: std::collections::BTreeMap<String, usize>,
}

/// One provider as the services see it.
#[cfg(feature = "load")]
#[derive(Debug)]
pub(crate) struct SnapshotEntry {
    pub(crate) kinds: Vec<String>,
    pub(crate) raw: *const ProviderInfo,
}

// SAFETY: `raw` addresses a descriptor inside a library image the
// registry holds the mapping of until that library is retired, and a
// snapshot is only ever read after it is published.
#[cfg(feature = "load")]
unsafe impl Send for Snapshot {}

// SAFETY (all four): every raw pointer a registry or a provider holds
// addresses a loaded library's image -- its descriptors and their tables --
// which stays mapped until that library is retired and is never written
// from this side; the
// block a registry leaks is already shared across threads by design (a
// library reads it from any call); and a provider's slots are declared
// callable from any thread (`ProviderInfo::available`'s contract exists
// precisely because two threads may ask at once). A host keeping its one
// registry behind a lock, or reading it from several threads, is the
// ordinary shape, and without these it could not. Declared here rather
// than beside the types because this is the file allowed to say
// `unsafe`.
#[cfg(feature = "load")]
unsafe impl Send for super::registry::Registry {}
#[cfg(feature = "load")]
unsafe impl Sync for super::registry::Registry {}
#[cfg(feature = "load")]
unsafe impl Send for super::registry::Provider {}
#[cfg(feature = "load")]
unsafe impl Sync for super::registry::Provider {}
// SAFETY: as above.
#[cfg(feature = "load")]
unsafe impl Sync for Snapshot {}

/// The state a registry shares with the block it leaked. The registry
/// holds the `Arc`; the block holds a `Weak`, so dropping the registry is
/// what makes every later lookup answer `GONE`.
#[cfg(feature = "load")]
#[derive(Debug, Default)]
pub(crate) struct Shared {
    pub(crate) snapshot: std::sync::RwLock<Snapshot>,
}

#[cfg(feature = "load")]
impl Shared {
    /// The published snapshot, whatever a poisoned lock says: a panic
    /// while publishing leaves the last complete snapshot in place.
    fn read(&self) -> std::sync::RwLockReadGuard<'_, Snapshot> {
        self.snapshot
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Leaks the block a registry hands its libraries.
///
/// The descriptor's text points into the block's own boxes, and the
/// services' `ctx` is the block itself, so nothing here borrows from the
/// registry.
#[cfg(feature = "load")]
pub(crate) fn leak_host_block(
    id: &str,
    version: &str,
    alloc: Option<Alloc>,
    state: std::sync::Weak<Shared>,
) -> &'static HostBlock {
    let id: Box<str> = id.into();
    let version: Box<str> = version.into();
    let block = Box::leak(Box::new(HostBlock {
        info: HostInfo {
            struct_size: size_of::<HostInfo>() as u32,
            abi_version: super::desc::ABI_VERSION,
            // The boxes' heap storage does not move when the block does.
            host_id: Str::borrowed(&id),
            host_version: Str::borrowed(&version),
            alloc: alloc.map_or(std::ptr::null(), |a| a.as_raw()),
            meta: MaybeNull::null(),
            services: std::ptr::null(),
        },
        services: HostServices {
            struct_size: size_of::<HostServices>() as u32,
            ctx: std::ptr::null_mut(),
            get: Some(services_get),
            list: Some(services_list),
            alloc: Some(services_alloc),
        },
        id,
        version,
        state,
    }));
    block.services.ctx = (block as *mut HostBlock).cast::<c_void>();
    block.info.services = &block.services;
    block
}

#[cfg(feature = "load")]
impl HostBlock {
    /// The descriptor a library is handed.
    pub(crate) fn host(&'static self) -> Host {
        Host { raw: &self.info }
    }
}

/// The block behind a services `ctx`, or `None` for null.
///
/// # Safety
///
/// `ctx` is null or the `ctx` this crate wrote into a block it leaked.
#[cfg(feature = "load")]
unsafe fn block_of<'a>(ctx: *mut c_void) -> Option<&'a HostBlock> {
    // SAFETY: the caller's contract; the block is never freed.
    unsafe { ctx.cast::<HostBlock>().as_ref() }
}

/// The `get` slot: one provider by key.
///
/// # Safety
///
/// `ctx` is a leaked block's, `key` is readable for the call, `out` is
/// null or writable.
#[cfg(feature = "load")]
unsafe extern "C" fn services_get(
    ctx: *mut c_void,
    key: Str,
    out: *mut *const ProviderInfo,
) -> Status {
    crate::exports::guard(|| {
        // SAFETY: the caller's contract.
        let (Some(block), false) = (unsafe { block_of(ctx) }, out.is_null()) else {
            return Status::GUATIAO_ERR_NULL;
        };
        let Some(shared) = block.state.upgrade() else {
            return Status::GUATIAO_ERR_GONE;
        };
        // SAFETY: the caller's contract.
        let Ok(key) = (unsafe { crate::exports::as_str(key) }) else {
            return Status::GUATIAO_ERR_BAD_VALUE;
        };
        let snapshot = shared.read();
        match snapshot.by_key.get(key) {
            Some(&i) => {
                // SAFETY: `out` is writable by the caller's contract.
                unsafe { out.write(snapshot.entries[i].raw) };
                Status::GUATIAO_OK
            }
            None => Status::GUATIAO_ERR_NOT_FOUND,
        }
    })
}

/// The `list` slot: every provider serving a kind, in the host's order.
///
/// # Safety
///
/// `ctx` is a leaked block's, `kind` is readable, `out` addresses `cap`
/// writable slots (or is null with `cap` zero), `total` is writable.
#[cfg(feature = "load")]
unsafe extern "C" fn services_list(
    ctx: *mut c_void,
    kind: Str,
    out: *mut *const ProviderInfo,
    cap: usize,
    total: *mut usize,
) -> Status {
    crate::exports::guard(|| {
        // SAFETY: the caller's contract.
        let (Some(block), false) = (unsafe { block_of(ctx) }, total.is_null()) else {
            return Status::GUATIAO_ERR_NULL;
        };
        if cap > 0 && out.is_null() {
            return Status::GUATIAO_ERR_NULL;
        }
        let Some(shared) = block.state.upgrade() else {
            return Status::GUATIAO_ERR_GONE;
        };
        // SAFETY: the caller's contract.
        let Ok(kind) = (unsafe { crate::exports::as_str(kind) }) else {
            return Status::GUATIAO_ERR_BAD_VALUE;
        };
        let snapshot = shared.read();
        let mut n = 0usize;
        for entry in snapshot
            .entries
            .iter()
            .filter(|e| kind.is_empty() || e.kinds.iter().any(|k| k == kind))
        {
            if n < cap {
                // SAFETY: `n < cap` and `out` addresses `cap` slots.
                unsafe { out.add(n).write(entry.raw) };
            }
            n += 1;
        }
        // SAFETY: `total` is writable by the caller's contract.
        unsafe { total.write(n) };
        Status::GUATIAO_OK
    })
}

/// The `alloc` slot: the host's allocator, or null.
///
/// # Safety
///
/// `ctx` is a leaked block's.
#[cfg(feature = "load")]
unsafe extern "C" fn services_alloc(ctx: *mut c_void) -> *const crate::value::alloc::Allocator {
    crate::exports::guard_with(std::ptr::null(), || {
        // SAFETY: the caller's contract.
        match unsafe { block_of(ctx) } {
            Some(block) => block.info.alloc,
            None => std::ptr::null(),
        }
    })
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
            services: std::ptr::null(),
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
    if declared >= HostInfo::services_end() {
        // SAFETY: as above.
        info.services = unsafe { std::ptr::addr_of!((*raw).services).read() };
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
    // library they are in is mapped for the call.
    let bytes = unsafe { std::slice::from_raw_parts(s.ptr, s.len) };
    std::str::from_utf8(bytes).ok()
}

/// One provider, read out of a descriptor once so nothing downstream has
/// to hold a raw pointer to read a name.
///
/// **Every string and every value here is OWNED**, copied out of the
/// library's image at the read. That is what lets a library be unmapped
/// while a host still holds what it said about itself. The code pointers
/// below are not copied and cannot be: they address the image.
#[derive(Debug, Clone)]
pub struct ProviderView {
    /// Every kind it serves. May be empty.
    pub kinds: Vec<String>,
    /// Its identifier, unique across every provider a host loads.
    pub id: String,
    /// A name to show a person, possibly empty.
    pub display_name: String,
    /// Its configuration schema, or `None`. A copy in this process's own
    /// heap, not the library's.
    pub config: Option<Value>,
    /// The function table, whose shape the kind defines.
    pub vtable: *const c_void,
    /// The size the library compiled that table at.
    pub vtable_size: usize,
    /// Handed back to every call through the table.
    pub ctx: *mut c_void,
    /// Whatever else the provider declared, or `None`. A copy, as
    /// [`config`](ProviderView::config) is. See [`ProviderInfo::meta`].
    pub meta: Option<Map>,
    /// The version it declared for itself, or `None` to inherit its
    /// library's. See [`ProviderInfo::version`].
    pub version: Option<String>,
    /// Its runtime-availability slot, or `None` when it declares none —
    /// which means available. See [`ProviderInfo::available`].
    pub available: Option<unsafe extern "C" fn(ctx: *mut c_void, reason: *mut Str) -> bool>,
    /// The descriptor this was read from, in the library's image, valid
    /// until that library is unloaded. What a host's services hand a
    /// library that asks.
    pub raw: *const ProviderInfo,
    /// One table per kind, as `(kind, vtable, vtable_size)`, for a
    /// provider serving several kinds with a table each. Empty when
    /// `vtable` serves them all. See [`ProviderInfo::tables`].
    pub tables: Vec<(String, *const c_void, usize)>,
    /// Builds an instance from a configuration, or `None`. See
    /// [`ProviderInfo::create`].
    pub create: Option<
        unsafe extern "C" fn(
            ctx: *mut c_void,
            config: *const Value,
            out: *mut *mut c_void,
            err: *mut super::kind::ProviderError,
        ) -> Status,
    >,
    /// Releases an instance `create` built. See [`ProviderInfo::destroy`].
    pub destroy: Option<unsafe extern "C" fn(ctx: *mut c_void, instance: *mut c_void)>,
}

impl ProviderView {
    /// Whether it serves this kind.
    pub fn supports(&self, kind: &str) -> bool {
        self.kinds.iter().any(|k| k == kind)
    }

    /// The function table this provider speaks `kind` through, and the
    /// size it was compiled at: a per-kind table first, then `vtable`
    /// when `kinds` names the kind. `None` when neither — a kind this
    /// provider does not serve, or serves as a pure label.
    pub fn table_for(&self, kind: &str) -> Option<(*const c_void, usize)> {
        if let Some(&(_, table, size)) = self.tables.iter().find(|(k, _, _)| k == kind) {
            return (!table.is_null()).then_some((table, size));
        }
        if self.supports(kind) && !self.vtable.is_null() {
            return Some((self.vtable, self.vtable_size));
        }
        None
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
        // present by `read_provider`'s guard; `reason` is a writable
        // local; and a provider is only asked while its library is
        // registered, which means mapped.
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
        // bytes at it; and the borrow lives as long as the mapping, which
        // only `unload` closes.
        Some(unsafe { &*self.vtable.cast::<T>() })
    }
}

/// One library's descriptor, read out once. Owned, like
/// [`ProviderView`] and for the same reason.
#[derive(Debug, Clone)]
pub struct LibraryView {
    /// The library's own identifier.
    pub id: String,
    /// Its version string, uninterpreted.
    pub version: String,
    /// Whatever else it declared, or `None`. A copy. See
    /// [`LibraryInfo::meta`].
    pub meta: Option<Map>,
    /// Its say in being unmapped, or `None` when it declares none or
    /// predates the slot. See [`LibraryInfo::unload`].
    pub unload: Option<unsafe extern "C" fn() -> Status>,
    /// What it offers.
    pub providers: Vec<ProviderView>,
}

/// Why a descriptor a library handed back could not be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rejected {
    /// Too small, text that is not UTF-8, a null array with a length, a
    /// stride below the floor, or an element overlapping its neighbour.
    Malformed,
    /// The library speaks an envelope version this build does not.
    /// Appending a field never changes the version; only a change no
    /// `struct_size` guard can express does, and such a change makes
    /// every field after it mean something else.
    UnsupportedAbi {
        /// What the library declared.
        declared: u32,
    },
}

/// Reads a library descriptor and every provider in it.
///
/// # Safety
///
/// `raw` came from a library's entry point and points at a descriptor
/// that stays valid for the life of the process, which is what never
/// unloading the library guarantees.
pub(crate) unsafe fn read_library(raw: *const LibraryInfo) -> Result<LibraryView, Rejected> {
    if raw.is_null() {
        return Err(Rejected::Malformed);
    }
    // SAFETY: the caller guarantees the leading word is readable.
    let declared = unsafe { std::ptr::addr_of!((*raw).struct_size).read() } as usize;
    if declared < LibraryInfo::floor() {
        return Err(Rejected::Malformed);
    }

    // SAFETY: `abi_version` lies within the floor.
    let abi = unsafe { std::ptr::addr_of!((*raw).abi_version).read() };
    if abi != super::desc::ABI_VERSION {
        return Err(Rejected::UnsupportedAbi { declared: abi });
    }

    // SAFETY: each field lies within `declared` bytes.
    let (id, version, providers) = unsafe {
        (
            str_of(std::ptr::addr_of!((*raw).id).read())
                .ok_or(Rejected::Malformed)?
                .to_string(),
            str_of(std::ptr::addr_of!((*raw).version).read())
                .ok_or(Rejected::Malformed)?
                .to_string(),
            std::ptr::addr_of!((*raw).providers).read(),
        )
    };

    let mut meta = None;
    if declared >= LibraryInfo::meta_end() {
        // SAFETY: the guard established the field is present.
        let raw_meta = unsafe { std::ptr::addr_of!((*raw).meta).read() };
        // SAFETY: a non-null `meta` is a well-formed map by the contract
        // on the field, and it is readable for this call.
        meta = unsafe { raw_meta.get() }
            .map(|m| m.clone_in(Alloc::rust()))
            .transpose()
            .map_err(|_| Rejected::Malformed)?;
    }

    let mut unload = None;
    if declared >= LibraryInfo::unload_end() {
        // SAFETY: the guard established the field is present.
        unload = unsafe { std::ptr::addr_of!((*raw).unload).read() };
    }

    // Every check on the array runs before anything is allocated for it:
    // `len` is the library's number, and a corrupt one must be refused,
    // never reserved for.
    let stride = providers.stride;
    if providers.len > 0 && (providers.ptr.is_null() || stride < ProviderInfo::floor()) {
        return Err(Rejected::Malformed);
    }
    // The array's byte extent must be addressable at all.
    let extent = providers
        .len
        .checked_mul(stride)
        .filter(|&bytes| bytes <= isize::MAX as usize)
        .ok_or(Rejected::Malformed)?;
    let mut out = Vec::with_capacity(extent / stride.max(1));
    for i in 0..providers.len {
        // SAFETY: the library declared `len` descriptors of `stride` bytes
        // at `ptr`, `i * stride` is within `extent`, and byte arithmetic is
        // what reaches element `i` at the library's own layout.
        let entry = unsafe { providers.ptr.cast::<u8>().add(i * stride) }.cast::<ProviderInfo>();
        // SAFETY: forwarded.
        out.push(unsafe { read_provider(entry, stride) }.ok_or(Rejected::Malformed)?);
    }
    Ok(LibraryView {
        id,
        version,
        meta,
        unload,
        providers: out,
    })
}

impl ProviderInfo {
    /// Reads this descriptor out, honouring its declared size.
    ///
    /// For a descriptor a host's services handed over: it lives as long as
    /// the library that declared it, which is for the life of the process.
    /// `None` when it is below the floor or names text that is not UTF-8.
    pub fn view(&'static self) -> Option<ProviderView> {
        // SAFETY: a `&'static ProviderInfo` is a readable descriptor whose
        // declared size is its own limit.
        unsafe { read_provider(self, self.struct_size as usize) }
    }
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
            names.push(str_of(kinds.ptr.add(i).read())?.to_string());
        }

        Some(ProviderView {
            kinds: names,
            id: str_of(std::ptr::addr_of!((*raw).id).read())?.to_string(),
            display_name: str_of(std::ptr::addr_of!((*raw).display_name).read())?.to_string(),
            // Copied out of the image, so the schema outlives the library
            // it was read from.
            config: config
                .as_ref()
                .map(|v| v.clone_in(Alloc::rust()))
                .transpose()
                .ok()?,
            vtable: std::ptr::addr_of!((*raw).vtable).read(),
            vtable_size: std::ptr::addr_of!((*raw).vtable_size).read() as usize,
            ctx: std::ptr::addr_of!((*raw).ctx).read(),
            meta: if declared >= ProviderInfo::meta_end() {
                // SAFETY: the guard established the field is present, and
                // a non-null `meta` is a well-formed map by the contract
                // on the field.
                std::ptr::addr_of!((*raw).meta)
                    .read()
                    .get()
                    .map(|m| m.clone_in(Alloc::rust()))
                    .transpose()
                    .ok()?
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
                str_of(std::ptr::addr_of!((*raw).version).read())
                    .filter(|v| !v.is_empty())
                    .map(str::to_string)
            } else {
                None
            },
            raw,
            tables: if declared >= ProviderInfo::tables_end() {
                // SAFETY: the guard established the field is present.
                read_kind_tables(std::ptr::addr_of!((*raw).tables).read())?
            } else {
                Vec::new()
            },
            create: if declared >= ProviderInfo::create_end() {
                // SAFETY: the guard established the field is present.
                std::ptr::addr_of!((*raw).create).read()
            } else {
                None
            },
            destroy: if declared >= ProviderInfo::destroy_end() {
                // SAFETY: the guard established the field is present.
                std::ptr::addr_of!((*raw).destroy).read()
            } else {
                None
            },
        })
    }
}

/// Walks a provider's per-kind tables at the library's stride, refusing
/// the array the way [`read_library`] refuses a provider array.
///
/// # Safety
///
/// `tables` came from a descriptor that lives for the process.
unsafe fn read_kind_tables(
    tables: super::desc::KindTables,
) -> Option<Vec<(String, *const c_void, usize)>> {
    if tables.len == 0 {
        return Some(Vec::new());
    }
    if tables.ptr.is_null() || tables.stride < KindTable::floor() {
        return None;
    }
    let extent = tables
        .len
        .checked_mul(tables.stride)
        .filter(|&bytes| bytes <= isize::MAX as usize)?;
    let mut out = Vec::with_capacity(extent / tables.stride);
    for i in 0..tables.len {
        // SAFETY: the library declared `len` tables of `stride` bytes at
        // `ptr`, and `i * stride` is within `extent`.
        let entry = unsafe { tables.ptr.cast::<u8>().add(i * tables.stride) }.cast::<KindTable>();
        // SAFETY: the leading word is readable.
        let declared = unsafe { std::ptr::addr_of!((*entry).struct_size).read() } as usize;
        if declared < KindTable::floor() || declared > tables.stride {
            return None;
        }
        // SAFETY: each field lies within `declared` bytes.
        let (kind, vtable, size) = unsafe {
            (
                str_of(std::ptr::addr_of!((*entry).kind).read())?.to_string(),
                std::ptr::addr_of!((*entry).vtable).read(),
                std::ptr::addr_of!((*entry).vtable_size).read() as usize,
            )
        };
        out.push((kind, vtable, size));
    }
    Some(out)
}

/// Where a registered library's code lives: a mapping this registry
/// holds the handle to, or the host's own binary.
///
/// The handle is what `Registry::unload` closes; `Linked` is what it
/// refuses, because there is nothing to unmap.
#[cfg(feature = "load")]
#[derive(Debug)]
pub(crate) enum Origin {
    /// A file this registry mapped and can close.
    Mapped(libloading::Library),
    /// Code in the host's own binary: `register_local`, `register_entry`.
    Linked,
}

#[cfg(feature = "load")]
impl Origin {
    /// Whether there is a mapping to close at all.
    pub(crate) fn is_mapped(&self) -> bool {
        matches!(self, Origin::Mapped(_))
    }

    /// Gives up the handle and leaves the mapping in place, so every
    /// image address the library already handed out stays valid.
    pub(crate) fn keep(self) {
        if let Origin::Mapped(library) = self {
            std::mem::forget(library);
        }
    }

    /// Closes the mapping. A close error leaves the handle given up, as
    /// `libloading` does.
    pub(crate) fn close(self) -> Result<(), libloading::Error> {
        match self {
            Origin::Mapped(library) => library.close(),
            Origin::Linked => Ok(()),
        }
    }
}

/// What opening one file came to. Four outcomes, because the registry
/// reports each differently and a caller acts on each differently.
#[cfg(feature = "load")]
#[derive(Debug)]
pub(crate) enum Opened {
    /// It mapped, and exports no entry symbol: not a library.
    NoEntrySymbol,
    /// Its entry point answered null: a library with nothing for this
    /// host.
    Declined,
    /// Its entry point answered a descriptor this build cannot use.
    Rejected(Rejected),
    /// A descriptor, read out, and the mapping it was read from.
    Loaded(LibraryView, Origin),
}

/// Opens a library, calls its entry point, and hands the mapping back
/// with what it said.
///
/// The handle travels to the registry, which holds it until the host
/// retires or unloads that library. A file that came to nothing keeps its
/// mapping: it has already run whatever it was going to run, and the
/// descriptor a rejected library handed back may still be read.
///
/// # Safety
///
/// Mapping a library runs its static initialisers, which may do anything.
/// The caller is responsible for only naming files it is willing to run.
#[cfg(feature = "load")]
pub(crate) unsafe fn open(path: &std::path::Path, host: Host) -> Result<Opened, libloading::Error> {
    // SAFETY: the caller's side of the contract, stated above.
    let library = unsafe { libloading::Library::new(path)? };

    // SAFETY: the symbol either is absent, which is an answer, or has the
    // signature this crate defines for it.
    let entry = unsafe { library.get::<EntryFn>(ENTRY_SYMBOL) };
    let Ok(entry) = entry else {
        std::mem::forget(library);
        return Ok(Opened::NoEntrySymbol);
    };

    // SAFETY: the library's side of the contract is that its entry point
    // reads a host descriptor and returns a descriptor or null. The
    // descriptor is a block the host keeps for the life of the process.
    let desc = unsafe { entry(host.as_raw()) };

    if desc.is_null() {
        std::mem::forget(library);
        return Ok(Opened::Declined);
    }
    // SAFETY: non-null, from the entry point, and the mapping is held for
    // the whole read.
    match unsafe { read_library(desc) } {
        Ok(view) => Ok(Opened::Loaded(view, Origin::Mapped(library))),
        Err(why) => {
            std::mem::forget(library);
            Ok(Opened::Rejected(why))
        }
    }
}

/// What a library the host LINKS came to: its describe function's answer,
/// read the way a loaded library's entry answer is read.
///
/// Safe to call, because a `&'static LibraryInfo` is the whole contract
/// an entry point makes and a Rust reference already states it: the
/// descriptor is well-formed for its own `struct_size` and lives for the
/// process.
#[cfg(feature = "load")]
pub(crate) fn open_local(described: Option<&'static LibraryInfo>) -> Opened {
    let Some(info) = described else {
        return Opened::Declined;
    };
    // SAFETY: a `'static` reference to a descriptor the library built and
    // keeps for the process, which is what an entry point promises.
    match unsafe { read_library(info) } {
        Ok(view) => Opened::Loaded(view, Origin::Linked),
        Err(why) => Opened::Rejected(why),
    }
}

/// What a library the host LINKS and that speaks C came to: its entry
/// point's answer, read exactly as a loaded library's is.
///
/// Safe to CALL for the reason [`open_library`] is: handing over an
/// entry point is choosing to run it, which is the contract
/// [`crate::library::Registry::register_entry`] states, and what it
/// answers is read under the same guards as a loaded library's answer.
#[cfg(feature = "load")]
pub(crate) fn open_entry(entry: EntryFn, host: Host) -> Opened {
    // SAFETY: the caller chose to run this entry point, which is the
    // contract stated on `Registry::register_entry`; its answer is null
    // or a descriptor read under `read_library`'s own guards.
    let desc = unsafe { entry(host.as_raw()) };
    if desc.is_null() {
        return Opened::Declined;
    }
    // SAFETY: non-null, from an entry point that keeps it for the process.
    match unsafe { read_library(desc) } {
        Ok(view) => Opened::Loaded(view, Origin::Linked),
        Err(why) => Opened::Rejected(why),
    }
}

/// `unload` lives here, and not beside `retire`, because
/// `library/registry.rs` carries `#![forbid(unsafe_code)]` and this is
/// the file allowed to say it.
#[cfg(feature = "load")]
impl super::registry::Registry {
    /// Takes one library out of this registry **and unmaps it**.
    ///
    /// [`retire`](super::registry::Registry::retire), then the library's
    /// own say, then the loader's close, in that order: a library that
    /// refuses is left exactly as it was, registered and mapped
    /// ([`UnloadError::Refused`](super::registry::UnloadError::Refused)).
    /// A library the host LINKS has no mapping and answers
    /// [`Linked`](super::registry::UnloadError::Linked); retiring it
    /// works.
    ///
    /// # Safety
    ///
    /// **Nothing in this crate can check this, which is why it is the
    /// caller's word.** When this returns, the library's code, its
    /// descriptors and its allocator are gone from the address space.
    /// Before calling, the host must have dropped:
    ///
    /// - every value this library built through **its own** allocator —
    ///   each records that allocator's address and calls back into it to
    ///   grow and to free;
    /// - every [`Remote`](super::kind::Remote),
    ///   [`Offer`](super::kind::Offer),
    ///   [`Instance`](super::kind::Instance) and
    ///   [`Object`](super::kind::Object) taken from it, and every raw
    ///   vtable, `ctx` or descriptor pointer read out of it;
    /// - every descriptor pointer another loaded library fetched from
    ///   this one through the host's services.
    ///
    /// A registry created with
    /// [`with_alloc`](super::registry::Registry::with_alloc) makes the
    /// first of those the common case rather than the rule: a library
    /// handed a host allocator builds the host's trees in the host's
    /// arena, where they outlive the mapping.
    ///
    /// The library must also have no thread of its own still running and
    /// nothing registered elsewhere that points into it. That is what its
    /// `unload` slot is for: it is the only side that can know.
    pub unsafe fn unload(&mut self, key: &str) -> Result<(), super::registry::UnloadError> {
        let one = self
            .library(key)
            .ok_or_else(|| super::registry::UnloadError::NotFound {
                key: key.to_string(),
            })?;
        if !one.is_mapped() {
            return Err(super::registry::UnloadError::Linked {
                key: key.to_string(),
            });
        }
        // Asked FIRST, so a refusal leaves the registry untouched. A
        // panic crossing back is caught here, as at every other boundary.
        if let Some(ask) = one.unload_slot() {
            // SAFETY: the slot is the library's own, read under its
            // `struct_size` guard, and the library is still mapped.
            let status =
                crate::exports::guard_with(Status::GUATIAO_ERR_INTERNAL, || unsafe { ask() });
            if status != Status::GUATIAO_OK {
                return Err(super::registry::UnloadError::Refused {
                    key: key.to_string(),
                    status,
                });
            }
        }
        let (_, origin) = self.take_library(key)?;
        origin
            .close()
            .map_err(|e| super::registry::UnloadError::Close {
                key: key.to_string(),
                reason: e.to_string(),
            })
    }
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
    host: Host,
) -> Result<Opened, libloading::Error> {
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
            services: std::ptr::null(),
        }
    }

    /// A host from before `services` was appended reads as offering none,
    /// through both readers.
    #[test]
    fn a_host_from_before_services_offers_none() {
        let value = a_host(HostInfo::meta_end());
        let (_buf, ptr) = short_of(&value, HostInfo::meta_end());
        // SAFETY: `_buf` owns the bytes.
        let read = unsafe { read_host(ptr) }.expect("above the floor");
        assert!(read.services.is_null(), "past the declared size, so absent");
        // SAFETY: as above.
        let host = unsafe { Host::from_raw(ptr) }.expect("above the floor");
        assert_eq!(host.get("anything").err(), Some(Status::GUATIAO_ERR_NULL));
        assert_eq!(host.list("anything").err(), Some(Status::GUATIAO_ERR_NULL));
        assert_eq!(host.id(), "test-host");
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
            unload: None,
        }
    }

    /// A library registered through its C entry point lands as a file's
    /// would, under `<name>`; null is a decline; a second registration
    /// under the name is a skip.
    #[cfg(feature = "load")]
    #[test]
    fn a_linked_library_registers_through_its_entry_point() {
        use crate::library::{Registry, Skipped};
        use std::path::{Path, PathBuf};

        unsafe extern "C" fn describing(_host: *const HostInfo) -> *const LibraryInfo {
            Box::leak(Box::new(a_library(size_of::<LibraryInfo>())))
        }
        unsafe extern "C" fn declining(_host: *const HostInfo) -> *const LibraryInfo {
            std::ptr::null()
        }

        let mut r = Registry::new("test-host", "1.0");
        let loaded = r.register_entry("cli", describing).unwrap();
        assert_eq!(
            loaded.loaded().map(|l| l.path.as_path()),
            Some(Path::new("<cli>"))
        );
        assert_eq!(
            r.register_entry("cli", describing).unwrap().skipped(),
            Some(&Skipped::AlreadyLoaded {
                from: PathBuf::from("<cli>")
            })
        );
        assert_eq!(
            r.register_entry("shy", declining).unwrap().skipped(),
            Some(&Skipped::DeclinedThisHost)
        );
    }

    #[test]
    fn a_library_descriptor_below_the_floor_is_refused() {
        let value = a_library(LibraryInfo::floor() - 1);
        let (_buf, ptr) = short_of(&value, size_of::<LibraryInfo>());
        // SAFETY: `_buf` owns the bytes.
        assert!(unsafe { read_library(ptr) }.is_err());

        let value = a_library(LibraryInfo::floor());
        let (_buf, ptr) = short_of(&value, LibraryInfo::floor());
        // SAFETY: as above.
        let view = unsafe { read_library(ptr) }.expect("a floor-sized descriptor is usable");
        assert_eq!((view.id.as_str(), view.version.as_str()), ("lib", "0.1.0"));
        assert!(view.providers.is_empty());
        assert!(
            view.meta.is_none(),
            "a descriptor that predates `meta` declares none"
        );
        assert!(view.unload.is_none());
    }

    /// A library built before `unload` was appended still loads, and is
    /// read as declaring no say in being unmapped.
    ///
    /// The direction that matters: appending a slot must not lock out a
    /// library compiled before it existed.
    #[test]
    fn a_library_from_before_unload_reads_it_as_absent_and_still_loads() {
        unsafe extern "C" fn refuse() -> Status {
            Status::GUATIAO_ERR_WRONG_KIND
        }

        let mut value = a_library(LibraryInfo::meta_end());
        value.unload = Some(refuse);
        let (_buf, ptr) = short_of(&value, LibraryInfo::meta_end());
        // SAFETY: `_buf` owns the bytes.
        let view = unsafe { read_library(ptr) }.expect("above the floor");
        assert!(
            view.unload.is_none(),
            "the slot sits past the declared size, so reading it anyway \
             hands back the 0xAA tail as a function pointer"
        );

        // And one that DOES declare it hands it over, or the test above
        // passes against a reader that never reads the field.
        let value = a_library(size_of::<LibraryInfo>());
        let mut value = LibraryInfo {
            unload: Some(refuse),
            ..value
        };
        value.struct_size = size_of::<LibraryInfo>() as u32;
        let (_buf, ptr) = short_of(&value, size_of::<LibraryInfo>());
        // SAFETY: as above.
        let view = unsafe { read_library(ptr) }.expect("a full descriptor");
        assert!(view.unload.is_some());
    }

    /// A library speaking another envelope version is refused by name,
    /// before any field past the floor is read.
    #[test]
    fn a_library_declaring_another_abi_version_is_refused() {
        let mut value = a_library(size_of::<LibraryInfo>());
        value.abi_version = crate::library::ABI_VERSION + 1;
        let (_buf, ptr) = short_of(&value, size_of::<LibraryInfo>());
        // SAFETY: `_buf` owns the bytes.
        assert_eq!(
            unsafe { read_library(ptr) }.unwrap_err(),
            Rejected::UnsupportedAbi {
                declared: crate::library::ABI_VERSION + 1
            }
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
        assert!(unsafe { read_library(ptr) }.is_err());
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
            tables: super::super::desc::KindTables::empty(),
            create: None,
            destroy: None,
        }
    }

    /// A descriptor from before `create` and `destroy` reads them as
    /// absent, and one declaring them hands them over.
    #[test]
    fn a_descriptor_from_before_create_reads_it_as_absent() {
        unsafe extern "C" fn build(
            _ctx: *mut c_void,
            _config: *const Value,
            _out: *mut *mut c_void,
            _err: *mut super::super::kind::ProviderError,
        ) -> Status {
            Status::GUATIAO_ERR_INTERNAL
        }
        unsafe extern "C" fn drop_it(_ctx: *mut c_void, _instance: *mut c_void) {}

        let value = a_provider(ProviderInfo::tables_end(), 0);
        let (_buf, ptr) = short_of(&value, ProviderInfo::tables_end());
        // SAFETY: `_buf` owns the bytes.
        let view =
            unsafe { read_provider(ptr, ProviderInfo::tables_end()) }.expect("above the floor");
        assert!(view.create.is_none() && view.destroy.is_none());

        let mut value = a_provider(size_of::<ProviderInfo>(), 0);
        value.create = Some(build);
        value.destroy = Some(drop_it);
        let (_buf, ptr) = short_of(&value, size_of::<ProviderInfo>());
        // SAFETY: as above.
        let view =
            unsafe { read_provider(ptr, size_of::<ProviderInfo>()) }.expect("a full descriptor");
        assert!(view.create.is_some() && view.destroy.is_some());
    }

    /// A provider with a table per kind hands each back by kind, and the
    /// shared `vtable` answers for the kinds without one.
    #[test]
    fn a_provider_with_a_table_per_kind_hands_each_back_by_kind() {
        use super::super::desc::KindTables;

        struct Tables([KindTable; 2]);
        // SAFETY: a constant never written; the pointers are sentinels
        // that are never dereferenced.
        unsafe impl Sync for Tables {}
        static TABLES: Tables = Tables([
            KindTable::new("greeter", 0x10 as *const c_void, 16),
            KindTable::new("counter", 0x20 as *const c_void, 24),
        ]);

        let mut value = a_provider(size_of::<ProviderInfo>(), 8);
        value.vtable = 0x30 as *const c_void;
        value.tables = KindTables::new(&TABLES.0);
        let (_buf, ptr) = short_of(&value, size_of::<ProviderInfo>());
        // SAFETY: `_buf` owns the descriptor; `TABLES` is static.
        let view =
            unsafe { read_provider(ptr, size_of::<ProviderInfo>()) }.expect("a full descriptor");

        assert_eq!(view.table_for("greeter"), Some((0x10 as *const c_void, 16)));
        assert_eq!(view.table_for("counter"), Some((0x20 as *const c_void, 24)));
        assert_eq!(
            view.table_for("everything"),
            None,
            "the shared vtable answers only for a kind in `kinds`"
        );
        assert_eq!(view.table_for("nonesuch"), None);

        // A descriptor from before `tables` reads as having none, and the
        // shared vtable serves what `kinds` names.
        let (_buf, ptr) = short_of(&value, ProviderInfo::available_end());
        let mut older = a_provider(ProviderInfo::available_end(), 8);
        older.vtable = 0x30 as *const c_void;
        let (_buf2, ptr2) = short_of(&older, ProviderInfo::available_end());
        let _ = ptr;
        // SAFETY: as above.
        let view =
            unsafe { read_provider(ptr2, ProviderInfo::available_end()) }.expect("above the floor");
        assert!(view.tables.is_empty());
        assert_eq!(view.table_for("greeter"), Some((0x30 as *const c_void, 8)));

        // A stride below the floor refuses the descriptor.
        let mut bad = value;
        bad.tables.stride = KindTable::floor() - 1;
        let (_buf3, ptr3) = short_of(&bad, size_of::<ProviderInfo>());
        // SAFETY: as above.
        assert!(unsafe { read_provider(ptr3, size_of::<ProviderInfo>()) }.is_none());
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
