// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Building and freeing a tree, from C.
//!
//! Reading needs none of these: a value is a plain struct and the
//! header's `static inline` helpers walk one with no call into any
//! library. These exist because building and freeing a tree needs code,
//! and a C or Dart caller has to reach that code somehow.
//!
//! The discipline every function here holds — the null checks, the
//! `catch_unwind`, the forgotten payload — is described once on the
//! module above.

#![allow(non_camel_case_types)]

use crate::value::convert::{TryAsMut, TryAsRef};
use std::ptr;

use super::{as_str, entry, out};

use crate::value::alloc::{Alloc, Allocator};
use crate::value::status::Status;
use crate::value::types::{Buffer, Bytes, List, Map, Number, Str, Tag, Text, Value};

/// # Safety
///
/// `b` is a view whose `len` bytes are readable for the call.
unsafe fn as_bytes<'a>(b: Bytes) -> Result<&'a [u8], Status> {
    if b.len == 0 {
        return Ok(&[]);
    }
    if b.ptr.is_null() {
        return Err(Status::GUATIAO_ERR_NULL);
    }
    // SAFETY: as above.
    Ok(unsafe { std::slice::from_raw_parts(b.ptr, b.len) })
}

// --- lifecycle --------------------------------------------------------

/// Frees everything `v` owns and leaves it null-tagged.
///
/// Safe to call twice, and safe on a value that owns nothing.
///
/// # Safety
///
/// `v` is null, or addresses a well-formed value nothing else is freeing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_value_free(v: *mut Value) -> Status {
    entry!(v => {
        // SAFETY: checked non-null; the caller guarantees it is
        // well-formed and unaliased, which is what `free` asks for.
        unsafe { (*v).free() };
        Status::GUATIAO_OK
    })
}

/// Deep-copies `src` into `alloc`, writing the copy through `out`.
///
/// `out` is written the absent marker on entry, so a failed call leaves
/// it ABSENT rather than untouched. A null or unusable allocator is
/// `GUATIAO_ERR_ALLOC`.
///
/// # Safety
///
/// The pointers are null or valid, and `out` addresses writable storage
/// for one value that does not already hold one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_value_clone(
    alloc: *const Allocator,
    src: *const Value,
    out: *mut Value,
) -> Status {
    out!(out);
    entry!(src, out => {
        // SAFETY: checked non-null.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: checked non-null and well-formed by contract, which is
        // what the dereference needs; `clone_in` itself is safe.
        match unsafe { (*src).clone_in(a) } {
            Ok(v) => {
                // SAFETY: `out` is writable and does not already hold a
                // value the caller still owns.
                unsafe { ptr::write(out, v) };
                Status::GUATIAO_OK
            }
            Err(e) => e.into(),
        }
    })
}

// --- constructors -----------------------------------------------------
//
// Written out one by one rather than through a macro, and the reason is
// not taste: cbindgen reads SOURCE, not macro expansions, so a
// macro-generated `#[unsafe(no_mangle)]` function is invisible to it and
// silently absent from the header. The duplication here is what keeps the
// C surface complete.

/// Writes the stored nothing through `out`.
///
/// # Safety
///
/// `out` addresses writable storage for one value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_value_null(out: *mut Value) -> Status {
    entry!(out => {
        // SAFETY: checked non-null and writable by contract.
        unsafe { ptr::write(out, Value::null()) };
        Status::GUATIAO_OK
    })
}

/// Writes the absent marker through `out` -- the answer to a lookup that
/// found nothing, and the tag of an optional slot left empty.
///
/// # Safety
///
/// `out` addresses writable storage for one value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_value_absent(out: *mut Value) -> Status {
    entry!(out => {
        // SAFETY: checked non-null and writable by contract.
        unsafe { ptr::write(out, Value::absent()) };
        Status::GUATIAO_OK
    })
}

/// Writes a boolean through `out`. Any non-zero `b` is true.
///
/// # Why this takes a byte and not a `bool`
///
/// A Rust `bool` must be 0 or 1, and a value outside that is undefined
/// behaviour **at the moment it arrives**, before this function's body
/// runs and before anything could reject it. A C caller can produce one
/// without trying: through a cast, a union, or an uninitialised local.
///
/// So the boundary takes the primitive and this crate decides, which is
/// the same rule the tag follows. It costs a C caller nothing —
/// `guatiao_value_bool(true, &v)` still compiles and still means true.
///
/// # Safety
///
/// `out` addresses writable storage for one value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_value_bool(b: u8, out: *mut Value) -> Status {
    entry!(out => {
        // SAFETY: checked non-null and writable by contract.
        unsafe { ptr::write(out, Value::from(b != 0)) };
        Status::GUATIAO_OK
    })
}

/// Writes an empty map through `out`, to be grown through `alloc`.
///
/// A null or unusable allocator is `GUATIAO_ERR_ALLOC`.
///
/// # Safety
///
/// The pointers are null or valid, and `out` addresses writable storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_value_map(alloc: *const Allocator, out: *mut Value) -> Status {
    entry!(out => {
        // SAFETY: null or valid by the caller's contract; `from_raw`
        // answers `Null` for a null one rather than reading it.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: checked non-null and writable by contract.
        unsafe { ptr::write(out, Map::new_in(a).into()) };
        Status::GUATIAO_OK
    })
}

/// Writes an empty list through `out`, to be grown through `alloc`.
///
/// A null or unusable allocator is `GUATIAO_ERR_ALLOC`.
///
/// # Safety
///
/// The pointers are null or valid, and `out` addresses writable storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_value_list(alloc: *const Allocator, out: *mut Value) -> Status {
    entry!(out => {
        // SAFETY: null or valid by the caller's contract; `from_raw`
        // answers `Null` for a null one rather than reading it.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: checked non-null and writable by contract.
        unsafe { ptr::write(out, List::new_in(a).into()) };
        Status::GUATIAO_OK
    })
}

/// Copies `text` into a new string value, through `out`.
///
/// Refuses anything that is not UTF-8: a string is UTF-8 by contract, so a
/// reader can hand back text without re-checking at every access. Content
/// that is not text belongs in `guatiao_value_bytes`.
///
/// # Safety
///
/// The pointers are null or valid, the view's bytes are readable for the
/// call, and `out` addresses writable storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_value_string(
    alloc: *const Allocator,
    text: Str,
    out: *mut Value,
) -> Status {
    entry!(out => {
        // SAFETY: null or valid by the caller's contract; `from_raw`
        // answers `Null` for a null one rather than reading it.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: the caller guarantees the view's bytes.
        let t = match unsafe { as_str(text) } { Ok(t) => t, Err(s) => return s };
        match Text::new_in(a, t).map(Value::from) {
            // SAFETY: checked non-null and writable by contract.
            Ok(v) => { unsafe { ptr::write(out, v) }; Status::GUATIAO_OK }
            Err(e) => e.into(),
        }
    })
}

/// Copies `text` into a new number value, through `out`, storing it
/// **verbatim**.
///
/// Refuses anything outside the JSON number grammar, at construction
/// rather than at read.
///
/// # Safety
///
/// The pointers are null or valid, the view's bytes are readable for the
/// call, and `out` addresses writable storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_value_number(
    alloc: *const Allocator,
    text: Str,
    out: *mut Value,
) -> Status {
    entry!(out => {
        // SAFETY: null or valid by the caller's contract; `from_raw`
        // answers `Null` for a null one rather than reading it.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: the caller guarantees the view's bytes.
        let t = match unsafe { as_str(text) } { Ok(t) => t, Err(s) => return s };
        match Number::new_in(a, t).map(Value::from) {
            // SAFETY: checked non-null and writable by contract.
            Ok(v) => { unsafe { ptr::write(out, v) }; Status::GUATIAO_OK }
            Err(e) => e.into(),
        }
    })
}

/// Copies `bytes` into a new bytes value, through `out`. Any content at
/// all, NULs included.
///
/// # Safety
///
/// The pointers are null or valid, the view's bytes are readable for the
/// call, and `out` addresses writable storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_value_bytes(
    alloc: *const Allocator,
    bytes: Bytes,
    out: *mut Value,
) -> Status {
    entry!(out => {
        // SAFETY: null or valid by the caller's contract; `from_raw`
        // answers `Null` for a null one rather than reading it.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: the caller guarantees the view's bytes.
        let b = match unsafe { as_bytes(bytes) } { Ok(b) => b, Err(s) => return s };
        match Buffer::new_in(a, b).map(Value::from) {
            // SAFETY: checked non-null and writable by contract.
            Ok(v) => { unsafe { ptr::write(out, v) }; Status::GUATIAO_OK }
            Err(e) => e.into(),
        }
    })
}

// --- map --------------------------------------------------------------

/// Stores `value` under `key`, **moving** it and leaving it null-tagged.
///
/// Replaces any existing entry in place, keeping its position. To store
/// something you only borrowed, call `guatiao_value_clone` first and pass
/// the copy: there is no copying variant, so the cost is always a line you
/// can see.
///
/// **`node` and `value` must not overlap**, and the same pointer for both
/// is `GUATIAO_ERR_BAD_VALUE`: this moves the 40 bytes at `value` into
/// `node`, which cannot be a slot inside the tree it is moving. A null or
/// unusable allocator is `GUATIAO_ERR_ALLOC`.
///
/// # Safety
///
/// The pointers are null or valid, the key's bytes are readable, and
/// `value` does not address a node inside `node`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_map_set(
    alloc: *const Allocator,
    node: *mut Value,
    key: Str,
    value: *mut Value,
) -> Status {
    entry!(node, value => {
        // The one overlap that can be checked. A `value` addressing some
        // deeper node inside `node` cannot be, which is why the contract
        // above says it and the header repeats it.
        if std::ptr::eq(node.cast_const(), value.cast_const()) {
            return Status::GUATIAO_ERR_BAD_VALUE;
        }
        // SAFETY: null or valid by the caller's contract.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: the caller guarantees the key's bytes.
        let k = match unsafe { as_str(key) } { Ok(k) => k, Err(s) => return s };
        // SAFETY: checked non-null and well-formed by contract; the
        // dereference is the only unsafety left.
        match unsafe { TryAsMut::<Map>::try_as_mut(&mut *node) } {
            // SAFETY: `value` is a well-formed node the caller hands
            // over; taking it leaves the caller's null-tagged, which is
            // what the contract promises.
            Some(m) => match m.set_in(k, unsafe { std::mem::take(&mut *value) }, a) {
                Ok(()) => Status::GUATIAO_OK,
                Err(e) => e.into(),
            },
            None => Status::GUATIAO_ERR_WRONG_KIND,
        }
    })
}

/// Removes `key` and frees its value. `GUATIAO_ERR_NOT_FOUND` if absent.
///
/// # Safety
///
/// The pointers are null or valid, and the key's bytes are readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_map_discard(node: *mut Value, key: Str) -> Status {
    entry!(node => {
        // SAFETY: the caller guarantees the key's bytes.
        let k = match unsafe { as_str(key) } { Ok(k) => k, Err(s) => return s };
        // SAFETY: checked non-null and well-formed by contract; the
        // dereference is the only unsafety left.
        if unsafe { TryAsMut::<Map>::try_as_mut(&mut *node).is_some_and(|m| m.discard(k)) } {
            Status::GUATIAO_OK
        } else {
            Status::GUATIAO_ERR_NOT_FOUND
        }
    })
}

/// Frees every entry and empties the map, keeping its capacity.
///
/// # Safety
///
/// `node` is null or a well-formed map.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_map_clear(node: *mut Value) -> Status {
    entry!(node => {
        // Through the container and not `Value::clear`, which empties a
        // list just as willingly: this symbol is the map one, and a list
        // handed to it must still come back `GUATIAO_ERR_WRONG_KIND`.
        //
        // SAFETY: checked non-null and well-formed by contract; the
        // dereference is the only unsafety left.
        match unsafe { TryAsMut::<Map>::try_as_mut(&mut *node) } {
            Some(m) => {
                m.clear();
                Status::GUATIAO_OK
            }
            None => Status::GUATIAO_ERR_WRONG_KIND,
        }
    })
}

/// Copies every entry of `src` into `dst`, replacing collisions.
///
/// **Use this before rebuilding a record**, or every field you do not
/// model is dropped on write-back.
///
/// **`dst` and `src` must not overlap**, and the same pointer for both is
/// `GUATIAO_ERR_BAD_VALUE`. `src` may be a node stored inside `dst`: the
/// source is copied whole before `dst` is touched. A null or unusable
/// allocator is `GUATIAO_ERR_ALLOC`.
///
/// Not atomic: a failure part-way leaves the entries already copied in
/// place.
///
/// # Safety
///
/// The pointers are null or valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_map_copy_from(
    alloc: *const Allocator,
    dst: *mut Value,
    src: *const Value,
) -> Status {
    entry!(dst, src => {
        // Copying a map onto itself is a caller mistake rather than a
        // no-op: every key would replace itself with a copy of itself.
        if std::ptr::eq(dst.cast_const(), src) {
            return Status::GUATIAO_ERR_BAD_VALUE;
        }
        // SAFETY: null or valid by the caller's contract.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: checked non-null and well-formed by contract.
        if unsafe { (*dst).tag() } != Ok(Tag::GUATIAO_MAP) {
            return Status::GUATIAO_ERR_WRONG_KIND;
        }
        // The whole source is copied BEFORE `dst` is touched, so `src`
        // may be a node stored inside `dst`.
        //
        // SAFETY: as above; the shared borrow ends with the copy.
        let copy = match unsafe { TryAsRef::<Map>::try_as_ref(&*src) } {
            Some(m) => match m.clone_in(a) {
                Ok(c) => c,
                Err(e) => return e.into(),
            },
            None => return Status::GUATIAO_ERR_WRONG_KIND,
        };
        // SAFETY: as above.
        match unsafe { TryAsMut::<Map>::try_as_mut(&mut *dst) } {
            Some(m) => match m.absorb(copy, a) {
                Ok(_) => Status::GUATIAO_OK,
                Err(e) => e.into(),
            },
            None => Status::GUATIAO_ERR_WRONG_KIND,
        }
    })
}

// --- list -------------------------------------------------------------

/// Appends `value`, **moving** it and leaving it null-tagged.
///
/// To append something you only borrowed, call `guatiao_value_clone`
/// first and pass the copy.
///
/// **`node` and `value` must not overlap**, and the same pointer for both
/// is `GUATIAO_ERR_BAD_VALUE`, as for `guatiao_map_set`. A null or
/// unusable allocator is `GUATIAO_ERR_ALLOC`.
///
/// # Safety
///
/// The pointers are null or valid, and `value` does not address a node
/// inside `node`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_list_push(
    alloc: *const Allocator,
    node: *mut Value,
    value: *mut Value,
) -> Status {
    entry!(node, value => {
        // As in `guatiao_map_set`: the one overlap that can be checked.
        if std::ptr::eq(node.cast_const(), value.cast_const()) {
            return Status::GUATIAO_ERR_BAD_VALUE;
        }
        // SAFETY: null or valid by the caller's contract.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: checked non-null and well-formed by contract; the
        // dereference is the only unsafety left.
        match unsafe { TryAsMut::<List>::try_as_mut(&mut *node) } {
            // SAFETY: as in `guatiao_map_set`.
            Some(l) => match l.push_in(unsafe { std::mem::take(&mut *value) }, a) {
                Ok(()) => Status::GUATIAO_OK,
                Err(e) => e.into(),
            },
            None => Status::GUATIAO_ERR_WRONG_KIND,
        }
    })
}

/// Removes and frees the element at `index`.
///
/// # Safety
///
/// `node` is null or a well-formed list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_list_discard(node: *mut Value, index: usize) -> Status {
    entry!(node => {
        // SAFETY: checked non-null and well-formed by contract; the
        // dereference is the only unsafety left.
        if unsafe { TryAsMut::<List>::try_as_mut(&mut *node).is_some_and(|l| l.discard(index)) } {
            Status::GUATIAO_OK
        } else {
            Status::GUATIAO_ERR_NOT_FOUND
        }
    })
}

/// Frees every element and empties the list, keeping its capacity.
///
/// # Safety
///
/// `node` is null or a well-formed list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_list_clear(node: *mut Value) -> Status {
    entry!(node => {
        // Through the container, as in `guatiao_map_clear`: a map handed
        // to the list symbol must still come back
        // `GUATIAO_ERR_WRONG_KIND`.
        //
        // SAFETY: checked non-null and well-formed by contract; the
        // dereference is the only unsafety left.
        match unsafe { TryAsMut::<List>::try_as_mut(&mut *node) } {
            Some(l) => {
                l.clear();
                Status::GUATIAO_OK
            }
            None => Status::GUATIAO_ERR_WRONG_KIND,
        }
    })
}

// --- appending --------------------------------------------------------

/// Appends UTF-8 text to a string value.
///
/// `text` may view the node's own bytes; an overlapping source is copied
/// out first. A null or unusable allocator is `GUATIAO_ERR_ALLOC`.
///
/// # Safety
///
/// The pointers are null or valid and the view's bytes are readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_string_push(
    alloc: *const Allocator,
    node: *mut Value,
    text: Str,
) -> Status {
    entry!(node => {
        // SAFETY: null or valid by the caller's contract.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: the caller guarantees the view's bytes.
        let t = match unsafe { as_str(text) } { Ok(t) => t, Err(s) => return s };
        // A STRING only: `TryAsMut<Text>` refuses a NUMBER, whose digits
        // share the arm and not the type.
        //
        // SAFETY: checked non-null and well-formed by contract.
        match unsafe { TryAsMut::<Text>::try_as_mut(&mut *node) } {
            Some(s) => match s.push_str_in(t, a) {
                Ok(()) => Status::GUATIAO_OK,
                Err(e) => e.into(),
            },
            None => Status::GUATIAO_ERR_WRONG_KIND,
        }
    })
}

/// Appends to a bytes value.
///
/// `bytes` may view the node's own buffer, as in `guatiao_string_push`. A
/// null or unusable allocator is `GUATIAO_ERR_ALLOC`.
///
/// # Safety
///
/// The pointers are null or valid and the view's bytes are readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_buffer_push(
    alloc: *const Allocator,
    node: *mut Value,
    bytes: Bytes,
) -> Status {
    entry!(node => {
        // SAFETY: null or valid by the caller's contract.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: the caller guarantees the view's bytes.
        let b = match unsafe { as_bytes(bytes) } { Ok(b) => b, Err(s) => return s };
        // SAFETY: checked non-null and well-formed by contract.
        match unsafe { TryAsMut::<Buffer>::try_as_mut(&mut *node) } {
            Some(buffer) => match buffer.push_in(b, a) {
                Ok(()) => Status::GUATIAO_OK,
                Err(e) => e.into(),
            },
            None => Status::GUATIAO_ERR_WRONG_KIND,
        }
    })
}

/// The allocator over Rust's global allocator, for a C caller that has no
/// reason to supply its own.
///
/// Writes it through `out`. The caller keeps the struct alive for as long
/// as any tree built through it: containers hold a pointer to it.
///
/// # Safety
///
/// `out` addresses writable storage for one allocator.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_alloc_default(out: *mut Allocator) -> Status {
    entry!(out => {
        // SAFETY: checked non-null and writable by contract.
        unsafe { ptr::write(out, crate::value::alloc::rust_alloc()) };
        Status::GUATIAO_OK
    })
}
