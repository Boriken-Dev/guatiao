// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The host's side of a kind: a provider's table as the trait (`Remote`),
//! an offer a chooser picks from, and an instance the host owns.

use super::*;
use crate::schema::read::SchemaRef;
use crate::schema::validate::validate_map;

/// A kind's table, validated, ready to call through.
///
/// Implements the kind's trait (the macro writes that impl), so a
/// consumer holds it as `dyn Trait`. `Copy`, `Send`, `Sync` and
/// `'static`: the table lives in a library's mapping, and unloading that
/// library is the host's word that nothing like this is still held.
pub struct Remote<K: ?Sized> {
    pub(super) table: *const c_void,
    pub(super) size: usize,
    pub(super) ctx: *mut c_void,
    kind: PhantomData<fn() -> K>,
}

impl<K: ?Sized> Clone for Remote<K> {
    fn clone(&self) -> Remote<K> {
        *self
    }
}

impl<K: ?Sized> Copy for Remote<K> {}

impl<K: ?Sized + Kind> core::fmt::Debug for Remote<K> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Remote")
            .field("kind", &K::NAME)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

// SAFETY: the table and `ctx` address a mapping the registry holds, and
// the kind's trait names `Send + Sync`, which the shims uphold by calling
// a `&self` method. Unbounded in `K` so the impl holds for `dyn Trait +
// 'a` at every `'a`, which a future needs; every constructor requires
// `K: Kind`, so no value exists for any other `K`.
unsafe impl<K: ?Sized> Send for Remote<K> {}
// SAFETY: as above.
unsafe impl<K: ?Sized> Sync for Remote<K> {}

impl<K: ?Sized + Kind> Remote<K> {
    /// Validates a table from anywhere: a C host passing one in, a plugin
    /// system that is not guatiao.
    ///
    /// The one door that accepts a `floor_hash` of `0`, because the
    /// caller states the promise a header would have carried.
    ///
    /// # Safety
    ///
    /// `table` is null, or addresses `size` readable bytes that are a
    /// table for the kind named [`K::NAME`](Kind::NAME), laid out as its
    /// `Vtable`, that stay valid for the life of the process; `ctx` is
    /// what the table's slots expect. For an object kind, the caller also
    /// keeps the borrow rules a `Remote` cannot, being `Copy` and `Sync`:
    /// no method runs through one copy while a `&mut self` method runs
    /// through another, and `&self` methods run on two threads at once
    /// only for a kind whose trait names `Sync`.
    pub unsafe fn from_raw(
        table: *const c_void,
        size: usize,
        ctx: *mut c_void,
    ) -> Result<Remote<K>, KindMismatch> {
        // SAFETY: forwarded.
        unsafe { Remote::validate(table, size, ctx, true) }
    }

    /// A provider's per-kind table for `K`, validated. `NoTable` when the
    /// provider declares none for this kind; `ObjectKind` for an object
    /// kind, which no provider serves whatever its descriptor claims.
    pub(crate) fn from_view(view: &ProviderView) -> Result<Remote<K>, KindMismatch> {
        if K::OBJECT {
            return Err(KindMismatch::ObjectKind);
        }
        let (table, size) = view
            .tables
            .iter()
            .find(|(kind, _, _)| *kind == K::NAME)
            .map(|&(_, table, size)| (table, size))
            .ok_or(KindMismatch::NoTable)?;
        // SAFETY: the view was read from a descriptor under its guards,
        // and the library declared this table as K::NAME's; the mapping
        // is the registry's until the host unloads it.
        unsafe { Remote::validate(table, size, view.ctx, false) }
    }

    /// The one validation every door runs.
    ///
    /// # Safety
    ///
    /// As [`Remote::from_raw`].
    pub(super) unsafe fn validate(
        table: *const c_void,
        size: usize,
        ctx: *mut c_void,
        allow_unchecked: bool,
    ) -> Result<Remote<K>, KindMismatch> {
        if table.is_null() {
            return Err(KindMismatch::NoTable);
        }
        if size < size_of::<KindHeader>() {
            return Err(KindMismatch::BelowFloor {
                size,
                floor: K::FLOOR,
            });
        }
        // SAFETY: the header lies within `size` bytes, checked above.
        let header = unsafe { table.cast::<KindHeader>().read_unaligned() };
        // The table's own size and the descriptor's agree or the smaller
        // wins: nothing past what either side declares is read.
        let declared = header.struct_size as usize;
        let size = if declared == 0 {
            size
        } else {
            size.min(declared)
        };
        if size < K::FLOOR {
            return Err(KindMismatch::BelowFloor {
                size,
                floor: K::FLOOR,
            });
        }
        let found = header.floor_hash;
        if found != K::FLOOR_HASH && !(found == 0 && allow_unchecked) {
            return Err(KindMismatch::HashMismatch {
                expected: K::FLOOR_HASH,
                found,
            });
        }
        for &(name, end) in K::REQUIRED {
            if end > size || end < size_of::<*const c_void>() {
                return Err(KindMismatch::BelowFloor {
                    size,
                    floor: K::FLOOR,
                });
            }
            // SAFETY: the slot lies within `size` bytes; a slot is one
            // pointer wide.
            let slot = unsafe {
                table
                    .cast::<u8>()
                    .add(end - size_of::<*const c_void>())
                    .cast::<*const c_void>()
                    .read_unaligned()
            };
            if slot.is_null() {
                return Err(KindMismatch::NullRequiredSlot(name));
            }
        }
        Ok(Remote {
            table,
            size,
            ctx,
            kind: PhantomData,
        })
    }

    /// The context every slot receives.
    pub fn ctx(&self) -> *mut c_void {
        self.ctx
    }

    /// The table, for a caller that speaks C.
    pub fn table(&self) -> *const c_void {
        self.table
    }

    /// The table's size as validated: the smaller of what the library
    /// declared on the table and on the descriptor.
    pub fn size(&self) -> usize {
        self.size
    }

    /// The kind's trait over this proxy.
    pub fn as_dyn(&self) -> &K {
        K::as_dyn(self)
    }

    /// One slot, read under the table's size: `None` when the table ends
    /// before `end`, or the slot is null.
    ///
    /// # Safety
    ///
    /// `F` is the function-pointer type of the slot at `offset` in
    /// [`K::Vtable`](Kind::Vtable), and `end` is `offset` plus its size.
    #[doc(hidden)]
    pub unsafe fn slot<F: Copy>(&self, offset: usize, end: usize) -> Option<F> {
        if self.size < end {
            return None;
        }
        // SAFETY: the caller states `F` is the slot's type and the guard
        // established it lies within the table.
        unsafe {
            self.table
                .cast::<u8>()
                .add(offset)
                .cast::<Option<F>>()
                .read_unaligned()
        }
    }
}

/// One provider that serves a kind, as the kind's trait plus what a
/// chooser needs to show.
///
/// **Every provider claiming the kind with a valid table is an offer,
/// unavailable ones included.** Ask [`available`](Offer::available) and
/// choose; nothing here picks.
///
/// **An offer is not always callable.** A provider built only from a
/// configuration has no default instance, so its `ctx` is null and its
/// methods could only fail; [`default_instance`](Offer::default_instance)
/// is `None` for it, and [`instantiate`](Offer::instantiate) is how it is
/// used.
pub struct Offer<K: ?Sized> {
    remote: Remote<K>,
    view: ProviderView,
    key: Option<String>,
    library: Option<String>,
    version: String,
    priority: i32,
}

impl<K: ?Sized + Kind> core::fmt::Debug for Offer<K> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Offer")
            .field("kind", &K::NAME)
            .field("id", &self.view.id)
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

impl<K: ?Sized + Kind> Offer<K> {
    pub(crate) fn new(
        remote: Remote<K>,
        view: ProviderView,
        key: Option<String>,
        library: Option<String>,
        version: String,
        priority: i32,
    ) -> Offer<K> {
        Offer {
            remote,
            view,
            key,
            library,
            version,
            priority,
        }
    }

    /// The provider's identifier.
    pub fn id(&self) -> &str {
        &self.view.id
    }

    /// A name to show a person, possibly empty.
    pub fn display_name(&self) -> &str {
        &self.view.display_name
    }

    /// Its version: its own, or its library's when it declared none.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The library offering it, when known: a registry knows, a host's
    /// services hand over the descriptor alone.
    pub fn library(&self) -> Option<&str> {
        self.library.as_deref()
    }

    /// What the registry filed it under, when it came from one.
    pub fn key(&self) -> Option<&str> {
        self.key.as_deref()
    }

    /// What the host ranked it. Zero from a host's services.
    pub fn priority(&self) -> i32 {
        self.priority
    }

    /// Whatever else it declared.
    pub fn meta(&self) -> Option<&Map> {
        self.view.meta.as_ref()
    }

    /// The schema for its configuration, or `None`.
    pub fn config_schema(&self) -> Option<&Value> {
        self.view.config.as_ref()
    }

    /// Whether it can run here, and why not when it cannot. Asked live,
    /// never cached.
    pub fn available(&self) -> Result<(), &'static str> {
        self.view.available()
    }

    /// Whether this offer can be called as it stands. See
    /// [`default_instance`](Offer::default_instance).
    fn callable(&self) -> bool {
        self.view.has_default_instance()
    }

    /// The provider's default instance, as the trait, or `None` for a
    /// provider built only from a configuration (`create` present, `ctx`
    /// null): call [`instantiate`](Offer::instantiate) for one of those. A
    /// provider with no `create` is its own instance, whatever `ctx` it
    /// declared, null included, as a stateless C table expects.
    pub fn default_instance(&self) -> Option<&K> {
        self.callable().then(|| K::as_dyn(&self.remote))
    }

    /// The validated proxy over the provider's own `ctx`, for a caller
    /// that speaks C or keeps the table; `None` exactly when
    /// [`default_instance`](Offer::default_instance) is, since a proxy
    /// over no instance could only fail.
    pub fn remote(&self) -> Option<Remote<K>> {
        self.callable().then_some(self.remote)
    }

    /// The descriptor as read.
    pub fn view(&self) -> &ProviderView {
        &self.view
    }

    /// The default instance, boxed, to store like any implementation;
    /// `None` as for [`default_instance`](Offer::default_instance).
    pub fn boxed(self) -> Option<Box<K>> {
        self.callable().then(|| K::boxed(self.remote))
    }

    /// The default instance, shared; `None` as for
    /// [`default_instance`](Offer::default_instance).
    pub fn shared(self) -> Option<Arc<K>> {
        self.callable().then(|| K::shared(self.remote))
    }
}

impl<K: ?Sized + Kind> Offer<K> {
    /// An instance of this provider built from `config`, as `K`.
    ///
    /// `config` is checked against [`config_schema`](Offer::config_schema)
    /// first, so a misspelled or ill-typed setting is
    /// [`InstantiateError::Invalid`] and the provider is never asked; a
    /// schema this build cannot read is not checked. The provider then
    /// decodes it and builds an instance whose address is the context
    /// every call on the returned [`Instance`] passes. A provider that
    /// builds no instances (`create` null) is
    /// [`InstantiateError::Refused`] with `GUATIAO_ERR_NULL` and a message
    /// saying so.
    pub fn instantiate(&self, config: &Value) -> Result<Instance<K>, InstantiateError> {
        Instance::build(&self.view, config)
    }

    /// Whether this provider builds instances from a configuration.
    pub fn builds_instances(&self) -> bool {
        self.view.create.is_some()
    }
}

/// One instance a provider built from a configuration: the kind's trait,
/// over that instance, released when this is dropped.
///
/// `Send + Sync`, since the kind's trait names both; not `Copy`, since it
/// owns the instance. It implements the kind's trait (the macro writes
/// that impl), so `Arc::new(instance)` is an `Arc<dyn Trait>` that
/// releases the instance with its last owner.
pub struct Instance<K: ?Sized> {
    remote: Remote<K>,
    lib_ctx: *mut c_void,
    destroy: Option<unsafe extern "C" fn(ctx: *mut c_void, instance: *mut c_void)>,
}

impl<K: ?Sized + Kind> core::fmt::Debug for Instance<K> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Instance")
            .field("kind", &K::NAME)
            .finish_non_exhaustive()
    }
}

// SAFETY: the instance is the provider's own, addressed only through the
// kind's `Send + Sync` trait, and released once, here.
unsafe impl<K: ?Sized> Send for Instance<K> {}
// SAFETY: as above.
unsafe impl<K: ?Sized> Sync for Instance<K> {}

impl<K: ?Sized + Kind> Instance<K> {
    /// Builds through a provider's `create` slot, after validating its
    /// table for `K` and `config` against its schema, or says why not.
    pub(crate) fn build(
        view: &ProviderView,
        config: &Value,
    ) -> Result<Instance<K>, InstantiateError> {
        let Some(create) = view.create else {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_NULL,
                "this provider builds no instances; it is its one instance",
            )
            .into());
        };
        // The table is validated first, so a provider whose table does not
        // fit `K` never builds an instance it would then have to destroy.
        let remote = Remote::<K>::from_view(view)
            .map_err(|why| ProviderError::new(Status::GUATIAO_ERR_WRONG_KIND, &why.to_string()))?;
        if let Some(schema) = view.config.as_ref().and_then(SchemaRef::new) {
            validate_map(schema, config)?;
        }
        let mut instance: *mut c_void = core::ptr::null_mut();
        let mut err = ProviderError::none();
        // SAFETY: the slot is the descriptor's own, read under its guard;
        // `config` is a well-formed value; the out-slots are writable
        // locals; the library is mapped for the call.
        let status = unsafe { create(view.ctx, config, &mut instance, &mut err) };
        if status != Status::GUATIAO_OK {
            return Err(take_err(err, status).into());
        }
        if instance.is_null() {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_NULL,
                "the provider answered OK and built nothing",
            )
            .into());
        }
        Ok(Instance {
            remote: Remote {
                table: remote.table,
                size: remote.size,
                ctx: instance,
                kind: PhantomData,
            },
            lib_ctx: view.ctx,
            destroy: view.destroy,
        })
    }

    /// The proxy over this instance, valid while `self` lives.
    pub fn remote(&self) -> Remote<K> {
        self.remote
    }

    /// The instance pointer, for a caller that speaks C.
    pub fn ctx(&self) -> *mut c_void {
        self.remote.ctx
    }
}

impl<K: ?Sized + Kind> core::ops::Deref for Instance<K> {
    type Target = K;

    fn deref(&self) -> &K {
        K::as_dyn(&self.remote)
    }
}

impl<K: ?Sized> Drop for Instance<K> {
    fn drop(&mut self) {
        if let Some(destroy) = self.destroy {
            // SAFETY: the instance came from this provider's `create` and
            // is released exactly once, here.
            unsafe { destroy(self.lib_ctx, self.remote.ctx) };
        }
    }
}

impl<K: ?Sized + Kind> Offer<K> {
    /// An offer from a view alone, as a host's services hand one over:
    /// no registry key, no library id, no rank.
    pub(crate) fn from_view(view: ProviderView) -> Result<Offer<K>, KindMismatch> {
        let remote = Remote::from_view(&view)?;
        let version = view.version.clone().unwrap_or_default();
        Ok(Offer::new(remote, view, None, None, version, 0))
    }
}

impl ProviderInfo<'static> {
    /// This provider's table for `K`, validated, over its own `ctx`; `Ok(None)`
    /// for a provider built only from a configuration, as
    /// [`Provider::as_kind`](crate::library::Provider::as_kind).
    ///
    /// For a descriptor a library built itself; a host's services answer
    /// with views, and [`Host::offers`] is the door there.
    pub fn as_kind<K: ?Sized + Kind>(&'static self) -> Result<Option<Remote<K>>, KindMismatch> {
        let view = self.view().ok_or(KindMismatch::NoTable)?;
        let remote = Remote::from_view(&view)?;
        Ok(view.has_default_instance().then_some(remote))
    }

    /// The same, as an offer carrying what a chooser needs.
    pub fn offer<K: ?Sized + Kind>(&'static self) -> Result<Offer<K>, KindMismatch> {
        let view = self.view().ok_or(KindMismatch::NoTable)?;
        Offer::from_view(view)
    }

    /// An instance built from `config`, as `K`. See [`Offer::instantiate`].
    pub fn instantiate<K: ?Sized + Kind>(
        &'static self,
        config: &Value,
    ) -> Result<Instance<K>, InstantiateError> {
        let view = self.view().ok_or_else(|| {
            ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, "an unreadable descriptor")
        })?;
        Instance::build(&view, config)
    }
}

impl Host {
    /// Every provider the host holds that serves `K` with a valid table,
    /// in the host's order, unavailable ones included.
    ///
    /// `Err` when the host offers no services or its registry is gone.
    pub fn offers<K: ?Sized + Kind>(&self) -> Result<Vec<Offer<K>>, Status> {
        Ok(self
            .list(K::NAME)?
            .into_iter()
            .filter_map(|view| Offer::from_view(view).ok())
            .collect())
    }

    /// Providers claiming `K` whose per-kind table failed validation,
    /// each with why — so a host can report them rather than lose them.
    /// One with no per-kind table is neither an offer nor a mismatch.
    pub fn mismatches<K: ?Sized + Kind>(
        &self,
    ) -> Result<Vec<(ProviderView, KindMismatch)>, Status> {
        Ok(self
            .list(K::NAME)?
            .into_iter()
            .filter_map(|view| match Remote::<K>::from_view(&view) {
                Ok(_) | Err(KindMismatch::NoTable) => None,
                Err(why) => Some((view, why)),
            })
            .collect())
    }

    /// One provider by the host's key, as `K`.
    ///
    /// `Ok(None)` when nothing answers to the key.
    pub fn offer<K: ?Sized + Kind>(
        &self,
        key: &str,
    ) -> Result<Option<Result<Offer<K>, KindMismatch>>, Status> {
        Ok(self.get(key)?.map(Offer::from_view))
    }
}
