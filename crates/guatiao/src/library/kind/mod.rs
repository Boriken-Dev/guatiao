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
//!   registry checked the descriptor where it entered.
//! - From a host, inside a library: [`Host::offers`] and [`Host::offer`].
//!   Safe for the same reason: the host's answers are checked the same
//!   way. [`ProviderInfo::as_kind`] reads a
//!   descriptor a library built itself the same way.
//! - From anywhere else: [`Remote::from_raw`], `unsafe`, with one promise
//!   — the bytes are a table for this kind's name.
//!
//! All three run one validation: the table covers every slot of the
//! kind's, its header hash matches, and no required slot is null. A
//! `floor_hash` of `0` — a hand-written C table naming no hash — is
//! accepted by `from_raw` alone, where the caller stated the promise; a
//! registry or host never offers such a table as a kind.
//!
//! # Which providers are offers
//!
//! Only a table in [`ProviderInfo::tables`] is validated as a kind. The
//! shared [`vtable`](ProviderInfo::vtable) stays the untyped path
//! ([`Provider::table_for`](super::Provider::table_for)), so a library
//! written before per-kind tables existed keeps working and is never
//! mistaken for a typed table it does not have a header for.

#![allow(non_camel_case_types)]

use alloc::sync::Arc;
use core::ffi::c_void;
use core::marker::PhantomData;
use core::mem::size_of;

use super::desc::{KindTable, KindTables, LibraryInfo, ProviderInfo};
use super::raw::{Hold, Host, ProviderView};
use crate::schema::ValidationError;
use crate::value::ValueError;
use crate::value::alloc::Alloc;
use crate::value::status::Status;
use crate::value::types::{List, Map, Str, Text, Value};

/// What a kind declares about its table. Implemented for `dyn Trait` by
/// `#[guatiao::kind]`; the host and the library share the impl.
pub trait Kind: 'static {
    /// The kind's name in a descriptor's `kinds` and `tables`.
    const NAME: &'static str;
    /// The `repr(C)` table, which starts with a [`KindHeader`].
    type Vtable: 'static;
    /// The size of [`Kind::Vtable`]: every slot. A table declaring less is
    /// refused; a larger one has slots appended after these.
    const FLOOR: usize;
    /// FNV-1a over the required methods' names and normalised signatures.
    /// A table carrying another number was built from a different
    /// declaration of this kind.
    ///
    /// The signatures are read as spelled, with standard and `guatiao`
    /// paths reduced to the bare name (`std::vec::Vec<T>` is `Vec<T>`);
    /// a type of the kind crate's own is hashed by the name it is written
    /// with, so renaming one is a new declaration.
    const FLOOR_HASH: u32;
    /// Every required slot as `(method name, one past its end)`, so a
    /// null one is refused by name.
    const REQUIRED: &'static [(&'static str, usize)];
    /// Whether this is an **object kind** (`#[guatiao::kind(object)]`): a
    /// handle one caller owns, with a `destroy` slot first and `&mut self`
    /// methods, never offered by a registry. See [`Object`].
    ///
    /// A registry or host door answers [`KindMismatch::ObjectKind`] for
    /// one, and `#[derive(Provider)]` refuses to name one at compile time:
    ///
    /// ```compile_fail
    /// use greeter_kind::Conversation;
    /// use guatiao::library::ProviderError;
    ///
    /// #[derive(Default, guatiao::Provider)]
    /// #[provider(Conversation)]
    /// struct Talk;
    ///
    /// impl Conversation for Talk {
    ///     fn say(&mut self, _: &str) -> Result<(), ProviderError> { Ok(()) }
    ///     fn read(&mut self, _: &mut [u8]) -> i64 { 0 }
    ///     fn turns(&self) -> i64 { 0 }
    /// }
    /// ```
    const OBJECT: bool = false;

    /// How many objects of this kind, and of the kinds its methods hand
    /// back, this image has made and not yet seen destroyed. `seen` stops
    /// a cycle of kinds. What a derived library's `unload` refuses on; a
    /// hand-written kind counts nothing.
    #[doc(hidden)]
    fn live_objects(_seen: &mut Vec<&'static str>) -> usize {
        0
    }

    /// Pushes this kind's description, then those of the object kinds
    /// its slots take or answer, onto `out`, each once: `seen` holds the
    /// names already pushed. What [`kind_document`] gathers for a
    /// provider's descriptor; a hand-written kind describes nothing.
    #[doc(hidden)]
    fn describe(_seen: &mut Vec<&'static str>, _out: &mut Vec<String>) {}

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
///
/// **A tree answer is built where the caller asks.** A slot whose out
/// parameter is a `guatiao_value *`, `guatiao_map *` or `guatiao_list *`
/// finds it holding a carrier, which owns nothing: an empty map or list,
/// or an ABSENT value, whose `alloc` (`payload.map.alloc`, for a value)
/// names the allocator the caller wants the answer in, or is null. Build
/// every container and string of the answer through that allocator, or
/// move what was built elsewhere into it before returning, and write the
/// answer over the carrier without freeing it: a caller may hold the
/// answer after the library is unloaded. A slot `#[guatiao::kind]`
/// generates always does; a Rust caller also copies, on arrival, an
/// answer some other slot built elsewhere, and a C caller does the same
/// with `guatiao_value_make_local` while the library is loaded. A caller
/// passes an initialised out slot: a carrier, or zeroes, which answers in
/// the slot's own allocator, to be freed while the library is loaded.
///
/// **A tree argument by value is a move.** A slot argument that is a
/// `guatiao_value *`, `guatiao_map *` or `guatiao_list *` without `const`
/// is a tree the caller hands over with the call: the slot owns it from
/// then on, whatever it answers, and the caller never frees it. It lives
/// in the caller's allocators, which outlive what is built through them.
///
/// **Unless the slot's comment says it is lent.** Then it is the caller's
/// tree for the call only (a Rust `&mut Map`, `&mut List` or `&mut
/// Value`): read it, change it, grow it, never free it or keep the
/// pointer. Build what goes into it through the allocator it records; a
/// Rust caller moves anything built elsewhere there once the call returns,
/// and removes a top-level key the slot wrote into a lent map that is not
/// UTF-8; a C caller calls `guatiao_map_settle_lent` on a lent map, which
/// does both, and `guatiao_value_make_local` on a lent value or list.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KindHeader {
    /// `sizeof` the table as the library compiled it. Always first; a
    /// table declaring less than its kind's whole table is refused.
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
impl From<core::convert::Infallible> for ProviderError {
    fn from(never: core::convert::Infallible) -> ProviderError {
        match never {}
    }
}

impl core::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let message = self.message();
        if message.is_empty() {
            write!(f, "the provider answered {:?}", self.status)
        } else {
            write!(f, "{message} ({:?})", self.status)
        }
    }
}

impl core::error::Error for ProviderError {}

/// Why a table could not be used as a kind. Each names the check that
/// failed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum KindMismatch {
    /// The provider declares no table for this kind.
    NoTable,
    /// The table is smaller than the kind's whole table.
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
    /// The kind is an object kind, which a provider never serves: its
    /// shims read `ctx` as an object cell one caller owns, and a provider's
    /// `ctx` is shared.
    ObjectKind,
    /// The table's address is not aligned for the kind's table type.
    Misaligned,
    /// The provider's descriptor was refused where it was read, before
    /// any table: which part, and what was wrong.
    Unreadable(String),
}

impl core::fmt::Display for KindMismatch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
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
            KindMismatch::ObjectKind => f.write_str("an object kind is never served by a provider"),
            KindMismatch::Misaligned => {
                f.write_str("the table is not aligned for the kind's table type")
            }
            KindMismatch::Unreadable(why) => {
                write!(f, "the provider's descriptor is refused: {why}")
            }
        }
    }
}

impl core::error::Error for KindMismatch {}

/// Why an instance could not be built from a configuration.
#[derive(Debug)]
#[non_exhaustive]
pub enum InstantiateError {
    /// The configuration does not fit the provider's schema, so the
    /// provider was never asked. Names the field, never its value.
    Invalid(ValidationError),
    /// The provider was asked and refused, or cannot build instances, or
    /// its table does not fit the kind: its own words and status.
    Refused(ProviderError),
}

impl InstantiateError {
    /// The status a C caller would see: `GUATIAO_ERR_BAD_VALUE` for a
    /// configuration that does not fit, the provider's own otherwise.
    pub fn status(&self) -> Status {
        match self {
            InstantiateError::Invalid(_) => Status::GUATIAO_ERR_BAD_VALUE,
            InstantiateError::Refused(e) => e.status,
        }
    }
}

impl core::fmt::Display for InstantiateError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            InstantiateError::Invalid(e) => write!(f, "the configuration does not fit: {e}"),
            InstantiateError::Refused(e) => write!(f, "{e}"),
        }
    }
}

impl core::error::Error for InstantiateError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            InstantiateError::Invalid(e) => Some(e),
            InstantiateError::Refused(e) => Some(e),
        }
    }
}

impl From<ValidationError> for InstantiateError {
    fn from(e: ValidationError) -> InstantiateError {
        InstantiateError::Invalid(e)
    }
}

impl From<ProviderError> for InstantiateError {
    fn from(e: ProviderError) -> InstantiateError {
        InstantiateError::Refused(e)
    }
}

mod description;
mod object;
mod parts;
mod proxy;
// Tables built at run time from a description, over a `Mechanism`.
// Carries `unsafe`, listed in `tests/forbid_unsafe_per_module.rs`.
mod runtime;
mod shim;

pub use self::description::*;
pub use self::object::*;
pub use self::parts::*;
pub use self::proxy::*;
pub use self::runtime::*;
pub use self::shim::*;

#[cfg(test)]
mod tests;
