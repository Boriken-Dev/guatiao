// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The C form of the value model: plain structs, no opaque handles.
//!
//! A C, C++ or Dart consumer reads a tree here with no call into any
//! library — the types are the interface. Mutation needs functions, and
//! those are ordinary Rust that a Rust host or library calls directly; the
//! `exports` module wraps them as `extern "C"` for callers that cannot.
//!
//! # This is the only module with `unsafe` in it
//!
//! Every other module in the crate carries `#![forbid(unsafe_code)]`, so
//! the value model stays provably safe and an auditor's scope is this one
//! directory. A test enforces it rather than a convention.
//!
//! # The four rules everything here obeys
//!
//! 1. **A node is born with all 40 of its bytes initialised.** Writing one
//!    union arm leaves the rest of the payload uninitialised, and reading
//!    a wide arm off that is undefined behaviour — not garbage, undefined.
//!    Nothing in the toolchain warns.
//! 2. **The tag is the only thing that selects an arm**, it is a plain
//!    `u32`, and an unrecognised one means skip this value rather than
//!    fail.
//! 3. **`cap == 0` never reaches an allocator.** Freeing a literal's
//!    pointer corrupts the heap immediately, with no unwinding and no
//!    chance for the host to log anything.
//! 4. **A refused operation changes nothing.** The allocator is supplied
//!    by the caller and may fail, so every mutation builds what it needs
//!    before it touches the target.

pub mod alloc;

// Converting a Rust type to and from a value: the four traits the derive
// implements. A `//` comment, never a `///`: see `read` below.
pub mod convert;
// Combining two values, later layer winning. A value operation and
// nothing else, which is why it lives here rather than beside the
// schema that can declare a mode for one of its options. A `//`
// comment, never a `///`.
pub mod merge;
pub mod mutate;
// What counts as a number: the JSON grammar, checked at construction.
// Private because it is the value model's own rule rather than a service
// a caller needs.
mod number;
mod raw;
// What a call across the boundary reports. Ungated, so a library can name
// it without exporting anything. A `//` comment, never a `///`.
pub mod status;

// Reading a value comfortably from Rust: iteration, comparison, a debug
// view, and the getters that take a caller default.
//
// THIS IS A `//` COMMENT AND MUST STAY ONE. A `///` here is MERGED with
// `read.rs`'s own `//!` header into one doc string, and rustdoc then
// resolves that header's links in THIS module's scope rather than in
// `read`'s -- so every `super::mutate` in it silently means `guatiao`
// rather than `guatiao::value` and becomes a dead link. rustdoc reports
// those with no file or line to find them by.
pub mod read;

pub mod types;
// The node itself. A `//` comment, never a `///`.
//
// `value::value` reads as a stutter to clippy, but every file here is
// named for the type it holds and `Value` is no exception.
#[allow(clippy::module_inception)]
pub mod value;

pub use alloc::{Alloc, AllocError, Allocator, rust_alloc};
pub use convert::{Bytes, FromValue, MapError, ToValue};
pub use mutate::{MAX_DEPTH, ValueError};
pub use read::{
    Dump, ReadValue, bool_or, bytes_or, entries, equal, float_or, int_or, items, keys, str_or,
};
pub use status::Status;
// `Str` is here because every C signature in `exports` names it and a
// caller building one should not have to find the module. The other
// three BORROWED views -- `Bytes`, `Values`, `Entries` -- stay behind
// `types::`: a Rust caller reads a `&[u8]` or a slice instead, and
// `types::Bytes` would collide with `convert::Bytes` (a field type that
// says "cross as the bytes kind").
pub use types::{Buffer, Entry, List, Map, Payload, Str, Tag, Text, Value};
