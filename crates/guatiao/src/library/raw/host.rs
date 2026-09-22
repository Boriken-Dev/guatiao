// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The host's side: the block a host hands its libraries, the services
//! behind it, and `Host`, a library's handle on it.

use super::*;

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
    raw: *const HostInfo<'static>,
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
    pub unsafe fn from_raw(raw: *const HostInfo<'static>) -> Option<Host> {
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
    pub fn as_raw(&self) -> *const HostInfo<'static> {
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
        unsafe { std::str::from_utf8(std::ptr::addr_of!((*self.raw).host_id).read().into()).ok() }
            .unwrap_or("")
    }

    /// The host's own version string, uninterpreted.
    pub fn version(&self) -> &'static str {
        // SAFETY: as `id`.
        unsafe {
            std::str::from_utf8(std::ptr::addr_of!((*self.raw).host_version).read().into()).ok()
        }
        .unwrap_or("")
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
    pub fn snapshot(&self) -> HostInfo<'static> {
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
    pub fn get(&self, key: &str) -> Result<Option<&'static ProviderInfo<'static>>, Status> {
        let table = self.services().ok_or(Status::GUATIAO_ERR_NULL)?;
        let get = table.get.ok_or(Status::GUATIAO_ERR_NULL)?;
        let mut out: *const ProviderInfo = std::ptr::null();
        // SAFETY: the slot is the host's own, read under its guard; `out`
        // is a writable local; `key` is readable for the call.
        match unsafe { get(table.ctx, Str::new(key), &mut out) } {
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
    pub fn list(&self, kind: &str) -> Result<Vec<&'static ProviderInfo<'static>>, Status> {
        let table = self.services().ok_or(Status::GUATIAO_ERR_NULL)?;
        let list = table.list.ok_or(Status::GUATIAO_ERR_NULL)?;
        let kind = Str::new(kind);
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
    info: HostInfo<'static>,
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
    pub(crate) raw: *const ProviderInfo<'static>,
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
unsafe impl Send for crate::library::registry::Registry {}
#[cfg(feature = "load")]
unsafe impl Sync for crate::library::registry::Registry {}
#[cfg(feature = "load")]
unsafe impl Send for crate::library::registry::Provider {}
#[cfg(feature = "load")]
unsafe impl Sync for crate::library::registry::Provider {}
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
/// A view of text a box owns, for a descriptor kept in the same struct as
/// that box. `'static` in name only: the struct hands its descriptor out
/// for no longer than itself.
///
/// # Safety
///
/// `text` is heap storage kept, unmoved and unchanged, for as long as the
/// view is read.
pub(crate) unsafe fn view_of_owned(text: &str) -> Str<'static> {
    // SAFETY: the caller's contract, and a `str` is UTF-8.
    unsafe { Str::from_raw_parts(text.as_ptr(), text.len()) }
}

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
            abi_version: crate::library::desc::ABI_VERSION,
            // SAFETY: the boxes' heap storage does not move when the
            // block does, and the block is leaked with them.
            host_id: unsafe { view_of_owned(&id) },
            host_version: unsafe { view_of_owned(&version) },
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
        let Ok(key) = std::str::from_utf8(key.into()) else {
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
        let Ok(kind) = std::str::from_utf8(kind.into()) else {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::raw::test_support::*;

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
        assert_eq!(read.host_id.len(), "test-host".len());
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
}
