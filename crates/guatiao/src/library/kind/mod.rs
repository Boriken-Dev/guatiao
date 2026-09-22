// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A provider kind as a Rust trait, and the runtime the kind and
//! provider macros expand into calls to.
//!
//! A **kind** is a trait the host and the library both compile against.
//! `#[guatiao::kind]` emits, beside the trait, a `repr(C)` function table
//! whose slots are monomorphised shims, an `impl Kind for dyn Trait`
//! naming the table's floor and hash, and an `impl Trait for Remote<dyn
//! Trait>` that calls through a table it was handed. Every `unsafe` those
//! expansions need is a call into this module, written once and reviewed
//! once; generated code only calls it.
//!
//! # The three doors into a table
//!
//! - From a registry: [`Registry::offers`](super::Registry::offers) and
//!   [`Provider::as_kind`](super::Provider::as_kind). Safe, because the
//!   registry read the descriptor under its guards.
//! - From a host, inside a library: [`Host::offers`] and
//!   [`ProviderInfo::as_kind`]. Safe for the same reason.
//! - From anywhere else: [`Remote::from_raw`], `unsafe`, with one promise
//!   — the bytes are a table for this kind's name.
//!
//! All three run one validation: the table is at least the kind's floor,
//! its header hash matches, and no required slot is null. A `floor_hash`
//! of `0` — a table with no header, or a hand-written C one — is accepted
//! by `from_raw` alone, where the caller stated the promise; a registry
//! or host never offers such a table as a kind.
//!
//! # Which providers are offers
//!
//! Only a table in [`ProviderInfo::tables`] is validated as a kind. The
//! shared [`vtable`](ProviderInfo::vtable) stays the untyped path
//! ([`Provider::table_for`](super::Provider::table_for)), so a library
//! written before per-kind tables existed keeps working and is never
//! mistaken for a typed table it does not have a header for.

#![allow(non_camel_case_types)]

use std::ffi::c_void;
use std::marker::PhantomData;
use std::mem::size_of;
use std::sync::Arc;

use super::desc::{KindTable, KindTables, LibraryInfo, ProviderInfo};
use super::raw::{Host, ProviderView};
use crate::value::ValueError;
use crate::value::alloc::Alloc;
use crate::value::status::Status;
use crate::value::types::{Map, Str, Text, Value};

/// What a kind declares about its table. Implemented for `dyn Trait` by
/// `#[guatiao::kind]`; the host and the library share the impl.
pub trait Kind: 'static {
    /// The kind's name in a descriptor's `kinds` and `tables`.
    const NAME: &'static str;
    /// The `repr(C)` table, which starts with a [`KindHeader`].
    type Vtable: 'static;
    /// One past the end of the last required slot. A table shorter than
    /// this cannot be called.
    const FLOOR: usize;
    /// FNV-1a over the required methods' names and normalised signatures.
    /// A table carrying another number was built from a different
    /// declaration of this kind.
    const FLOOR_HASH: u32;
    /// Every required slot as `(method name, one past its end)`, so a
    /// null one is refused by name.
    const REQUIRED: &'static [(&'static str, usize)];
    /// Whether this is an **object kind** (`#[guatiao::kind(object)]`): a
    /// handle one caller owns, with a `destroy` slot first and `&mut self`
    /// methods, never offered by a registry. See [`Object`].
    const OBJECT: bool = false;

    /// How many objects of this kind, and of the kinds its methods hand
    /// back, this image has made and not yet seen destroyed. `seen` stops
    /// a cycle of kinds. What a derived library's `unload` refuses on; a
    /// hand-written kind counts nothing.
    #[doc(hidden)]
    fn live_objects(_seen: &mut Vec<&'static str>) -> usize {
        0
    }

    /// The trait object over a proxy. The macro writes `remote`, because
    /// only it knows the trait.
    fn as_dyn(remote: &Remote<Self>) -> &Self;
    /// The same, mutably — what an [`Object`] hands out.
    fn as_dyn_mut(remote: &mut Remote<Self>) -> &mut Self;
    /// The same, boxed.
    fn boxed(remote: Remote<Self>) -> Box<Self>;
    /// The same, shared.
    fn shared(remote: Remote<Self>) -> Arc<Self>;
}

/// The first eight bytes of every kind table.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KindHeader {
    /// `sizeof` the table as the library compiled it. Always first.
    pub struct_size: u32,
    /// [`Kind::FLOOR_HASH`] of the declaration the table was built from.
    pub floor_hash: u32,
}

impl KindHeader {
    /// A header for a table of `size` bytes built from a declaration
    /// hashing to `hash`.
    pub const fn new(size: usize, hash: u32) -> KindHeader {
        KindHeader {
            struct_size: size as u32,
            floor_hash: hash,
        }
    }
}

/// Why a call across a kind failed, as a provider states it.
///
/// A shim writes one through an out-pointer; the proxy hands it back as
/// the `Err` of the trait method. `message` may be empty.
#[repr(C)]
#[derive(Debug)]
pub struct ProviderError {
    /// What went wrong, as a status.
    pub status: Status,
    /// The provider's own words, possibly empty.
    pub message: Text,
}

impl ProviderError {
    /// An error with a message.
    pub fn new(status: Status, message: &str) -> ProviderError {
        ProviderError {
            status,
            message: Text::new(message),
        }
    }

    /// The placeholder a proxy passes to a shim: not an error. A shim
    /// that fails overwrites it.
    pub fn none() -> ProviderError {
        ProviderError {
            status: Status::GUATIAO_OK,
            message: Text::new(""),
        }
    }

    /// The provider's own words, or empty.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl From<Status> for ProviderError {
    fn from(status: Status) -> ProviderError {
        ProviderError::new(status, "")
    }
}

impl From<ValueError> for ProviderError {
    fn from(e: ValueError) -> ProviderError {
        ProviderError::new(Status::from(e), &e.to_string())
    }
}

/// So a provider built through an infallible `From<C>` (a type that IS
/// its configuration, `config = Self`) satisfies `TryFrom<C, Error:
/// Into<ProviderError>>` with nothing written.
impl From<std::convert::Infallible> for ProviderError {
    fn from(never: std::convert::Infallible) -> ProviderError {
        match never {}
    }
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = self.message();
        if message.is_empty() {
            write!(f, "the provider answered {:?}", self.status)
        } else {
            write!(f, "{message} ({:?})", self.status)
        }
    }
}

impl std::error::Error for ProviderError {}

/// Why a table could not be used as a kind. Each names the check that
/// failed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum KindMismatch {
    /// The provider declares no table for this kind.
    NoTable,
    /// The table is shorter than the kind's last required slot.
    BelowFloor {
        /// What the library declared.
        size: usize,
        /// What the kind needs.
        floor: usize,
    },
    /// The table was built from a different declaration of this kind.
    HashMismatch {
        /// This kind's hash.
        expected: u32,
        /// The table's.
        found: u32,
    },
    /// A required slot is null, named.
    NullRequiredSlot(&'static str),
}

impl std::fmt::Display for KindMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KindMismatch::NoTable => f.write_str("the provider declares no table for this kind"),
            KindMismatch::BelowFloor { size, floor } => {
                write!(f, "the table is {size} bytes and the kind needs {floor}")
            }
            KindMismatch::HashMismatch { expected, found } => write!(
                f,
                "the table was built from another declaration of this kind ({found:#010x}, expected {expected:#010x})"
            ),
            KindMismatch::NullRequiredSlot(name) => {
                write!(f, "the required slot `{name}` is null")
            }
        }
    }
}

impl std::error::Error for KindMismatch {}

mod object;
mod parts;
mod proxy;
mod shim;

pub use self::object::*;
pub use self::parts::*;
pub use self::proxy::*;
pub use self::shim::*;

#[cfg(test)]
mod tests;
