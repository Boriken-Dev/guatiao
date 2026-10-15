// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Objects: a handle one caller owns, its table travelling with it, and
//! the writable byte view an object's method fills.

use super::*;

// --- objects: a handle one caller owns -----------------------------------

/// An object as it crosses the boundary: its table, the table's size, and
/// the context every slot takes. **Ownership crosses with it**: whoever
/// receives one destroys it, through the table's `destroy` slot.
///
/// The out-parameter of a method returning [`Object`], and the argument
/// type of a method taking one. All three fields null or zero is "no
/// object".
///
/// **Lent, it is passed by pointer and ownership stays.** A method taking
/// `&mut dyn K` or `&dyn K` receives a `const guatiao_object *` on the
/// caller's stack, valid for that call only: the callee calls its slots
/// during the call, never keeps the pointer, and **never calls its
/// `destroy`**. A table Rust lends as `&dyn K` answers
/// `GUATIAO_ERR_UNSUPPORTED` from each slot whose method takes `&mut
/// self`.
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
            table: core::ptr::null(),
            size: 0,
            ctx: core::ptr::null_mut(),
        }
    }
}

/// Borrowed **writable** bytes: a pointer and a length. The out-buffer
/// argument an object kind's `&mut [u8]` crosses as.
///
/// Check `len` before `ptr`, as with a bytes view: an empty buffer may carry
/// a null pointer.
#[repr(C)]
#[derive(Debug)]
pub struct BytesMut<'a> {
    /// First byte. May be null when `len` is 0.
    ptr: *mut u8,
    /// Length in bytes.
    len: usize,
    /// What the view borrows, for the compiler; nothing in C.
    _borrows: PhantomData<&'a mut [u8]>,
}

impl<'a> BytesMut<'a> {
    /// A view of bytes this program holds and lends to be written.
    ///
    /// Neither `Clone` nor `Copy`: it converts to a `&mut [u8]`, and a
    /// copy would be a second one.
    ///
    /// ```compile_fail
    /// let mut bytes = [0u8; 4];
    /// let view = guatiao::library::BytesMut::new(&mut bytes);
    /// let copy = view;
    /// let _: &mut [u8] = view.into();
    /// ```
    pub fn new(bytes: &'a mut [u8]) -> BytesMut<'a> {
        BytesMut {
            ptr: bytes.as_mut_ptr(),
            len: bytes.len(),
            _borrows: PhantomData,
        }
    }

    /// A view described by hand.
    ///
    /// # Safety
    ///
    /// `len` is 0, or `ptr` addresses `len` bytes writable for `'a` and
    /// reached by nothing else meanwhile.
    pub const unsafe fn from_raw_parts(ptr: *mut u8, len: usize) -> BytesMut<'a> {
        BytesMut {
            ptr,
            len,
            _borrows: PhantomData,
        }
    }

    /// The first byte. May be null when `len` is 0.
    pub const fn ptr(&self) -> *mut u8 {
        self.ptr
    }

    /// How many bytes.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether it views nothing.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<'a> From<&'a mut [u8]> for BytesMut<'a> {
    fn from(bytes: &'a mut [u8]) -> BytesMut<'a> {
        BytesMut::new(bytes)
    }
}

/// The bytes to write. A null pointer views nothing.
impl<'a> From<BytesMut<'a>> for &'a mut [u8] {
    fn from(view: BytesMut<'a>) -> &'a mut [u8] {
        if view.len == 0 || view.ptr.is_null() {
            return &mut [];
        }
        // SAFETY: the view was made from a `&'a mut`, or by
        // `from_raw_parts` or a foreign caller promising the same, and it
        // is consumed here, so this is the only `&mut` it gives.
        unsafe { core::slice::from_raw_parts_mut(view.ptr, view.len) }
    }
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

impl<V, T> core::fmt::Debug for ObjectCell<V, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
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
pub struct Object<K: ?Sized> {
    remote: Remote<K>,
}

impl<K: ?Sized + Kind> core::fmt::Debug for Object<K> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Object")
            .field("kind", &K::NAME)
            .finish_non_exhaustive()
    }
}

// SAFETY: the object kind's trait names `Send`, and this is the one
// handle to the value; the table addresses a library that is never
// unloaded or the cell the handle owns. Unbounded in `K`, as `Remote` is.
unsafe impl<K: ?Sized> Send for Object<K> {}

impl<K: ?Sized + Kind> Object<K> {
    /// An object from a cell the derive's `into_object` built: the table
    /// is the cell's own, validated for `K`.
    ///
    /// `Err` only for a table that does not fit the kind, which a table
    /// `of::<T>()` built cannot be; the derive `expect`s it.
    ///
    /// # Safety
    ///
    /// `cell.table` is the table `#[guatiao::kind]` emits for an object
    /// kind, built by its `of::<T>()` for this `T`: its shims read the
    /// value past the table as a `T`, and its `destroy` frees the cell as
    /// an `ObjectCell<V, T>`. Validation checks the header, not that.
    pub unsafe fn from_cell<V: 'static, T: 'static>(
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
        core::mem::forget(self);
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

impl<K: ?Sized + Kind> core::ops::Deref for Object<K> {
    type Target = K;

    fn deref(&self) -> &K {
        K::as_dyn(&self.remote)
    }
}

impl<K: ?Sized + Kind> core::ops::DerefMut for Object<K> {
    fn deref_mut(&mut self) -> &mut K {
        K::as_dyn_mut(&mut self.remote)
    }
}

impl<K: ?Sized> Drop for Object<K> {
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

// --- lent objects: a borrow for one call ----------------------------------

/// An object lent for one call as `&dyn K`: what a generated shim hands
/// the method for a `&dyn K` argument. Derefs to the kind's trait.
///
/// **No `Drop`**: the lender owns the object, so nothing here can destroy
/// it. Built from the `const guatiao_object *` the caller passed, which
/// is valid for the call alone, so this does not outlive it.
///
/// A kind method lends an object kind's value with `&mut dyn K`, or with
/// `&dyn K` when the kind has a `&self` method to call:
///
/// ```
/// use guatiao::library::ProviderError;
///
/// #[guatiao::kind]
/// pub trait Tell: Send + Sync {
///     fn tell(&self, to: &mut dyn greeter_kind::Listener) -> Result<(), ProviderError>;
///     fn look(&self, at: &dyn greeter_kind::Conversation) -> Result<i64, ProviderError>;
/// }
/// ```
///
/// `Listener` has only `&mut self` methods, so lending it shared would
/// leave nothing to call, and is refused at compile time:
///
/// ```compile_fail
/// use guatiao::library::ProviderError;
///
/// #[guatiao::kind]
/// pub trait Tell: Send + Sync {
///     fn tell(&self, to: &dyn greeter_kind::Listener) -> Result<(), ProviderError>;
/// }
/// ```
pub struct Borrowed<'a, K: ?Sized> {
    remote: Remote<K>,
    _lent: PhantomData<&'a K>,
}

/// An object lent for one call as `&mut dyn K`: what a generated shim
/// hands the method for a `&mut dyn K` argument. Derefs, mutably, to the
/// kind's trait. **No `Drop`**, as [`Borrowed`].
///
/// Only an object kind is lent; a provider kind is shared and offered,
/// and lending one is refused at compile time:
///
/// ```compile_fail
/// use guatiao::library::ProviderError;
///
/// #[guatiao::kind]
/// pub trait Relay: Send + Sync {
///     fn relay(&self, to: &mut dyn greeter_kind::Greeter) -> Result<(), ProviderError>;
/// }
/// ```
pub struct BorrowedMut<'a, K: ?Sized> {
    remote: Remote<K>,
    _lent: PhantomData<&'a mut K>,
}

impl<K: ?Sized + Kind> core::fmt::Debug for Borrowed<'_, K> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Borrowed")
            .field("kind", &K::NAME)
            .finish_non_exhaustive()
    }
}

impl<K: ?Sized + Kind> core::fmt::Debug for BorrowedMut<'_, K> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BorrowedMut")
            .field("kind", &K::NAME)
            .finish_non_exhaustive()
    }
}

impl<'a, K: ?Sized + Kind> Borrowed<'a, K> {
    /// A lent object, validated for `K`.
    ///
    /// # Safety
    ///
    /// `raw.table` addresses `raw.size` readable bytes that are a table
    /// for the kind named [`K::NAME`](Kind::NAME), laid out as its
    /// `Vtable`, and every slot of a method taking `&self` may be called
    /// with `raw.ctx` from any thread, for `'a`.
    pub unsafe fn from_raw(raw: &'a ObjectRaw) -> Result<Borrowed<'a, K>, KindMismatch> {
        // SAFETY: forwarded.
        let remote = unsafe { Remote::<K>::validate(raw.table, raw.size, raw.ctx, false) }?;
        Ok(Borrowed {
            remote,
            _lent: PhantomData,
        })
    }
}

impl<'a, K: ?Sized + Kind> BorrowedMut<'a, K> {
    /// A lent object, validated for `K`.
    ///
    /// # Safety
    ///
    /// As [`Borrowed::from_raw`], for every slot, and nothing else
    /// reaches the object through `raw.ctx` for `'a`.
    pub unsafe fn from_raw(raw: &'a ObjectRaw) -> Result<BorrowedMut<'a, K>, KindMismatch> {
        // SAFETY: forwarded.
        let remote = unsafe { Remote::<K>::validate(raw.table, raw.size, raw.ctx, false) }?;
        Ok(BorrowedMut {
            remote,
            _lent: PhantomData,
        })
    }
}

impl<K: ?Sized + Kind> core::ops::Deref for Borrowed<'_, K> {
    type Target = K;

    fn deref(&self) -> &K {
        K::as_dyn(&self.remote)
    }
}

impl<K: ?Sized + Kind> core::ops::Deref for BorrowedMut<'_, K> {
    type Target = K;

    fn deref(&self) -> &K {
        K::as_dyn(&self.remote)
    }
}

impl<K: ?Sized + Kind> core::ops::DerefMut for BorrowedMut<'_, K> {
    fn deref_mut(&mut self) -> &mut K {
        K::as_dyn_mut(&mut self.remote)
    }
}

/// The tables a value of a kind is lent through for one call.
/// `#[guatiao::kind]` implements it for every kind it declares, with
/// tables for an object kind and `None` for a provider kind; a lent
/// argument's proxy reads them here, so a hand-written `impl Kind`
/// cannot hand a callee a table of its choosing.
///
/// # Safety
///
/// `LENT_MUT`, when `Some`, is a table for `Self` whose every slot reads
/// its `ctx` as the address of a `*mut Self` live for the call, takes a
/// shared reference for a `&self` method and an exclusive one for a
/// `&mut self` method, and whose `destroy` frees nothing. `LENT`, when
/// `Some`, is the same over a `*const Self`, except that a slot whose
/// method takes `&mut self` touches nothing and refuses.
#[doc(hidden)]
pub unsafe trait Lend: Kind {
    /// The table a value is lent through as `&mut dyn K`.
    const LENT_MUT: Option<&'static Self::Vtable> = None;
    /// The table a value is lent through as `&dyn K`; `None` also for an
    /// object kind with no `&self` method.
    const LENT: Option<&'static Self::Vtable> = None;
}

/// `&mut dyn K` lent for one call, as it crosses: the kind's lent table,
/// whose slots read `slot` as the address of the lender's `*mut dyn K`.
/// A kind with no lent table (a provider kind) gives a null table, which
/// the callee refuses; the kind attribute refuses that shape at compile
/// time first.
///
/// `slot` must outlive the call the result is passed to; the generated
/// proxy keeps both on its stack.
#[doc(hidden)]
pub fn lend_mut<K: ?Sized + Lend>(slot: *mut c_void) -> ObjectRaw {
    lent_raw::<K>(K::LENT_MUT, slot)
}

/// `&dyn K` lent for one call, as [`lend_mut`]: `slot` holds the
/// lender's `*const dyn K`, and the table's `&mut self` slots refuse.
#[doc(hidden)]
pub fn lend<K: ?Sized + Lend>(slot: *mut c_void) -> ObjectRaw {
    lent_raw::<K>(K::LENT, slot)
}

fn lent_raw<K: ?Sized + Kind>(table: Option<&'static K::Vtable>, slot: *mut c_void) -> ObjectRaw {
    match table {
        Some(table) => ObjectRaw {
            table: (table as *const K::Vtable).cast::<c_void>(),
            size: size_of::<K::Vtable>(),
            ctx: slot,
        },
        None => ObjectRaw::null(),
    }
}
