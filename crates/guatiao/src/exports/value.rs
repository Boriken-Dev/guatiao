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
use core::alloc::Layout;
use core::ffi::c_void;
use core::ptr;

use super::{entry, out};

use crate::value::alloc::{Alloc, Allocator};
use crate::value::status::Status;
use crate::value::types::{Buffer, Bytes, List, Map, Number, Str, Tag, Text, Value};

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
/// for one value that does not already hold one. `out` is not an input:
/// pointing it at `src` is a contract violation, because the marker written
/// on entry replaces `src` before it is read.
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

/// Moves the tree in `v` into `alloc` where it stands: every block
/// another allocator made is moved into one from `alloc` and the old one
/// freed through the allocator that made it. Allocates nothing when
/// everything already records `alloc`.
///
/// What a caller of a kind table calls on a tree answer, or a lent tree,
/// a slot may have built in the library's own allocator: afterwards the
/// tree outlives the library. Call it while the library is loaded. A map
/// or list moves inside a value tagged `GUATIAO_MAP` or `GUATIAO_LIST`
/// holding it, and lives in that value afterwards: copy its `payload.map`
/// or `payload.list` back over a struct it came from, whose array moved.
///
/// A null or unusable allocator is `GUATIAO_ERR_ALLOC`, and so is
/// `alloc` refusing: the tree is then whole, every block either moved or
/// where it was.
///
/// # Safety
///
/// The pointers are null or valid, and `v` addresses a well-formed value
/// nothing else is reading or freeing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_value_make_local(
    v: *mut Value,
    alloc: *const Allocator,
) -> Status {
    entry!(v => {
        // SAFETY: null or valid by the caller's contract; `from_raw`
        // answers `Null` for a null one rather than reading it.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: checked non-null; well-formed and unaliased by contract.
        match unsafe { (*v).make_local_in(a) } {
            Ok(()) => Status::GUATIAO_OK,
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
    out!(out);
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
    out!(out);
    entry!(out => {
        // SAFETY: checked non-null and writable by contract.
        unsafe { ptr::write(out, Value::absent()) };
        Status::GUATIAO_OK
    })
}

/// Writes a boolean through `out`.
///
/// # Safety
///
/// `out` addresses writable storage for one value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_value_bool(b: bool, out: *mut Value) -> Status {
    out!(out);
    entry!(out => {
        // SAFETY: checked non-null and writable by contract.
        unsafe { ptr::write(out, Value::from(b)) };
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
    out!(out);
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
    out!(out);
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
    out!(out);
    entry!(out => {
        // SAFETY: null or valid by the caller's contract; `from_raw`
        // answers `Null` for a null one rather than reading it.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: the caller guarantees the view's bytes.
        let t = match core::str::from_utf8(text.into()) { Ok(t) => t, Err(e) => return e.into() };
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
    out!(out);
    entry!(out => {
        // SAFETY: null or valid by the caller's contract; `from_raw`
        // answers `Null` for a null one rather than reading it.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: the caller guarantees the view's bytes.
        let t = match core::str::from_utf8(text.into()) { Ok(t) => t, Err(e) => return e.into() };
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
    out!(out);
    entry!(out => {
        // SAFETY: null or valid by the caller's contract; `from_raw`
        // answers `Null` for a null one rather than reading it.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: the caller guarantees the view's bytes.
        let b = <&[u8]>::from(bytes);
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
/// can see. **A refused call leaves `value` as it was**, still the
/// caller's to free.
///
/// A map has no index, so each call compares `key` with every key stored:
/// a map filled by N calls costs O(N^2) comparisons. One of many thousand
/// keys is better decoded whole (`guatiao_wire_decode`) than set key by
/// key.
///
/// **`node` and `value` must not overlap**, and the same pointer for both
/// is `GUATIAO_ERR_BAD_VALUE`: this moves the 40 bytes at `value` into
/// `node`, which cannot be a slot inside the tree it is moving. A null or
/// unusable allocator is `GUATIAO_ERR_ALLOC`; a `node` that is not a map
/// is `GUATIAO_ERR_WRONG_KIND`, and a map whose keys are not UTF-8, or a
/// `key` that is not, is `GUATIAO_ERR_BAD_VALUE`.
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
        if core::ptr::eq(node.cast_const(), value.cast_const()) {
            return Status::GUATIAO_ERR_BAD_VALUE;
        }
        // SAFETY: null or valid by the caller's contract.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: checked non-null and well-formed by contract.
        let m = match unsafe { map_of(node) } {
            Ok(m) => m,
            Err(status) => return status,
        };
        // SAFETY: the caller guarantees the key's bytes.
        let k = match core::str::from_utf8(key.into()) { Ok(k) => k, Err(e) => return e.into() };
        // SAFETY: `value` is a well-formed node the caller hands over, and
        // it is taken only once the key is copied and the room reserved,
        // so a refusal leaves it the caller's.
        match m.set_with(k, a, || unsafe { core::mem::take(&mut *value) }) {
            Ok(()) => Status::GUATIAO_OK,
            Err(e) => e.into(),
        }
    })
}

/// The map at `node`: `GUATIAO_ERR_WRONG_KIND` for another kind, and
/// `GUATIAO_ERR_BAD_VALUE` for a map whose keys a reader refuses. The kind
/// is checked before the content, so each answer means one thing.
///
/// # Safety
///
/// `node` is non-null and addresses a well-formed value that outlives
/// the borrow and is not otherwise borrowed for it.
unsafe fn map_of<'a>(node: *mut Value) -> Result<&'a mut Map, Status> {
    // SAFETY: by this function's contract.
    let node = unsafe { &mut *node };
    if node.tag() != Ok(Tag::GUATIAO_MAP) {
        return Err(Status::GUATIAO_ERR_WRONG_KIND);
    }
    TryAsMut::<Map>::try_as_mut(node).ok_or(Status::GUATIAO_ERR_BAD_VALUE)
}

/// Removes `key` and frees its value. `GUATIAO_ERR_NOT_FOUND` if absent,
/// `GUATIAO_ERR_WRONG_KIND` if `node` is not a map, and
/// `GUATIAO_ERR_BAD_VALUE` for a key, or a map's keys, that are not UTF-8.
///
/// # Safety
///
/// The pointers are null or valid, and the key's bytes are readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_map_discard(node: *mut Value, key: Str) -> Status {
    entry!(node => {
        // SAFETY: checked non-null and well-formed by contract.
        let m = match unsafe { map_of(node) } {
            Ok(m) => m,
            Err(status) => return status,
        };
        // SAFETY: the caller guarantees the key's bytes.
        let k = match core::str::from_utf8(key.into()) { Ok(k) => k, Err(e) => return e.into() };
        if m.discard(k) {
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
        // SAFETY: checked non-null and well-formed by contract.
        match unsafe { map_of(node) } {
            Ok(m) => {
                m.clear();
                Status::GUATIAO_OK
            }
            Err(status) => status,
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
        if core::ptr::eq(dst.cast_const(), src) {
            return Status::GUATIAO_ERR_BAD_VALUE;
        }
        // SAFETY: null or valid by the caller's contract.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: checked non-null and well-formed by contract.
        if unsafe { (*dst).tag() } != Ok(Tag::GUATIAO_MAP) || unsafe { (*src).tag() } != Ok(Tag::GUATIAO_MAP) {
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
            None => return Status::GUATIAO_ERR_BAD_VALUE,
        };
        // SAFETY: as above.
        match unsafe { map_of(dst) } {
            Ok(m) => match m.absorb(copy, a) {
                Ok(_) => Status::GUATIAO_OK,
                Err(e) => e.into(),
            },
            Err(status) => status,
        }
    })
}

// --- list -------------------------------------------------------------

/// Appends `value`, **moving** it and leaving it null-tagged.
///
/// To append something you only borrowed, call `guatiao_value_clone`
/// first and pass the copy. **A refused call leaves `value` as it was**,
/// still the caller's to free.
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
        if core::ptr::eq(node.cast_const(), value.cast_const()) {
            return Status::GUATIAO_ERR_BAD_VALUE;
        }
        // SAFETY: null or valid by the caller's contract.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: checked non-null and well-formed by contract; the
        // dereference is the only unsafety left.
        match unsafe { TryAsMut::<List>::try_as_mut(&mut *node) } {
            // SAFETY: as in `guatiao_map_set`: taken only once the room
            // is reserved.
            Some(l) => match l.push_with(a, || unsafe { core::mem::take(&mut *value) }) {
                Ok(()) => Status::GUATIAO_OK,
                Err(e) => e.into(),
            },
            None => Status::GUATIAO_ERR_WRONG_KIND,
        }
    })
}

/// Removes and frees the element at `index`. `GUATIAO_ERR_NOT_FOUND` past
/// the end, `GUATIAO_ERR_WRONG_KIND` if `node` is not a list.
///
/// # Safety
///
/// `node` is null or a well-formed value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_list_discard(node: *mut Value, index: usize) -> Status {
    entry!(node => {
        // SAFETY: checked non-null and well-formed by contract; the
        // dereference is the only unsafety left.
        match unsafe { TryAsMut::<List>::try_as_mut(&mut *node) }.map(|l| l.discard(index)) {
            Some(true) => Status::GUATIAO_OK,
            Some(false) => Status::GUATIAO_ERR_NOT_FOUND,
            None => Status::GUATIAO_ERR_WRONG_KIND,
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
/// out first. A null or unusable allocator is `GUATIAO_ERR_ALLOC`; a node
/// that is not a STRING (a NUMBER included) is `GUATIAO_ERR_WRONG_KIND`,
/// and a string, or `text`, that is not UTF-8 is `GUATIAO_ERR_BAD_VALUE`.
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
        // A STRING only: a NUMBER's digits share the arm and not the type.
        //
        // SAFETY: checked non-null and well-formed by contract.
        let node = unsafe { &mut *node };
        if node.tag() != Ok(Tag::GUATIAO_STRING) {
            return Status::GUATIAO_ERR_WRONG_KIND;
        }
        let Some(s) = TryAsMut::<Text>::try_as_mut(node) else {
            return Status::GUATIAO_ERR_BAD_VALUE;
        };
        // SAFETY: the caller guarantees the view's bytes.
        let t = match core::str::from_utf8(text.into()) { Ok(t) => t, Err(e) => return e.into() };
        match s.push_str_in(t, a) {
            Ok(()) => Status::GUATIAO_OK,
            Err(e) => e.into(),
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
        let b = <&[u8]>::from(bytes);
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

/// `size` zeroed bytes aligned to `align` from this library's own heap,
/// or null when `size` is 0, `align` is not a power of two, or the heap
/// is exhausted.
///
/// For a host that shares no heap with the library, a JavaScript or Dart
/// page calling a WebAssembly module above all: what it passes by
/// address (a view's bytes, an out-parameter, a root it keeps) has to be
/// in the module's memory. Released with `guatiao_memory_free`.
#[unsafe(no_mangle)]
pub extern "C" fn guatiao_memory_alloc(size: usize, align: usize) -> *mut c_void {
    let Ok(layout) = Layout::from_size_align(size, align) else {
        return ptr::null_mut();
    };
    if size == 0 {
        return ptr::null_mut();
    }
    // SAFETY: the layout has a non-zero size.
    unsafe { alloc::alloc::alloc_zeroed(layout) }.cast()
}

/// Releases what `guatiao_memory_alloc` answered. A null `ptr`, or a
/// `size` and `align` that are no layout, does nothing.
///
/// # Safety
///
/// `ptr` is null, or came from `guatiao_memory_alloc` with this `size`
/// and `align` and has not been released.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_memory_free(ptr: *mut c_void, size: usize, align: usize) {
    if ptr.is_null() {
        return;
    }
    let Ok(layout) = Layout::from_size_align(size, align) else {
        return;
    };
    // SAFETY: the caller's contract: this block, with this layout.
    unsafe { alloc::alloc::dealloc(ptr.cast(), layout) };
}

// --- the wire encoding ------------------------------------------------

/// Encodes `value` in the wire encoding (`guatiao::value::wire`), writing
/// the bytes through `out` as a BYTES value built in `alloc`.
///
/// `out` is written the absent marker on entry. A node `value` holds that
/// its own reader refuses (text that is not UTF-8, a number outside the
/// grammar, an unknown tag) is `GUATIAO_ERR_BAD_VALUE`; a null or unusable
/// allocator is `GUATIAO_ERR_ALLOC`.
///
/// # Safety
///
/// The pointers are null or valid, and `out` addresses writable storage
/// for one value that does not already hold one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_wire_encode(
    alloc: *const Allocator,
    value: *const Value,
    out: *mut Value,
) -> Status {
    out!(out);
    entry!(value, out => {
        // SAFETY: null or valid by the caller's contract.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: checked non-null and well-formed by contract.
        let bytes = match crate::value::wire::to_bytes(unsafe { &*value }) {
            Ok(bytes) => bytes,
            Err(e) => return e.into(),
        };
        match Buffer::new_in(a, &bytes).map(Value::from) {
            // SAFETY: `out` is writable and holds no value the caller owns.
            Ok(v) => { unsafe { ptr::write(out, v) }; Status::GUATIAO_OK }
            Err(e) => e.into(),
        }
    })
}

/// Decodes the wire encoding in `bytes`, writing the value through `out`,
/// built in `alloc`.
///
/// `out` is written the absent marker on entry. Bytes that are not a
/// value — truncated, an unknown tag, text that is not UTF-8, a number
/// outside the grammar, a repeated key, anything after the value — are
/// `GUATIAO_ERR_BAD_VALUE`; the Rust API names the byte.
///
/// # Safety
///
/// `bytes` is readable for the call; `out` addresses writable storage for
/// one value that does not already hold one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_wire_decode(
    alloc: *const Allocator,
    bytes: Bytes,
    out: *mut Value,
) -> Status {
    out!(out);
    entry!(out => {
        // SAFETY: null or valid by the caller's contract.
        let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        match crate::value::wire::decode_in(a, bytes.into()) {
            // SAFETY: `out` is writable and holds no value the caller owns.
            Ok(v) => { unsafe { ptr::write(out, v) }; Status::GUATIAO_OK }
            Err(crate::value::wire::WireError::Alloc { error, .. }) => error.into(),
            Err(_) => Status::GUATIAO_ERR_BAD_VALUE,
        }
    })
}
