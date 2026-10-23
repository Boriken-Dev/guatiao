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
//! use guatiao::Map;
//!
//! let mut tls = Map::new();
//! tls.set("verify", true);
//!
//! let mut map = Map::new();
//! map.set("host", "10.0.0.1");
//! map.set("port", 5900);
//! map.set("tls", tls);
//!
//! // Get the value, then convert it: no per-kind getters on a map and no
//! // per-kind readers on a value, just a lookup and `TryInto`.
//! let host: &str = map.required("host")?.try_into()?;
//! let port: u16 = map.required("port")?.try_into()?;
//! let tls: &Map = map.required("tls")?.try_into()?;
//! let verify: bool = tls.required("verify")?.try_into()?;
//! assert_eq!((host, port, verify), ("10.0.0.1", 5900, true));
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```
//!
//! # Where things are
//!
//! - [`value`] is the model itself: the containers, which carry every
//!   operation on what they hold, the node they become, the allocator
//!   that travels with an owned tree, and the reading helpers. Its raw
//!   layer is one of the three places `unsafe` is allowed, beside the
//!   loader and the `extern "C"` surface; every other module carries
//!   `#![forbid(unsafe_code)]`.
//! - [`value::convert`] turns a Rust type into a value and back, which is what
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
extern crate alloc;

// The value model: plain `repr(C)` structs a foreign consumer reads with
// no call into any library, the allocator that travels with an owned
// tree, and the conversions.
//
// Its raw layer is one of the three places `unsafe` is allowed; the
// list is `tests/forbid_unsafe_per_module.rs`.
//
// A `//` comment, never a `///`: see `merge` below.

// A browser has no files to map and no symbols to look up: the loader
// means nothing there, and turning it on is a mistake worth a clear error
// rather than a link failure deep inside `libloading`.
#[cfg(all(feature = "load", target_family = "wasm"))]
compile_error!("the `load` feature maps libraries from files, which a wasm target cannot do");

pub mod value;

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

/// Paths generated code reaches `alloc` through, so a crate that expands a
/// macro of ours need not declare `extern crate alloc` itself.
#[doc(hidden)]
pub mod __private {
    pub extern crate alloc;

    use crate::schema::{FieldBuilder, KindBuilder};
    use crate::value::{Alloc, Map, Value, ValueError};

    /// Moves every entry of the map `flat` into `map`: a flattened member,
    /// or a tagged enum's newtype arm beside its tag. Not a map is
    /// [`ValueError::WrongKind`]; a key `map` already holds is
    /// [`ValueError::DuplicateKey`].
    pub fn flatten_into(map: &mut Map, flat: Value, alloc: Alloc) -> Result<(), ValueError> {
        let flat = Map::try_from(flat).map_err(|_| ValueError::WrongKind)?;
        for (key, value) in flat {
            if map.contains_key(&key) {
                return Err(ValueError::DuplicateKey);
            }
            map.set_in(&key, value, alloc)?;
        }
        Ok(())
    }

    /// The fields an object kind declares, to be declared on another.
    pub fn fields_of(kind: KindBuilder) -> alloc::vec::Vec<FieldBuilder> {
        kind.into_fields()
    }

    /// The kind of a value, for an enum told apart by kind; an unknown
    /// tag is no kind, which no variant reads.
    pub fn kinds_of(value: &Value) -> crate::Kinds {
        value.tag().map_or(crate::Kinds::NONE, crate::Kinds::of)
    }

    /// What an enum told apart by kind refuses a value with: the kinds
    /// its variants read, never the value.
    pub fn untagged_refused(expected: crate::Kinds) -> crate::MapError {
        crate::MapError::bad_value(alloc::string::ToString::to_string(&expected))
    }

    /// A `#[map(as_text)]` field written: its `Display` text.
    pub fn text_of<T: core::fmt::Display + ?Sized>(
        value: &T,
        alloc: Alloc,
    ) -> Result<Value, ValueError> {
        let text = alloc::string::ToString::to_string(value);
        Ok(crate::Text::new_in(alloc, &text)?.into())
    }

    /// A `#[map(as_text)]` field read: text through its `FromStr`, whose
    /// refusal is reported in the parse error's own words.
    pub fn from_text<T>(value: &Value) -> Result<T, crate::MapError>
    where
        T: core::str::FromStr,
        T::Err: core::fmt::Display,
    {
        let text = <&str>::try_from(value)?;
        text.parse().map_err(|e: T::Err| {
            crate::MapError::bad_value(alloc::format!("text this field reads ({e})"))
        })
    }

    /// An `Option<T>` `#[map(as_text)]` field read: a stored null is
    /// `None`, as for any `Option`.
    pub fn from_text_or_null<T>(value: &Value) -> Result<Option<T>, crate::MapError>
    where
        T: core::str::FromStr,
        T::Err: core::fmt::Display,
    {
        if value.tag() == Ok(crate::Tag::GUATIAO_NULL) {
            return Ok(None);
        }
        from_text(value).map(Some)
    }

    /// A `#[map(as_text)]` field's kind: a string, accepting null too for
    /// an `Option`.
    pub fn text_kind(alloc: Alloc, nullable: bool) -> KindBuilder {
        let kind = KindBuilder::string_in(alloc);
        if nullable { kind.nullable() } else { kind }
    }
}

pub use value::convert::{Bytes, FromValue, Kinds, MapError, ToValue, TryAsMut, TryAsRef};
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
// `Bytes` above is `value::convert::Bytes`, the marker that says a field crosses
// as the bytes kind. The BORROWED view of the same name stays at
// `value::types::Bytes`, since one name cannot be both.
pub use value::types::Str;
pub use value::{
    Alloc, AllocError, Buffer, Date, DateTime, Entry, List, MAX_DEPTH, Map, Number, Payload,
    Secret, Span, Status, Tag, Text, Time, Timestamp, Value, ValueError,
};

/// # What `#[derive(Provider)]` refuses
///
/// Each kind once: a second table for one kind would never be read.
///
/// ```compile_fail
/// use greeter_kind::Counter;
///
/// #[derive(Default, guatiao::Provider)]
/// #[provider(Counter, Counter)]
/// struct Twice;
///
/// impl Counter for Twice {
///     fn count(&self) -> i64 { 0 }
/// }
/// ```
///
/// Named once, the same provider is accepted:
///
/// ```
/// use greeter_kind::Counter;
///
/// #[derive(Default, guatiao::Provider)]
/// #[provider(Counter)]
/// struct Once;
///
/// impl Counter for Once {
///     fn count(&self) -> i64 { 0 }
/// }
/// ```
#[cfg(feature = "provider")]
pub use guatiao_derive::Provider;
/// # What `#[guatiao::kind]` refuses, and that it really does
///
/// The messages are pinned by unit tests in `guatiao-derive`; these pin
/// that the compiler agrees. An argument may be named like the glue's own
/// parameters, and guatiao's and the standard types are read under any
/// of their usual paths:
///
/// ```
/// use guatiao::library::ProviderError;
///
/// #[guatiao::kind]
/// pub trait Named: Send + Sync {
///     fn f(&self, ctx: i64, out: &str, err: bool) -> Result<String, ProviderError>;
///     fn g(&self, v: &::guatiao::Value) -> Result<std::string::String, guatiao::library::ProviderError>;
/// }
/// ```
///
/// A method cannot take the name of a field every table has:
///
/// ```compile_fail
/// #[guatiao::kind]
/// pub trait Headed: Send + Sync {
///     fn header(&self) -> i64;
/// }
/// ```
///
/// ```compile_fail
/// #[guatiao::kind(object)]
/// pub trait Destroyed: Send {
///     fn destroy(&mut self) -> i64;
/// }
/// ```
///
/// A type ending in `Value` (or `Map`, `List`, `String`, `Object`,
/// `ProviderError`) under a path the attribute does not read is refused,
/// since it cannot tell whether the type is guatiao's:
///
/// ```compile_fail
/// mod foreign {
///     pub struct Value(pub u8);
/// }
/// use guatiao::library::ProviderError;
///
/// #[guatiao::kind]
/// pub trait Foreign: Send + Sync {
///     fn f(&self, v: &foreign::Value) -> Result<i64, ProviderError>;
/// }
/// ```
///
/// ```compile_fail
/// mod foreign {
///     #[derive(Debug)]
///     pub struct String(pub u8);
/// }
/// use guatiao::library::ProviderError;
///
/// #[guatiao::kind]
/// pub trait Foreign: Send + Sync {
///     fn f(&self) -> Result<foreign::String, ProviderError>;
/// }
/// ```
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
/// let node = original.to_value_in(alloc)?;
/// let map: &guatiao::Map = (&node).try_into()?;
/// assert_eq!(str_or(map.get("host"), ""), "10.0.0.1");
/// assert_eq!(int_or(map.get("port"), 0), 5900);
/// assert!(bool_or(map.get("view-only"), false));
/// // `None` omits the key rather than storing a null.
/// assert!(map.get("motd").is_none());
///
/// assert_eq!(Connection::from_value(&node)?, original);
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
///
/// # The shapes it refuses, and that it really does refuse them
///
/// Each of the following fails to compile. The messages themselves are
/// pinned by unit tests in `guatiao-derive`; these pin the half a
/// token-level test cannot show, which is that the compiler agrees.
///
/// A tuple struct is a list by position, so a position has no key to
/// rename:
///
/// ```compile_fail
/// use guatiao::ToValue;
/// #[derive(ToValue)]
/// struct Endpoint(#[map(rename = "host")] String, i64);
/// ```
///
/// Without the attribute it is accepted, and so are a newtype (its one
/// field's value), a unit struct (null) and a generic struct (each type
/// parameter bound by the trait derived):
///
/// ```
/// use guatiao::{FromValue, ToValue};
/// #[derive(ToValue, FromValue, Debug, PartialEq)]
/// struct Endpoint(String, i64);
/// #[derive(ToValue, FromValue, Debug, PartialEq)]
/// struct Port(u16);
/// #[derive(ToValue, FromValue, Debug, PartialEq)]
/// struct Labelled<T> { label: String, value: T }
///
/// let v = Endpoint("10.0.0.1".into(), 5900).to_value()?;
/// assert_eq!(Endpoint::from_value(&v)?, Endpoint("10.0.0.1".into(), 5900));
/// assert_eq!(Port(5900).to_value()?, 5900u16.to_value()?);
/// let l = Labelled { label: "x".into(), value: Port(1) };
/// assert_eq!(Labelled::<Port>::from_value(&l.to_value()?)?, l);
/// # Ok::<(), Box<dyn core::error::Error>>(())
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
/// let tcp = Transport::Tcp { port: 5900 }.to_value_in(alloc)?;
/// let tcp: &guatiao::Map = (&tcp).try_into()?;
/// let tag: &str = tcp.required("transport")?.try_into()?;
/// assert_eq!(tag, "Tcp");
/// let level = Level::Warning.to_value_in(alloc)?;
/// assert_eq!(<&str>::try_from(&level)?, "warn");
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
///
/// Or tell the variants apart by kind with `#[map(untagged)]`: no name is
/// stored, each value is what its variant writes, and a reader takes the
/// variant whose kind the value has. The unit variants are strings, named
/// fields a map, and `Name(T)` whatever `T` reads:
///
/// ```
/// use guatiao::{FromValue, ToValue};
/// #[derive(ToValue, FromValue, Debug, PartialEq)]
/// #[map(untagged, rename_all = "lowercase")]
/// enum Tls { Identity, Files { certificate: String, key: String } }
///
/// #[derive(ToValue, FromValue, Debug, PartialEq)]
/// #[map(untagged)]
/// enum OneOrMany<T> { One(T), Many(Vec<T>) }
///
/// assert_eq!(<&str>::try_from(&Tls::Identity.to_value()?)?, "identity");
/// let files = Tls::Files { certificate: "c.pem".into(), key: "k.pem".into() };
/// assert_eq!(Tls::from_value(&files.to_value()?)?, files);
/// let many = OneOrMany::<u16>::from_value(&vec![1u16, 2].to_value()?)?;
/// assert_eq!(many, OneOrMany::Many(vec![1, 2]));
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
///
/// Two variants that read one kind could not be told apart, so deriving
/// `FromValue` refuses them, naming both:
///
/// ```compile_fail
/// use guatiao::FromValue;
/// #[derive(FromValue)]
/// #[map(untagged)]
/// enum Location { Text(String), Path(std::path::PathBuf) }
/// ```
///
/// A generic one is refused for the instance that overlaps, a list in a
/// list here:
///
/// ```compile_fail
/// use guatiao::FromValue;
/// #[derive(FromValue)]
/// #[map(untagged)]
/// enum OneOrMany<T> { One(T), Many(Vec<T>) }
/// let _ = OneOrMany::<Vec<u8>>::from_value(&guatiao::Value::null());
/// ```
///
/// A tagged variant of two unnamed fields has no keys to store them under
/// beside the tag (one, `Name(T)`, writes `T`'s map there):
///
/// ```compile_fail
/// use guatiao::ToValue;
/// #[derive(ToValue)]
/// #[map(tag = "transport")]
/// enum Transport { Tcp(String, i64) }
/// ```
///
/// A field type with no conversion is rejected at the field, not deep
/// inside the expansion — `std::time::Instant`, a reading of one process's
/// monotonic clock, means nothing to another and implements neither
/// [`ToValue`] nor [`FromValue`]:
///
/// ```compile_fail
/// use guatiao::ToValue;
/// #[derive(ToValue)]
/// struct Timeout { started: std::time::Instant }
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
