// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a generated shim calls: reading its arguments, writing its
//! answers and errors, and catching a panic at the boundary.

use super::*;

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
    // SAFETY: a live local, written by the shim.
    } else if unsafe { Text::from_ptr(&written.message) }.is_err() {
        ProviderError::from(written.status)
    } else {
        written
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

/// A map a shim wrote through a `*mut Map`, its keys checked as
/// [`text_ret`] checks a text.
#[doc(hidden)]
pub fn map_ret(out: Map) -> Result<Map, ProviderError> {
    // SAFETY: a live local, written by the shim.
    unsafe { Map::from_ptr(&out) }?;
    Ok(out)
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
