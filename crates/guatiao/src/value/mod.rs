// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The C form of the value model: plain structs, no opaque handles.
//!
//! Four rules hold throughout: a node is born with all 40 of its bytes
//! initialised; the tag alone selects an arm, and an unknown one means
//! skip the value rather than fail; `cap == 0` never reaches an
//! allocator; a refused operation changes nothing. `unsafe` is confined
//! here, and `tests/forbid_unsafe_per_module.rs` enforces it.

pub mod alloc;

// A `//` comment on every `mod` line, never a `///`: rustdoc merges one
// with that module's own `//!` header, resolves its links in THIS scope,
// and reports the dead ones with no file or line to find them by.
pub mod convert;
pub mod error;
pub mod merge;
mod raw;
pub mod read;
pub mod status;
pub mod types;
pub mod wire;
// The whole-tree walks besides drop: a deep copy, and equality.
mod walk;
// A stutter to clippy; every file here is named for the type it holds.
#[allow(clippy::module_inception)]
pub mod value;

pub use alloc::{Alloc, AllocError, Allocator, rust_alloc};
pub use convert::{Bytes, FromValue, MapError, ToValue, TryAsMut, TryAsRef};
pub use error::{MAX_DEPTH, ValueError};
pub use read::{Dump, bool_or, bytes_or, float_or, int_or, str_or};
pub use status::Status;
// `Str` is here because every C signature in `exports` names it; the
// other views stay behind `types::`, clear of `convert::Bytes`.
pub use types::{Buffer, Entry, List, Map, Number, Payload, Str, Tag, Text, Value};
