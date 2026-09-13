// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The C form of a value: plain structs, no opaque handles.
//!
//! Two families, named the way Rust names them. A **view** is
//! `{const ptr, len}` and is what every parameter takes and every read
//! returns. An **owned** container is `{ptr, len, cap, alloc}` and can be
//! grown through the allocator it carries.
//!
//! | Rust | view (16 bytes) | owned (32 bytes) |
//! | --- | --- | --- |
//! | `&str` / `String` | [`Str`] | [`Text`] |
//! | `&[u8]` / `Vec<u8>` | [`Bytes`] | [`Buffer`] |
//! | `&[Value]` / `Vec<Value>` | [`Values`] | [`List`] |
//! | `&[(K, V)]` / `Vec<(K, V)>` | [`Entries`] | [`Map`] |
//! | `&Value` / `Value` | `*const Value` | [`Value`] |
//!
//! # One node type, holding owned payloads
//!
//! A [`Value`] is a `Value`, so it holds owned containers, exactly
//! as Rust's enum holds `String` and `Vec`. A *view* of a node is a const
//! pointer to one. A second, view-shaped node type was considered and
//! rejected: a list's element type is the node type, so two node layouts
//! would double every accessor and force a copy merely to read an owned
//! tree through the view API.
//!
//! # `cap == 0` means the buffer is not owned
//!
//! That is `Vec`'s own rule rather than an invention: a capacity of zero
//! never deallocates. It is what lets a C literal be written as a brace
//! initialiser with `cap = 0, alloc = NULL`, read like any other value,
//! and never be freed. The first growth copies out of it and leaves the
//! original untouched.
//!
//! **`cap >= len` therefore does NOT hold on input.** A literal is
//! legitimately `len = 5, cap = 0`. Spare capacity is `cap - len` only
//! after `cap == 0` has been handled, and the arithmetic in
//! the private `raw` module is written that way.

#![allow(non_camel_case_types)]
// The crate root sets `#![warn(missing_docs)]`, which is right for the
// model: a `Map` method's contract is not obvious from its name. It is
// wrong for SCREAMING_CASE C constants that say exactly what they are.
// cbindgen copies any doc comment here verbatim into the generated
// header, so a per-variant `/// The bool kind.` would add a line of noise
// to that header for every one of them. Scoped to this module rather than
// silenced per item, because the exception is a property of the whole C
// surface; the variants that DO carry a non-obvious distinction still
// document it, and this allow does not discourage that.
#![allow(missing_docs)]

// One module per type, each carrying its own definition and, after phase
// 2, its own impl. A `//` comment, never a `///`: rustdoc merges a `///`
// on a `mod` line with that module's own `//!` header and then resolves
// the header's links here.
pub mod buffer;
pub mod list;
pub mod map;
pub mod maybe_null;
pub mod text;

pub use buffer::{Buffer, Bytes};
pub use list::{List, Values};
pub use map::{Entries, Entry, Map};
pub use maybe_null::MaybeNull;
pub use text::{Str, Text};

// The node lives beside this directory rather than in it, and every path
// that named it through `types` keeps working.
pub use super::value::{Payload, Tag, Value};

use std::ffi::c_void;

// These are the numbers a foreign consumer compiles against, so a change
// to any of them is an ABI break and must fail the build here rather than
// in somebody else's program. The C side asserts the same numbers with
// `_Static_assert`; both halves are needed, because Rust agreeing with
// itself proves nothing about the header.
const _: () = {
    use std::mem::{align_of, offset_of, size_of};

    assert!(size_of::<Str>() == 16);
    assert!(size_of::<Bytes>() == 16);
    assert!(size_of::<Values>() == 16);
    assert!(size_of::<Entries>() == 16);

    assert!(size_of::<Text>() == 32);
    assert!(size_of::<Buffer>() == 32);
    assert!(size_of::<List>() == 32);
    assert!(size_of::<Map>() == 32);

    assert!(size_of::<Payload>() == 32);
    assert!(size_of::<Value>() == 40);
    assert!(size_of::<Entry>() == 72);

    assert!(offset_of!(Value, tag) == 0);
    assert!(offset_of!(Value, _pad) == 4);
    assert!(offset_of!(Value, payload) == 8);
    assert!(offset_of!(Entry, key) == 0);
    assert!(offset_of!(Entry, value) == 32);

    // Every container asks its allocator for the element type's own
    // alignment, and a C caller may reasonably wire `malloc` straight
    // through. `malloc` guarantees only `max_align_t`, which on the
    // targets in view is 8. So an element type that needed more would
    // silently break every malloc-backed allocator; this makes adding one
    // a build failure instead.
    assert!(align_of::<Value>() <= 8);
    assert!(align_of::<Entry>() <= 8);
    assert!(align_of::<Text>() <= 8);
};

// A `*mut c_void` is what the allocator vtable speaks, and it must be the
// same width as the pointers in the containers above, or the `(size,
// align)` pair handed back to `free` describes a different block.
const _: () = assert!(size_of::<*mut c_void>() == size_of::<*mut u8>());
