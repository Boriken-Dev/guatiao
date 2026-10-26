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
/// from any later call. A host declaring a `struct_size` below this
/// build's [`HostInfo`] is refused; fields past it are never read.
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
    /// `None` for null, for a pointer that is misaligned or no address at
    /// all, or for a descriptor below the floor.
    ///
    /// # Safety
    ///
    /// `raw` is null, or points at `raw->struct_size` readable, aligned
    /// bytes that stay valid and unchanged for the life of the process.
    pub unsafe fn from_raw(raw: *const HostInfo<'static>) -> Option<Host> {
        if raw.is_null() || !addressable(raw) {
            return None;
        }
        // SAFETY: the caller guarantees the first four bytes are readable.
        let declared = unsafe { core::ptr::addr_of!((*raw).struct_size).read() } as usize;
        if declared < HostInfo::floor() {
            return None;
        }
        Some(Host { raw })
    }

    /// The pointer as it was handed over, for a caller that speaks C.
    pub fn as_raw(&self) -> *const HostInfo<'static> {
        self.raw
    }

    /// The envelope version the host speaks.
    pub fn abi_version(&self) -> u32 {
        // SAFETY: within the floor `from_raw` checked.
        unsafe { core::ptr::addr_of!((*self.raw).abi_version).read() }
    }

    /// Who the host is. Empty when it said nothing readable.
    pub fn id(&self) -> &'static str {
        // SAFETY: within the floor; the text lives in the host's block.
        unsafe { core::str::from_utf8(core::ptr::addr_of!((*self.raw).host_id).read().into()).ok() }
            .unwrap_or("")
    }

    /// The host's own version string, uninterpreted.
    pub fn version(&self) -> &'static str {
        // SAFETY: as `id`.
        unsafe {
            core::str::from_utf8(core::ptr::addr_of!((*self.raw).host_version).read().into()).ok()
        }
        .unwrap_or("")
    }

    /// The host's allocator, or `None` when it offers none.
    pub fn alloc(&self) -> Option<Alloc> {
        // SAFETY: within the floor `from_raw` checked, and the allocator it
        // names outlives the process by the block's contract.
        let raw = unsafe { core::ptr::addr_of!((*self.raw).alloc).read() };
        unsafe { Alloc::from_raw(raw) }.ok()
    }

    /// Whatever else the host declared, or `None`.
    pub fn meta(&self) -> Option<&'static Map> {
        // SAFETY: within the floor `from_raw` checked; non-null
        // means a well-formed map by the contract on the field; the block
        // lives for the process.
        unsafe { core::ptr::addr_of!((*self.raw).meta).read().get() }
    }

    /// A copy of the descriptor as it reads today. For a caller that wants
    /// a snapshot rather than a handle.
    pub fn snapshot(&self) -> HostInfo<'static> {
        // SAFETY: `from_raw` established the contract `read_host` needs.
        unsafe { read_host(self.raw) }.expect("a Host was built from a readable descriptor")
    }

    /// The services table, or `None` for a host that offers none or one
    /// declaring a `struct_size` below this build's.
    fn services(&self) -> Option<Services> {
        // SAFETY: within the floor `from_raw` checked.
        let table = unsafe { core::ptr::addr_of!((*self.raw).services).read() };
        if table.is_null() || !addressable(table) {
            return None;
        }
        // SAFETY: a non-null table points at one the host keeps for the
        // process, declaring its own size; the leading word is readable.
        let size = unsafe { core::ptr::addr_of!((*table).struct_size).read() } as usize;
        (size >= HostServices::floor()).then_some(Services { table })
    }

    /// One provider by the key the host files it under, read out of the
    /// descriptor the host answers with, under the size it declares.
    ///
    /// **The view holds the host's pin on that provider's library**, when
    /// the host pins (this crate's does): the library stays loaded while
    /// the view, or any `Offer` or `Remote` taken from it, lives, and the
    /// host's `unload` answers busy until it is dropped. From a host that
    /// does not pin, the view copies the tables and holds nothing.
    ///
    /// `Ok(None)` when nothing answers to the key, or the descriptor is
    /// not one this build can read. `Err` when the host offers no services
    /// (`GUATIAO_ERR_NULL`) or its registry is gone (`GUATIAO_ERR_GONE`).
    pub fn get(&self, key: &str) -> Result<Option<ProviderView>, Status> {
        let services = self.services().ok_or(Status::GUATIAO_ERR_NULL)?;
        let pinned = services.pinned();
        let get = match pinned {
            Some(p) => p.get,
            None => services.get().ok_or(Status::GUATIAO_ERR_NULL)?,
        };
        let mut out: *const ProviderInfo = core::ptr::null();
        // SAFETY: the slot is the host's own, read under its guard; `out`
        // is a writable local; `key` is readable for the call.
        match unsafe { get(services.ctx(), Str::new(key), &mut out) } {
            // SAFETY: the host answers with a descriptor valid while it is
            // pinned, or while its library is registered; it is read
            // through the raw pointer, never as a reference to a whole
            // `ProviderInfo`.
            Status::GUATIAO_OK if !out.is_null() => Ok(unsafe { answer(out, &services, pinned) }),
            Status::GUATIAO_OK | Status::GUATIAO_ERR_NOT_FOUND => Ok(None),
            other => Err(other),
        }
    }

    /// Every provider serving `kind` — every provider when it is empty —
    /// in the host's own order, unavailable ones included, each read as
    /// [`get`](Host::get) reads one, pins and all. The caller asks each
    /// and chooses. A descriptor this build cannot read is left out.
    ///
    /// `Err` when the host offers no services or its registry is gone.
    pub fn list(&self, kind: &str) -> Result<Vec<ProviderView>, Status> {
        let services = self.services().ok_or(Status::GUATIAO_ERR_NULL)?;
        let pinned = services.pinned();
        let list = match pinned {
            Some(p) => p.list,
            None => services.list().ok_or(Status::GUATIAO_ERR_NULL)?,
        };
        let kind = Str::new(kind);
        let mut total = 0usize;
        // SAFETY: as `get`; a zero capacity asks for the count alone and
        // pins nothing.
        match unsafe { list(services.ctx(), kind, core::ptr::null_mut(), 0, &mut total) } {
            Status::GUATIAO_OK => {}
            other => return Err(other),
        }
        let mut found: Vec<*const ProviderInfo> = vec![core::ptr::null(); total];
        let mut written = 0usize;
        // SAFETY: `found` has room for `total` entries.
        match unsafe {
            list(
                services.ctx(),
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
        // written is what is real, and each written one is pinned.
        found.truncate(written.min(total));
        Ok(found
            .into_iter()
            .filter(|p| !p.is_null())
            // SAFETY: as `get`.
            .filter_map(|p| unsafe { answer(p, &services, pinned) })
            .collect())
    }
}

/// A host's services table, its `struct_size` checked against this
/// build's.
struct Services {
    table: *const HostServices,
}

/// The services' `get` slots.
type GetFn =
    unsafe extern "C" fn(ctx: *mut c_void, key: Str, out: *mut *const ProviderInfo) -> Status;
/// The services' `list` slots.
type ListFn = unsafe extern "C" fn(
    ctx: *mut c_void,
    kind: Str,
    out: *mut *const ProviderInfo,
    cap: usize,
    total: *mut usize,
) -> Status;
/// The services' `pin` and `unpin` slots.
type PinFn = unsafe extern "C" fn(ctx: *mut c_void, desc: *const ProviderInfo);

/// The four pinning slots, filled together or not at all.
#[derive(Clone, Copy)]
struct Pinning {
    get: GetFn,
    list: ListFn,
    pin: PinFn,
    unpin: PinFn,
}

impl Services {
    fn ctx(&self) -> *mut c_void {
        // SAFETY: within the floor.
        unsafe { core::ptr::addr_of!((*self.table).ctx).read() }
    }

    fn get(&self) -> Option<GetFn> {
        // SAFETY: within the floor.
        unsafe { core::ptr::addr_of!((*self.table).get).read() }
    }

    fn list(&self) -> Option<ListFn> {
        // SAFETY: within the floor.
        unsafe { core::ptr::addr_of!((*self.table).list).read() }
    }

    /// The pinning slots, when the table fills all four.
    fn pinned(&self) -> Option<Pinning> {
        // SAFETY: within the floor.
        unsafe {
            Some(Pinning {
                get: core::ptr::addr_of!((*self.table).get_pinned).read()?,
                list: core::ptr::addr_of!((*self.table).list_pinned).read()?,
                pin: core::ptr::addr_of!((*self.table).pin).read()?,
                unpin: core::ptr::addr_of!((*self.table).unpin).read()?,
            })
        }
    }
}

/// A descriptor a host's services answered with, read as a view: holding
/// the host's pin when it was pinned, over copies of its tables when not.
/// A pinned one this build cannot read is unpinned at once.
///
/// # Safety
///
/// `raw` is non-null and addresses a provider descriptor the host built,
/// readable for its `struct_size`, valid while it is pinned or, unpinned,
/// for the read; `pinned` is `Some` exactly when the host pinned it.
unsafe fn answer(
    raw: *const ProviderInfo<'static>,
    services: &Services,
    pinned: Option<Pinning>,
) -> Option<ProviderView> {
    let hold = pinned.map(|p| Pin::taken(services.ctx(), p.pin, p.unpin, raw));
    // SAFETY: forwarded.
    let mut view = unsafe { read_answer(raw) }?;
    Some(match hold {
        Some(pin) => {
            view.hold = Hold::Pin(pin);
            view
        }
        None => Lease::alone(view),
    })
}

/// A descriptor a host's services answered with, read under its own
/// declared size. Its tables are left where the host put them.
///
/// # Safety
///
/// `raw` is non-null and addresses a provider descriptor, readable for
/// its `struct_size`, valid for the read.
unsafe fn read_answer(raw: *const ProviderInfo<'static>) -> Option<ProviderView> {
    let bounds = super::check::Bounds::ANY;
    bounds
        .data(raw as usize, size_of::<u32>(), align_of::<ProviderInfo>())
        .ok()?;
    // SAFETY: the caller guarantees the leading word is readable.
    let declared = unsafe { core::ptr::addr_of!((*raw).struct_size).read() } as usize;
    // SAFETY: the caller's contract; `declared` is the descriptor's own
    // limit.
    unsafe { super::image::read_provider(raw, declared, bounds) }.ok()
}

/// Whether `raw` could address a `T` at all: aligned, and an address.
fn addressable<T>(raw: *const T) -> bool {
    super::check::Bounds::ANY
        .data(raw as usize, size_of::<u32>(), align_of::<T>())
        .is_ok()
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
    /// `String`, not `Box<str>`: moving a `Box` into the block would
    /// retag it as unique and invalidate `info`'s views of it.
    #[allow(dead_code)]
    id: String,
    #[allow(dead_code)]
    version: String,
    state: alloc::sync::Weak<Shared>,
}

/// What the registry publishes for its services to read: the providers
/// it holds, already in its order, with each descriptor's address.
#[cfg(feature = "load")]
#[derive(Debug, Default)]
pub(crate) struct Snapshot {
    /// In `(priority DESC, key ASC)` order.
    pub(crate) entries: Vec<SnapshotEntry>,
    /// Rendered key to an index into `entries`.
    pub(crate) by_key: alloc::collections::BTreeMap<String, usize>,
    /// One hold per library the entries name, which keeps every
    /// descriptor they hand out alive while the snapshot lists it.
    pub(crate) leases: Vec<alloc::sync::Arc<Lease>>,
}

/// One provider as the services see it: the kinds it serves, and which
/// descriptor of which of the snapshot's leases is its own.
#[cfg(feature = "load")]
#[derive(Debug)]
pub(crate) struct SnapshotEntry {
    pub(crate) kinds: Vec<String>,
    /// An index into [`Snapshot::leases`].
    pub(crate) lease: usize,
    /// Which of that lease's descriptors.
    pub(crate) at: usize,
}

#[cfg(feature = "load")]
impl Snapshot {
    /// The descriptor handed to a library that asks for `entry`, valid
    /// while this snapshot, or a pin, holds its lease.
    fn desc(&self, entry: &SnapshotEntry) -> *const ProviderInfo<'static> {
        self.leases[entry.lease].desc(entry.at)
    }
}

// SAFETY (all four): every raw pointer a registry or a provider holds
// addresses a lease's copies and descriptors, kept while it is held, or a
// loaded library's code and `ctx`, which stay mapped until the registry
// closes that library; none is written from this side. The block a
// registry leaks is already shared across threads by design (a library
// reads it from any call), and a provider's slots are declared callable
// from any thread (`ProviderInfo::available`'s contract exists precisely
// because two threads may ask at once). A host keeping its one registry
// behind a lock, or reading it from several threads, is the ordinary
// shape, and without these it could not. Declared here rather than
// beside the types because this is the file allowed to say `unsafe`.
#[cfg(feature = "load")]
unsafe impl Send for crate::library::registry::Registry {}
#[cfg(feature = "load")]
unsafe impl Sync for crate::library::registry::Registry {}
#[cfg(feature = "load")]
unsafe impl Send for crate::library::registry::Provider {}
#[cfg(feature = "load")]
unsafe impl Sync for crate::library::registry::Provider {}

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

/// The descriptor's text points into the block's own strings, and the
/// services' `ctx` is the block itself, so nothing here borrows from the
/// registry.
#[cfg(feature = "load")]
pub(crate) fn leak_host_block(
    id: &str,
    version: &str,
    alloc: Option<Alloc>,
    state: alloc::sync::Weak<Shared>,
) -> &'static HostBlock {
    let id = String::from(id);
    let version = String::from(version);
    // Leaked as a raw pointer, and the two self-references written through
    // it: a `&mut` taken first and written through after `ctx` is derived
    // would invalidate `ctx` under Stacked Borrows.
    let block = Box::into_raw(Box::new(HostBlock {
        info: HostInfo {
            struct_size: size_of::<HostInfo>() as u32,
            abi_version: crate::library::desc::ABI_VERSION,
            // SAFETY: the strings' heap storage does not move when the
            // block does, and the block is leaked with them.
            host_id: unsafe { view_of_owned(&id) },
            host_version: unsafe { view_of_owned(&version) },
            alloc: alloc.map_or(core::ptr::null(), |a| a.as_raw()),
            meta: MaybeNull::null(),
            services: core::ptr::null(),
        },
        services: HostServices {
            struct_size: size_of::<HostServices>() as u32,
            ctx: core::ptr::null_mut(),
            get: Some(services_get),
            list: Some(services_list),
            alloc: Some(services_alloc),
            get_pinned: Some(services_get_pinned),
            list_pinned: Some(services_list_pinned),
            pin: Some(services_pin),
            unpin: Some(services_unpin),
        },
        id,
        version,
        state,
    }));
    // SAFETY: `block` is a live, never-freed allocation; both writes are
    // field projections through it, and the shared reference handed back
    // is the last thing derived, after every write.
    unsafe {
        core::ptr::addr_of_mut!((*block).services.ctx).write(block.cast::<c_void>());
        core::ptr::addr_of_mut!((*block).info.services)
            .write(core::ptr::addr_of!((*block).services));
        &*block
    }
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
    // SAFETY: forwarded.
    unsafe { get_one(ctx, key, out, false) }
}

/// The `get_pinned` slot: `get`, the answer pinned for the caller.
///
/// # Safety
///
/// As [`services_get`].
#[cfg(feature = "load")]
unsafe extern "C" fn services_get_pinned(
    ctx: *mut c_void,
    key: Str,
    out: *mut *const ProviderInfo,
) -> Status {
    // SAFETY: forwarded.
    unsafe { get_one(ctx, key, out, true) }
}

/// # Safety
///
/// As [`services_get`].
#[cfg(feature = "load")]
unsafe fn get_one(
    ctx: *mut c_void,
    key: Str,
    out: *mut *const ProviderInfo,
    pinned: bool,
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
        let Ok(key) = core::str::from_utf8(key.into()) else {
            return Status::GUATIAO_ERR_BAD_VALUE;
        };
        let snapshot = shared.read();
        match snapshot.by_key.get(key) {
            Some(&i) => {
                let desc = snapshot.desc(&snapshot.entries[i]);
                if pinned {
                    // SAFETY: the snapshot holds this descriptor's lease
                    // while it is read.
                    unsafe { super::lease::pin(desc) };
                }
                // SAFETY: `out` is writable by the caller's contract.
                unsafe { out.write(desc) };
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
    // SAFETY: forwarded.
    unsafe { list_all(ctx, kind, out, cap, total, false) }
}

/// The `list_pinned` slot: `list`, each answer written pinned.
///
/// # Safety
///
/// As [`services_list`].
#[cfg(feature = "load")]
unsafe extern "C" fn services_list_pinned(
    ctx: *mut c_void,
    kind: Str,
    out: *mut *const ProviderInfo,
    cap: usize,
    total: *mut usize,
) -> Status {
    // SAFETY: forwarded.
    unsafe { list_all(ctx, kind, out, cap, total, true) }
}

/// # Safety
///
/// As [`services_list`].
#[cfg(feature = "load")]
unsafe fn list_all(
    ctx: *mut c_void,
    kind: Str,
    out: *mut *const ProviderInfo,
    cap: usize,
    total: *mut usize,
    pinned: bool,
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
        let Ok(kind) = core::str::from_utf8(kind.into()) else {
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
                let desc = snapshot.desc(entry);
                if pinned {
                    // SAFETY: the snapshot holds this descriptor's lease
                    // while it is read.
                    unsafe { super::lease::pin(desc) };
                }
                // SAFETY: `n < cap` and `out` addresses `cap` slots.
                unsafe { out.add(n).write(desc) };
            }
            n += 1;
        }
        // SAFETY: `total` is writable by the caller's contract.
        unsafe { total.write(n) };
        Status::GUATIAO_OK
    })
}

/// The `pin` slot: another hold on a pinned descriptor, whether or not
/// the registry is still there: a pin outlives it.
///
/// # Safety
///
/// `desc` came pinned from this block's `get_pinned` or `list_pinned`,
/// and the caller holds a pin on it.
#[cfg(feature = "load")]
unsafe extern "C" fn services_pin(_ctx: *mut c_void, desc: *const ProviderInfo) {
    crate::exports::guard_with((), || {
        if !desc.is_null() {
            // SAFETY: the caller's contract: its own pin holds the lease.
            unsafe { super::lease::pin(desc.cast()) };
        }
    });
}

/// The `unpin` slot: gives back one hold.
///
/// # Safety
///
/// As [`services_pin`]; the hold given back is the caller's.
#[cfg(feature = "load")]
unsafe extern "C" fn services_unpin(_ctx: *mut c_void, desc: *const ProviderInfo) {
    crate::exports::guard_with((), || {
        if !desc.is_null() {
            // SAFETY: the caller's contract.
            unsafe { super::lease::unpin(desc.cast()) };
        }
    });
}

/// The `alloc` slot: the host's allocator, or null.
///
/// # Safety
///
/// `ctx` is a leaked block's.
#[cfg(feature = "load")]
unsafe extern "C" fn services_alloc(ctx: *mut c_void) -> *const crate::value::alloc::Allocator {
    crate::exports::guard_with(core::ptr::null(), || {
        // SAFETY: the caller's contract.
        match unsafe { block_of(ctx) } {
            Some(block) => block.info.alloc,
            None => core::ptr::null(),
        }
    })
}

/// Reads a host descriptor: `None` for null, or for one declaring a
/// `struct_size` below this build's [`HostInfo`].
///
/// # Safety
///
/// `raw` is null, or points at `raw->struct_size` readable, aligned bytes
/// that stay valid for the call. Nothing past this build's fields is read.
pub unsafe fn read_host(raw: *const HostInfo) -> Option<HostInfo> {
    if raw.is_null() || !addressable(raw) {
        return None;
    }
    // The leading `u32` only, through the raw pointer. Forming `&*raw`
    // first would assert the whole struct is dereferenceable, which is
    // exactly what a caller with a smaller one does not have.
    // SAFETY: the caller guarantees the first four bytes are readable.
    let declared = unsafe { core::ptr::addr_of!((*raw).struct_size).read() } as usize;
    if declared < HostInfo::floor() {
        return None;
    }

    // SAFETY: every read below is a field projection through the raw
    // pointer, and each field lies within the floor just checked.
    let info = unsafe {
        HostInfo {
            struct_size: declared as u32,
            abi_version: core::ptr::addr_of!((*raw).abi_version).read(),
            host_id: core::ptr::addr_of!((*raw).host_id).read(),
            host_version: core::ptr::addr_of!((*raw).host_version).read(),
            alloc: core::ptr::addr_of!((*raw).alloc).read(),
            meta: core::ptr::addr_of!((*raw).meta).read(),
            services: core::ptr::addr_of!((*raw).services).read(),
        }
    };
    Some(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::raw::test_support::*;

    /// A host one byte short of this build's struct is refused, by both
    /// readers, with nothing past the leading word read.
    #[test]
    fn a_host_short_of_this_builds_struct_is_refused() {
        for declared in [
            0usize,
            4,
            HostInfo::floor() - size_of::<usize>(),
            HostInfo::floor() - 1,
        ] {
            let value = a_host(declared);
            let (_buf, ptr) = short_of(&value, size_of::<HostInfo>());
            // SAFETY: `_buf` owns the bytes and outlives the calls.
            assert!(
                unsafe { read_host(ptr) }.is_none(),
                "{declared} is below the floor of {}",
                HostInfo::floor()
            );
            // SAFETY: as above.
            assert!(unsafe { Host::from_raw(ptr) }.is_none());
        }
        // SAFETY: null is the other case.
        assert!(unsafe { read_host(core::ptr::null()) }.is_none());
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
