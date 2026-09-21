// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A provider kind as a Rust trait, and the runtime the kind and
//! provider macros expand into calls to.
//!
//! A **kind** is a trait the host and the library both compile against.
//! `#[guatiao::kind]` emits, beside the trait, a `repr(C)` function table
//! whose slots are monomorphised shims, an `impl Kind for dyn Trait`
//! naming the table's floor and hash, and an `impl Trait for Remote<dyn
//! Trait>` that calls through a table it was handed. Every `unsafe` those
//! expansions need is a call into this module, written once and reviewed
//! once; generated code only calls it.
//!
//! # The three doors into a table
//!
//! - From a registry: [`Registry::offers`](super::Registry::offers) and
//!   [`Provider::as_kind`](super::Provider::as_kind). Safe, because the
//!   registry read the descriptor under its guards.
//! - From a host, inside a library: [`Host::offers`] and
//!   [`ProviderInfo::as_kind`]. Safe for the same reason.
//! - From anywhere else: [`Remote::from_raw`], `unsafe`, with one promise
//!   — the bytes are a table for this kind's name.
//!
//! All three run one validation: the table is at least the kind's floor,
//! its header hash matches, and no required slot is null. A `floor_hash`
//! of `0` — a table with no header, or a hand-written C one — is accepted
//! by `from_raw` alone, where the caller stated the promise; a registry
//! or host never offers such a table as a kind.
//!
//! # Which providers are offers
//!
//! Only a table in [`ProviderInfo::tables`] is validated as a kind. The
//! shared [`vtable`](ProviderInfo::vtable) stays the untyped path
//! ([`Provider::table_for`](super::Provider::table_for)), so a library
//! written before per-kind tables existed keeps working and is never
//! mistaken for a typed table it does not have a header for.

#![allow(non_camel_case_types)]

use std::ffi::c_void;
use std::marker::PhantomData;
use std::mem::size_of;
use std::sync::Arc;

use super::desc::{KindTable, KindTables, LibraryInfo, ProviderInfo};
use super::raw::{Host, ProviderView};
use crate::value::ValueError;
use crate::value::alloc::Alloc;
use crate::value::status::Status;
use crate::value::types::{Bytes, Map, Str, Text, Value};

/// What a kind declares about its table. Implemented for `dyn Trait` by
/// `#[guatiao::kind]`; the host and the library share the impl.
pub trait Kind: 'static {
    /// The kind's name in a descriptor's `kinds` and `tables`.
    const NAME: &'static str;
    /// The `repr(C)` table, which starts with a [`KindHeader`].
    type Vtable: 'static;
    /// One past the end of the last required slot. A table shorter than
    /// this cannot be called.
    const FLOOR: usize;
    /// FNV-1a over the required methods' names and normalised signatures.
    /// A table carrying another number was built from a different
    /// declaration of this kind.
    const FLOOR_HASH: u32;
    /// Every required slot as `(method name, one past its end)`, so a
    /// null one is refused by name.
    const REQUIRED: &'static [(&'static str, usize)];
    /// Whether this is an **object kind** (`#[guatiao::kind(object)]`): a
    /// handle one caller owns, with a `destroy` slot first and `&mut self`
    /// methods, never offered by a registry. See [`Object`].
    const OBJECT: bool = false;

    /// How many objects of this kind, and of the kinds its methods hand
    /// back, this image has made and not yet seen destroyed. `seen` stops
    /// a cycle of kinds. What a derived library's `unload` refuses on; a
    /// hand-written kind counts nothing.
    #[doc(hidden)]
    fn live_objects(_seen: &mut Vec<&'static str>) -> usize {
        0
    }

    /// The trait object over a proxy. The macro writes `remote`, because
    /// only it knows the trait.
    fn as_dyn(remote: &Remote<Self>) -> &Self;
    /// The same, mutably — what an [`Object`] hands out.
    fn as_dyn_mut(remote: &mut Remote<Self>) -> &mut Self;
    /// The same, boxed.
    fn boxed(remote: Remote<Self>) -> Box<Self>;
    /// The same, shared.
    fn shared(remote: Remote<Self>) -> Arc<Self>;
}

/// The first eight bytes of every kind table.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KindHeader {
    /// `sizeof` the table as the library compiled it. Always first.
    pub struct_size: u32,
    /// [`Kind::FLOOR_HASH`] of the declaration the table was built from.
    pub floor_hash: u32,
}

impl KindHeader {
    /// A header for a table of `size` bytes built from a declaration
    /// hashing to `hash`.
    pub const fn new(size: usize, hash: u32) -> KindHeader {
        KindHeader {
            struct_size: size as u32,
            floor_hash: hash,
        }
    }
}

/// Why a call across a kind failed, as a provider states it.
///
/// A shim writes one through an out-pointer; the proxy hands it back as
/// the `Err` of the trait method. `message` may be empty.
#[repr(C)]
#[derive(Debug)]
pub struct ProviderError {
    /// What went wrong, as a status.
    pub status: Status,
    /// The provider's own words, possibly empty.
    pub message: Text,
}

impl ProviderError {
    /// An error with a message.
    pub fn new(status: Status, message: &str) -> ProviderError {
        ProviderError {
            status,
            message: Text::new(message),
        }
    }

    /// The placeholder a proxy passes to a shim: not an error. A shim
    /// that fails overwrites it.
    pub fn none() -> ProviderError {
        ProviderError {
            status: Status::GUATIAO_OK,
            message: Text::new(""),
        }
    }

    /// The provider's own words, or empty.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl From<Status> for ProviderError {
    fn from(status: Status) -> ProviderError {
        ProviderError::new(status, "")
    }
}

impl From<ValueError> for ProviderError {
    fn from(e: ValueError) -> ProviderError {
        ProviderError::new(Status::from(e), &e.to_string())
    }
}

/// So a provider built through an infallible `From<C>` (a type that IS
/// its configuration, `config = Self`) satisfies `TryFrom<C, Error:
/// Into<ProviderError>>` with nothing written.
impl From<std::convert::Infallible> for ProviderError {
    fn from(never: std::convert::Infallible) -> ProviderError {
        match never {}
    }
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = self.message();
        if message.is_empty() {
            write!(f, "the provider answered {:?}", self.status)
        } else {
            write!(f, "{message} ({:?})", self.status)
        }
    }
}

impl std::error::Error for ProviderError {}

/// Why a table could not be used as a kind. Each names the check that
/// failed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum KindMismatch {
    /// The provider declares no table for this kind.
    NoTable,
    /// The table is shorter than the kind's last required slot.
    BelowFloor {
        /// What the library declared.
        size: usize,
        /// What the kind needs.
        floor: usize,
    },
    /// The table was built from a different declaration of this kind.
    HashMismatch {
        /// This kind's hash.
        expected: u32,
        /// The table's.
        found: u32,
    },
    /// A required slot is null, named.
    NullRequiredSlot(&'static str),
}

impl std::fmt::Display for KindMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KindMismatch::NoTable => f.write_str("the provider declares no table for this kind"),
            KindMismatch::BelowFloor { size, floor } => {
                write!(f, "the table is {size} bytes and the kind needs {floor}")
            }
            KindMismatch::HashMismatch { expected, found } => write!(
                f,
                "the table was built from another declaration of this kind ({found:#010x}, expected {expected:#010x})"
            ),
            KindMismatch::NullRequiredSlot(name) => {
                write!(f, "the required slot `{name}` is null")
            }
        }
    }
}

impl std::error::Error for KindMismatch {}

/// A kind's table, validated, ready to call through.
///
/// Implements the kind's trait (the macro writes that impl), so a
/// consumer holds it as `dyn Trait`. `Copy`, `Send`, `Sync` and
/// `'static`: the table lives in a library's mapping, and unloading that
/// library is the host's word that nothing like this is still held.
pub struct Remote<K: ?Sized + Kind> {
    table: *const c_void,
    size: usize,
    ctx: *mut c_void,
    kind: PhantomData<fn() -> K>,
}

impl<K: ?Sized + Kind> Clone for Remote<K> {
    fn clone(&self) -> Remote<K> {
        *self
    }
}

impl<K: ?Sized + Kind> Copy for Remote<K> {}

impl<K: ?Sized + Kind> std::fmt::Debug for Remote<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Remote")
            .field("kind", &K::NAME)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

// SAFETY: the table and `ctx` address a mapping the registry holds, and
// the kind's trait names `Send + Sync`, which the shims uphold by calling
// a `&self` method.
unsafe impl<K: ?Sized + Kind> Send for Remote<K> {}
// SAFETY: as above.
unsafe impl<K: ?Sized + Kind> Sync for Remote<K> {}

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
    /// what the table's slots expect.
    pub unsafe fn from_raw(
        table: *const c_void,
        size: usize,
        ctx: *mut c_void,
    ) -> Result<Remote<K>, KindMismatch> {
        // SAFETY: forwarded.
        unsafe { Remote::validate(table, size, ctx, true) }
    }

    /// A provider's per-kind table for `K`, validated. `NoTable` when the
    /// provider declares none for this kind.
    pub(crate) fn from_view(view: &ProviderView) -> Result<Remote<K>, KindMismatch> {
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
    unsafe fn validate(
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
pub struct Offer<K: ?Sized + Kind> {
    remote: Remote<K>,
    view: ProviderView,
    key: Option<String>,
    library: Option<String>,
    version: String,
    priority: i32,
}

impl<K: ?Sized + Kind> std::fmt::Debug for Offer<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

    /// The validated proxy, to keep.
    pub fn remote(&self) -> Remote<K> {
        self.remote
    }

    /// The descriptor as read.
    pub fn view(&self) -> &ProviderView {
        &self.view
    }

    /// The trait, boxed, to store like any implementation.
    pub fn boxed(self) -> Box<K> {
        K::boxed(self.remote)
    }

    /// The trait, shared.
    pub fn shared(self) -> Arc<K> {
        K::shared(self.remote)
    }
}

impl<K: ?Sized + Kind> std::ops::Deref for Offer<K> {
    type Target = K;

    fn deref(&self) -> &K {
        K::as_dyn(&self.remote)
    }
}

impl<K: ?Sized + Kind> Offer<K> {
    /// An instance of this provider built from `config`, as `K`.
    ///
    /// The configuration is a value fitting [`config_schema`](Offer::config_schema);
    /// the provider decodes it and builds an instance whose address is
    /// the context every call on the returned [`Instance`] passes. A
    /// provider that builds no instances (`create` null) answers
    /// `GUATIAO_ERR_NULL` with a message saying so.
    pub fn instantiate(&self, config: &Value) -> Result<Instance<K>, ProviderError> {
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
/// owns the instance.
pub struct Instance<K: ?Sized + Kind> {
    remote: Remote<K>,
    lib_ctx: *mut c_void,
    destroy: Option<unsafe extern "C" fn(ctx: *mut c_void, instance: *mut c_void)>,
}

impl<K: ?Sized + Kind> std::fmt::Debug for Instance<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Instance")
            .field("kind", &K::NAME)
            .finish_non_exhaustive()
    }
}

// SAFETY: the instance is the provider's own, addressed only through the
// kind's `Send + Sync` trait, and released once, here.
unsafe impl<K: ?Sized + Kind> Send for Instance<K> {}
// SAFETY: as above.
unsafe impl<K: ?Sized + Kind> Sync for Instance<K> {}

impl<K: ?Sized + Kind> Instance<K> {
    /// Builds through a provider's `create` slot and validates its table
    /// for `K`, or says why not.
    pub(crate) fn build(view: &ProviderView, config: &Value) -> Result<Instance<K>, ProviderError> {
        let Some(create) = view.create else {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_NULL,
                "this provider builds no instances; it is its one instance",
            ));
        };
        // The table is validated first, so a provider whose table does not
        // fit `K` never builds an instance it would then have to destroy.
        let remote = Remote::<K>::from_view(view)
            .map_err(|why| ProviderError::new(Status::GUATIAO_ERR_WRONG_KIND, &why.to_string()))?;
        let mut instance: *mut c_void = std::ptr::null_mut();
        let mut err = ProviderError::none();
        // SAFETY: the slot is the descriptor's own, read under its guard;
        // `config` is a well-formed value; the out-slots are writable
        // locals; the library is mapped for the call.
        let status = unsafe { create(view.ctx, config, &mut instance, &mut err) };
        if status != Status::GUATIAO_OK {
            return Err(take_err(err, status));
        }
        if instance.is_null() {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_NULL,
                "the provider answered OK and built nothing",
            ));
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

impl<K: ?Sized + Kind> std::ops::Deref for Instance<K> {
    type Target = K;

    fn deref(&self) -> &K {
        K::as_dyn(&self.remote)
    }
}

impl<K: ?Sized + Kind> Drop for Instance<K> {
    fn drop(&mut self) {
        if let Some(destroy) = self.destroy {
            // SAFETY: the instance came from this provider's `create` and
            // is released exactly once, here.
            unsafe { destroy(self.lib_ctx, self.remote.ctx) };
        }
    }
}

impl<K: ?Sized + Kind> From<Offer<K>> for Remote<K> {
    fn from(offer: Offer<K>) -> Remote<K> {
        offer.remote
    }
}

// --- objects: a handle one caller owns -----------------------------------

/// An object as it crosses the boundary: its table, the table's size, and
/// the context every slot takes. **Ownership crosses with it**: whoever
/// receives one destroys it, through the table's `destroy` slot.
///
/// The out-parameter of a method returning [`Object`], and the argument
/// type of a method taking one. All three fields null or zero is "no
/// object".
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ObjectRaw {
    /// The object kind's table.
    pub table: *const c_void,
    /// `sizeof` that table as the library compiled it.
    pub size: usize,
    /// What the table's slots take.
    pub ctx: *mut c_void,
}

impl ObjectRaw {
    /// No object.
    pub const fn null() -> ObjectRaw {
        ObjectRaw {
            table: std::ptr::null(),
            size: 0,
            ctx: std::ptr::null_mut(),
        }
    }
}

/// Borrowed **writable** bytes: a pointer and a length. The out-buffer
/// argument an object kind's `&mut [u8]` crosses as.
///
/// Check `len` before `ptr`, as with [`Bytes`]: an empty buffer may carry
/// a null pointer.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BytesMut {
    /// First byte. May be null when `len` is 0.
    pub ptr: *mut u8,
    /// Length in bytes.
    pub len: usize,
}

/// One Rust-built object: its kind's table, then the value the table's
/// shims address. **The table travels with the object** rather than
/// living in a `static`, so building one needs no registration and
/// destroying it frees everything at once.
///
/// `repr(C)` so the table is at offset 0: the `ctx` an object crosses
/// with addresses this cell, and the shims read the value past the table.
#[repr(C)]
#[doc(hidden)]
pub struct ObjectCell<V, T> {
    /// The kind's table, filled by `<Trait>Vtable::of::<T>()`.
    pub table: V,
    /// The object.
    pub value: T,
}

impl<V, T> std::fmt::Debug for ObjectCell<V, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectCell").finish_non_exhaustive()
    }
}

/// Where an object kind's `destroy` slot ends: right after the header.
const DESTROY_END: usize = size_of::<KindHeader>() + size_of::<*const c_void>();

/// An object kind's handle: one owner, `&mut` access, destroyed on drop.
///
/// The thing a provider hands back and one caller drives — a session, a
/// scan, a stream — as opposed to a provider, which is shared and
/// offered. Built on the Rust side by the trait's `into_object()` (the
/// kind attribute writes it), received across the boundary by a
/// generated proxy, and destroyed exactly once through the table's
/// `destroy` slot when this is dropped.
///
/// `Send` (an object kind names `Send`); not `Sync` and not `Clone`,
/// because the shims hand out `&mut` to the value behind it. Share one
/// through a `Mutex` if two threads need it.
pub struct Object<K: ?Sized + Kind> {
    remote: Remote<K>,
}

impl<K: ?Sized + Kind> std::fmt::Debug for Object<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Object")
            .field("kind", &K::NAME)
            .finish_non_exhaustive()
    }
}

// SAFETY: the object kind's trait names `Send`, and this is the one
// handle to the value; the table addresses a library that is never
// unloaded or the cell the handle owns.
unsafe impl<K: ?Sized + Kind> Send for Object<K> {}

impl<K: ?Sized + Kind> Object<K> {
    /// An object from a cell the derive's `into_object` built: the table
    /// is the cell's own, validated for `K`.
    ///
    /// `Err` only for a table that does not fit the kind, which a table
    /// `of::<T>()` built cannot be; the derive `expect`s it.
    pub fn from_cell<V: 'static, T: 'static>(
        cell: Box<ObjectCell<V, T>>,
    ) -> Result<Object<K>, KindMismatch> {
        let size = size_of::<V>();
        let ctx = Box::into_raw(cell);
        // SAFETY: the cell's table is `V`, `size` bytes, at offset 0 of a
        // heap block this handle now owns; `ctx` addresses that block.
        let validated = unsafe {
            Remote::<K>::validate(ctx.cast::<c_void>(), size, ctx.cast::<c_void>(), false)
        };
        match validated {
            Ok(remote) => Ok(Object { remote }),
            Err(why) => {
                // SAFETY: the box was made just above and is not used again.
                drop(unsafe { Box::from_raw(ctx) });
                Err(why)
            }
        }
    }

    /// An object that crossed the boundary, validated for `K`. Takes
    /// ownership: on a mismatch the object is destroyed here, since the
    /// caller was handed it and nothing else will.
    ///
    /// # Safety
    ///
    /// `raw` is all-null, or its `table` addresses `size` readable bytes
    /// that are a table for the kind named [`K::NAME`](Kind::NAME), laid
    /// out as its `Vtable` with a `destroy` slot after the header, valid
    /// until that slot is called with `ctx`.
    pub unsafe fn from_raw(raw: ObjectRaw) -> Result<Object<K>, KindMismatch> {
        // SAFETY: forwarded.
        match unsafe { Remote::<K>::validate(raw.table, raw.size, raw.ctx, false) } {
            Ok(remote) => Ok(Object { remote }),
            Err(why) => {
                // SAFETY: the caller's contract; a table long enough to hold
                // the destroy slot has one.
                unsafe { destroy_raw(raw) };
                Err(why)
            }
        }
    }

    /// Hands the object across: the caller of this owns nothing
    /// afterwards, and whoever receives the raw form destroys it.
    pub fn into_raw(self) -> ObjectRaw {
        let raw = ObjectRaw {
            table: self.remote.table,
            size: self.remote.size,
            ctx: self.remote.ctx,
        };
        std::mem::forget(self);
        raw
    }

    /// The context every slot receives: the object itself.
    pub fn ctx(&self) -> *mut c_void {
        self.remote.ctx
    }

    /// The table, for a caller that speaks C.
    pub fn table(&self) -> *const c_void {
        self.remote.table
    }

    /// The table's size as validated.
    pub fn size(&self) -> usize {
        self.remote.size
    }
}

impl<K: ?Sized + Kind> std::ops::Deref for Object<K> {
    type Target = K;

    fn deref(&self) -> &K {
        K::as_dyn(&self.remote)
    }
}

impl<K: ?Sized + Kind> std::ops::DerefMut for Object<K> {
    fn deref_mut(&mut self) -> &mut K {
        K::as_dyn_mut(&mut self.remote)
    }
}

impl<K: ?Sized + Kind> Drop for Object<K> {
    fn drop(&mut self) {
        // SAFETY: the table was validated for an object kind, whose
        // `destroy` slot follows the header; released exactly once, here.
        unsafe {
            destroy_raw(ObjectRaw {
                table: self.remote.table,
                size: self.remote.size,
                ctx: self.remote.ctx,
            })
        };
    }
}

/// Calls the `destroy` slot of a raw object, if its table reaches one.
///
/// # Safety
///
/// As [`Object::from_raw`]; `raw` is not used again.
unsafe fn destroy_raw(raw: ObjectRaw) {
    if raw.table.is_null() || raw.size < DESTROY_END {
        return;
    }
    // SAFETY: the slot lies within `size` bytes, checked above.
    let destroy = unsafe {
        raw.table
            .cast::<u8>()
            .add(size_of::<KindHeader>())
            .cast::<Option<unsafe extern "C" fn(*mut c_void)>>()
            .read_unaligned()
    };
    if let Some(destroy) = destroy {
        // SAFETY: the caller's contract.
        unsafe { destroy(raw.ctx) };
    }
}

impl ProviderInfo {
    /// This provider's table for `K`, validated.
    ///
    /// For a descriptor a host's services handed over.
    pub fn as_kind<K: ?Sized + Kind>(&'static self) -> Result<Remote<K>, KindMismatch> {
        let view = self.view().ok_or(KindMismatch::NoTable)?;
        Remote::from_view(&view)
    }

    /// The same, as an offer carrying what a chooser needs.
    pub fn offer<K: ?Sized + Kind>(&'static self) -> Result<Offer<K>, KindMismatch> {
        let view = self.view().ok_or(KindMismatch::NoTable)?;
        let remote = Remote::from_view(&view)?;
        let version = view.version.clone().unwrap_or_default();
        Ok(Offer::new(remote, view, None, None, version, 0))
    }

    /// An instance built from `config`, as `K`. See [`Offer::instantiate`].
    pub fn instantiate<K: ?Sized + Kind>(
        &'static self,
        config: &Value,
    ) -> Result<Instance<K>, ProviderError> {
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
            .filter_map(|p| p.offer::<K>().ok())
            .collect())
    }

    /// Providers claiming `K` whose per-kind table failed validation,
    /// each with why — so a host can report them rather than lose them.
    /// One with no per-kind table is neither an offer nor a mismatch.
    pub fn mismatches<K: ?Sized + Kind>(
        &self,
    ) -> Result<Vec<(&'static ProviderInfo, KindMismatch)>, Status> {
        Ok(self
            .list(K::NAME)?
            .into_iter()
            .filter_map(|p| p.as_kind::<K>().err().map(|why| (p, why)))
            .filter(|(_, why)| *why != KindMismatch::NoTable)
            .collect())
    }

    /// One provider by the host's key, as `K`.
    ///
    /// `Ok(None)` when nothing answers to the key.
    pub fn offer<K: ?Sized + Kind>(
        &self,
        key: &str,
    ) -> Result<Option<Result<Offer<K>, KindMismatch>>, Status> {
        Ok(self.get(key)?.map(|p| p.offer::<K>()))
    }
}

// --- what a shim calls ----------------------------------------------------
//
// Hidden from the docs, public to the expansion. Each is the one `unsafe`
// step a generated shim or proxy needs, with its contract stated once.

/// The instance behind a shim's `ctx`, or `None` for null.
///
/// # Safety
///
/// `ctx` is null or the `&T` the provider's descriptor declared.
#[doc(hidden)]
pub unsafe fn ctx_ref<'a, T>(ctx: *const c_void) -> Option<&'a T> {
    // SAFETY: the caller's contract.
    unsafe { ctx.cast::<T>().as_ref() }
}

/// A `&str` argument from a view, or the status a shim answers.
///
/// # Safety
///
/// `s` is readable for the call.
#[doc(hidden)]
pub unsafe fn str_arg<'a>(s: Str) -> Result<&'a str, Status> {
    // SAFETY: forwarded.
    unsafe { crate::exports::as_str(s) }
}

/// A `&[u8]` argument from a view.
///
/// # Safety
///
/// `b` is readable for the call.
#[doc(hidden)]
pub unsafe fn bytes_arg<'a>(b: Bytes) -> Result<&'a [u8], Status> {
    if b.len == 0 {
        return Ok(&[]);
    }
    if b.ptr.is_null() {
        return Err(Status::GUATIAO_ERR_NULL);
    }
    // SAFETY: the caller's contract.
    Ok(unsafe { std::slice::from_raw_parts(b.ptr, b.len) })
}

/// A `&Value` argument, refusing null.
///
/// # Safety
///
/// `v` is null or addresses a well-formed value for the call.
#[doc(hidden)]
pub unsafe fn value_arg<'a>(v: *const Value) -> Result<&'a Value, Status> {
    // SAFETY: the caller's contract.
    unsafe { v.as_ref() }.ok_or(Status::GUATIAO_ERR_NULL)
}

/// An `Option<&Value>` argument: null is `None`.
///
/// # Safety
///
/// As [`value_arg`].
#[doc(hidden)]
pub unsafe fn value_opt<'a>(v: *const Value) -> Option<&'a Value> {
    // SAFETY: the caller's contract.
    unsafe { v.as_ref() }
}

/// A `&Map` argument, refusing null.
///
/// # Safety
///
/// `m` is null or addresses a well-formed map for the call.
#[doc(hidden)]
pub unsafe fn map_arg<'a>(m: *const Map) -> Result<&'a Map, Status> {
    // SAFETY: the caller's contract.
    let map = unsafe { m.as_ref() }.ok_or(Status::GUATIAO_ERR_NULL)?;
    // Reached by pointer, not through a value's door, so checked here.
    if map.keys_are_text() {
        Ok(map)
    } else {
        Err(ValueError::NotUtf8.into())
    }
}

/// Writes a result through an out-pointer, refusing null. The previous
/// contents are the caller's to have dealt with.
///
/// # Safety
///
/// `out` is null or addresses writable storage for one `V`.
#[doc(hidden)]
pub unsafe fn write_out<V>(out: *mut V, v: V) -> Status {
    if out.is_null() {
        return Status::GUATIAO_ERR_NULL;
    }
    // SAFETY: the caller's contract.
    unsafe { out.write(v) };
    Status::GUATIAO_OK
}

/// Writes an error through its out-pointer and answers its status. A
/// null `err` drops the message and answers the status alone.
///
/// # Safety
///
/// `err` is null or addresses writable storage for one `ProviderError`
/// whose previous contents the caller has dealt with.
#[doc(hidden)]
pub unsafe fn write_err(err: *mut ProviderError, e: ProviderError) -> Status {
    let status = e.status;
    if !err.is_null() {
        // SAFETY: the caller's contract.
        unsafe { err.write(e) };
    }
    status
}

/// Runs a shim body, converting a panic into `GUATIAO_ERR_INTERNAL`.
#[doc(hidden)]
pub fn catch(body: impl FnOnce() -> Status) -> Status {
    crate::exports::guard(body)
}

/// A configuration argument, decoded as `C`, or the error a `create` shim
/// answers.
///
/// # Safety
///
/// `config` is null or addresses a well-formed value for the call.
#[doc(hidden)]
pub unsafe fn config_arg<C: crate::value::convert::FromValue>(
    config: *const Value,
) -> Result<C, ProviderError> {
    // SAFETY: the caller's contract.
    let value = unsafe { config.as_ref() }
        .ok_or_else(|| ProviderError::new(Status::GUATIAO_ERR_NULL, "no configuration"))?;
    C::from_value(value)
        .map_err(|e| ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, &e.to_string()))
}

/// Writes a built instance through `out`, boxed, or the error through
/// `err`; answers the status either way.
///
/// # Safety
///
/// `out` and `err` are null or writable.
#[doc(hidden)]
pub unsafe fn instance_out<T>(
    out: *mut *mut c_void,
    err: *mut ProviderError,
    built: Result<T, ProviderError>,
) -> Status {
    match built {
        Ok(instance) => {
            if out.is_null() {
                return Status::GUATIAO_ERR_NULL;
            }
            // SAFETY: the caller's contract.
            unsafe { out.write(Box::into_raw(Box::new(instance)).cast::<c_void>()) };
            Status::GUATIAO_OK
        }
        // SAFETY: the caller's contract.
        Err(e) => unsafe { write_err(err, e) },
    }
}

/// Releases an instance [`instance_out`] boxed. Null is a no-op.
///
/// # Safety
///
/// `instance` is null or came from `instance_out::<T>` and is not used
/// again.
#[doc(hidden)]
pub unsafe fn destroy_instance<T>(instance: *mut c_void) {
    if instance.is_null() {
        return;
    }
    // SAFETY: the caller's contract.
    drop(unsafe { Box::from_raw(instance.cast::<T>()) });
}

/// The value behind an object kind's `ctx`, mutably: the cell the
/// derive's `into_object` boxed, past its table. `None` for null.
///
/// # Safety
///
/// `ctx` is null or addresses an `ObjectCell<V, T>` this call has the
/// only reference to for its duration — which an [`Object`] guarantees by
/// being the one handle and handing out `&mut`.
#[doc(hidden)]
pub unsafe fn object_mut<'a, V: 'a, T: 'a>(ctx: *mut c_void) -> Option<&'a mut T> {
    // SAFETY: the caller's contract.
    unsafe { ctx.cast::<ObjectCell<V, T>>().as_mut() }.map(|cell| &mut cell.value)
}

/// Releases the cell [`Object::from_cell`] took. Null is a no-op.
///
/// # Safety
///
/// `ctx` is null or came from `from_cell::<V, T>` and is not used again.
#[doc(hidden)]
pub unsafe fn destroy_object<V, T>(ctx: *mut c_void) {
    if ctx.is_null() {
        return;
    }
    // SAFETY: the caller's contract.
    drop(unsafe { Box::from_raw(ctx.cast::<ObjectCell<V, T>>()) });
}

/// A `&mut [u8]` argument from a writable view.
///
/// # Safety
///
/// `b` is writable for the call and aliased by nothing else during it.
#[doc(hidden)]
pub unsafe fn bytes_mut_arg<'a>(b: BytesMut) -> Result<&'a mut [u8], Status> {
    if b.len == 0 {
        return Ok(&mut []);
    }
    if b.ptr.is_null() {
        return Err(Status::GUATIAO_ERR_NULL);
    }
    // SAFETY: the caller's contract.
    Ok(unsafe { std::slice::from_raw_parts_mut(b.ptr, b.len) })
}

/// An object argument, validated for `K`, or the error a shim answers.
/// Takes ownership either way.
///
/// # Safety
///
/// As [`Object::from_raw`].
#[doc(hidden)]
pub unsafe fn object_arg<K: ?Sized + Kind>(raw: ObjectRaw) -> Result<Object<K>, ProviderError> {
    // SAFETY: forwarded.
    unsafe { Object::<K>::from_raw(raw) }
        .map_err(|why| ProviderError::new(Status::GUATIAO_ERR_WRONG_KIND, &why.to_string()))
}

/// Writes an object through its out-slot, handing it across. A null
/// `out` destroys the object and answers the status.
///
/// # Safety
///
/// `out` is null or addresses writable storage for one `ObjectRaw`.
#[doc(hidden)]
pub unsafe fn object_out<K: ?Sized + Kind>(out: *mut ObjectRaw, object: Object<K>) -> Status {
    if out.is_null() {
        return Status::GUATIAO_ERR_NULL;
    }
    // SAFETY: the caller's contract.
    unsafe { out.write(object.into_raw()) };
    Status::GUATIAO_OK
}

/// An object that crossed as a return, validated for `K`, or the error a
/// proxy hands back.
///
/// # Safety
///
/// As [`Object::from_raw`].
#[doc(hidden)]
pub unsafe fn object_ret<K: ?Sized + Kind>(raw: ObjectRaw) -> Result<Object<K>, ProviderError> {
    // SAFETY: forwarded.
    unsafe { object_arg::<K>(raw) }
}

/// The error a proxy hands back: what the shim wrote, or one built from
/// the status when it wrote nothing (an older library).
#[doc(hidden)]
///
/// A message that is not UTF-8 is dropped, keeping the status: the shim
/// wrote the text itself, past the checks a value's doors make.
pub fn take_err(written: ProviderError, status: Status) -> ProviderError {
    if written.status == Status::GUATIAO_OK {
        ProviderError::from(status)
    } else if std::str::from_utf8(written.message.bytes()).is_err() {
        ProviderError::from(written.status)
    } else {
        written
    }
}

/// A text a shim wrote through a `*mut Text`, checked: the callee writes
/// the storage itself, past the checks a value's doors make.
#[doc(hidden)]
pub fn text_ret(out: Text) -> Result<Text, ProviderError> {
    if std::str::from_utf8(out.bytes()).is_ok() {
        Ok(out)
    } else {
        Err(ValueError::NotUtf8.into())
    }
}

/// A map a shim wrote through a `*mut Map`, its keys checked as
/// [`text_ret`] checks a text.
#[doc(hidden)]
pub fn map_ret(out: Map) -> Result<Map, ProviderError> {
    if out.keys_are_text() {
        Ok(out)
    } else {
        Err(ValueError::NotUtf8.into())
    }
}

/// The `available` slot over a Rust method: runs `ask` on the instance
/// behind `ctx`, writes a borrowed reason on refusal. A panic is a
/// refusal with a reason.
///
/// # Safety
///
/// `ctx` is null or the `&T` the provider's descriptor declared, and
/// `reason` is null or writable.
#[doc(hidden)]
pub unsafe fn available_via<T>(
    ctx: *mut c_void,
    reason: *mut Str,
    ask: impl FnOnce(&T) -> Result<(), &'static str>,
) -> bool {
    // SAFETY: the caller's contract.
    let Some(this) = (unsafe { ctx_ref::<T>(ctx) }) else {
        return false;
    };
    let answer = crate::exports::guard_with(Err("the provider panicked while asked"), || ask(this));
    match answer {
        Ok(()) => true,
        Err(why) => {
            if !reason.is_null() {
                // SAFETY: the caller's contract.
                unsafe { reason.write(Str::new(why)) };
            }
            false
        }
    }
}

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
    kinds: Box<[Str]>,
    #[allow(dead_code)]
    tables: Box<[KindTable]>,
    #[allow(dead_code)]
    config: Option<Box<Value>>,
    info: ProviderInfo,
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
        tables: Vec<KindTable>,
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
            kinds: super::desc::Kinds {
                ptr: kinds.as_ptr(),
                len: kinds.len(),
            },
            id: Str::new(&id),
            display_name: Str::new(&name),
            config: config
                .as_deref()
                .map_or(std::ptr::null(), |v| v as *const Value),
            vtable: std::ptr::null(),
            ctx,
            meta: crate::value::types::MaybeNull::null(),
            version: Str::new(&version),
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
    pub fn info(&self) -> &ProviderInfo {
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
    infos: Box<[ProviderInfo]>,
    info: LibraryInfo,
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
        let infos: Box<[ProviderInfo]> = providers.iter().map(|p| *p.info()).collect();
        let info = LibraryInfo {
            struct_size: size_of::<LibraryInfo>() as u32,
            abi_version: super::desc::ABI_VERSION,
            id: Str::new(&id),
            version: Str::new(&version),
            providers: super::desc::Providers {
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
    pub fn info(&self) -> &LibraryInfo {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::offset_of;

    // A kind written by hand with only these helpers: what the macro
    // will emit, spelled out once.

    trait Greeter: Send + Sync {
        fn greet(&self, name: &str) -> Result<String, ProviderError>;
        /// An appended slot with a default body: an older table lacks it
        /// and the proxy runs the default.
        fn shout(&self) -> i64 {
            -1
        }
    }

    #[repr(C)]
    struct GreeterVtable {
        header: KindHeader,
        greet:
            Option<unsafe extern "C" fn(*mut c_void, Str, *mut Text, *mut ProviderError) -> Status>,
        shout: Option<unsafe extern "C" fn(*mut c_void, *mut i64) -> Status>,
    }

    const GREET_END: usize = offset_of!(GreeterVtable, greet) + size_of::<usize>();
    const SHOUT_END: usize = offset_of!(GreeterVtable, shout) + size_of::<usize>();
    const HASH: u32 = fnv1a("greet(&self, name: &str) -> Result<String, ProviderError>");

    unsafe extern "C" fn greet_shim<T: Greeter>(
        ctx: *mut c_void,
        name: Str,
        out: *mut Text,
        err: *mut ProviderError,
    ) -> Status {
        catch(|| {
            // SAFETY: the table was built for `T` and the descriptor's
            // `ctx` is a `&T`.
            let Some(this) = (unsafe { ctx_ref::<T>(ctx) }) else {
                return Status::GUATIAO_ERR_NULL;
            };
            // SAFETY: the proxy passes a readable view.
            let name = match unsafe { str_arg(name) } {
                Ok(n) => n,
                Err(s) => return s,
            };
            match this.greet(name) {
                // SAFETY: the proxy passes writable out-pointers.
                Ok(text) => unsafe { write_out(out, Text::new(&text)) },
                // SAFETY: as above.
                Err(e) => unsafe { write_err(err, e) },
            }
        })
    }

    unsafe extern "C" fn shout_shim<T: Greeter>(ctx: *mut c_void, out: *mut i64) -> Status {
        catch(|| {
            // SAFETY: as in `greet_shim`.
            let Some(this) = (unsafe { ctx_ref::<T>(ctx) }) else {
                return Status::GUATIAO_ERR_NULL;
            };
            // SAFETY: as above.
            unsafe { write_out(out, this.shout()) }
        })
    }

    impl GreeterVtable {
        const fn of<T: Greeter>() -> GreeterVtable {
            GreeterVtable {
                header: KindHeader::new(size_of::<GreeterVtable>(), HASH),
                greet: Some(greet_shim::<T>),
                shout: Some(shout_shim::<T>),
            }
        }
    }

    impl Kind for dyn Greeter {
        const NAME: &'static str = "greeter";
        type Vtable = GreeterVtable;
        const FLOOR: usize = GREET_END;
        const FLOOR_HASH: u32 = HASH;
        const REQUIRED: &'static [(&'static str, usize)] = &[("greet", GREET_END)];

        fn as_dyn(remote: &Remote<Self>) -> &Self {
            remote
        }
        fn as_dyn_mut(remote: &mut Remote<Self>) -> &mut Self {
            remote
        }
        fn boxed(remote: Remote<Self>) -> Box<Self> {
            Box::new(remote)
        }
        fn shared(remote: Remote<Self>) -> Arc<Self> {
            Arc::new(remote)
        }
    }

    impl Greeter for Remote<dyn Greeter> {
        fn greet(&self, name: &str) -> Result<String, ProviderError> {
            type Slot =
                unsafe extern "C" fn(*mut c_void, Str, *mut Text, *mut ProviderError) -> Status;
            // SAFETY: `greet` is the slot at that offset in the table
            // validation checked.
            let f: Slot = unsafe { self.slot(offset_of!(GreeterVtable, greet), GREET_END) }
                .expect("validated: a required slot is present and non-null");
            let mut out = Text::new("");
            let mut err = ProviderError::none();
            // SAFETY: the slot's own signature; the locals are writable.
            let status = unsafe { f(self.ctx(), Str::new(name), &mut out, &mut err) };
            if status == Status::GUATIAO_OK {
                Ok(text_ret(out)?.to_string())
            } else {
                Err(take_err(err, status))
            }
        }

        fn shout(&self) -> i64 {
            type Slot = unsafe extern "C" fn(*mut c_void, *mut i64) -> Status;
            // SAFETY: as `greet`; an absent slot is the default body.
            match unsafe { self.slot::<Slot>(offset_of!(GreeterVtable, shout), SHOUT_END) } {
                Some(f) => {
                    let mut out = 0i64;
                    // SAFETY: the slot's own signature.
                    if unsafe { f(self.ctx(), &mut out) } == Status::GUATIAO_OK {
                        out
                    } else {
                        -1
                    }
                }
                None => -1,
            }
        }
    }

    struct Hello;
    impl Greeter for Hello {
        fn greet(&self, name: &str) -> Result<String, ProviderError> {
            if name.is_empty() {
                return Err(ProviderError::new(
                    Status::GUATIAO_ERR_BAD_VALUE,
                    "nobody to greet",
                ));
            }
            Ok(format!("hello, {name}"))
        }
        fn shout(&self) -> i64 {
            11
        }
    }

    struct Panics;
    impl Greeter for Panics {
        fn greet(&self, _name: &str) -> Result<String, ProviderError> {
            panic!("a provider bug");
        }
    }

    static HELLO_TABLE: GreeterVtable = GreeterVtable::of::<Hello>();
    static PANICS_TABLE: GreeterVtable = GreeterVtable::of::<Panics>();
    static HELLO: Hello = Hello;
    static PANICS: Panics = Panics;

    fn remote_of(
        table: &'static GreeterVtable,
        ctx: *const c_void,
        size: usize,
    ) -> Result<Remote<dyn Greeter>, KindMismatch> {
        // SAFETY: a table this test built for the kind, with the ctx it
        // was built for.
        unsafe {
            Remote::<dyn Greeter>::from_raw(
                table as *const GreeterVtable as *const c_void,
                size,
                ctx.cast_mut(),
            )
        }
    }

    /// A text or a map a shim wrote itself is checked before a proxy
    /// hands it out, and a message that is not UTF-8 is dropped.
    #[test]
    fn what_a_shim_writes_itself_is_checked() {
        let alloc = Alloc::rust();
        let raw = |bytes: &[u8]| {
            let (p, l, c, a) = crate::value::types::Buffer::new_in(alloc, bytes)
                .unwrap()
                .into_raw_parts();
            // SAFETY: consistent storage; the bytes are under test.
            unsafe { Text::from_raw_parts(p, l, c, a) }
        };
        assert_eq!(&*text_ret(raw(b"ok")).unwrap(), "ok");
        let refused = text_ret(raw(b"\xff")).expect_err("not UTF-8");
        assert_eq!(refused.status, Status::from(ValueError::NotUtf8));

        let written = ProviderError {
            status: Status::GUATIAO_ERR_BAD_VALUE,
            message: raw(b"\xfe"),
        };
        let taken = take_err(written, Status::GUATIAO_ERR_BAD_VALUE);
        assert_eq!(
            taken.status,
            Status::GUATIAO_ERR_BAD_VALUE,
            "the status stays"
        );
        assert_eq!(taken.message(), "", "the unreadable words go");

        let map: Map = [("k", 1)].into_iter().collect();
        assert!(map_ret(map).is_ok());
    }

    #[test]
    fn a_call_round_trips_through_a_remote() {
        let r = remote_of(
            &HELLO_TABLE,
            &HELLO as *const Hello as *const c_void,
            size_of::<GreeterVtable>(),
        )
        .expect("a full table validates");
        assert_eq!(r.greet("ana").unwrap(), "hello, ana");
        assert_eq!(r.shout(), 11, "the appended slot is present and called");

        let e = r.greet("").unwrap_err();
        assert_eq!(e.status, Status::GUATIAO_ERR_BAD_VALUE);
        assert_eq!(e.message(), "nobody to greet", "the message crosses intact");

        let kept: Box<dyn Greeter> = <dyn Greeter as Kind>::boxed(r);
        assert_eq!(kept.greet("bo").unwrap(), "hello, bo");
        let shared: Arc<dyn Greeter> = <dyn Greeter as Kind>::shared(r);
        assert_eq!(shared.shout(), 11);
    }

    #[test]
    fn a_panic_in_the_implementation_is_a_status_and_the_process_lives() {
        let r = remote_of(
            &PANICS_TABLE,
            &PANICS as *const Panics as *const c_void,
            size_of::<GreeterVtable>(),
        )
        .unwrap();
        let e = r.greet("x").unwrap_err();
        assert_eq!(e.status, Status::GUATIAO_ERR_INTERNAL);
        assert_eq!(e.message(), "", "a panic has no words the shim could write");
    }

    #[test]
    fn a_truncated_table_runs_the_default_body_for_an_appended_slot() {
        let r = remote_of(
            &HELLO_TABLE,
            &HELLO as *const Hello as *const c_void,
            GREET_END,
        )
        .expect("the floor is enough");
        assert_eq!(r.greet("ana").unwrap(), "hello, ana");
        assert_eq!(
            r.shout(),
            -1,
            "the slot is past the table, so the default runs"
        );
        assert_eq!(r.size(), GREET_END);
    }

    #[test]
    fn each_check_names_itself() {
        let ctx = &HELLO as *const Hello as *const c_void;
        assert_eq!(
            remote_of(&HELLO_TABLE, ctx, GREET_END - 1).unwrap_err(),
            KindMismatch::BelowFloor {
                size: GREET_END - 1,
                floor: GREET_END
            }
        );

        let mut forged = GreeterVtable::of::<Hello>();
        forged.header.floor_hash = HASH ^ 1;
        let forged: &'static GreeterVtable = Box::leak(Box::new(forged));
        assert_eq!(
            remote_of(forged, ctx, size_of::<GreeterVtable>()).unwrap_err(),
            KindMismatch::HashMismatch {
                expected: HASH,
                found: HASH ^ 1
            }
        );

        let mut nulled = GreeterVtable::of::<Hello>();
        nulled.greet = None;
        let nulled: &'static GreeterVtable = Box::leak(Box::new(nulled));
        assert_eq!(
            remote_of(nulled, ctx, size_of::<GreeterVtable>()).unwrap_err(),
            KindMismatch::NullRequiredSlot("greet")
        );

        // SAFETY: null is refused before anything is read.
        assert_eq!(
            unsafe { Remote::<dyn Greeter>::from_raw(std::ptr::null(), 64, std::ptr::null_mut()) }
                .unwrap_err(),
            KindMismatch::NoTable
        );
    }

    /// A header hash of zero is "unchecked" through `from_raw` alone; a
    /// registry or host never offers such a table as a kind.
    #[test]
    fn a_zero_hash_is_accepted_only_at_the_raw_door() {
        let mut headerless = GreeterVtable::of::<Hello>();
        headerless.header.floor_hash = 0;
        let headerless: &'static GreeterVtable = Box::leak(Box::new(headerless));
        let ctx = &HELLO as *const Hello as *const c_void;
        assert!(remote_of(headerless, ctx, size_of::<GreeterVtable>()).is_ok());

        let view = ProviderView {
            kinds: vec!["greeter".to_string()],
            id: "x".to_string(),
            display_name: String::new(),
            config: None,
            vtable: std::ptr::null(),
            vtable_size: 0,
            ctx: ctx.cast_mut(),
            meta: None,
            version: None,
            available: None,
            raw: std::ptr::null(),
            create: None,
            destroy: None,
            tables: vec![(
                "greeter".to_string(),
                headerless as *const GreeterVtable as *const c_void,
                size_of::<GreeterVtable>(),
            )],
        };
        assert_eq!(
            Remote::<dyn Greeter>::from_view(&view).unwrap_err(),
            KindMismatch::HashMismatch {
                expected: HASH,
                found: 0
            }
        );

        // And the legacy `vtable` is never a typed table: no per-kind
        // entry means no table, whatever `vtable` holds.
        let legacy = ProviderView {
            tables: Vec::new(),
            vtable: &HELLO_TABLE as *const GreeterVtable as *const c_void,
            vtable_size: size_of::<GreeterVtable>(),
            ..view.clone()
        };
        assert_eq!(
            Remote::<dyn Greeter>::from_view(&legacy).unwrap_err(),
            KindMismatch::NoTable
        );
    }

    /// Three providers claiming one kind: one available, one refusing,
    /// one with a short table. `offers` lists the first two in registry
    /// order; `mismatches` holds the third.
    #[cfg(feature = "load")]
    #[test]
    fn a_registry_offers_every_valid_provider_and_reports_the_rest() {
        use super::super::raw::{LibraryView, Opened};
        use super::super::registry::Registry;
        use std::path::Path;

        unsafe extern "C" fn refuses(_ctx: *mut c_void, reason: *mut Str) -> bool {
            if !reason.is_null() {
                // SAFETY: the test passes writable storage.
                unsafe { reason.write(Str::new("not today")) };
            }
            false
        }

        let ctx = (&HELLO as *const Hello).cast_mut().cast::<c_void>();
        let table = &HELLO_TABLE as *const GreeterVtable as *const c_void;
        let provider = |id: &str, size: usize, available| ProviderView {
            kinds: vec!["greeter".to_string()],
            id: id.to_string(),
            display_name: String::new(),
            config: None,
            vtable: std::ptr::null(),
            vtable_size: 0,
            ctx,
            meta: None,
            version: None,
            available,
            raw: std::ptr::null(),
            create: None,
            destroy: None,
            tables: vec![("greeter".to_string(), table, size)],
        };

        let mut registry = Registry::new("kind-tests", "1.0");
        registry
            .absorb(
                Path::new("kinds.so"),
                Opened::Loaded(
                    LibraryView {
                        id: "kinds".to_string(),
                        version: "1.0.0".to_string(),
                        meta: None,
                        unload: None,
                        providers: vec![
                            provider("kinds_short", GREET_END - 1, None),
                            provider("kinds_refuses", size_of::<GreeterVtable>(), Some(refuses)),
                            provider("kinds_ok", size_of::<GreeterVtable>(), None),
                        ],
                    },
                    super::super::raw::Origin::Linked,
                ),
            )
            .unwrap();

        let offers: Vec<Offer<dyn Greeter>> = registry.offers::<dyn Greeter>().collect();
        let ids: Vec<&str> = offers.iter().map(Offer::id).collect();
        assert_eq!(
            ids,
            ["kinds_ok", "kinds_refuses"],
            "registry order, both offered"
        );
        assert_eq!(offers[0].available(), Ok(()));
        assert_eq!(offers[1].available(), Err("not today"));
        assert_eq!(offers[0].greet("ana").unwrap(), "hello, ana");
        assert_eq!(offers[0].library(), Some("kinds"));
        assert_eq!(offers[0].key(), Some("kinds_ok"));

        let mismatches: Vec<(&str, KindMismatch)> = registry
            .mismatches::<dyn Greeter>()
            .map(|(p, why)| (p.id(), why))
            .collect();
        assert_eq!(
            mismatches,
            [(
                "kinds_short",
                KindMismatch::BelowFloor {
                    size: GREET_END - 1,
                    floor: GREET_END
                }
            )]
        );

        let one = registry
            .offer::<dyn Greeter>("kinds_ok")
            .expect("filed under that key")
            .expect("valid");
        assert_eq!(one.greet("bo").unwrap(), "hello, bo");
        assert!(registry.offer::<dyn Greeter>("nobody").is_none());
        let kept: Box<dyn Greeter> = one.boxed();
        assert_eq!(kept.shout(), 11);
    }
}
