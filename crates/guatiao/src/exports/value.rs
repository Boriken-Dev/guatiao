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

use std::ptr;

use super::{as_str, entry};

use crate::value::alloc::{Alloc, Allocator};
use crate::value::status::Status;
use crate::value::types::{Bytes, Str, Value};

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
    entry!(alloc, src, out => {
        // SAFETY: checked non-null.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: as above; `clone_in` asks only that `src` be well-formed.
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
        unsafe { ptr::write(out, Value::bool(b != 0)) };
        Status::GUATIAO_OK
    })
}

/// Writes an empty map through `out`, to be grown through `alloc`.
///
/// # Safety
///
/// The pointers are null or valid, and `out` addresses writable storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_value_map(alloc: *const Allocator, out: *mut Value) -> Status {
    entry!(alloc, out => {
        // SAFETY: checked non-null.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: checked non-null and writable by contract.
        unsafe { ptr::write(out, Value::map_in(a)) };
        Status::GUATIAO_OK
    })
}

/// Writes an empty list through `out`, to be grown through `alloc`.
///
/// # Safety
///
/// The pointers are null or valid, and `out` addresses writable storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_value_list(alloc: *const Allocator, out: *mut Value) -> Status {
    entry!(alloc, out => {
        // SAFETY: checked non-null.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: checked non-null and writable by contract.
        unsafe { ptr::write(out, Value::list_in(a)) };
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
    entry!(alloc, out => {
        // SAFETY: checked non-null.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: the caller guarantees the view's bytes.
        let t = match unsafe { as_str(text) } { Ok(t) => t, Err(s) => return s };
        match Value::string_in(a, t) {
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
    entry!(alloc, out => {
        // SAFETY: checked non-null.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: the caller guarantees the view's bytes.
        let t = match unsafe { as_str(text) } { Ok(t) => t, Err(s) => return s };
        match Value::number_in(a, t) {
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
    entry!(alloc, out => {
        // SAFETY: checked non-null.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: the caller guarantees the view's bytes.
        let b = match unsafe { as_bytes(bytes) } { Ok(b) => b, Err(s) => return s };
        match Value::bytes_in(a, b) {
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
/// # Safety
///
/// The pointers are null or valid, and the key's bytes are readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_map_set(
    alloc: *const Allocator,
    node: *mut Value,
    key: Str,
    value: *mut Value,
) -> Status {
    entry!(alloc, node, value => {
        // SAFETY: checked non-null.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: the caller guarantees the key's bytes.
        let k = match unsafe { as_str(key) } { Ok(k) => k, Err(s) => return s };
        // SAFETY: checked non-null and well-formed by contract, which is
        // what `set_in` asks for.
        match unsafe { (*node).set_in(k, &mut *value, a) } {
            Ok(()) => Status::GUATIAO_OK,
            Err(e) => e.into(),
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
        if unsafe { (*node).discard(k) } {
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
        match unsafe { (*node).as_map_mut() } {
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
/// # Safety
///
/// The pointers are null or valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_map_copy_from(
    alloc: *const Allocator,
    dst: *mut Value,
    src: *const Value,
) -> Status {
    entry!(alloc, dst, src => {
        // SAFETY: checked non-null.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: checked non-null and well-formed by contract, which is
        // what `copy_from` asks for.
        match unsafe { (*dst).copy_from(&*src, a) } {
            Ok(_) => Status::GUATIAO_OK,
            Err(e) => e.into(),
        }
    })
}

// --- list -------------------------------------------------------------

/// Appends `value`, **moving** it and leaving it null-tagged.
///
/// To append something you only borrowed, call `guatiao_value_clone`
/// first and pass the copy.
///
/// # Safety
///
/// The pointers are null or valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_list_push(
    alloc: *const Allocator,
    node: *mut Value,
    value: *mut Value,
) -> Status {
    entry!(alloc, node, value => {
        // SAFETY: checked non-null.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: checked non-null and well-formed by contract, which is
        // what `push_in` asks for.
        match unsafe { (*node).push_in(&mut *value, a) } {
            Ok(()) => Status::GUATIAO_OK,
            Err(e) => e.into(),
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
        if unsafe { (*node).discard_at(index) } {
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
        match unsafe { (*node).as_list_mut() } {
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
/// # Safety
///
/// The pointers are null or valid and the view's bytes are readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_string_push(
    alloc: *const Allocator,
    node: *mut Value,
    text: Str,
) -> Status {
    entry!(alloc, node => {
        // SAFETY: checked non-null.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: the caller guarantees the view's bytes.
        let t = match unsafe { as_str(text) } { Ok(t) => t, Err(s) => return s };
        // SAFETY: checked non-null and well-formed by contract, which is
        // what `push_str` asks for.
        match unsafe { (*node).push_str(t, a) } {
            Ok(()) => Status::GUATIAO_OK,
            Err(e) => e.into(),
        }
    })
}

/// Appends to a bytes value.
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
    entry!(alloc, node => {
        // SAFETY: checked non-null.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: the caller guarantees the view's bytes.
        let b = match unsafe { as_bytes(bytes) } { Ok(b) => b, Err(s) => return s };
        // SAFETY: checked non-null and well-formed by contract, which is
        // what `push_bytes` asks for.
        match unsafe { (*node).push_bytes(b, a) } {
            Ok(()) => Status::GUATIAO_OK,
            Err(e) => e.into(),
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
