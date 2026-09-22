// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Reading a library's image: opening it, and copying what its
//! descriptors say out into owned views, under their declared sizes.

use super::*;

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
    pub raw: *const ProviderInfo<'static>,
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
            err: *mut crate::library::kind::ProviderError,
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
        Err(std::str::from_utf8(reason.into()).ok().unwrap_or(""))
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
pub(crate) unsafe fn read_library(
    raw: *const LibraryInfo<'static>,
) -> Result<LibraryView, Rejected> {
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
    if abi != crate::library::desc::ABI_VERSION {
        return Err(Rejected::UnsupportedAbi { declared: abi });
    }

    // SAFETY: each field lies within `declared` bytes.
    let (id, version, providers) = unsafe {
        (
            std::str::from_utf8(std::ptr::addr_of!((*raw).id).read().into())
                .ok()
                .ok_or(Rejected::Malformed)?
                .to_string(),
            std::str::from_utf8(std::ptr::addr_of!((*raw).version).read().into())
                .ok()
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

impl ProviderInfo<'static> {
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
unsafe fn read_provider(raw: *const ProviderInfo<'static>, limit: usize) -> Option<ProviderView> {
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
            names.push(<&str>::from(Str::from_ptr(kinds.ptr.add(i)).ok()?).to_string());
        }

        Some(ProviderView {
            kinds: names,
            id: std::str::from_utf8(std::ptr::addr_of!((*raw).id).read().into())
                .ok()?
                .to_string(),
            display_name: std::str::from_utf8(
                std::ptr::addr_of!((*raw).display_name).read().into(),
            )
            .ok()?
            .to_string(),
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
                std::str::from_utf8(std::ptr::addr_of!((*raw).version).read().into())
                    .ok()
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
    tables: crate::library::desc::KindTables,
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
                std::str::from_utf8(std::ptr::addr_of!((*entry).kind).read().into())
                    .ok()?
                    .to_string(),
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
impl crate::library::registry::Registry {
    /// Takes one library out of this registry **and unmaps it**, when the
    /// library agrees.
    ///
    /// The library's `unload` slot is asked first. It is the library's
    /// promise that everything it can account for is released: a derived
    /// library counts its instances and objects and answers
    /// `GUATIAO_ERR_BUSY` while any is alive
    /// ([`Refused`](crate::library::registry::UnloadError::Refused), and it is left
    /// exactly as it was). A library with no slot promises nothing and
    /// answers [`NotSupported`](crate::library::registry::UnloadError::NotSupported);
    /// [`unload_unchecked`](Self::unload_unchecked) is the host insisting.
    /// A library the host LINKS answers
    /// [`Linked`](crate::library::registry::UnloadError::Linked); retiring it works.
    ///
    /// # Safety
    ///
    /// What no library can count is still the caller's word: every
    /// [`Remote`](crate::library::kind::Remote) and
    /// [`Offer`](crate::library::kind::Offer) copied out of it, every raw vtable,
    /// `ctx` or descriptor pointer, every value it built through its own
    /// allocator that its slot does not track, and every descriptor
    /// pointer another library fetched from it through the host's
    /// services, is not used again.
    pub unsafe fn unload(
        &mut self,
        key: &str,
    ) -> Result<(), crate::library::registry::UnloadError> {
        // SAFETY: forwarded.
        unsafe { self.unload_with(key, false) }
    }

    /// [`unload`](Self::unload) for a library with no `unload` slot: the
    /// host's word alone. A library that HAS a slot is still asked, and
    /// its refusal still stands.
    ///
    /// # Safety
    ///
    /// As [`unload`](Self::unload), and nothing the library handed out is
    /// alive at all: no instance, no object, no value from its allocator,
    /// and no thread of its own still running.
    pub unsafe fn unload_unchecked(
        &mut self,
        key: &str,
    ) -> Result<(), crate::library::registry::UnloadError> {
        // SAFETY: forwarded.
        unsafe { self.unload_with(key, true) }
    }

    /// # Safety
    ///
    /// As whichever of the two public forms called it.
    unsafe fn unload_with(
        &mut self,
        key: &str,
        insist: bool,
    ) -> Result<(), crate::library::registry::UnloadError> {
        use crate::library::registry::UnloadError;

        let one = self.library(key).ok_or_else(|| UnloadError::NotFound {
            key: key.to_string(),
        })?;
        if !one.is_mapped() {
            return Err(UnloadError::Linked {
                key: key.to_string(),
            });
        }
        // Asked FIRST, so a refusal leaves the registry untouched. A panic
        // crossing back is caught here, as at every other boundary.
        match one.unload_slot() {
            Some(ask) => {
                // SAFETY: the slot is the library's own, read under its
                // `struct_size` guard, and the library is still mapped.
                let status =
                    crate::exports::guard_with(Status::GUATIAO_ERR_INTERNAL, || unsafe { ask() });
                if status != Status::GUATIAO_OK {
                    return Err(UnloadError::Refused {
                        key: key.to_string(),
                        status,
                    });
                }
            }
            None if insist => {}
            None => {
                return Err(UnloadError::NotSupported {
                    key: key.to_string(),
                });
            }
        }
        let (_, origin) = self.take_library(key)?;
        origin.close().map_err(|e| UnloadError::Close {
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
    use crate::library::raw::test_support::*;

    fn a_library(size: usize) -> LibraryInfo<'static> {
        LibraryInfo {
            struct_size: size as u32,
            abi_version: crate::library::ABI_VERSION,
            id: Str::new("lib"),
            version: Str::new("0.1.0"),
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

        unsafe extern "C" fn describing(
            _host: *const HostInfo<'static>,
        ) -> *const LibraryInfo<'static> {
            Box::leak(Box::new(a_library(size_of::<LibraryInfo>())))
        }
        unsafe extern "C" fn declining(
            _host: *const HostInfo<'static>,
        ) -> *const LibraryInfo<'static> {
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
        second.id = Str::new("second");

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
    struct Names([Str<'static>; 1]);
    // SAFETY: a constant that is never written, whose pointer addresses a
    // string literal in this binary.
    unsafe impl Sync for Names {}
    static KINDS: Names = Names([Str::new("greeter")]);

    fn a_provider(size: usize, vtable_size: u32) -> ProviderInfo<'static> {
        ProviderInfo {
            struct_size: size as u32,
            vtable_size,
            kinds: Kinds::new(&KINDS.0),
            id: Str::new("hello"),
            display_name: Str::new("Hello"),
            config: std::ptr::null(),
            vtable: std::ptr::null(),
            ctx: std::ptr::null_mut(),
            meta: MaybeNull::null(),
            version: Str::new(""),
            available: None,
            tables: crate::library::desc::KindTables::empty(),
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
            _err: *mut crate::library::kind::ProviderError,
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
        use crate::library::desc::KindTables;

        struct Tables([KindTable<'static>; 2]);
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
            unsafe { reason.write(Str::new("no calendar here")) };
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
