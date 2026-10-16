// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Reading a library's image: opening it, and copying what its
//! descriptors say out into owned views, under their declared sizes.

use super::check::{Bounds, MAX_KINDS, MAX_PROVIDERS, MAX_TABLE, MAX_TABLES, Refusal};
use super::*;

/// One provider, read out of a descriptor once so nothing downstream has
/// to hold a raw pointer to read a name.
///
/// **Every string and every value here is OWNED**, copied out of the
/// library's image at the read, and its tables are copies its lease owns.
/// What still addresses the image — the code its slots name and its
/// `ctx` — is kept mapped by that lease: a view holds one, so the
/// registry neither unloads nor closes the library while it lives.
///
/// **Built only by reading a descriptor**, so its fields are private: a
/// safe [`available`](ProviderView::available) calls a slot with `ctx`,
/// and a view whose `ctx` safe code could set would let it call that slot
/// with anything.
#[derive(Debug, Clone)]
pub struct ProviderView {
    pub(crate) kinds: Vec<String>,
    pub(crate) id: String,
    pub(crate) display_name: String,
    pub(crate) config: Option<Value>,
    pub(crate) vtable: *const c_void,
    pub(crate) vtable_size: usize,
    pub(crate) ctx: *mut c_void,
    pub(crate) meta: Option<Map>,
    pub(crate) version: Option<String>,
    pub(crate) available: Option<unsafe extern "C" fn(ctx: *mut c_void, reason: *mut Str) -> bool>,
    pub(crate) raw: *const ProviderInfo<'static>,
    pub(crate) tables: Vec<(String, *const c_void, usize)>,
    pub(crate) create: Option<CreateFn>,
    pub(crate) destroy: Option<DestroyFn>,
    pub(crate) hold: Hold,
}

/// [`ProviderInfo::create`]'s slot.
pub type CreateFn = unsafe extern "C" fn(
    ctx: *mut c_void,
    config: *const Value,
    out: *mut *mut c_void,
    err: *mut crate::library::kind::ProviderError,
) -> Status;

/// [`ProviderInfo::destroy`]'s slot.
pub type DestroyFn = unsafe extern "C" fn(ctx: *mut c_void, instance: *mut c_void);

impl ProviderView {
    /// Every kind it serves. May be empty.
    pub fn kinds(&self) -> &[String] {
        &self.kinds
    }

    /// Its identifier, unique across every provider a host loads.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// A name to show a person, possibly empty.
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Its configuration schema, or `None`. A copy in this process's own
    /// heap, not the library's.
    pub fn config(&self) -> Option<&Value> {
        self.config.as_ref()
    }

    /// Whatever else the provider declared, or `None`. A copy, as
    /// [`config`](ProviderView::config) is. See [`ProviderInfo::meta`].
    pub fn meta(&self) -> Option<&Map> {
        self.meta.as_ref()
    }

    /// The version it declared for itself, or `None` to inherit its
    /// library's. See [`ProviderInfo::version`].
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    /// The shared function table, whose shape the kind defines.
    pub fn vtable(&self) -> *const c_void {
        self.vtable
    }

    /// The size the library compiled [`vtable`](ProviderView::vtable) at.
    pub fn vtable_size(&self) -> usize {
        self.vtable_size
    }

    /// Handed back to every call through a table, for a caller that
    /// speaks C. Null for a provider built only from a configuration.
    pub fn ctx(&self) -> *mut c_void {
        self.ctx
    }

    /// One table per kind, as `(kind, vtable, vtable_size)`, for a
    /// provider serving several kinds with a table each. Empty when
    /// `vtable` serves them all. See [`ProviderInfo::tables`].
    pub fn tables(&self) -> &[(String, *const c_void, usize)] {
        &self.tables
    }

    /// The descriptor this was read from, in the library's image, valid
    /// until that library is unloaded. What a host's services hand a
    /// library that asks.
    pub fn raw(&self) -> *const ProviderInfo<'static> {
        self.raw
    }

    /// Builds an instance from a configuration, or `None`. See
    /// [`ProviderInfo::create`].
    pub fn create(&self) -> Option<CreateFn> {
        self.create
    }

    /// Releases an instance `create` built. See [`ProviderInfo::destroy`].
    pub fn destroy(&self) -> Option<DestroyFn> {
        self.destroy
    }

    /// Whether it builds instances from a configuration.
    pub fn builds_instances(&self) -> bool {
        self.create.is_some()
    }

    /// Whether its table can be called as it stands: `false` for a
    /// provider that builds instances and keeps no default one (`create`
    /// present, `ctx` null). A provider with no `create` is called with
    /// whatever `ctx` it declared, null included, as a stateless C table
    /// expects.
    pub(crate) fn has_default_instance(&self) -> bool {
        !(self.ctx.is_null() && self.create.is_some())
    }
}

// SAFETY: the pointers address a loaded library's image, which this side
// never writes, and a provider's slots are callable from any thread.
unsafe impl Send for ProviderView {}
// SAFETY: as above.
unsafe impl Sync for ProviderView {}

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
        // SAFETY: the slot and `ctx` were read together from one
        // descriptor, and no safe code can change either; `reason` is a
        // writable local; the library stays mapped until the host's
        // `unsafe` unload, whose contract is that this view is not used.
        if unsafe { ask(self.ctx, &mut reason) } {
            return Ok(());
        }
        // SAFETY: the contract on the field is that a written reason
        // outlives every reader.
        Err(core::str::from_utf8(reason.into()).ok().unwrap_or(""))
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
    /// A check where it entered refused it, naming the part and the
    /// fault: a size below the floor, text that is not UTF-8, a pointer
    /// that is null with a length, misaligned or no address at all, an
    /// array past its cap, a code pointer outside the image's code.
    Malformed(Refusal),
    /// The library speaks an envelope version this build does not.
    /// Appending a field never changes the version; only a change no
    /// `struct_size` guard can express does, and such a change makes
    /// every field after it mean something else.
    UnsupportedAbi {
        /// What the library declared.
        declared: u32,
    },
}

impl From<Refusal> for Rejected {
    fn from(why: Refusal) -> Rejected {
        Rejected::Malformed(why)
    }
}

/// Text a descriptor names, checked and copied out.
///
/// # Safety
///
/// `view` was read out of a descriptor: its producer's contract is that a
/// pointer passing `bounds` addresses `len` readable bytes.
unsafe fn text(view: Str, bounds: Bounds, what: &'static str) -> Result<String, Refusal> {
    bounds
        .text(view.ptr() as usize, view.len())
        .map_err(|why| Refusal::new(what, why))?;
    core::str::from_utf8(view.into())
        .map(str::to_string)
        .map_err(|_| Refusal::new(what, "is not UTF-8"))
}

/// A map or a value a descriptor points at, checked and copied out.
///
/// # Safety
///
/// As [`text`]: a non-null pointer passing `bounds` addresses a
/// well-formed `T`.
unsafe fn tree<T>(
    ptr: *const T,
    bounds: Bounds,
    what: &'static str,
    copy: impl FnOnce(&T) -> Result<T, crate::value::ValueError>,
) -> Result<Option<T>, Refusal> {
    if ptr.is_null() {
        return Ok(None);
    }
    bounds
        .data(ptr as usize, size_of::<T>(), align_of::<T>())
        .map_err(|why| Refusal::new(what, why))?;
    // SAFETY: the caller's contract, and the checks above.
    copy(unsafe { &*ptr })
        .map(Some)
        .map_err(|_| Refusal::new(what, "could not be copied"))
}

/// A slot a descriptor names: a function pointer, checked as code.
fn slot<F: Copy>(
    f: Option<F>,
    as_addr: impl Fn(F) -> usize,
    bounds: Bounds,
    what: &'static str,
) -> Result<Option<F>, Refusal> {
    if let Some(f) = f {
        bounds
            .code(as_addr(f))
            .map_err(|why| Refusal::new(what, why))?;
    }
    Ok(f)
}

/// Reads a library descriptor and every provider in it, under the
/// checks every descriptor gets and, for a mapped library, its image's
/// bounds.
///
/// # Safety
///
/// `raw` came from a library's entry point or a host's registration, and
/// its producer's contract is that it stays valid while it is read and
/// that every pointer passing the checks addresses what its field says.
pub(crate) unsafe fn read_library(
    raw: *const LibraryInfo<'static>,
    bounds: Bounds,
) -> Result<LibraryView, Rejected> {
    const WHAT: &str = "the library descriptor";
    if raw.is_null() {
        return Err(Refusal::new(WHAT, "is null").into());
    }
    bounds
        .data(raw as usize, size_of::<u32>(), align_of::<LibraryInfo>())
        .map_err(|why| Refusal::new(WHAT, why))?;
    // SAFETY: the leading word passed the checks and is readable by the
    // producer's contract.
    let declared = unsafe { core::ptr::addr_of!((*raw).struct_size).read() } as usize;
    if declared < LibraryInfo::floor() {
        return Err(
            Refusal::new(WHAT, "is smaller than the oldest layout a reader accepts").into(),
        );
    }
    bounds
        .data(raw as usize, declared, align_of::<LibraryInfo>())
        .map_err(|why| Refusal::new(WHAT, why))?;

    // SAFETY: `abi_version` lies within the floor.
    let abi = unsafe { core::ptr::addr_of!((*raw).abi_version).read() };
    if abi != crate::library::desc::ABI_VERSION {
        return Err(Rejected::UnsupportedAbi { declared: abi });
    }

    // SAFETY: each field lies within `declared` bytes; the text is read
    // under the checks.
    let (id, version, providers) = unsafe {
        (
            text(
                core::ptr::addr_of!((*raw).id).read(),
                bounds,
                "the library's id",
            )?,
            text(
                core::ptr::addr_of!((*raw).version).read(),
                bounds,
                "the library's version",
            )?,
            core::ptr::addr_of!((*raw).providers).read(),
        )
    };

    let mut meta = None;
    if declared >= LibraryInfo::meta_end() {
        // SAFETY: the guard established the field is present, and a
        // non-null `meta` is a well-formed map by the contract on it.
        meta = unsafe {
            tree(
                core::ptr::addr_of!((*raw).meta).read().as_ptr(),
                bounds,
                "the library's meta",
                |m| m.clone_in(Alloc::rust()),
            )
        }?;
    }

    let mut unload = None;
    if declared >= LibraryInfo::unload_end() {
        // SAFETY: the guard established the field is present.
        let found = unsafe { core::ptr::addr_of!((*raw).unload).read() };
        unload = slot(found, |f| f as usize, bounds, "the library's `unload` slot")?;
    }

    // `len` is the library's number: it is capped, bounds the walk and is
    // never a capacity, so a corrupt one costs no allocation before an
    // element is read.
    let stride = providers.stride;
    if providers.len > 0 && stride < ProviderInfo::floor() {
        return Err(Refusal::new(
            "the provider array",
            "declares a stride below the oldest provider layout",
        )
        .into());
    }
    bounds
        .array(
            providers.ptr as usize,
            providers.len,
            stride,
            align_of::<ProviderInfo>(),
            MAX_PROVIDERS,
        )
        .map_err(|why| Refusal::new("the provider array", why))?;
    let mut out = Vec::new();
    for i in 0..providers.len {
        // SAFETY: the library declared `len` descriptors of `stride` bytes
        // at `ptr`, the array passed the checks, and byte arithmetic is
        // what reaches element `i` at the library's own layout.
        let entry = unsafe { providers.ptr.cast::<u8>().add(i * stride) }.cast::<ProviderInfo>();
        // SAFETY: forwarded.
        out.push(unsafe { read_provider(entry, stride, bounds) }?);
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
    /// `None` when a check where it enters refuses it: below the floor,
    /// text that is not UTF-8, a pointer that is misaligned or no address
    /// at all. A host's services answer with views already read
    /// ([`Host::get`], [`Host::list`]); this is for a descriptor a library
    /// built itself. Its fields are its producer's contract, as every
    /// descriptor's are.
    pub fn view(&'static self) -> Option<ProviderView> {
        self.read().ok()
    }

    /// [`view`](ProviderInfo::view), saying why not.
    pub(crate) fn read(&'static self) -> Result<ProviderView, Refusal> {
        // SAFETY: a descriptor's memory shape is its producer's contract,
        // and `'static` keeps it for every view taken from it; the
        // declared size is the limit, and the tables it names passed the
        // read's checks.
        unsafe { read_provider(self, self.struct_size as usize, Bounds::ANY) }.map(Lease::alone)
    }
}

/// `limit` is how many bytes this descriptor may claim: the array's
/// stride, or its own size when it stands alone.
///
/// # Safety
///
/// `raw` points at a provider descriptor declaring its own size, whose
/// leading word is readable, and whose producer's contract is that every
/// pointer passing the checks addresses what its field says.
pub(super) unsafe fn read_provider(
    raw: *const ProviderInfo<'static>,
    limit: usize,
    bounds: Bounds,
) -> Result<ProviderView, Refusal> {
    // SAFETY: the caller guarantees the leading word is readable.
    let declared = unsafe { core::ptr::addr_of!((*raw).struct_size).read() } as usize;
    // Above the floor, and not past its neighbour: an element claiming
    // more than the stride overlaps the next one, and every field it
    // reads beyond the stride belongs to that one.
    if declared < ProviderInfo::floor() || declared > limit {
        return Err(Refusal::new(
            "a provider descriptor",
            "declares a size below the oldest layout or past its array's stride",
        ));
    }
    // SAFETY: each field lies within `declared` bytes, and each pointer is
    // read through only after its checks.
    unsafe {
        let config = tree(
            core::ptr::addr_of!((*raw).config).read(),
            bounds,
            "a provider's configuration schema",
            |v| v.clone_in(Alloc::rust()),
        )?;
        // A fixed stride, unlike the provider array: a `Str` declares no
        // `struct_size`, so it has no way to grow and no skew to survive.
        let kinds = core::ptr::addr_of!((*raw).kinds).read();
        bounds
            .array(
                kinds.ptr as usize,
                kinds.len,
                size_of::<Str>(),
                align_of::<Str>(),
                MAX_KINDS,
            )
            .map_err(|why| Refusal::new("a provider's kind list", why))?;
        let mut names = Vec::new();
        for i in 0..kinds.len {
            names.push(text(
                kinds.ptr.add(i).read(),
                bounds,
                "a provider's kind name",
            )?);
        }

        let vtable = core::ptr::addr_of!((*raw).vtable).read();
        let vtable_size = core::ptr::addr_of!((*raw).vtable_size).read() as usize;
        if !vtable.is_null() {
            // Untyped: its shape is the kind's, so only where it lies is
            // checked here.
            if vtable_size > MAX_TABLE {
                return Err(Refusal::new(
                    "a provider's vtable",
                    "declares a size larger than a reader accepts",
                ));
            }
            bounds
                .data(vtable as usize, vtable_size, 1)
                .map_err(|why| Refusal::new("a provider's vtable", why))?;
        }

        Ok(ProviderView {
            kinds: names,
            id: text(
                core::ptr::addr_of!((*raw).id).read(),
                bounds,
                "a provider's id",
            )?,
            display_name: text(
                core::ptr::addr_of!((*raw).display_name).read(),
                bounds,
                "a provider's display name",
            )?,
            // Copied out of the image, so the schema outlives the library
            // it was read from.
            config,
            vtable,
            vtable_size,
            ctx: core::ptr::addr_of!((*raw).ctx).read(),
            meta: if declared >= ProviderInfo::meta_end() {
                // SAFETY: the guard established the field is present, and
                // a non-null `meta` is a well-formed map by the contract
                // on the field.
                tree(
                    core::ptr::addr_of!((*raw).meta).read().as_ptr(),
                    bounds,
                    "a provider's meta",
                    |m| m.clone_in(Alloc::rust()),
                )?
            } else {
                None
            },
            available: if declared >= ProviderInfo::available_end() {
                // SAFETY: the guard established the field is present.
                slot(
                    core::ptr::addr_of!((*raw).available).read(),
                    |f| f as usize,
                    bounds,
                    "a provider's `available` slot",
                )?
            } else {
                None
            },
            version: if declared >= ProviderInfo::version_end() {
                // SAFETY: the guard established the field is present.
                // Empty is how a provider says "my library's", which is a
                // different statement from a descriptor that predates the
                // field — and both land on `None` deliberately, because
                // both mean the same thing to a reader.
                Some(text(
                    core::ptr::addr_of!((*raw).version).read(),
                    bounds,
                    "a provider's version",
                )?)
                .filter(|v| !v.is_empty())
            } else {
                None
            },
            raw,
            tables: if declared >= ProviderInfo::tables_end() {
                // SAFETY: the guard established the field is present.
                read_kind_tables(core::ptr::addr_of!((*raw).tables).read(), bounds)?
            } else {
                Vec::new()
            },
            create: if declared >= ProviderInfo::create_end() {
                // SAFETY: the guard established the field is present.
                slot(
                    core::ptr::addr_of!((*raw).create).read(),
                    |f| f as usize,
                    bounds,
                    "a provider's `create` slot",
                )?
            } else {
                None
            },
            destroy: if declared >= ProviderInfo::destroy_end() {
                // SAFETY: the guard established the field is present.
                slot(
                    core::ptr::addr_of!((*raw).destroy).read(),
                    |f| f as usize,
                    bounds,
                    "a provider's `destroy` slot",
                )?
            } else {
                None
            },
            hold: Hold::None,
        })
    }
}

/// Walks a provider's per-kind tables at the library's stride, refusing
/// the array the way [`read_library`] refuses a provider array.
///
/// # Safety
///
/// `tables` came from a descriptor whose producer's contract is that a
/// pointer passing the checks addresses what its field says.
unsafe fn read_kind_tables(
    tables: crate::library::desc::KindTables,
    bounds: Bounds,
) -> Result<Vec<(String, *const c_void, usize)>, Refusal> {
    const WHAT: &str = "a provider's table array";
    if tables.len == 0 {
        return Ok(Vec::new());
    }
    if tables.stride < KindTable::floor() {
        return Err(Refusal::new(
            WHAT,
            "declares a stride below the oldest table layout",
        ));
    }
    bounds
        .array(
            tables.ptr as usize,
            tables.len,
            tables.stride,
            align_of::<KindTable>(),
            MAX_TABLES,
        )
        .map_err(|why| Refusal::new(WHAT, why))?;
    let mut out = Vec::new();
    for i in 0..tables.len {
        // SAFETY: the library declared `len` tables of `stride` bytes at
        // `ptr`, and the array passed the checks.
        let entry = unsafe { tables.ptr.cast::<u8>().add(i * tables.stride) }.cast::<KindTable>();
        // SAFETY: the leading word is readable.
        let declared = unsafe { core::ptr::addr_of!((*entry).struct_size).read() } as usize;
        if declared < KindTable::floor() || declared > tables.stride {
            return Err(Refusal::new(
                "a per-kind table entry",
                "declares a size below the oldest layout or past its array's stride",
            ));
        }
        // SAFETY: each field lies within `declared` bytes.
        let (kind, vtable, size) = unsafe {
            (
                text(
                    core::ptr::addr_of!((*entry).kind).read(),
                    bounds,
                    "a per-kind table's kind",
                )?,
                core::ptr::addr_of!((*entry).vtable).read(),
                core::ptr::addr_of!((*entry).vtable_size).read() as usize,
            )
        };
        if !vtable.is_null() {
            // SAFETY: forwarded.
            unsafe { check_table(vtable, size, bounds) }?;
        }
        out.push((kind, vtable, size));
    }
    Ok(out)
}

/// A per-kind table: where it lies, and every slot after its header null
/// or code. A kind's table is its [`KindHeader`](crate::library::KindHeader)
/// followed by function pointers.
///
/// # Safety
///
/// A table passing the checks addresses `size` readable bytes, by the
/// producer's contract.
unsafe fn check_table(table: *const c_void, size: usize, bounds: Bounds) -> Result<(), Refusal> {
    const WHAT: &str = "a per-kind table";
    if size > MAX_TABLE {
        return Err(Refusal::new(
            WHAT,
            "declares a size larger than a reader accepts",
        ));
    }
    bounds
        .data(table as usize, size, align_of::<*const c_void>())
        .map_err(|why| Refusal::new(WHAT, why))?;
    let header_size = size_of::<crate::library::KindHeader>();
    if size < header_size {
        // Too short to be a table at all, which validation names.
        return Ok(());
    }
    // SAFETY: the header lies within `size` checked bytes.
    let header = unsafe { table.cast::<crate::library::KindHeader>().read_unaligned() };
    let declared = header.struct_size as usize;
    let size = if declared == 0 {
        size
    } else {
        size.min(declared)
    };
    let word = size_of::<usize>();
    let mut at = header_size;
    while at + word <= size {
        // SAFETY: the word lies within `size` checked bytes.
        let slot = unsafe {
            table
                .cast::<u8>()
                .add(at)
                .cast::<*const c_void>()
                .read_unaligned()
        };
        if !slot.is_null() {
            bounds
                .code(slot.addr())
                .map_err(|why| Refusal::new("a per-kind table's slot", why))?;
        }
        at += word;
    }
    Ok(())
}

/// Where a registered library's code lives: a mapping this registry
/// holds the handle to, or the host's own binary.
///
/// The handle is what `Registry::unload` closes; `Linked` is what it
/// refuses, because there is nothing to unmap. A linked library built as
/// owned parts carries them here as far as the registry, which hands them
/// to the library's lease.
///
/// **Dropping one never unmaps.** The handle is held in `ManuallyDrop`, so
/// a registry going out of scope, or a file skipped after its entry point
/// ran, leaves the image where every `'static` handle into it expects it;
/// [`close`](Origin::close) is the one path to the loader's close, taken
/// by `unload` and for a skipped file that is a second handle to an image
/// the registry already holds.
#[cfg(feature = "load")]
#[derive(Debug)]
pub(crate) enum Origin {
    /// A file this registry mapped and can close, and the loader's handle
    /// for its image: two opens of one image answer the same handle.
    Mapped {
        library: core::mem::ManuallyDrop<libloading::Library>,
        image: usize,
    },
    /// Code in the host's own binary: `register_local`, `register_entry`,
    /// and the descriptor's parts when the registry owns them.
    Linked(Option<Box<crate::library::kind::LibraryParts>>),
}

#[cfg(feature = "load")]
impl Origin {
    /// A mapping this registry holds.
    pub(crate) fn mapped(library: libloading::Library) -> Origin {
        let (library, image) = image_of(library);
        Origin::Mapped {
            library: core::mem::ManuallyDrop::new(library),
            image,
        }
    }

    /// Whether there is a mapping to close at all.
    pub(crate) fn is_mapped(&self) -> bool {
        matches!(self, Origin::Mapped { .. })
    }

    /// The parts of a library the host built, for its lease to own.
    pub(crate) fn take_parts(&mut self) -> Option<Box<crate::library::kind::LibraryParts>> {
        match self {
            Origin::Linked(parts) => parts.take(),
            Origin::Mapped { .. } => None,
        }
    }

    /// Whether both are handles to one image: a second open the loader
    /// counted against a mapping already held, as a hard link to a loaded
    /// file is on Linux.
    pub(crate) fn same_image(&self, other: &Origin) -> bool {
        matches!(
            (self, other),
            (Origin::Mapped { image: a, .. }, Origin::Mapped { image: b, .. }) if a == b
        )
    }

    /// Gives up the handle and leaves the mapping in place, so every
    /// image address the library already handed out stays valid. What
    /// dropping it does too; spelled out where that is the point.
    pub(crate) fn keep(self) {}

    /// Closes the mapping. A close error leaves the handle given up, as
    /// `libloading` does.
    pub(crate) fn close(self) -> Result<(), libloading::Error> {
        match self {
            Origin::Mapped { library, .. } => core::mem::ManuallyDrop::into_inner(library).close(),
            Origin::Linked(_) => Ok(()),
        }
    }
}

/// The library, and the loader's handle for its image as a number.
#[cfg(all(feature = "load", unix))]
fn image_of(library: libloading::Library) -> (libloading::Library, usize) {
    let raw = libloading::os::unix::Library::from(library).into_raw();
    // SAFETY: `raw` is the handle just given up, taken back exactly once.
    let library = unsafe { libloading::os::unix::Library::from_raw(raw) };
    (library.into(), raw as usize)
}

/// The library, and the loader's handle for its image as a number.
#[cfg(all(feature = "load", windows))]
fn image_of(library: libloading::Library) -> (libloading::Library, usize) {
    let raw = libloading::os::windows::Library::from(library).into_raw();
    // SAFETY: `raw` is the handle just given up, taken back exactly once.
    let library = unsafe { libloading::os::windows::Library::from_raw(raw) };
    (library.into(), raw as usize)
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
        core::mem::forget(library);
        return Ok(Opened::NoEntrySymbol);
    };
    // Where the image's code and data lie in this process, measured from
    // where its entry symbol landed. A file that cannot be described is
    // read under the checks that need no image.
    let entry_at = *entry as usize;
    let image = crate::library::scan::layout(path)
        .and_then(|l| super::check::Image::place(l.entry, l.span, &l.code, entry_at));

    // SAFETY: the library's side of the contract is that its entry point
    // reads a host descriptor and returns a descriptor or null. The
    // descriptor is a block the host keeps for the life of the process.
    let desc = unsafe { entry(host.as_raw()) };

    if desc.is_null() {
        core::mem::forget(library);
        return Ok(Opened::Declined);
    }
    // SAFETY: non-null, from the entry point, and the mapping is held for
    // the whole read.
    match unsafe { read_library(desc, Bounds::of(image.as_ref())) } {
        Ok(view) => Ok(Opened::Loaded(view, Origin::mapped(library))),
        Err(why) => {
            core::mem::forget(library);
            Ok(Opened::Rejected(why))
        }
    }
}

/// What a library the host LINKS came to: its describe function's answer,
/// read the way a loaded library's entry answer is read.
#[cfg(feature = "load")]
pub(crate) fn open_local(described: Option<LocalLibrary>) -> Opened {
    let Some(described) = described else {
        return Opened::Declined;
    };
    // SAFETY: registering a descriptor is the host's trust in its
    // producer, as naming a file is; its memory shape is that producer's
    // contract, and `read_library` checks what can be checked. Owned parts
    // are read before they move: the views copy what they name, and the
    // descriptor pointers they keep address the parts' heap storage,
    // which the move does not touch.
    let raw: *const LibraryInfo<'static> = (described.info() as *const LibraryInfo<'_>).cast();
    let read = unsafe { read_library(raw, Bounds::ANY) };
    let parts = match described {
        LocalLibrary::Owned(parts) => Some(parts),
        LocalLibrary::Static(_) => None,
    };
    match read {
        Ok(view) => Opened::Loaded(view, Origin::Linked(parts)),
        Err(why) => Opened::Rejected(why),
    }
}

/// What a library the host LINKS and that speaks C came to: its entry
/// point's answer, read exactly as a loaded library's is.
#[cfg(feature = "load")]
pub(crate) fn open_entry(entry: EntryFn, host: Host) -> Opened {
    // SAFETY: handing over an entry point is choosing to run it, as naming
    // a file is; its answer is null or a descriptor read under
    // `read_library`'s own checks.
    let desc = unsafe { entry(host.as_raw()) };
    if desc.is_null() {
        return Opened::Declined;
    }
    // SAFETY: non-null, from an entry point that keeps it for the process.
    match unsafe { read_library(desc, Bounds::ANY) } {
        Ok(view) => Opened::Loaded(view, Origin::Linked(None)),
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
    /// [`Offer`](crate::library::kind::Offer) copied out of it, every
    /// [`ProviderView`] of one of its providers (from
    /// [`Provider::view`](crate::library::Provider::view),
    /// [`Offer::view`](crate::library::kind::Offer::view) or a
    /// [`Host`]'s `get` and `list`, whose `available()` calls into the
    /// image), every raw vtable, `ctx` or descriptor pointer, every value
    /// it built through its own allocator that its slot does not track,
    /// and every descriptor pointer another library fetched from it
    /// through the host's services, is not used again.
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
            core::ptr::null()
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
        assert!(unsafe { read_library(ptr, Bounds::ANY) }.is_err());

        let value = a_library(LibraryInfo::floor());
        let (_buf, ptr) = short_of(&value, LibraryInfo::floor());
        // SAFETY: as above.
        let view =
            unsafe { read_library(ptr, Bounds::ANY) }.expect("a floor-sized descriptor is usable");
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
        let view = unsafe { read_library(ptr, Bounds::ANY) }.expect("above the floor");
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
        let view = unsafe { read_library(ptr, Bounds::ANY) }.expect("a full descriptor");
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
            unsafe { read_library(ptr, Bounds::ANY) }.unwrap_err(),
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
            core::ptr::write_bytes(base, TAIL, words * size_of::<u64>());
            core::ptr::copy_nonoverlapping(
                (&first as *const ProviderInfo).cast::<u8>(),
                base,
                stride,
            );
            core::ptr::copy_nonoverlapping(
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
        let view = unsafe { read_library(ptr, Bounds::ANY) }.expect("both providers are readable");

        assert_eq!(view.providers.len(), 2);
        assert_eq!(view.providers[0].id, "hello");
        assert_eq!(view.providers[1].id, "second");
    }

    /// A length is a bound to walk, never a capacity: one past its cap is
    /// refused before any element is read, and nothing is reserved for
    /// it.
    #[test]
    fn a_huge_length_is_walked_and_never_reserved_for() {
        let stride = size_of::<ProviderInfo>();
        let mut below = a_provider(stride, 0);
        below.struct_size = 0;
        let array = [a_provider(stride, 0), below];
        let mut value = a_library(size_of::<LibraryInfo>());
        value.providers = Providers {
            ptr: array.as_ptr(),
            len: isize::MAX as usize / stride,
            stride,
        };
        let (_buf, ptr) = short_of(&value, size_of::<LibraryInfo>());
        // SAFETY: `_buf` owns the descriptor and `array` both elements the
        // walk reaches; it stops at the second.
        assert!(matches!(
            unsafe { read_library(ptr, Bounds::ANY) }.unwrap_err(),
            Rejected::Malformed(_)
        ));

        let mut value = a_provider(stride, 0);
        value.kinds = Kinds {
            ptr: KINDS.0.as_ptr(),
            len: usize::MAX / 2,
        };
        let (_buf, ptr) = short_of(&value, stride);
        // SAFETY: `_buf` owns the bytes; no name is read.
        assert!(unsafe { read_provider(ptr, stride, Bounds::ANY) }.is_err());

        let mut short = KindTable::new("greeter", core::ptr::null(), 0);
        short.struct_size = 0;
        let tables = [KindTable::new("greeter", core::ptr::null(), 0), short];
        let mut value = a_provider(stride, 0);
        value.tables = crate::library::desc::KindTables {
            ptr: tables.as_ptr(),
            len: isize::MAX as usize / size_of::<KindTable>(),
            stride: size_of::<KindTable>(),
        };
        let (_buf, ptr) = short_of(&value, stride);
        // SAFETY: `_buf` owns the descriptor and `tables` both tables the
        // walk reaches.
        assert!(unsafe { read_provider(ptr, stride, Bounds::ANY) }.is_err());
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
        assert!(unsafe { read_library(ptr, Bounds::ANY) }.is_err());
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
            config: core::ptr::null(),
            vtable: core::ptr::null(),
            ctx: core::ptr::null_mut(),
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
        let view = unsafe { read_provider(ptr, ProviderInfo::tables_end(), Bounds::ANY) }
            .expect("above the floor");
        assert!(view.create.is_none() && view.destroy.is_none());

        let mut value = a_provider(size_of::<ProviderInfo>(), 0);
        value.create = Some(build);
        value.destroy = Some(drop_it);
        let (_buf, ptr) = short_of(&value, size_of::<ProviderInfo>());
        // SAFETY: as above.
        let view = unsafe { read_provider(ptr, size_of::<ProviderInfo>(), Bounds::ANY) }
            .expect("a full descriptor");
        assert!(view.create.is_some() && view.destroy.is_some());
    }

    /// A provider with a table per kind hands each back by kind, and the
    /// shared `vtable` answers for the kinds without one.
    #[test]
    fn a_provider_with_a_table_per_kind_hands_each_back_by_kind() {
        use crate::library::desc::KindTables;

        // Zeroed tables: a header and null slots, which the reader walks.
        static GREETER: [usize; 4] = [0; 4];
        static COUNTER: [usize; 4] = [0; 4];
        static SHARED: [usize; 4] = [0; 4];
        let (greeter, counter, shared) = (
            GREETER.as_ptr().cast::<c_void>(),
            COUNTER.as_ptr().cast::<c_void>(),
            SHARED.as_ptr().cast::<c_void>(),
        );
        struct Tables([KindTable<'static>; 2]);
        // SAFETY: a constant never written, whose pointers address the
        // zeroed statics above.
        unsafe impl Sync for Tables {}
        static TABLES: Tables = Tables([
            KindTable::new("greeter", GREETER.as_ptr().cast(), 16),
            KindTable::new("counter", COUNTER.as_ptr().cast(), 24),
        ]);

        let mut value = a_provider(size_of::<ProviderInfo>(), 8);
        value.vtable = shared;
        value.tables = KindTables::new(&TABLES.0);
        let (_buf, ptr) = short_of(&value, size_of::<ProviderInfo>());
        // SAFETY: `_buf` owns the descriptor; `TABLES` is static.
        let view = unsafe { read_provider(ptr, size_of::<ProviderInfo>(), Bounds::ANY) }
            .expect("a full descriptor");

        assert_eq!(view.table_for("greeter"), Some((greeter, 16)));
        assert_eq!(view.table_for("counter"), Some((counter, 24)));
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
        older.vtable = shared;
        let (_buf2, ptr2) = short_of(&older, ProviderInfo::available_end());
        let _ = ptr;
        // SAFETY: as above.
        let view = unsafe { read_provider(ptr2, ProviderInfo::available_end(), Bounds::ANY) }
            .expect("above the floor");
        assert!(view.tables.is_empty());
        assert_eq!(view.table_for("greeter"), Some((shared, 8)));

        // A stride below the floor refuses the descriptor.
        let mut bad = value;
        bad.tables.stride = KindTable::floor() - 1;
        let (_buf3, ptr3) = short_of(&bad, size_of::<ProviderInfo>());
        // SAFETY: as above.
        assert!(unsafe { read_provider(ptr3, size_of::<ProviderInfo>(), Bounds::ANY) }.is_err());
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
        let view = unsafe { read_provider(ptr, ProviderInfo::version_end(), Bounds::ANY) }
            .expect("above the floor");
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
        let view = unsafe { read_provider(ptr, size_of::<ProviderInfo>(), Bounds::ANY) }
            .expect("a full descriptor");
        assert_eq!(view.available(), Err("no calendar here"));
    }

    #[test]
    fn a_provider_descriptor_below_the_floor_is_refused() {
        let value = a_provider(ProviderInfo::floor() - 1, 0);
        let (_buf, ptr) = short_of(&value, size_of::<ProviderInfo>());
        // SAFETY: `_buf` owns the bytes.
        assert!(unsafe { read_provider(ptr, size_of::<ProviderInfo>(), Bounds::ANY) }.is_err());
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
        let view = unsafe { read_provider(ptr, ProviderInfo::floor(), Bounds::ANY) }
            .expect("a floor-sized provider");
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
        let view = unsafe { read_provider(ptr, size_of::<ProviderInfo>(), Bounds::ANY) }
            .expect("a full descriptor");

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

    /// A misaligned provider array is refused by name, before an element
    /// is read.
    #[test]
    fn a_misaligned_provider_array_is_refused_by_name() {
        let array = [a_provider(size_of::<ProviderInfo>(), 0); 2];
        let mut value = a_library(size_of::<LibraryInfo>());
        value.providers = Providers {
            // SAFETY: one byte into `array`, never read through.
            ptr: unsafe { array.as_ptr().cast::<u8>().add(1) }.cast(),
            len: 1,
            stride: size_of::<ProviderInfo>(),
        };
        let (_buf, ptr) = short_of(&value, size_of::<LibraryInfo>());
        // SAFETY: `_buf` owns the descriptor; the array is never read.
        let Err(Rejected::Malformed(why)) = (unsafe { read_library(ptr, Bounds::ANY) }) else {
            panic!("a misaligned array is refused");
        };
        assert_eq!(why.what, "the provider array");
        assert!(why.to_string().contains("aligned"), "{why}");
    }

    /// A length past its cap is refused before anything is read, however
    /// little the pointer addresses.
    #[test]
    fn a_length_past_its_cap_is_refused_before_anything_is_read() {
        let one = a_provider(size_of::<ProviderInfo>(), 0);
        let mut value = a_library(size_of::<LibraryInfo>());
        value.providers = Providers {
            ptr: &one,
            len: super::super::check::MAX_PROVIDERS + 1,
            stride: size_of::<ProviderInfo>(),
        };
        let (_buf, ptr) = short_of(&value, size_of::<LibraryInfo>());
        // SAFETY: `_buf` owns the descriptor; no element is read.
        let Err(Rejected::Malformed(why)) = (unsafe { read_library(ptr, Bounds::ANY) }) else {
            panic!("a length past the cap is refused");
        };
        assert_eq!(why.what, "the provider array");
        assert!(why.to_string().contains("longer"), "{why}");
    }

    /// A pointer into the low 64 KiB, or one filled with a repeated byte,
    /// is no address: refused before it is read.
    #[test]
    fn a_wild_pointer_is_refused_before_it_is_read() {
        let mut wild = vec![0x1000usize];
        if cfg!(any(target_arch = "x86_64", target_arch = "aarch64")) {
            wild.push(0xAAAA_AAAA_AAAA_AAAAu64 as usize);
        }
        for addr in wild {
            let mut value = a_provider(size_of::<ProviderInfo>(), 0);
            value.config = addr as *const Value;
            let (_buf, ptr) = short_of(&value, size_of::<ProviderInfo>());
            // SAFETY: `_buf` owns the descriptor; `config` is never read.
            let why = unsafe { read_provider(ptr, size_of::<ProviderInfo>(), Bounds::ANY) }
                .expect_err("a wild pointer is refused");
            assert_eq!(why.what, "a provider's configuration schema", "{addr:#x}");
        }
    }

    /// Where the test binary finds `hello_library`'s artifact: beside it in
    /// `deps/`, where a dev-dependency's cdylib lands.
    #[cfg(feature = "load")]
    fn hello_library() -> std::path::PathBuf {
        let exe = std::env::current_exe().expect("a test binary knows its path");
        let deps = exe.parent().expect("it is in a directory");
        let name = format!(
            "{}hello_library{}",
            std::env::consts::DLL_PREFIX,
            std::env::consts::DLL_SUFFIX
        );
        [
            deps.to_path_buf(),
            deps.parent().map(Into::into).unwrap_or_default(),
        ]
        .into_iter()
        .map(|d| d.join(&name))
        .find(|p| p.exists())
        .expect("hello_library is built as a dev-dependency")
    }

    /// A real mapped image: its own layout places it, a code pointer into
    /// its data is refused, and one into its code is accepted.
    #[cfg(feature = "load")]
    #[cfg_attr(miri, ignore = "Miri cannot map a library")]
    #[test]
    fn a_code_pointer_into_a_loaded_images_data_is_refused() {
        use super::super::check::Image;

        let path = hello_library();
        // SAFETY: a fixture of this repository, mapped and never unmapped.
        let library = unsafe { libloading::Library::new(&path) }.expect("it maps");
        // SAFETY: the entry symbol has this crate's entry type; the data
        // symbol is a byte array, read here only as an address.
        let (entry_at, data_at) = unsafe {
            let entry = library
                .get::<EntryFn>(ENTRY_SYMBOL)
                .expect("an entry symbol");
            let declares = library
                .get::<*const u8>(DECLARES_SYMBOL)
                .expect("a declaration");
            (*entry as usize, *declares as usize)
        };
        let layout = crate::library::scan::layout(&path).expect("its file has a layout");
        let image = Image::place(layout.entry, layout.span, &layout.code, entry_at)
            .expect("its layout places at the mapping");
        let bounds = Bounds::of(Some(&image));
        core::mem::forget(library);

        type Available = unsafe extern "C" fn(*mut c_void, *mut Str) -> bool;
        // SAFETY: never called: only its address is read.
        let into_data: Available = unsafe { core::mem::transmute::<usize, Available>(data_at) };
        // SAFETY: as above.
        let into_code: Available = unsafe { core::mem::transmute::<usize, Available>(entry_at) };

        let mut value = a_provider(size_of::<ProviderInfo>(), 0);
        value.available = Some(into_data);
        let (_buf, ptr) = short_of(&value, size_of::<ProviderInfo>());
        // SAFETY: `_buf` owns the descriptor; the slot is never called.
        let why = unsafe { read_provider(ptr, size_of::<ProviderInfo>(), bounds) }
            .expect_err("a slot into data is refused");
        assert_eq!(why.what, "a provider's `available` slot");
        assert!(why.to_string().contains("executable"), "{why}");

        value.available = Some(into_code);
        let (_buf, ptr) = short_of(&value, size_of::<ProviderInfo>());
        // SAFETY: as above.
        unsafe { read_provider(ptr, size_of::<ProviderInfo>(), bounds) }
            .expect("a slot into its code is accepted");

        // A per-kind table whose slot points into the image's data.
        let table: [usize; 3] = [0, data_at, 0];
        let tables = [KindTable::new(
            "greeter",
            table.as_ptr().cast(),
            size_of_val(&table),
        )];
        let mut value = a_provider(size_of::<ProviderInfo>(), 0);
        value.tables = crate::library::desc::KindTables {
            ptr: tables.as_ptr(),
            len: 1,
            stride: size_of::<KindTable>(),
        };
        let (_buf, ptr) = short_of(&value, size_of::<ProviderInfo>());
        // SAFETY: `_buf` owns the descriptor and `table` the table.
        let why = unsafe { read_provider(ptr, size_of::<ProviderInfo>(), bounds) }
            .expect_err("a table slot into data is refused");
        assert_eq!(why.what, "a per-kind table's slot");
    }
}
