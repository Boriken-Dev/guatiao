// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The C form of a value: plain structs, no opaque handles.
//!
//! A **view** is `{const ptr, len}`, an **owned** container is
//! `{ptr, len, cap, alloc}`: [`Str`]/[`Text`], [`Bytes`]/[`Buffer`],
//! [`Values`]/[`List`], [`Entries`]/[`Map`]. A [`Value`] holds owned
//! containers, and a view of a node is a const pointer to one because a
//! list's element type IS the node type.
//!
//! **`cap == 0` means the buffer is not owned**, `Vec`'s own rule, which
//! is what lets a C literal be read like any other value and never
//! freed. Only GROWTH copies out, so a literal that will be mutated must
//! live in writable storage, and `cap >= len` does not hold on input.
//!
//! **The fields are private, and that is the safety argument**: every
//! `SAFETY:` comment here assumes a node reached in safe code is well
//! formed, which holds because safe code cannot build one by hand.
//!
//! ```compile_fail
//! let v = guatiao::Value { tag: 7, _pad: 0, payload: unreachable!() };
//! ```
//!
//! ```compile_fail
//! let mut m = guatiao::Map::new();
//! m.len = 4096;
//! ```
//!
//! ```compile_fail
//! let a = guatiao::Text::new("hello");
//! let b = guatiao::Text { ptr: a.ptr, len: a.len, cap: a.cap, alloc: a.alloc };
//! ```
//!
//! A NUMBER and a STRING share the `text` arm and not a type, so no
//! `&mut Text` comes off a number:
//!
//! ```
//! use guatiao::{Text, TryAsMut, Value};
//! let mut v = Value::from(1i64);
//! assert!(TryAsMut::<Text>::try_as_mut(&mut v).is_none());
//! ```

#![allow(non_camel_case_types)]
// Right for the model, wrong for SCREAMING_CASE C constants: cbindgen
// copies every doc comment here into the header.
#![allow(missing_docs)]

// A `//` comment, never a `///`: see `value/mod.rs`.
pub mod buffer;
pub mod list;
pub mod map;
pub mod maybe_null;
pub mod number;
pub mod text;

pub use buffer::{Buffer, Bytes};
pub use list::{List, Values};
pub use map::{Entries, Entry, Map};
pub use maybe_null::MaybeNull;
pub use number::Number;
pub use text::{Str, Text};

// The node lives beside this directory, and every path through `types`
// still reaches it.
pub use super::value::{Payload, Tag, Value};

use std::ffi::c_void;

use crate::value::error::ValueError;

/// The short constructors abort on an allocation failure; the `_in` ones
/// report it.
///
/// A short constructor allocates on Rust's own heap, and a failure there
/// is the condition `String::from` and `Vec::push` already meet: the
/// standard library aborts rather than returning. A foreign allocator
/// returning null is a different statement -- it may be an arena that is
/// merely full -- so `_in` hands the refusal back.
pub(crate) fn or_abort<T>(built: Result<T, ValueError>) -> T {
    match built {
        Ok(v) => v,
        Err(e) => panic!("guatiao: building a value on Rust's heap failed: {e}"),
    }
}

// The numbers a foreign consumer compiles against: a change to one is an
// ABI break and fails the build here. The C side asserts the same
// numbers with `_Static_assert`, because Rust agreeing with itself
// proves nothing about the header.
const _: () = {
    use std::mem::{align_of, offset_of, size_of};

    // In pointer widths, so the one layout holds on every target: a view
    // is two words, an owned container four, a node a tag and its padding
    // then a container, an entry a key then a node.
    const P: usize = size_of::<usize>();

    assert!(size_of::<Str>() == 2 * P);
    assert!(size_of::<Bytes>() == 2 * P);
    assert!(size_of::<Values>() == 2 * P);
    assert!(size_of::<Entries>() == 2 * P);

    // `Number` is a `Text` under a different tag, so it is the same
    // storage; nothing new crosses the boundary.
    assert!(size_of::<Number>() == 4 * P);
    assert!(size_of::<Text>() == 4 * P);
    assert!(size_of::<Buffer>() == 4 * P);
    assert!(size_of::<List>() == 4 * P);
    assert!(size_of::<Map>() == 4 * P);

    assert!(size_of::<Payload>() == 4 * P);
    assert!(size_of::<Value>() == 8 + 4 * P);
    assert!(size_of::<Entry>() == 4 * P + size_of::<Value>());

    assert!(offset_of!(Value, tag) == 0);
    assert!(offset_of!(Value, _pad) == 4);
    assert!(offset_of!(Value, payload) == 8);
    assert!(offset_of!(Entry, key) == 0);
    assert!(offset_of!(Entry, value) == 4 * P);

    // A C caller may wire `malloc` straight through, and `malloc`
    // guarantees only `max_align_t` -- 8 on the targets in view. An
    // element type needing more would silently break every malloc-backed
    // allocator, so adding one is a build failure instead.
    assert!(align_of::<Value>() <= 8);
    assert!(align_of::<Entry>() <= 8);
    assert!(align_of::<Text>() <= 8);
};

// The allocator vtable speaks `*mut c_void`, which must be as wide as
// the containers' pointers or the `(size, align)` pair handed to `free`
// describes a different block.
const _: () = assert!(size_of::<*mut c_void>() == size_of::<*mut u8>());
