// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! One contract for passing values between languages: a value model whose
//! C form is plain structs, a schema that describes a value so a consumer
//! that did not write it can understand it, and an envelope through which a
//! library registers the providers it offers.
//!
//! # The value model
//!
//! A value is one of seven kinds — null, bool, number, string, bytes, list
//! and map — and it is a plain `repr(C)` struct, not a handle. A C, C++ or
//! Dart consumer reads a whole tree without calling into anything. Strings
//! and slices cross as pointer and length, never NUL-terminated, so a key
//! may contain any byte.
//!
//! A number is stored as the **exact text** that declared it, so `1.10`
//! reads back as `1.10` and a 200-digit integer survives the round trip
//! that an `i64` would not.
//!
//! ## Every owned tree carries its allocator
//!
//! There is no global allocator to agree on: an owned container records
//! the allocator that made it, which is what lets a tree built inside a
//! library be freed correctly after it crosses into the host. [`Alloc::rust`]
//! is Rust's own, for a consumer that has no opinion.
//!
//! ```
//! use guatiao::{Map, ReadValue};
//!
//! let mut tls = Map::new();
//! tls.set("verify", true)?;
//!
//! let mut map = Map::new();
//! map.set("host", "10.0.0.1")?;
//! map.set("port", 5900)?;
//! map.set("tls", tls)?;
//!
//! // Get the value, then convert it: no per-kind getters on a map and no
//! // per-kind readers on a value, just a lookup and `TryInto`.
//! let host: &str = map.get("host").ok_or_missing()?.try_into()?;
//! let port: u16 = map.get("port").ok_or_missing()?.try_into()?;
//! let verify: bool = map.get("tls").get("verify").ok_or_missing()?.try_into()?;
//! assert_eq!((host, port, verify), ("10.0.0.1", 5900, true));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Where things are
//!
//! - [`value`] is the model itself: the types, the allocator, the mutation
//!   functions and the reading helpers. Its raw layer is one of the three
//!   places `unsafe` is allowed, beside the loader and the `extern "C"`
//!   surface; every other module carries `#![forbid(unsafe_code)]`.
//! - [`convert`] turns a Rust type into a value and back, which is what
//!   `#[derive(ToValue)]` and `#[derive(FromValue)]` write for you.
//! - [`schema`] describes a value: what a provider needs to be configured,
//!   and equally a record, a set of capabilities or metadata one library
//!   hands another. A schema is an ordinary value, and that value is a JSON
//!   Schema, so a consumer in any language reads one by walking a map it
//!   already knows how to walk. `#[derive(Schema)]` writes it from the
//!   declaration.
//! - [`value::merge`] layers two configurations with provenance. It is
//!   part of the value model because that is all it touches.
//! - [`library`] is the envelope a library registers through: one exported
//!   symbol, a descriptor per provider, and a registry the host owns.
//!
//! # What is deliberately not here
//!
//! **Nothing that parses or writes a format.** A parser is the dependency
//! a consumer comes here to avoid, and a serialiser is a specification
//! this crate would then be tracking on behalf of every consumer —
//! including the ones that wanted a different spelling, or no text form at
//! all. That belongs in a layer above, which can carry more than one
//! format.
//!
//! **No process-global state.** Nothing is interned, registered or
//! memoised, which is what lets a host and a library each link their own
//! copy of this crate with nothing to disagree about. A test enforces it
//! rather than a convention.

#![warn(missing_debug_implementations)]
#![warn(missing_docs)]

// The value model: plain `repr(C)` structs a foreign consumer reads with
// no call into any library, the allocator that travels with an owned
// tree, the mutation functions and the conversions.
//
// Its raw layer is one of the three places `unsafe` is allowed; the
// list is `tests/forbid_unsafe_per_module.rs`.
//
// A `//` comment, never a `///`: see `merge` below.
pub mod value;

/// Converting a Rust type to and from a value.
///
/// Public as a MODULE, for the same reason [`value::merge`] is: the reasoning a
/// caller has to understand before using these is longer than an item's
/// doc comment can hold — why the traits are named rather than
/// `From`/`TryFrom`, why the error distinguishes its cases, what
/// `Option<T>` does about the absent-versus-null distinction, and why a
/// sequence is a `Vec<T>` while bytes are opt-in. Every item is
/// re-exported at the crate root as well, so `guatiao::ToValue` works and
/// the module path is documentation rather than an obligation.
///
/// This is a re-export rather than the module itself, which is why the
/// doc comment here is safe: there is no `//!` header for it to be
/// merged with.
pub mod convert {
    pub use crate::value::convert::*;
}

// The `extern "C"` surface: everything public that is not a Rust
// convenience, reachable by a caller that cannot link Rust. A `//`
// comment, never a `///`.
//
// UNGATED, on purpose. This is an FFI library and Rust is one of its
// consumers, not the privileged one; a surface that appears only when
// somebody remembers a feature flag is a surface a C caller cannot rely
// on being there. The `c-header` feature renders the declarations, not
// the symbols.
//
// The three below are in alphabetical order because rustfmt sorts `mod`
// declarations and leaves their comments where they were: a comment that
// drifts onto the wrong item is what this block already was.
pub mod exports;

// How one library offers any number of providers to a host that has never
// heard of it: one entry symbol, descriptors that borrow, and a registry
// the host owns. A `//` comment, never a `///`.
pub mod library;

// What a value is and what a valid one looks like: the fields it
// accepts, what they default to, and what makes a value valid.
//
// THIS IS A `//` COMMENT AND MUST STAY ONE: a `///` here is MERGED with
// `schema/mod.rs`'s own
// `//!` header into one doc string, and every intra-doc link in that
// header would then resolve in the scope of THIS file — the crate root —
// where `build`, `read` and `SchemaBuilder` are not names. Every one of
// them silently became a dead link the moment this was a `///`, and
// rustdoc reports those with no file or line to find them by.
pub mod schema;

pub use convert::{Bytes, FromValue, MapError, ToValue};
// The names generated code reaches for, at the root where it names them.
// Keeping the derive's paths rooted here rather than at `value::` is what
// lets the module underneath be rearranged without touching a macro every
// consumer has already expanded.
pub use schema::Schema;
// The container types and the node's own vocabulary, beside the four
// already here: `Text::new`, `Buffer::new`, `Entry::key` and `Tag` are
// named at this level by the API header and by generated code, so they
// resolve at this level.
//
// `Bytes` above is `convert::Bytes`, the marker that says a field crosses
// as the bytes kind. The BORROWED view of the same name stays at
// `value::types::Bytes`, since one name cannot be both.
pub use value::types::Str;
pub use value::{
    Alloc, Buffer, Entry, List, MAX_DEPTH, Map, ReadValue, Status, Tag, Text, Value, ValueError,
};

#[cfg(feature = "provider")]
pub use guatiao_derive::kind;
/// `#[derive(ToValue)]` and `#[derive(FromValue)]`, behind the
/// `derive` feature.
///
/// A macro and a trait live in different namespaces, so these share their
/// names with the traits above rather than shadowing them — serde's
/// arrangement, for the same reason.
///
/// ```
/// use guatiao::Alloc;
/// use guatiao::value::read::{bool_or, int_or, str_or};
/// use guatiao::{FromValue, ToValue};
///
/// #[derive(Debug, PartialEq, ToValue, FromValue)]
/// struct Connection {
///     host: String,
///     port: i64,
///     #[map(rename = "view-only")]
///     view_only: bool,
///     motd: Option<String>,
/// }
///
/// let alloc = Alloc::rust();
///
/// let original = Connection {
///     host: "10.0.0.1".into(),
///     port: 5900,
///     view_only: true,
///     motd: None,
/// };
///
/// let map = original.to_value(alloc)?;
/// assert_eq!(str_or(map.get("host"), ""), "10.0.0.1");
/// assert_eq!(int_or(map.get("port"), 0), 5900);
/// assert!(bool_or(map.get("view-only"), false));
/// // `None` omits the key rather than storing a null.
/// assert!(map.get("motd").is_none());
///
/// assert_eq!(Connection::from_value(&map)?, original);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # The shapes it refuses, and that it really does refuse them
///
/// Each of the following fails to compile. The messages themselves are
/// pinned by unit tests in `guatiao-derive`; these pin the half a
/// token-level test cannot show, which is that the compiler agrees.
///
/// A tuple struct has no names to key a map by:
///
/// ```compile_fail
/// use guatiao::ToValue;
/// #[derive(ToValue)]
/// struct Endpoint(String, i64);
/// ```
///
/// The same declaration with names is accepted, which is what makes the
/// case above a rejection of the *shape* rather than of something
/// incidental:
///
/// ```
/// use guatiao::ToValue;
/// #[derive(ToValue)]
/// struct Endpoint { host: String, port: i64 }
/// ```
///
/// An enum whose variants carry fields has no one set of keys, and the key
/// that tells its variants apart is a wire-format decision this crate does
/// not make for you:
///
/// ```compile_fail
/// use guatiao::ToValue;
/// #[derive(ToValue)]
/// enum Transport { Tcp { port: i64 }, Unix { path: String } }
/// ```
///
/// Name it and the same declaration is accepted — each value a map with the
/// variant's name under the tag, beside its fields. An enum of **unit**
/// variants needs no tag at all: its value is the variant's name.
///
/// ```
/// use guatiao::{FromValue, ToValue};
/// #[derive(ToValue, FromValue, Debug, PartialEq)]
/// #[map(tag = "transport")]
/// enum Transport { Tcp { port: i64 }, Unix { path: String } }
///
/// #[derive(ToValue, FromValue, Debug, PartialEq)]
/// enum Level { Off, #[map(rename = "warn")] Warning, On }
///
/// let alloc = guatiao::Alloc::rust();
/// let tcp = Transport::Tcp { port: 5900 }.to_value(alloc)?;
/// assert_eq!(tcp.get("transport").and_then(guatiao::Value::as_str), Some("Tcp"));
/// assert_eq!(Level::Warning.to_value(alloc)?.as_str(), Some("warn"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// A generic parameter would need a bound the macro cannot infer:
///
/// ```compile_fail
/// use guatiao::ToValue;
/// #[derive(ToValue)]
/// struct Wrapper<T> { inner: T }
/// ```
///
/// A field type with no conversion is rejected at the field, not deep
/// inside the expansion — `std::time::Duration` implements neither
/// [`ToValue`] nor [`FromValue`]:
///
/// ```compile_fail
/// use guatiao::ToValue;
/// #[derive(ToValue)]
/// struct Timeout { after: std::time::Duration }
/// ```
///
/// And two fields cannot land on one key, because the second would
/// silently replace the first:
///
/// ```compile_fail
/// use guatiao::ToValue;
/// #[derive(ToValue)]
/// struct Clash { host: String, #[map(rename = "host")] hostname: String }
/// ```
#[cfg(feature = "derive")]
pub use guatiao_derive::{FromValue, Schema, ToValue};
pub use value::merge::{MergeError, MergeMode, MergeOptions, MergeOverrides, Provenance, Source};
