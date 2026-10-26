// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a handle holds on its library: the registry's own copies of the
//! library's tables, the descriptors it built from them, and a count.
//!
//! A [`Lease`] is one per registered library. Every handle that can reach
//! the library's code (`Remote`, `Offer`, `Instance`, `Object`,
//! `ProviderView`) holds a [`Hold`] on it, and so does each descriptor a
//! host's services hand out until it is unpinned; the registry counts
//! them. `unload` refuses while one is held, and `collect` closes a
//! retired library once none is. Dropping a lease frees memory and never
//! unmaps.

use alloc::sync::{Arc, Weak};

use super::*;

/// One run of table bytes: sixteen-aligned, so a copy is aligned for any
/// table a kind can declare.
#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct Chunk([u8; 16]);

/// One library's tables as this side copied them, a descriptor per
/// provider built over those copies, and the parts of a library the host
/// built.
pub(crate) struct Lease {
    /// Each table, copied byte for byte under the size its descriptor
    /// declared. Only read through the pointers handed out.
    #[allow(dead_code)]
    tables: Vec<Vec<Chunk>>,
    /// What a host's services answer for each provider: registry-owned
    /// text and values, the copied tables, the library's code and `ctx`.
    built: Vec<Built>,
    /// A library the host built, owned until the last handle goes.
    #[allow(dead_code)]
    parts: Option<Box<crate::library::kind::LibraryParts>>,
}

impl core::fmt::Debug for Lease {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Lease")
            .field("tables", &self.tables.len())
            .field("providers", &self.built.len())
            .field("owned", &self.parts.is_some())
            .finish()
    }
}

/// One provider's descriptor as the registry built it: `info` first, so
/// its address is the descriptor's, then the lease it belongs to and the
/// storage `info` points into.
///
/// The owners are `String` and `Vec`, never `Box`: moving a `Box`
/// retags it as unique and invalidates the pointers `info` holds into it.
#[repr(C)]
struct Built {
    info: ProviderInfo<'static>,
    /// What [`pin`] and [`unpin`] count: this lease, by the address its
    /// `Arc` hands out.
    lease: *const Lease,
    #[allow(dead_code)]
    id: String,
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    version: String,
    #[allow(dead_code)]
    kind_names: Vec<String>,
    #[allow(dead_code)]
    kinds: Vec<Str<'static>>,
    #[allow(dead_code)]
    table_kinds: Vec<String>,
    #[allow(dead_code)]
    tables: Vec<KindTable<'static>>,
    #[allow(dead_code)]
    config: Vec<Value>,
    #[allow(dead_code)]
    meta: Vec<Map>,
    #[allow(dead_code)]
    kind_descriptions: String,
}

// SAFETY: built once and never written; every pointer in `info` addresses
// this struct's own storage, the lease's copies, or the library's code and
// `ctx`, which a provider's contract makes callable from any thread.
unsafe impl Send for Built {}
// SAFETY: as above.
unsafe impl Sync for Built {}

impl Built {
    /// The descriptor for one adopted view: its tables already the
    /// lease's copies.
    fn of(view: &ProviderView, lease: *const Lease) -> Built {
        let id = view.id.clone();
        let name = view.display_name.clone();
        let version = view.version.clone().unwrap_or_default();
        let kind_names = view.kinds.clone();
        // SAFETY (each `view_of_owned`): the strings' heap storage does
        // not move when they move into the struct, which keeps them.
        let kinds: Vec<Str<'static>> = kind_names
            .iter()
            .map(|k| unsafe { view_of_owned(k) })
            .collect();
        let table_kinds: Vec<String> = view.tables.iter().map(|(k, _, _)| k.clone()).collect();
        let tables: Vec<KindTable<'static>> = view
            .tables
            .iter()
            .zip(&table_kinds)
            .map(|(&(_, table, size), kind)| KindTable {
                struct_size: size_of::<KindTable>() as u32,
                vtable_size: size as u32,
                // SAFETY: as above.
                kind: unsafe { view_of_owned(kind) },
                vtable: table,
            })
            .collect();
        let config: Vec<Value> = view.config.iter().cloned().collect();
        let meta: Vec<Map> = view.meta.iter().cloned().collect();
        let kind_descriptions = view.kind_descriptions.clone().unwrap_or_default();
        let info = ProviderInfo {
            struct_size: size_of::<ProviderInfo>() as u32,
            vtable_size: view.vtable_size as u32,
            kinds: crate::library::desc::Kinds {
                ptr: kinds.as_ptr(),
                len: kinds.len(),
            },
            // SAFETY: as above.
            id: unsafe { view_of_owned(&id) },
            // SAFETY: as above.
            display_name: unsafe { view_of_owned(&name) },
            config: config
                .first()
                .map_or(core::ptr::null(), |v| v as *const Value),
            vtable: view.vtable,
            ctx: view.ctx,
            meta: MaybeNull::from_ptr(meta.first().map_or(core::ptr::null(), |m| m as *const Map)),
            // SAFETY: as above.
            version: unsafe { view_of_owned(&version) },
            available: view.available,
            tables: crate::library::desc::KindTables {
                ptr: tables.as_ptr(),
                len: tables.len(),
                stride: size_of::<KindTable>(),
            },
            create: view.create,
            destroy: view.destroy,
            // SAFETY: as above.
            kind_descriptions: unsafe { view_of_owned(&kind_descriptions) },
        };
        Built {
            info,
            lease,
            id,
            name,
            version,
            kind_names,
            kinds,
            table_kinds,
            tables,
            config,
            meta,
            kind_descriptions,
        }
    }
}

/// `size` bytes at `table`, copied into storage the lease owns.
///
/// # Safety
///
/// `table` is null, or addresses `size` readable bytes.
unsafe fn copy_table(table: *const c_void, size: usize) -> Option<Vec<Chunk>> {
    if table.is_null() {
        return None;
    }
    // A `Vec`, never a `Box`: moving a `Box` retags it as unique, which
    // invalidates the table pointer taken from it before the move.
    let mut copy = vec![Chunk([0; 16]); size.div_ceil(size_of::<Chunk>())];
    // SAFETY: the caller's contract for the source; the copy holds at
    // least `size` bytes and is its own allocation.
    unsafe {
        core::ptr::copy_nonoverlapping(table.cast::<u8>(), copy.as_mut_ptr().cast::<u8>(), size)
    };
    Some(copy)
}

impl Lease {
    /// A lease over copies of every table `views` name, with each view's
    /// table pointers moved onto the copies and holding the lease, and a
    /// descriptor built for each view, in order, for a host's services.
    ///
    /// Safe because a view is built only by reading a descriptor, whose
    /// every table pointer passed the read's checks for the size it
    /// declares; what those checks cannot see is the producer's contract.
    pub(crate) fn adopt(
        views: &mut [ProviderView],
        parts: Option<Box<crate::library::kind::LibraryParts>>,
    ) -> Arc<Lease> {
        Lease::over(views, parts, true)
    }

    /// One view adopted on its own: what a descriptor read with no
    /// registry behind it holds. Builds no descriptor; nothing serves it.
    pub(crate) fn alone(mut view: ProviderView) -> ProviderView {
        Lease::over(core::slice::from_mut(&mut view), None, false);
        view
    }

    fn over(
        views: &mut [ProviderView],
        parts: Option<Box<crate::library::kind::LibraryParts>>,
        describe: bool,
    ) -> Arc<Lease> {
        let mut tables = Vec::new();
        let mut keep = |table: &mut *const c_void, size: usize| {
            // SAFETY: a view's table addresses the bytes its descriptor
            // declared, as above.
            if let Some(copy) = unsafe { copy_table(*table, size) } {
                *table = copy.as_ptr().cast::<c_void>();
                tables.push(copy);
            }
        };
        for view in views.iter_mut() {
            keep(&mut view.vtable, view.vtable_size);
            for (_, table, size) in &mut view.tables {
                keep(table, *size);
            }
        }
        let read: &[ProviderView] = views;
        let lease = Arc::new_cyclic(|me: &Weak<Lease>| {
            let me = me.as_ptr();
            let built = if describe {
                read.iter().map(|view| Built::of(view, me)).collect()
            } else {
                Vec::new()
            };
            Lease {
                tables,
                built,
                parts,
            }
        });
        for view in views.iter_mut() {
            view.hold = Hold::Lease(Arc::clone(&lease));
        }
        lease
    }

    /// The descriptor built for the provider at `at`, for a host's
    /// services to hand out. Valid while this lease is held.
    pub(crate) fn desc(&self, at: usize) -> *const ProviderInfo<'static> {
        // From the whole `Built`, so `pin` and `unpin` may read past
        // `info` through it.
        core::ptr::from_ref(&self.built[at]).cast::<ProviderInfo<'static>>()
    }
}

/// Takes another hold on the lease a descriptor [`Lease::desc`] handed
/// out belongs to.
///
/// # Safety
///
/// `desc` came from [`Lease::desc`], and its lease is held while this
/// runs: by the caller's own pin, or by the snapshot it was read from.
pub(crate) unsafe fn pin(desc: *const ProviderInfo<'static>) {
    // SAFETY: the caller's contract; `info` is the first field of a
    // `Built`, whose `lease` is the address its `Arc` hands out.
    unsafe {
        let lease = core::ptr::addr_of!((*desc.cast::<Built>()).lease).read();
        Arc::increment_strong_count(lease);
    }
}

/// Lets go of one hold [`pin`] took.
///
/// # Safety
///
/// As [`pin`], and the hold is the caller's to give up; `desc` is not
/// read again unless another hold is still held.
pub(crate) unsafe fn unpin(desc: *const ProviderInfo<'static>) {
    // SAFETY: the caller's contract.
    unsafe {
        let lease = core::ptr::addr_of!((*desc.cast::<Built>()).lease).read();
        Arc::decrement_strong_count(lease);
    }
}

/// A hold on another library's descriptor, taken through a host's
/// services: what a view a library read with `Host::get` or `list`
/// holds. Cloning it pins again; dropping it unpins.
pub(crate) struct Pin {
    ctx: *mut c_void,
    pin: unsafe extern "C" fn(ctx: *mut c_void, desc: *const ProviderInfo),
    unpin: unsafe extern "C" fn(ctx: *mut c_void, desc: *const ProviderInfo),
    desc: *const ProviderInfo<'static>,
}

// SAFETY: the two slots are the host's, callable from any thread by the
// services' contract, and `desc` is only passed back to them.
unsafe impl Send for Pin {}
// SAFETY: as above.
unsafe impl Sync for Pin {}

impl Pin {
    /// A hold the host already took for this caller.
    pub(crate) fn taken(
        ctx: *mut c_void,
        pin: unsafe extern "C" fn(ctx: *mut c_void, desc: *const ProviderInfo),
        unpin: unsafe extern "C" fn(ctx: *mut c_void, desc: *const ProviderInfo),
        desc: *const ProviderInfo<'static>,
    ) -> Pin {
        Pin {
            ctx,
            pin,
            unpin,
            desc,
        }
    }
}

impl core::fmt::Debug for Pin {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Pin").finish_non_exhaustive()
    }
}

impl Clone for Pin {
    fn clone(&self) -> Pin {
        // SAFETY: this holds a pin on `desc`, which the slot counts again.
        unsafe { (self.pin)(self.ctx, self.desc) };
        Pin {
            ctx: self.ctx,
            pin: self.pin,
            unpin: self.unpin,
            desc: self.desc,
        }
    }
}

impl Drop for Pin {
    fn drop(&mut self) {
        // SAFETY: the pin this holds, given back once.
        unsafe { (self.unpin)(self.ctx, self.desc) };
    }
}

/// What one handle holds on the library it reaches.
///
/// `None` for a handle with no library behind it: a table from
/// [`Remote::from_raw`](crate::library::Remote::from_raw), a lent object,
/// an object the host built, or a view a host answered unpinned.
#[derive(Debug, Clone, Default)]
pub(crate) enum Hold {
    #[default]
    None,
    /// A lease this side's registry, or a lone read, made.
    Lease(Arc<Lease>),
    /// A lease another side's registry holds, pinned through its services.
    Pin(#[allow(dead_code)] Pin),
}

impl Hold {
    /// Whether this holds `lease`.
    pub(crate) fn holds(&self, lease: &Arc<Lease>) -> bool {
        matches!(self, Hold::Lease(held) if Arc::ptr_eq(held, lease))
    }

    /// The lease this holds, when this side made it.
    pub(crate) fn lease(&self) -> Option<&Arc<Lease>> {
        match self {
            Hold::Lease(lease) => Some(lease),
            _ => None,
        }
    }
}
