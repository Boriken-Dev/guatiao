// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a generated shim calls: reading its arguments, writing its
//! answers and errors, and catching a panic at the boundary.

use super::*;
use crate::value::alloc::Allocator;
use crate::value::types::Tag;

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
    if m.is_null() {
        return Err(Status::GUATIAO_ERR_NULL);
    }
    // SAFETY: the caller's contract; reached by pointer, not through a
    // value's door, so `from_ptr` checks the keys.
    Ok(unsafe { Map::from_ptr(m) }?)
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

/// Runs a shim body that answers nothing, such as a `destroy` slot, so a
/// panic stops at the boundary rather than unwinding into the caller.
#[doc(hidden)]
pub fn catch_unit(body: impl FnOnce()) {
    crate::exports::guard_with((), body)
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
/// derive's `into_object` boxed, past its table. `None` for null. What a
/// `&mut self` slot reads.
///
/// # Safety
///
/// `ctx` is null or addresses an `ObjectCell<V, T>` whose value nothing
/// else reaches for `'a` — which an [`Object`] guarantees by being the one
/// handle and reaching a `&mut self` slot only through `&mut` to it.
#[doc(hidden)]
pub unsafe fn object_mut<'a, V: 'a, T: 'a>(ctx: *mut c_void) -> Option<&'a mut T> {
    let cell = ctx.cast::<ObjectCell<V, T>>();
    if cell.is_null() {
        return None;
    }
    // SAFETY: the caller's contract; only the value is borrowed, not the
    // table beside it.
    Some(unsafe { &mut (*cell).value })
}

/// The same, shared: what a `&self` slot reads.
///
/// # Safety
///
/// `ctx` is null or addresses an `ObjectCell<V, T>` whose value nothing
/// reaches mutably for `'a`, and from one thread at a time unless `T` is
/// `Sync` — which an [`Object`] guarantees by reaching a `&self` slot
/// through a shared borrow, and being `Sync` only for a `Sync` kind.
#[doc(hidden)]
pub unsafe fn object_ref<'a, V: 'a, T: 'a>(ctx: *const c_void) -> Option<&'a T> {
    let cell = ctx.cast::<ObjectCell<V, T>>();
    if cell.is_null() {
        return None;
    }
    // SAFETY: the caller's contract; only the value is borrowed.
    Some(unsafe { &(*cell).value })
}

/// Releases the cell [`Object::from_cell`] took. Null is a no-op.
///
/// # Safety
///
/// `ctx` is null or came from `from_cell` over an `ObjectCell<V, T>` and
/// is not used again.
#[doc(hidden)]
pub unsafe fn destroy_object<V, T>(ctx: *mut c_void) {
    if ctx.is_null() {
        return;
    }
    // SAFETY: the caller's contract.
    drop(unsafe { Box::from_raw(ctx.cast::<ObjectCell<V, T>>()) });
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

/// A `&mut dyn K` argument: the lent object, validated for `K`, or the
/// error a shim answers. Nothing is owned, so nothing is destroyed.
///
/// # Safety
///
/// `raw` is null, or addresses an [`ObjectRaw`] as
/// [`BorrowedMut::from_raw`] takes one, for `'a`.
#[doc(hidden)]
pub unsafe fn borrowed_mut_arg<'a, K: ?Sized + Kind>(
    raw: *const ObjectRaw,
) -> Result<BorrowedMut<'a, K>, ProviderError> {
    // SAFETY: the caller's contract.
    let raw = unsafe { raw.as_ref() }
        .ok_or_else(|| ProviderError::new(Status::GUATIAO_ERR_NULL, "no object was lent"))?;
    // SAFETY: the caller's contract.
    unsafe { BorrowedMut::from_raw(raw) }
        .map_err(|why| ProviderError::new(Status::GUATIAO_ERR_WRONG_KIND, &why.to_string()))
}

/// A `&dyn K` argument, as [`borrowed_mut_arg`].
///
/// # Safety
///
/// `raw` is null, or addresses an [`ObjectRaw`] as [`Borrowed::from_raw`]
/// takes one, for `'a`.
#[doc(hidden)]
pub unsafe fn borrowed_arg<'a, K: ?Sized + Kind>(
    raw: *const ObjectRaw,
) -> Result<Borrowed<'a, K>, ProviderError> {
    // SAFETY: the caller's contract.
    let raw = unsafe { raw.as_ref() }
        .ok_or_else(|| ProviderError::new(Status::GUATIAO_ERR_NULL, "no object was lent"))?;
    // SAFETY: the caller's contract.
    unsafe { Borrowed::from_raw(raw) }
        .map_err(|why| ProviderError::new(Status::GUATIAO_ERR_WRONG_KIND, &why.to_string()))
}

/// The lender's object behind a lent table's `ctx`, mutably: `ctx`
/// addresses the `*mut K` [`lend_mut`] was given. `None` for null. What a
/// `&mut self` slot of the `&mut` lent table reads.
///
/// # Safety
///
/// `ctx` is null or addresses a `*mut K` that is null or points at a
/// live `K` this call has the only reference to for `'a`.
#[doc(hidden)]
pub unsafe fn lent_mut<'a, K: ?Sized>(ctx: *mut c_void) -> Option<&'a mut K> {
    // SAFETY: the caller's contract.
    let object = unsafe { ctx.cast::<*mut K>().as_ref() }?;
    // SAFETY: the caller's contract.
    unsafe { object.as_mut() }
}

/// The same, shared: what a `&self` slot of either lent table reads.
/// `ctx` addresses a `*const K`, or the `*mut K` [`lend_mut`] was given,
/// read as one.
///
/// # Safety
///
/// `ctx` is null or addresses such a pointer that is null or points at a
/// live `K` nothing reaches mutably for `'a`, and from one thread at a
/// time unless `K` is `Sync`.
#[doc(hidden)]
pub unsafe fn lent_ref<'a, K: ?Sized>(ctx: *mut c_void) -> Option<&'a K> {
    // SAFETY: the caller's contract.
    let object = unsafe { ctx.cast::<*const K>().as_ref() }?;
    // SAFETY: the caller's contract.
    unsafe { object.as_ref() }
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

/// An object that crossed as a return from a call through `from`,
/// validated for `K` and holding what `from` holds on its library, or the
/// error a proxy hands back.
///
/// # Safety
///
/// As [`Object::from_raw`].
#[doc(hidden)]
pub unsafe fn object_ret<K: ?Sized + Kind, R: ?Sized>(
    raw: ObjectRaw,
    from: &Remote<R>,
) -> Result<Object<K>, ProviderError> {
    // SAFETY: forwarded.
    unsafe { object_arg::<K>(raw) }.map(|object| object.held_by(from))
}

/// The error a proxy hands back: what the shim wrote, or one built from
/// the status when it wrote nothing (an older library). Its message is
/// moved into this side's allocator, as [`value_ret`] moves a value.
#[doc(hidden)]
///
/// A message that is not UTF-8 is dropped, keeping the status: the shim
/// wrote the text itself, past the checks a value's doors make.
pub fn take_err(written: ProviderError, status: Status) -> ProviderError {
    if written.status == Status::GUATIAO_OK {
        return ProviderError::from(status);
    }
    // SAFETY: a live local, written by the shim.
    if unsafe { Text::from_ptr(&written.message) }.is_err() {
        return ProviderError::from(written.status);
    }
    let home = Alloc::rust();
    if written
        .message
        .alloc()
        .is_ok_and(|a| a.as_raw() == home.as_raw())
    {
        return written;
    }
    match written.message.clone_in(home) {
        Ok(message) => ProviderError {
            status: written.status,
            message,
        },
        Err(_) => ProviderError::from(written.status),
    }
}

/// A text a shim wrote through a `*mut Text`, checked: the callee writes
/// the storage itself, past the checks a value's doors make.
#[doc(hidden)]
pub fn text_ret(out: Text) -> Result<Text, ProviderError> {
    // SAFETY: a live local, written by the shim.
    unsafe { Text::from_ptr(&out) }?;
    Ok(out)
}

// --- the caller's allocator, carried in the out slot ----------------------
//
// Before a call that answers a tree, the proxy writes into the out slot a
// carrier: an empty container recording the allocator it wants the answer
// in, which allocates nothing. The shim reads it with `return_alloc` and
// builds there; whatever it writes replaces the carrier, which owns
// nothing. A library that ignores it is copied as before, and a caller
// that writes none leaves the shim building in its own allocator.

/// What a tree return crosses in: [`Value`], [`Map`] or [`List`].
#[doc(hidden)]
pub trait Carrier: Sized {
    /// An empty one recording `alloc`, allocating nothing: what a proxy
    /// writes into the out slot before the call.
    fn carrier(alloc: Alloc) -> Self;
    /// The allocator an empty one records, or null for anything else.
    fn carried(&self) -> *const Allocator;
}

/// A map or list: empty, owning no buffer, and recording an allocator.
fn carried_by(len: usize, cap: usize, alloc: *const Allocator) -> *const Allocator {
    if len == 0 && cap == 0 {
        alloc
    } else {
        core::ptr::null()
    }
}

impl Carrier for Map {
    fn carrier(alloc: Alloc) -> Map {
        Map::new_in(alloc)
    }
    fn carried(&self) -> *const Allocator {
        carried_by(self.len, self.cap, self.alloc)
    }
}

impl Carrier for List {
    fn carrier(alloc: Alloc) -> List {
        List::new_in(alloc)
    }
    fn carried(&self) -> *const Allocator {
        carried_by(self.len, self.cap, self.alloc)
    }
}

/// A value carries its allocator as ABSENT whose map arm records it, so a
/// library that writes nothing still answers ABSENT. An empty map or list
/// is read as a carrier too.
impl Carrier for Value {
    fn carrier(alloc: Alloc) -> Value {
        let mut value = Value::absent();
        // SAFETY: ABSENT owns nothing whatever its payload holds; the map
        // arm is the one `blank` wrote, so this sets a field of a live arm.
        unsafe { (*value.payload.map).alloc = alloc.as_raw() };
        value
    }
    fn carried(&self) -> *const Allocator {
        // SAFETY: every arm is the same 32 bytes of plain data, which a
        // node built here or by a caller initialises whole, and the map
        // arm's fields are read as numbers and an address, never followed.
        let map = unsafe { &*self.payload.map };
        match self.tag {
            t if t == u32::from(Tag::GUATIAO_ABSENT) => map.alloc,
            t if t == u32::from(Tag::GUATIAO_MAP) || t == u32::from(Tag::GUATIAO_LIST) => {
                carried_by(map.len, map.cap, map.alloc)
            }
            _ => core::ptr::null(),
        }
    }
}

/// The allocator a shim builds its answer in: the one the caller's
/// carrier names, or this side's own when `out` is null, holds no
/// carrier, or names an allocator that does not check out.
///
/// # Safety
///
/// `out` is null or addresses an initialised `C` — a carrier, or anything
/// the caller wrote there — and an allocator a carrier names outlives
/// what is built through it.
#[doc(hidden)]
pub unsafe fn return_alloc<C: Carrier>(out: *const C) -> Alloc {
    // SAFETY: the caller's contract.
    let Some(out) = (unsafe { out.as_ref() }) else {
        return Alloc::rust();
    };
    let raw = out.carried();
    if raw.is_null() {
        return Alloc::rust();
    }
    // SAFETY: a carrier's allocator outlives what is built through it, by
    // the caller's contract; `from_raw` checks the rest.
    unsafe { Alloc::from_raw(raw) }.unwrap_or_else(|_| Alloc::rust())
}

/// A map a shim wrote through a `*mut Map`, its keys checked as
/// [`text_ret`] checks a text, and moved into `home` as [`value_ret`]
/// moves a value.
#[doc(hidden)]
pub fn map_ret(out: Map, home: Alloc) -> Result<Map, ProviderError> {
    // SAFETY: a live local, written by the shim.
    unsafe { Map::from_ptr(&out) }?;
    if crate::value::walk::map_recorded_by(&out, home.as_raw()) {
        return Ok(out);
    }
    // The original is freed here, through the allocator that made it,
    // while the library that made it is still leased by the caller.
    Ok(out.clone_in(home)?)
}

/// A value a call returned, as the caller gets it: unchanged when every
/// container in it records `home`, the allocator the caller carried in,
/// otherwise copied into `home` and the original freed through its own.
/// So nothing a host holds needs a library's allocator, or its image,
/// after the call.
#[doc(hidden)]
pub fn value_ret(out: Value, home: Alloc) -> Result<Value, ProviderError> {
    if crate::value::walk::recorded_by(&out, home.as_raw()) {
        return Ok(out);
    }
    Ok(out.clone_in(home)?)
}

/// [`value_ret`] for a list.
#[doc(hidden)]
pub fn list_ret(out: List, home: Alloc) -> Result<List, ProviderError> {
    if crate::value::walk::list_recorded_by(&out, home.as_raw()) {
        return Ok(out);
    }
    Ok(out.clone_in(home)?)
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

#[cfg(test)]
mod carrier_tests {
    use super::*;
    use crate::value::alloc::rust_alloc;

    /// An allocator that is not this side's own: Rust's heap, under
    /// another address.
    fn other(vtable: &Allocator) -> Alloc {
        // SAFETY: a complete vtable on the caller's stack, outliving the test.
        unsafe { Alloc::from_raw(vtable) }.expect("complete")
    }

    fn named(found: Alloc, want: Alloc) -> bool {
        found.as_raw() == want.as_raw()
    }

    #[test]
    fn each_carrier_names_its_allocator_and_owns_nothing() {
        let vtable = rust_alloc();
        let theirs = other(&vtable);
        let value = Value::carrier(theirs);
        assert_eq!(
            value.tag(),
            Ok(Tag::GUATIAO_ABSENT),
            "a value carrier is ABSENT"
        );
        let map = Map::carrier(theirs);
        let list = List::carrier(theirs);
        // SAFETY: live locals.
        unsafe {
            assert!(named(return_alloc(&value), theirs));
            assert!(named(return_alloc(&map), theirs));
            assert!(named(return_alloc(&list), theirs));
        }
        assert_eq!((map.capacity(), list.capacity()), (0, 0));
        assert!(
            value == Value::absent(),
            "an ABSENT carrier reads as ABSENT"
        );
    }

    #[test]
    fn an_empty_map_or_list_value_carries_too() {
        let vtable = rust_alloc();
        let theirs = other(&vtable);
        let as_map = Value::from(Map::new_in(theirs));
        let as_list = Value::from(List::new_in(theirs));
        // SAFETY: live locals.
        unsafe {
            assert!(named(return_alloc(&as_map), theirs));
            assert!(named(return_alloc(&as_list), theirs));
        }
    }

    /// An older host passes ABSENT, a zeroed container or nothing: the
    /// shim builds in its own allocator, as it did before carriers.
    #[test]
    fn no_carrier_falls_back_to_this_sides_allocator() {
        let ours = Alloc::rust();
        let zeroed_map = Map {
            ptr: core::ptr::null_mut(),
            len: 0,
            cap: 0,
            alloc: core::ptr::null(),
        };
        // SAFETY: null, or live locals.
        unsafe {
            assert!(named(return_alloc(core::ptr::null::<Value>()), ours));
            assert!(named(return_alloc(&Value::absent()), ours));
            assert!(named(return_alloc(&Value::null()), ours));
            assert!(named(return_alloc(&Value::from(5i64)), ours));
            assert!(named(return_alloc(&zeroed_map), ours));
        }
        core::mem::forget(zeroed_map);
    }

    /// A container that holds something is not a carrier, and an
    /// allocator that does not check out is not used.
    #[test]
    fn a_full_container_or_a_broken_allocator_is_not_a_carrier() {
        let vtable = rust_alloc();
        let theirs = other(&vtable);
        let mut full = Map::new_in(theirs);
        full.set_in("k", 1i64, theirs).expect("stored");
        let mut broken = rust_alloc();
        broken.struct_size = 0;
        let ours = Alloc::rust();
        let mut carrier = Map::new_in(theirs);
        carrier.alloc = &broken;
        // SAFETY: live locals.
        unsafe {
            assert!(named(return_alloc(&full), ours));
            assert!(named(return_alloc(&Value::from(full)), ours));
            assert!(named(return_alloc(&carrier), ours));
        }
    }

    /// A tree already in the carried allocator is handed over as it is;
    /// one built elsewhere is copied there.
    #[test]
    fn a_tree_in_the_carried_allocator_is_not_copied() {
        let vtable = rust_alloc();
        let theirs = other(&vtable);
        let mut map = Map::new_in(theirs);
        map.set_in("k", Text::new_in(theirs, "v").expect("built"), theirs)
            .expect("stored");
        let buffer = map.ptr;
        let same = map_ret(map, theirs).expect("checked");
        assert_eq!(same.ptr, buffer, "handed over, not copied");

        let mut built = Map::new();
        built.set("k", "v");
        let buffer = built.ptr;
        let moved = map_ret(built, theirs).expect("copied");
        assert_ne!(moved.ptr, buffer);
        assert!(named(moved.alloc().expect("recorded"), theirs));

        let mut list = List::new_in(theirs);
        list.push_in(true, theirs).expect("pushed");
        let buffer = list.ptr;
        assert_eq!(list_ret(list, theirs).expect("kept").ptr, buffer);
        let value = Value::from(Text::new_in(theirs, "x").expect("built"));
        assert!(named(
            value_ret(value, theirs)
                .expect("kept")
                .alloc()
                .expect("text"),
            theirs
        ));
    }
}
