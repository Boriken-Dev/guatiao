// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Building and changing a C value tree.
//!
//! These are ordinary Rust functions, so a Rust host or library calls them
//! directly whichever side of a boundary it is on. [`crate::exports`]
//! wraps them for callers that cannot.
//!
//! # A refused operation changes nothing
//!
//! The allocator belongs to the caller and may fail, so every operation
//! that could fail part-way builds what it needs into scratch first and
//! commits only once nothing can fail. A `set` that ran out of memory
//! after copying two of a subtree's three buffers must leave the target
//! exactly as it was, and free the two it made.
//!
//! # Inserting always MOVES
//!
//! `map_set` and `list_push` take `&mut Value` and leave it
//! **null-tagged**. There is deliberately no copying variant beside each
//! of them: a caller who wants to insert something it only borrowed calls
//! `value_clone` and hands over the result, which makes the copy a
//! visible line rather than a hidden cost inside a setter.
//!
//! Two spellings of one operation is what that would have been, and the
//! cheap one is not the one a reader would have guessed from the name.
//!
//! # What a move actually costs
//!
//! The node's 40 bytes are copied into the slot. **No buffer is copied and
//! nothing is allocated** — the string, list and map pointers inside the
//! node are carried over as they are, which is what makes a move cheap
//! whatever the subtree weighs.
//!
//! The 40 bytes cannot be avoided, and the reason is a layout decision
//! made deliberately: a [`Entry`] holds its value **inline**, not
//! behind a pointer, so a map is one contiguous array. The node therefore
//! has to end up physically at `entries[i].value`. Storing pointers
//! instead would cost an allocation per value, an indirection for every
//! reader walking a map, and both the literal-tree syntax and the
//! plain-slice view.
//!
//! Null-tagging is the consequence of that copy rather than a separate
//! idea: once the 40 bytes exist in two places, two structs name the same
//! buffers. Emptying the caller's is what stops the second one from being
//! an owner. A by-value move would instead leave the caller holding a
//! struct that still contains the stolen pointers, so freeing it in a
//! cleanup path is a double free — unrepresentable again once the source
//! is null-tagged, which is a property the opaque-handle design had for
//! free and this one has to arrange.
//!
//! # Depth
//!
//! `value_free` is iterative, because a tree can arrive from a foreign
//! caller and recursion on adversarial depth is a stack overflow, which on
//! Windows is not catchable and kills the host. `value_clone` recurses
//! but is bounded by [`MAX_DEPTH`] and reports [`ValueError::TooDeep`]
//! rather than overflowing.
//!
//! # Not API
//!
//! Everything below is `pub(crate)`. A caller reaches a value through the
//! methods on [`Value`], [`Map`], [`List`], [`Text`], [`Buffer`] and
//! [`Entry`]; the `extern "C"` surface in [`crate::exports`] is what
//! wraps these for a caller that cannot link Rust. Two names are the
//! exception because they are genuinely public: [`ValueError`] and
//! [`MAX_DEPTH`], both re-exported at [`crate::value`] and at the crate
//! root.
//!
//! Those two resolve:
//!
//! ```
//! use guatiao::value::mutate::{MAX_DEPTH, ValueError};
//! # let _ = (MAX_DEPTH, ValueError::WrongKind);
//! ```
//!
//! An arm accessor does not, which is what keeps a `&mut Text` off a
//! NUMBER node in safe code:
//!
//! ```compile_fail
//! let mut v = guatiao::Value::int(1);
//! let _ = guatiao::value::mutate::as_text_mut(&mut v);
//! ```

use std::mem::ManuallyDrop;
use std::ptr;

use super::alloc::{Alloc, AllocError, Allocator};
use super::number::validate_json_number;
use super::raw::{dangling, release_buffer, reserve};
use super::types::{Buffer, Entry, List, Map, Payload, Tag, Text, Value};

/// How deep a tree any walk here follows: cloning one, merging two,
/// comparing two for equality, and checking one against a schema.
///
/// Configuration trees are a handful of levels deep; this is far above any
/// real one and far below what would exhaust a stack. It exists so a
/// hostile or corrupt tree is an error rather than a dead process: a stack
/// overflow on Windows is not catchable and takes the host with it.
pub const MAX_DEPTH: u32 = 128;

/// Why a mutation was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ValueError {
    /// The allocator could not satisfy the request, or was itself unusable.
    Alloc(AllocError),
    /// Text offered as a number did not match the JSON number grammar
    /// (RFC 8259 section 6). See [`Value::number`].
    NotANumber,
    /// A key, or the text of a string value, was not valid UTF-8.
    ///
    /// Refused at the point it is offered rather than accepted and fixed
    /// up later: a lossy conversion does not fail, it **renames the key**,
    /// producing a map that is quietly not the one it came from.
    NotUtf8,
    /// The operation does not apply to this value's kind — pushing to
    /// something that is not a list, say.
    WrongKind,
    /// The tag is not one this build knows. Skip this value; do not stop.
    UnknownTag(u32),
    /// An index was past the end.
    OutOfRange,
    /// The tree is nested deeper than [`MAX_DEPTH`].
    TooDeep,
}

impl From<AllocError> for ValueError {
    fn from(e: AllocError) -> ValueError {
        ValueError::Alloc(e)
    }
}

impl std::fmt::Display for ValueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValueError::Alloc(e) => write!(f, "{e}"),
            ValueError::NotANumber => f.write_str("not a JSON number (RFC 8259 section 6)"),
            ValueError::NotUtf8 => f.write_str("not valid UTF-8"),
            ValueError::WrongKind => f.write_str("the value is not of the kind this needs"),
            ValueError::UnknownTag(t) => write!(f, "tag {t} is not one this build knows"),
            ValueError::OutOfRange => f.write_str("the index is past the end"),
            ValueError::TooDeep => write!(f, "nested deeper than {MAX_DEPTH}"),
        }
    }
}

impl std::error::Error for ValueError {}

// --- constructors -----------------------------------------------------

/// A node with every one of its bytes initialised.
///
/// This is the only way a node is ever created, and the reason is the
/// sharpest edge in the whole module: writing a single union arm leaves
/// the rest of the payload uninitialised, and reading a wide arm off that
/// is undefined behaviour rather than garbage. Nothing in the toolchain
/// warns about it.
pub(crate) fn blank(tag: Tag) -> Value {
    Value {
        tag: u32::from(tag),
        _pad: 0,
        // The widest arm, fully written. Every other arm is the same 32
        // bytes, so this initialises all of them at once.
        payload: Payload {
            map: ManuallyDrop::new(Map {
                ptr: dangling::<Entry>(),
                len: 0,
                cap: 0,
                alloc: ptr::null(),
            }),
        },
    }
}

/// The answer to a lookup that found nothing, and the tag of an optional
/// slot that is not filled in. Never stored inside a list.
pub(crate) fn value_absent() -> Value {
    blank(Tag::GUATIAO_ABSENT)
}

/// The stored nothing.
pub(crate) fn value_null() -> Value {
    blank(Tag::GUATIAO_NULL)
}

/// A boolean.
///
/// Stores 1 or 0. The arm is a byte rather than a `bool` so that a value
/// written by somebody else is still readable; see the field's own note.
pub(crate) fn value_bool(b: bool) -> Value {
    let mut v = blank(Tag::GUATIAO_BOOL);
    v.payload.b = u8::from(b);
    v
}

/// Copies `bytes` into a buffer owned by `alloc`.
pub(crate) fn owned_bytes(alloc: Alloc, bytes: &[u8]) -> Result<Buffer, AllocError> {
    let mut b = Buffer {
        ptr: dangling::<u8>(),
        len: 0,
        cap: 0,
        alloc: alloc.as_raw(),
    };
    if !bytes.is_empty() {
        // SAFETY: the container is consistent -- empty, owning nothing.
        unsafe { reserve(&mut b, bytes.len(), Some(alloc))? };
        // SAFETY: `reserve` guaranteed room for `bytes.len()` elements at
        // `b.ptr`, and the two regions cannot overlap since one was just
        // allocated.
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), b.ptr, bytes.len()) };
        b.len = bytes.len();
    }
    Ok(b)
}

/// The same, typed as text. The bytes must already be known to be UTF-8.
///
/// **The `ManuallyDrop` is the transfer.** The buffer's storage BECOMES
/// the text's: the four fields are copied across, and from that moment two
/// containers describe one allocation. Letting the `Buffer` fall out of
/// scope would free what the `Text` now points at — which is exactly what
/// happened the first time the containers grew a `Drop`, and it showed up
/// as a key read back as invalid UTF-8 three calls later.
pub(crate) fn owned_text(alloc: Alloc, text: &str) -> Result<Text, AllocError> {
    let b = ManuallyDrop::new(owned_bytes(alloc, text.as_bytes())?);
    Ok(Text {
        ptr: b.ptr,
        len: b.len,
        cap: b.cap,
        alloc: b.alloc,
    })
}

/// A bytes value, copied into `alloc`. Any content at all, NULs included.
pub(crate) fn value_bytes(alloc: Alloc, bytes: &[u8]) -> Result<Value, ValueError> {
    let b = owned_bytes(alloc, bytes)?;
    let mut v = blank(Tag::GUATIAO_BYTES);
    v.payload = Payload {
        bytes: ManuallyDrop::new(b),
    };
    Ok(v)
}

/// A number, from its **exact text**, which is stored verbatim.
///
/// Rejects anything outside the JSON number grammar, at construction
/// rather than at read: a number that only failed when somebody asked for
/// an integer would put the error a long way from the mistake.
pub(crate) fn value_number(alloc: Alloc, text: &str) -> Result<Value, ValueError> {
    validate_json_number(text.as_bytes()).map_err(|_| ValueError::NotANumber)?;
    let s = owned_text(alloc, text)?;
    let mut v = blank(Tag::GUATIAO_NUMBER);
    v.payload = Payload {
        text: ManuallyDrop::new(s),
    };
    Ok(v)
}

/// A number from an `i64`. Cannot be refused: every `i64` has a JSON
/// spelling.
///
/// Exists so a C caller does not format its own and get the grammar
/// subtly wrong.
pub(crate) fn value_int(alloc: Alloc, v: i64) -> Result<Value, ValueError> {
    value_number(alloc, &v.to_string())
}

/// A number from an `f64`, refusing a non-finite one.
///
/// `NaN` and the infinities have no JSON spelling at all, so a container
/// that accepted one would have to invent a spelling or lose the value at
/// the first boundary it crossed. Keeping the refusal here keeps it in one
/// place instead of in every consumer's `snprintf`.
pub(crate) fn value_float(alloc: Alloc, v: f64) -> Result<Value, ValueError> {
    if !v.is_finite() {
        return Err(ValueError::NotANumber);
    }
    value_number(alloc, &v.to_string())
}

// --- reading the shape ------------------------------------------------

/// The tag, or the raw integer if this build does not know it.
pub(crate) fn value_tag(v: &Value) -> Result<Tag, ValueError> {
    Tag::try_from(v.tag)
}

macro_rules! arm {
    ($name:ident, $name_mut:ident, $field:ident, $ty:ty, $($tag:pat_param)|+) => {
        /// The live arm, or `None` when the tag selects a different one.
        pub(crate) fn $name(v: &Value) -> Option<&$ty> {
            match Tag::try_from(v.tag) {
                // SAFETY: the tag says this arm is the live one, and every
                // node is born with all 40 bytes initialised, so the arm is
                // a valid value of its type either way.
                $(Ok($tag))|+ => Some(unsafe { &v.payload.$field }),
                _ => None,
            }
        }

        /// The live arm, mutably.
        pub(crate) fn $name_mut(v: &mut Value) -> Option<&mut $ty> {
            match Tag::try_from(v.tag) {
                // SAFETY: as above.
                $(Ok($tag))|+ => Some(unsafe { &mut v.payload.$field }),
                _ => None,
            }
        }
    };
}

arm!(
    as_text,
    as_text_mut,
    text,
    Text,
    Tag::GUATIAO_STRING | Tag::GUATIAO_NUMBER
);
arm!(as_buffer, as_buffer_mut, bytes, Buffer, Tag::GUATIAO_BYTES);
arm!(as_list, as_list_mut, list, List, Tag::GUATIAO_LIST);
arm!(as_map, as_map_mut, map, Map, Tag::GUATIAO_MAP);

/// The boolean, or `None` for any other kind.
///
/// Any non-zero byte reads as `true`, matching C's own rule for a scalar
/// used as a condition. There is nothing defensive about this read: the
/// arm is a `u8`, which has no invalid bit patterns, so every byte a
/// producer could have written is a valid value of the arm's type. That
/// is the whole reason the arm is not a `bool`.
pub(crate) fn as_bool(v: &Value) -> Option<bool> {
    match Tag::try_from(v.tag) {
        // SAFETY: the tag says the `b` arm is live, and a node is born
        // with all 40 of its bytes initialised, so the byte is readable.
        Ok(Tag::GUATIAO_BOOL) => Some(unsafe { v.payload.b } != 0),
        _ => None,
    }
}

/// The bytes of a string or a number, borrowed.
pub(crate) fn text_bytes(v: &Value) -> Option<&[u8]> {
    let s = as_text(v)?;
    // A view with length 0 may carry a dangling pointer, and building a
    // slice from one is only sound because the length is zero -- but the
    // pointer must still be non-null and aligned, which `dangling` and
    // every allocation both guarantee.
    if s.len == 0 {
        return Some(&[]);
    }
    // SAFETY: an owned text container's first `len` bytes are initialised
    // and live for as long as the node is borrowed.
    Some(unsafe { std::slice::from_raw_parts(s.ptr, s.len) })
}

/// The exact text of a number, or `None` for any other kind.
///
/// **The accessor that cannot lie.** `1.10` reads back as `1.10` and a
/// 200-digit integer survives, because nothing ever converted it.
pub(crate) fn number_str(v: &Value) -> Option<&str> {
    if Tag::try_from(v.tag) != Ok(Tag::GUATIAO_NUMBER) {
        return None;
    }
    std::str::from_utf8(text_bytes(v)?).ok()
}

/// The text of a string, or `None` for any other kind.
pub(crate) fn str_of(v: &Value) -> Option<&str> {
    if Tag::try_from(v.tag) != Ok(Tag::GUATIAO_STRING) {
        return None;
    }
    std::str::from_utf8(text_bytes(v)?).ok()
}

/// The bytes of a bytes value, or `None` for any other kind.
pub(crate) fn bytes_of(v: &Value) -> Option<&[u8]> {
    let b = as_buffer(v)?;
    if b.len == 0 {
        return Some(&[]);
    }
    // SAFETY: an owned buffer's first `len` bytes are initialised and live
    // for as long as the node is borrowed.
    Some(unsafe { std::slice::from_raw_parts(b.ptr, b.len) })
}

/// A map's entries, in insertion order.
pub(crate) fn entries_of(v: &Value) -> Option<&[Entry]> {
    let m = as_map(v)?;
    if m.len == 0 {
        return Some(&[]);
    }
    // SAFETY: the first `len` entries of an owned map are initialised.
    Some(unsafe { std::slice::from_raw_parts(m.ptr, m.len) })
}

/// A list's elements, in order.
pub(crate) fn items_of(v: &Value) -> Option<&[Value]> {
    let l = as_list(v)?;
    if l.len == 0 {
        return Some(&[]);
    }
    // SAFETY: the first `len` elements of an owned list are initialised.
    Some(unsafe { std::slice::from_raw_parts(l.ptr, l.len) })
}

/// A key as bytes, borrowed from its entry.
pub(crate) fn key_bytes(e: &Entry) -> &[u8] {
    if e.key.len == 0 {
        return &[];
    }
    // SAFETY: an entry's key is an owned text container whose first `len`
    // bytes are initialised.
    unsafe { std::slice::from_raw_parts(e.key.ptr, e.key.len) }
}

// --- freeing ----------------------------------------------------------

/// Replaces a node with the stored nothing and hands back what was there.
pub(crate) fn take(v: &mut Value) -> Value {
    std::mem::replace(v, value_null())
}

/// Whether this node's tag selects an arm that owns a buffer.
///
/// Absent, null, bool and a tag this build does not know own nothing.
fn owns_storage(v: &Value) -> bool {
    matches!(
        Tag::try_from(v.tag),
        Ok(Tag::GUATIAO_STRING
            | Tag::GUATIAO_NUMBER
            | Tag::GUATIAO_BYTES
            | Tag::GUATIAO_LIST
            | Tag::GUATIAO_MAP)
    )
}

/// Frees everything a node owns and leaves it null-tagged.
///
/// Safe to call on a node that owns nothing: a literal's buffers have
/// `cap == 0` and are never touched, which is what makes a statically
/// declared tree safe to hand to the same function as an allocated one.
///
/// **Iterative on purpose.** A tree may have arrived from a foreign
/// caller, and recursion on adversarial depth is a stack overflow — which
/// on Windows is not catchable, so it would kill the host rather than
/// produce an error.
///
/// # Safety
///
/// The node's containers describe their own storage: `cap > 0` means the
/// buffer came from the allocator the container names, with a layout of
/// `cap` elements, and the first `len` elements are initialised.
pub(crate) unsafe fn value_free(v: &mut Value) {
    // A scalar owns nothing, so null-tagging it IS the whole operation:
    // no walk, and no heap for the walk's stack. Scalars are most of the
    // nodes in a tree, and every container's own free calls this once per
    // element.
    //
    // The node is taken and forgotten rather than overwritten in place:
    // assigning over it would DROP it, and `Drop` is this function.
    if !owns_storage(v) {
        let _ = ManuallyDrop::new(take(v));
        return;
    }

    let mut stack = vec![take(v)];

    // **Every node here is held in a `ManuallyDrop`**, and that is not
    // tidiness. A `Value` frees itself when it goes out of scope, so a
    // node this loop has already dismantled would be freed a SECOND time
    // the moment the binding ended — the walk exists precisely to take
    // each node apart by hand, so it has to say that Rust must not also.
    while let Some(node) = stack.pop() {
        let mut node = std::mem::ManuallyDrop::new(node);
        match Tag::try_from(node.tag) {
            Ok(Tag::GUATIAO_STRING | Tag::GUATIAO_NUMBER) => {
                // SAFETY: the tag says the text arm is live.
                let s = unsafe { &mut *node.payload.text };
                // SAFETY: the caller's invariant describes this buffer.
                unsafe { release_buffer(s) };
            }
            Ok(Tag::GUATIAO_BYTES) => {
                // SAFETY: the tag says the bytes arm is live.
                let b = unsafe { &mut *node.payload.bytes };
                // SAFETY: as above.
                unsafe { release_buffer(b) };
            }
            Ok(Tag::GUATIAO_LIST) => {
                // SAFETY: the tag says the list arm is live.
                let l = unsafe { &mut *node.payload.list };
                for i in 0..l.len {
                    // SAFETY: the first `len` elements are initialised.
                    let elem = unsafe { &mut *l.ptr.add(i) };
                    stack.push(take(elem));
                }
                l.len = 0;
                // SAFETY: the elements have been moved out, so nothing
                // reads the buffer again.
                unsafe { release_buffer(l) };
            }
            Ok(Tag::GUATIAO_MAP) => {
                // SAFETY: the tag says the map arm is live.
                let m = unsafe { &mut *node.payload.map };
                for i in 0..m.len {
                    // SAFETY: the first `len` entries are initialised.
                    let entry = unsafe { &mut *m.ptr.add(i) };
                    // SAFETY: the key is an owned text container.
                    unsafe { release_buffer(&mut entry.key) };
                    stack.push(take(&mut entry.value));
                }
                m.len = 0;
                // SAFETY: as above.
                unsafe { release_buffer(m) };
            }
            // Absent, null, bool, and any tag this build does not know:
            // nothing is owned, so there is nothing to release. An unknown
            // tag is deliberately not an error here — refusing to free a
            // tree because one node came from a newer producer would leak
            // the whole tree to punish the one node.
            _ => {}
        }
    }
}

// --- a container frees what it owns -------------------------------------
//
// The four owned containers each release their buffer when they go out of
// scope, which is what makes `Map::new()` able to hand back a `Map`: a
// caller holds the container itself, grows it, and lets it go.
//
// **None of this double-frees the hand-written walks below.**
// [`release_buffer`] leaves the container empty with `cap == 0`, and
// [`value_free`] leaves the node null-tagged, so both are idempotent. A
// site that frees by hand and then lets the binding end runs the second
// release against nothing.
//
// The union arms are `ManuallyDrop`, so a container living inside a
// [`Value`] is freed by [`value_free`]'s walk and never twice.

/// The allocator a node's live container recorded, if it has one.
///
/// Null and bool own nothing, so they have none; every other kind keeps
/// the address of the allocator its buffer came from.
pub(crate) fn alloc_of(v: &Value) -> Option<*const Allocator> {
    // SAFETY: each arm is read only under the tag that selects it.
    unsafe {
        Some(match value_tag(v).ok()? {
            Tag::GUATIAO_MAP => v.payload.map.alloc,
            Tag::GUATIAO_LIST => v.payload.list.alloc,
            Tag::GUATIAO_STRING | Tag::GUATIAO_NUMBER => v.payload.text.alloc,
            Tag::GUATIAO_BYTES => v.payload.bytes.alloc,
            _ => return None,
        })
    }
}

// --- building one -------------------------------------------------------
//
// **Every constructor is on the KIND**, because that is what a caller is
// naming: `Value::map()`, `Value::string("x")`. Two lists of the same
// constructors under two names drift, so there is one.
//
// **The short form names no allocator**, which is the whole point. A
// program written in Rust that builds a value is building it on Rust's
// heap; saying so at every call site is noise, and the crate's own
// allocator is a constant this borrows. The `_in` form names one, which
// is what a library building into a host's arena has to do, and that form
// stays fallible — see [`or_abort`].

/// The short constructors abort on an allocation failure; the `_in` ones
/// report it.
///
/// **Why the two differ.** A short constructor allocates on Rust's own
/// heap, and a failure there is the condition `String::from` and
/// `Vec::push` already meet: there is no memory, the process is over, and
/// the standard library aborts rather than returning. Threading a
/// `Result` through every `Value::int(5900)` would buy a recovery nobody
/// writes.
///
/// A foreign allocator returning null is a different statement. It may be
/// an arena that is merely full, and the library that named it may have
/// somewhere else to put the value — so `_in` hands the refusal back.
///
/// Nothing but an allocation failure can reach here: the callers pass a
/// `&str` or an `i64`, whose text is a JSON number by construction.
pub(crate) fn or_abort<T>(built: Result<T, ValueError>) -> T {
    match built {
        Ok(v) => v,
        Err(e) => panic!("guatiao: building a value on Rust's heap failed: {e}"),
    }
}

// --- reading and changing one -------------------------------------------
//
// A value's own operations live on the value. The free functions below
// are what these call, and they are what the `extern "C"` layer wraps;
// nothing in Rust should have to name them.

// --- what a value can be made from --------------------------------------
//
// So that `map.set("port", 5900)` is the whole call. Every one of these
// builds on Rust's heap, which is the right default for code written in
// Rust; a tree that must live in a host's arena is built with the `_in`
// constructors and passed in, and `Into<Value>` for `Value` is then the
// identity, so nothing is copied on the way.
//
// A mixed tree — a host's map holding a string Rust allocated — is well
// formed, because every owned container records the allocator that made
// it and the free walk asks each node rather than the root.

// --- cloning ----------------------------------------------------------

/// Deep-copies a tree into `alloc`.
///
/// The copy is complete or it does not exist: a failure part-way frees
/// everything already built and reports the error, so the caller is never
/// handed a half-built tree and never leaks the half that succeeded.
///
/// # Safety
///
/// `src` is a well-formed node, as described on [`value_free`].
pub(crate) unsafe fn value_clone(alloc: Alloc, src: &Value) -> Result<Value, ValueError> {
    // SAFETY: forwarded.
    unsafe { clone_at(alloc, src, 0) }
}

pub(crate) unsafe fn clone_at(alloc: Alloc, src: &Value, depth: u32) -> Result<Value, ValueError> {
    if depth >= MAX_DEPTH {
        return Err(ValueError::TooDeep);
    }
    match value_tag(src)? {
        Tag::GUATIAO_ABSENT => Ok(value_absent()),
        Tag::GUATIAO_NULL => Ok(value_null()),
        Tag::GUATIAO_BOOL => Ok(value_bool(as_bool(src).unwrap_or(false))),
        Tag::GUATIAO_NUMBER | Tag::GUATIAO_STRING => {
            let bytes = text_bytes(src).unwrap_or(&[]);
            let s = owned_text(
                alloc,
                std::str::from_utf8(bytes).map_err(|_| ValueError::NotUtf8)?,
            )?;
            let mut v = blank(value_tag(src)?);
            v.payload = Payload {
                text: ManuallyDrop::new(s),
            };
            Ok(v)
        }
        Tag::GUATIAO_BYTES => value_bytes(alloc, bytes_of(src).unwrap_or(&[])),
        Tag::GUATIAO_LIST => {
            let items = items_of(src).unwrap_or(&[]);
            // The guard owns everything built so far, so an early return
            // frees it rather than leaking it.
            let mut out = Value::list_in(alloc);
            if !items.is_empty() {
                let l = as_list_mut(&mut out).expect("just built as a list");
                // SAFETY: the container is consistent and empty.
                unsafe { reserve(l, items.len(), Some(alloc))? };
            }
            for item in items {
                // SAFETY: forwarded from this function's own contract.
                let copy = unsafe { clone_at(alloc, item, depth + 1)? };
                // SAFETY: room was reserved above, and `len` counts what
                // has actually been written.
                unsafe { push_written(&mut out, copy, alloc)? };
            }
            Ok(out)
        }
        Tag::GUATIAO_MAP => {
            let entries = entries_of(src).unwrap_or(&[]);
            let mut out = Value::map_in(alloc);
            if !entries.is_empty() {
                let m = as_map_mut(&mut out).expect("just built as a map");
                // SAFETY: the container is consistent and empty.
                unsafe { reserve(m, entries.len(), Some(alloc))? };
            }
            for entry in entries {
                let key = std::str::from_utf8(key_bytes(entry)).map_err(|_| ValueError::NotUtf8)?;
                // SAFETY: forwarded.
                let copy = unsafe { clone_at(alloc, &entry.value, depth + 1)? };
                // SAFETY: as above.
                unsafe { map_insert_written(&mut out, key, copy, alloc)? };
            }
            Ok(out)
        }
    }
}

// --- list mutation ----------------------------------------------------

/// Appends an already-built node, taking ownership of it.
///
/// # Safety
///
/// `node` is a list and `value`'s buffers came from an allocator that
/// outlives the list.
pub(crate) unsafe fn push_written(
    node: &mut Value,
    value: Value,
    alloc: Alloc,
) -> Result<(), ValueError> {
    let mut value = value;
    let l = match as_list_mut(node) {
        Some(l) => l,
        None => {
            // SAFETY: the value was built here and owns its buffers.
            unsafe { value_free(&mut value) };
            return Err(ValueError::WrongKind);
        }
    };
    // SAFETY: the container is consistent.
    if let Err(e) = unsafe { reserve(l, 1, Some(alloc)) } {
        // SAFETY: as above.
        unsafe { value_free(&mut value) };
        return Err(e.into());
    }
    // SAFETY: `reserve` guaranteed room for one more element past `len`,
    // and that slot is uninitialised, so it is written rather than
    // assigned.
    unsafe { l.ptr.add(l.len).write(value) };
    l.len += 1;
    Ok(())
}

/// Appends `value`, **moving** it and leaving it null-tagged.
///
/// To append something you only borrowed, clone it first:
/// `value_clone(alloc, src)` then push the result. The copy is then a
/// line a reader can see rather than a cost hidden in a setter.
///
/// Null-tagging is the other half: a caller that kept a copy of the struct
/// and freed it would otherwise be double-freeing the buffers this list
/// now owns. After this returns, the caller's node owns nothing and
/// freeing it is a no-op.
///
/// # Safety
///
/// `node` is a list and `value` is a well-formed node whose buffers came
/// from an allocator that outlives the list.
pub(crate) unsafe fn list_push(
    node: &mut Value,
    value: &mut Value,
    alloc: Alloc,
) -> Result<(), ValueError> {
    if as_list(node).is_none() {
        return Err(ValueError::WrongKind);
    }
    let moved = take(value);
    // SAFETY: forwarded; `moved` owns what `value` owned.
    unsafe { push_written(node, moved, alloc) }
}

/// Removes the element at `index` and hands it back, keeping the order of
/// the rest.
///
/// The returned node **owns its buffers** and frees them when it goes out
/// of scope, so dropping it on the floor is a release rather than a leak.
/// [`list_discard`] is the form for a caller that never wants to name it.
///
/// # Safety
///
/// `node` is a well-formed list.
pub(crate) unsafe fn list_remove(node: &mut Value, index: usize) -> Option<Value> {
    let l = as_list_mut(node)?;
    if index >= l.len {
        return None;
    }
    // SAFETY: `index < len`, so this element is initialised; the shift
    // below closes the hole it leaves.
    let out = unsafe { l.ptr.add(index).read() };
    // SAFETY: moving the tail down one slot over the hole just vacated.
    unsafe { ptr::copy(l.ptr.add(index + 1), l.ptr.add(index), l.len - index - 1) };
    l.len -= 1;
    Some(out)
}

/// Removes and frees the element at `index`. Answers whether there was
/// one.
///
/// # Safety
///
/// `node` is a well-formed list.
pub(crate) unsafe fn list_discard(node: &mut Value, index: usize) -> bool {
    // SAFETY: forwarded.
    match unsafe { list_remove(node, index) } {
        Some(mut v) => {
            // SAFETY: the removed node owns its buffers and nothing else
            // refers to them.
            unsafe { value_free(&mut v) };
            true
        }
        None => false,
    }
}

/// Frees every element and empties the list, **keeping its capacity**.
///
/// Not the same as freeing the node: the value stays a list, and the
/// buffer it already has is reused rather than returned and asked for
/// again.
///
/// # Safety
///
/// `node` is a well-formed list.
pub(crate) unsafe fn list_clear(node: &mut Value) -> Result<(), ValueError> {
    let l = as_list_mut(node).ok_or(ValueError::WrongKind)?;
    for i in 0..l.len {
        // SAFETY: the first `len` elements are initialised.
        let mut elem = unsafe { l.ptr.add(i).read() };
        // SAFETY: each element owns its own buffers.
        unsafe { value_free(&mut elem) };
    }
    l.len = 0;
    Ok(())
}

// --- map mutation -----------------------------------------------------

/// The position of `key`, by exact byte comparison.
///
/// Bytes, not `strcmp`: a key may contain a NUL, and comparing only to the
/// first one would make two different keys look identical.
pub(crate) fn position(node: &Value, key: &str) -> Option<usize> {
    position_in(entries_of(node)?, key)
}

/// The same, over entries already in hand, which is what a [`Map`] has.
pub(crate) fn position_in(entries: &[Entry], key: &str) -> Option<usize> {
    entries.iter().position(|e| key_bytes(e) == key.as_bytes())
}

/// Inserts an already-built value under a key that is known to be absent.
///
/// # Safety
///
/// `node` is a map, `key` is not already present, and `value` owns its
/// buffers.
pub(crate) unsafe fn map_insert_written(
    node: &mut Value,
    key: &str,
    value: Value,
    alloc: Alloc,
) -> Result<(), ValueError> {
    let mut value = value;
    // The key copy is made before anything is touched, so a failure here
    // leaves the map exactly as it was.
    let key_owned = match owned_text(alloc, key) {
        Ok(k) => k,
        Err(e) => {
            // SAFETY: the value owns its buffers and nothing else refers
            // to them.
            unsafe { value_free(&mut value) };
            return Err(e.into());
        }
    };

    let m = match as_map_mut(node) {
        Some(m) => m,
        None => {
            let mut k = key_owned;
            // SAFETY: both were just built here.
            unsafe {
                release_buffer(&mut k);
                value_free(&mut value);
            }
            return Err(ValueError::WrongKind);
        }
    };

    // SAFETY: the container is consistent.
    if let Err(e) = unsafe { reserve(m, 1, Some(alloc)) } {
        let mut k = key_owned;
        // SAFETY: as above.
        unsafe {
            release_buffer(&mut k);
            value_free(&mut value);
        }
        return Err(e.into());
    }

    // SAFETY: `reserve` guaranteed room for one more entry past `len`, and
    // that slot is uninitialised.
    unsafe {
        m.ptr.add(m.len).write(Entry {
            key: key_owned,
            value,
        })
    };
    m.len += 1;
    Ok(())
}

/// Stores `value` under `key`, **moving** it and leaving it null-tagged.
///
/// Replacement keeps the original position, because the ordinary use is
/// "build a map, then override two fields", and moving a replaced key to
/// the end would re-order a caller's rendered form under it.
///
/// To store something you only borrowed, clone it first. See
/// [`list_push`] for why this null-tags rather than taking the node by
/// value.
///
/// # Safety
///
/// `node` is a well-formed map and `value` is a well-formed node whose
/// buffers came from an allocator that outlives the map.
pub(crate) unsafe fn map_set(
    node: &mut Value,
    key: &str,
    value: &mut Value,
    alloc: Alloc,
) -> Result<(), ValueError> {
    if as_map(node).is_none() {
        return Err(ValueError::WrongKind);
    }
    let moved = take(value);
    // SAFETY: forwarded.
    unsafe { map_set_written(node, key, moved, alloc) }
}

/// # Safety
///
/// `node` is a map and `value` owns its buffers.
pub(crate) unsafe fn map_set_written(
    node: &mut Value,
    key: &str,
    value: Value,
    alloc: Alloc,
) -> Result<(), ValueError> {
    match position(node, key) {
        Some(i) => {
            let m = as_map_mut(node).ok_or(ValueError::WrongKind)?;
            // SAFETY: `i < len`, so this entry is initialised.
            let entry = unsafe { &mut *m.ptr.add(i) };
            let mut old = std::mem::replace(&mut entry.value, value);
            // SAFETY: the replaced value owned its buffers and nothing
            // refers to them now.
            unsafe { value_free(&mut old) };
            Ok(())
        }
        // SAFETY: forwarded; the key is known absent.
        None => unsafe { map_insert_written(node, key, value, alloc) },
    }
}

/// The value under `key`, or `None`.
pub(crate) fn map_get<'a>(node: &'a Value, key: &str) -> Option<&'a Value> {
    let i = position(node, key)?;
    entries_of(node).map(|e| &e[i].value)
}

/// The value under `key`, mutably.
pub(crate) fn map_get_mut<'a>(node: &'a mut Value, key: &str) -> Option<&'a mut Value> {
    let i = position(node, key)?;
    let m = as_map_mut(node)?;
    // SAFETY: `i` came from a scan of the same map and is in range.
    Some(unsafe { &mut (*m.ptr.add(i)).value })
}

/// Removes `key` and hands back its value, keeping the order of the rest.
///
/// The returned node **owns its buffers** and frees them on drop; see
/// [`map_discard`] for the form that never hands it back.
///
/// # Safety
///
/// `node` is a well-formed map.
pub(crate) unsafe fn map_remove(node: &mut Value, key: &str) -> Option<Value> {
    let i = position(node, key)?;
    let m = as_map_mut(node)?;
    // SAFETY: `i < len`, so the entry is initialised.
    let entry = unsafe { m.ptr.add(i).read() };
    // SAFETY: closing the hole the removed entry left.
    unsafe { ptr::copy(m.ptr.add(i + 1), m.ptr.add(i), m.len - i - 1) };
    m.len -= 1;

    let Entry { key: k, value } = entry;
    let mut k = k;
    // SAFETY: the key was owned by the entry and is not referenced now.
    unsafe { release_buffer(&mut k) };
    Some(value)
}

/// Removes `key` and frees its value. Answers whether it was there.
///
/// # Safety
///
/// `node` is a well-formed map.
pub(crate) unsafe fn map_discard(node: &mut Value, key: &str) -> bool {
    // SAFETY: forwarded.
    match unsafe { map_remove(node, key) } {
        Some(mut v) => {
            // SAFETY: the removed value owns its buffers.
            unsafe { value_free(&mut v) };
            true
        }
        None => false,
    }
}

/// Frees every entry and empties the map, **keeping its capacity**.
///
/// # Safety
///
/// `node` is a well-formed map.
pub(crate) unsafe fn map_clear(node: &mut Value) -> Result<(), ValueError> {
    let m = as_map_mut(node).ok_or(ValueError::WrongKind)?;
    for i in 0..m.len {
        // SAFETY: the first `len` entries are initialised.
        let entry = unsafe { m.ptr.add(i).read() };
        let Entry { key, mut value } = entry;
        let mut key = key;
        // SAFETY: both were owned by the entry.
        unsafe {
            release_buffer(&mut key);
            value_free(&mut value);
        }
    }
    m.len = 0;
    Ok(())
}

/// Removes the first entry and hands it back, keeping the order of the
/// rest.
///
/// # Safety
///
/// `node` is a well-formed map.
unsafe fn map_take_first(node: &mut Value) -> Option<Entry> {
    let m = as_map_mut(node)?;
    if m.len == 0 {
        return None;
    }
    // SAFETY: the first entry is initialised.
    let out = unsafe { m.ptr.read() };
    // SAFETY: closing the hole the removed entry left. At `len == 1` the
    // source is a legal one-past-the-end pointer and the count is zero.
    unsafe { ptr::copy(m.ptr.add(1), m.ptr, m.len - 1) };
    m.len -= 1;
    Some(out)
}

/// Copies every entry of `src` into `dst`, replacing keys that collide and
/// appending the rest. Answers how many were copied.
///
/// **This exists because losing the keys you do not model is a real,
/// recorded failure**: a consumer that reads a record, rebuilds it from
/// the fields it knows about, and writes it back drops every field a newer
/// producer added — a password among them. Copying the whole map and then
/// overriding what you mean to change is the shape that cannot do that,
/// and it is worth one named function so it gets written.
///
/// **The whole source is copied before `dst` is touched.** `src` may be
/// `dst` itself, or a node inside it: a slice of `src`'s entries dangles
/// the moment an append reallocates `dst`'s buffer or a replacement frees
/// the entry that slice points at. Copying first makes the two cases one
/// case, at the cost of the copy a caller was paying per entry anyway.
///
/// **Not atomic.** A failure at entry *k* leaves entries `0..k` applied;
/// nothing is leaked and nothing is half-written.
///
/// # Safety
///
/// Both are well-formed maps.
pub(crate) unsafe fn map_copy_from(
    dst: &mut Value,
    src: &Value,
    alloc: Alloc,
) -> Result<usize, ValueError> {
    if as_map(dst).is_none() || as_map(src).is_none() {
        return Err(ValueError::WrongKind);
    }
    // SAFETY: `src` is a well-formed map, checked above.
    let mut copy = unsafe { value_clone(alloc, src)? };

    let mut n = 0;
    let mut result = Ok(());
    // Each entry MOVES out of the clone, and the clone's length falls with
    // it, so whatever is left when this stops is freed below exactly once.
    // SAFETY: the clone is a well-formed map this function just built.
    while let Some(entry) = unsafe { map_take_first(&mut copy) } {
        let Entry { key: mut k, value } = entry;
        let mut value = value;
        match k.as_str() {
            // SAFETY: `dst` is a map, checked above, and the value owns
            // its buffers.
            Some(key) => match unsafe { map_set_written(dst, key, value, alloc) } {
                Ok(()) => n += 1,
                Err(e) => result = Err(e),
            },
            None => {
                // SAFETY: the value came out of the clone and nothing
                // else refers to what it owns.
                unsafe { value_free(&mut value) };
                result = Err(ValueError::NotUtf8);
            }
        }
        // SAFETY: the key was the entry's own and is not referenced now.
        unsafe { release_buffer(&mut k) };
        if result.is_err() {
            break;
        }
    }
    // SAFETY: whatever the loop did not move is still the clone's, and
    // nothing else refers to it.
    unsafe { value_free(&mut copy) };
    result?;
    Ok(n)
}

// --- appending to text and bytes --------------------------------------

/// Whether two byte ranges share a byte. Empty ranges touch nothing.
fn overlaps(a: *const u8, a_len: usize, b: *const u8, b_len: usize) -> bool {
    if a_len == 0 || b_len == 0 {
        return false;
    }
    let (a, b) = (a as usize, b as usize);
    a < b.saturating_add(b_len) && b < a.saturating_add(a_len)
}

/// Appends bytes to a string value. Refuses anything that is not UTF-8.
///
/// The source may address the node's own text — a C caller can hand this
/// a view of the value it is appending to — so an overlapping source is
/// copied out before anything grows.
///
/// # Safety
///
/// `node` is a well-formed string.
pub(crate) unsafe fn string_push(
    node: &mut Value,
    text: &str,
    alloc: Alloc,
) -> Result<(), ValueError> {
    if Tag::try_from(node.tag) != Ok(Tag::GUATIAO_STRING) {
        return Err(ValueError::WrongKind);
    }
    let s = as_text_mut(node).ok_or(ValueError::WrongKind)?;
    if text.is_empty() {
        return Ok(());
    }
    // Growth frees the buffer the source may be pointing into, and
    // staying in place would make the copy below overlap itself. Taking
    // the bytes first makes both cases one case.
    let staged;
    let bytes = if overlaps(s.ptr, s.len, text.as_ptr(), text.len()) {
        staged = text.as_bytes().to_vec();
        staged.as_slice()
    } else {
        text.as_bytes()
    };
    // SAFETY: the container is consistent.
    unsafe { reserve(s, bytes.len(), Some(alloc))? };
    // SAFETY: `reserve` guaranteed room for `bytes.len()` more bytes past
    // `len`, and the source is either disjoint from the buffer or the
    // staged copy of it.
    unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), s.ptr.add(s.len), bytes.len()) };
    s.len += bytes.len();
    Ok(())
}

/// Appends to a bytes value.
///
/// An overlapping source is copied out first, as in [`string_push`].
///
/// # Safety
///
/// `node` is a well-formed bytes value.
pub(crate) unsafe fn buffer_push(
    node: &mut Value,
    bytes: &[u8],
    alloc: Alloc,
) -> Result<(), ValueError> {
    let b = as_buffer_mut(node).ok_or(ValueError::WrongKind)?;
    if bytes.is_empty() {
        return Ok(());
    }
    // As in `string_push`: the source may be this buffer.
    let staged;
    let bytes = if overlaps(b.ptr, b.len, bytes.as_ptr(), bytes.len()) {
        staged = bytes.to_vec();
        staged.as_slice()
    } else {
        bytes
    };
    // SAFETY: the container is consistent.
    unsafe { reserve(b, bytes.len(), Some(alloc))? };
    // SAFETY: as above.
    unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), b.ptr.add(b.len), bytes.len()) };
    b.len += bytes.len();
    Ok(())
}
