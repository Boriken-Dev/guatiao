// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a derived provider and library build: the descriptors, and the
//! storage they point into.

use super::*;

// --- what a derived provider and library build --------------------------

/// A type `#[derive(Provider)]` made a provider of.
pub trait ProviderDecl {
    /// The name of every kind this provider serves, so a library can
    /// declare them at compile time.
    const KINDS: &'static [&'static str];

    /// Builds this provider's descriptor, its tables and its configuration
    /// schema, once. `host` is what the library was loaded by; `alloc` is
    /// what the schema is built through.
    fn provider(host: Host, alloc: Alloc) -> Result<ProviderParts, ValueError>;

    /// How many instances and objects this provider has handed out and
    /// not seen destroyed. See [`Kind::live_objects`].
    #[doc(hidden)]
    fn live(_seen: &mut Vec<&'static str>) -> usize {
        0
    }
}

/// Everything one provider's descriptor points at, owned in one place so
/// the descriptor stays valid for as long as this does.
#[derive(Debug)]
pub struct ProviderParts {
    #[allow(dead_code)]
    id: Box<str>,
    #[allow(dead_code)]
    name: Box<str>,
    #[allow(dead_code)]
    version: Box<str>,
    #[allow(dead_code)]
    kinds: Box<[Str<'static>]>,
    #[allow(dead_code)]
    tables: Box<[KindTable<'static>]>,
    #[allow(dead_code)]
    config: Option<Box<Value>>,
    info: ProviderInfo<'static>,
}

// SAFETY: built once and never written; every pointer in `info` addresses
// this struct's own boxes, a `static`, or an instance that lives for the
// process.
unsafe impl Send for ProviderParts {}
// SAFETY: as above.
unsafe impl Sync for ProviderParts {}

impl ProviderParts {
    /// Owns the parts and builds the descriptor over them.
    ///
    /// `ctx` and every table must live for the process, as a `static` or
    /// a leaked instance does.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: &str,
        name: &str,
        version: &str,
        kinds: &[&'static str],
        tables: Vec<KindTable<'static>>,
        config: Option<Value>,
        ctx: *mut c_void,
        available: Option<unsafe extern "C" fn(ctx: *mut c_void, reason: *mut Str) -> bool>,
        create: Option<
            unsafe extern "C" fn(
                ctx: *mut c_void,
                config: *const Value,
                out: *mut *mut c_void,
                err: *mut ProviderError,
            ) -> Status,
        >,
        destroy: Option<unsafe extern "C" fn(ctx: *mut c_void, instance: *mut c_void)>,
    ) -> ProviderParts {
        let id: Box<str> = id.into();
        let name: Box<str> = name.into();
        let version: Box<str> = version.into();
        let kinds: Box<[Str]> = kinds.iter().map(|k| Str::new(k)).collect();
        let tables: Box<[KindTable]> = tables.into_boxed_slice();
        let config = config.map(Box::new);
        // The boxes' heap storage does not move when this struct does.
        let info = ProviderInfo {
            struct_size: size_of::<ProviderInfo>() as u32,
            vtable_size: 0,
            kinds: crate::library::desc::Kinds {
                ptr: kinds.as_ptr(),
                len: kinds.len(),
            },
            // SAFETY: the boxes are kept beside `info`, below.
            id: unsafe { crate::library::raw::view_of_owned(&id) },
            display_name: unsafe { crate::library::raw::view_of_owned(&name) },
            config: config
                .as_deref()
                .map_or(std::ptr::null(), |v| v as *const Value),
            vtable: std::ptr::null(),
            ctx,
            meta: crate::value::types::MaybeNull::null(),
            // SAFETY: as `id`.
            version: unsafe { crate::library::raw::view_of_owned(&version) },
            available,
            tables: KindTables {
                ptr: tables.as_ptr(),
                len: tables.len(),
                stride: size_of::<KindTable>(),
            },
            create,
            destroy,
        };
        ProviderParts {
            id,
            name,
            version,
            kinds,
            tables,
            config,
            info,
        }
    }

    /// The descriptor, pointing into this.
    pub fn info(&self) -> &ProviderInfo<'_> {
        &self.info
    }
}

/// Everything a derived library's descriptor points at.
#[derive(Debug)]
pub struct LibraryParts {
    #[allow(dead_code)]
    id: Box<str>,
    #[allow(dead_code)]
    version: Box<str>,
    #[allow(dead_code)]
    providers: Vec<ProviderParts>,
    #[allow(dead_code)]
    infos: Box<[ProviderInfo<'static>]>,
    info: LibraryInfo<'static>,
}

// SAFETY: as `ProviderParts`.
unsafe impl Send for LibraryParts {}
// SAFETY: as above.
unsafe impl Sync for LibraryParts {}

impl LibraryParts {
    /// Owns the providers and builds the library descriptor over them.
    pub fn new(id: &str, version: &str, providers: Vec<ProviderParts>) -> LibraryParts {
        let id: Box<str> = id.into();
        let version: Box<str> = version.into();
        let infos: Box<[ProviderInfo<'static>]> = providers.iter().map(|p| p.info).collect();
        let info = LibraryInfo {
            struct_size: size_of::<LibraryInfo>() as u32,
            abi_version: crate::library::desc::ABI_VERSION,
            // SAFETY: the boxes are kept beside `info`, below.
            id: unsafe { crate::library::raw::view_of_owned(&id) },
            version: unsafe { crate::library::raw::view_of_owned(&version) },
            providers: crate::library::desc::Providers {
                ptr: infos.as_ptr(),
                len: infos.len(),
                stride: size_of::<ProviderInfo>(),
            },
            meta: crate::value::types::MaybeNull::null(),
            unload: None,
        };
        LibraryParts {
            id,
            version,
            providers,
            infos,
            info,
        }
    }

    /// The library's say in being unmapped. See [`LibraryInfo::unload`].
    pub fn unloading(mut self, unload: Option<unsafe extern "C" fn() -> Status>) -> LibraryParts {
        self.info.unload = unload;
        self
    }

    /// The descriptor, pointing into this.
    pub fn info(&self) -> &LibraryInfo<'_> {
        &self.info
    }
}

/// FNV-1a over a string, at compile time. The kind macro emits the
/// normalised signature string and this hashes it, so the number is
/// never computed by the macro itself.
#[doc(hidden)]
pub const fn fnv1a(s: &str) -> u32 {
    let bytes = s.as_bytes();
    let mut hash = 0x811c_9dc5u32;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u32;
        hash = hash.wrapping_mul(0x0100_0193);
        i += 1;
    }
    hash
}
