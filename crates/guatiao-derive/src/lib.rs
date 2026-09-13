// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `#[derive(ToValue)]` and `#[derive(FromValue)]` for `guatiao`.
//!
//! **You almost certainly do not want to depend on this crate directly.**
//! Write
//!
//! ```toml
//! guatiao = { version = "0.1", features = ["derive"] }
//! ```
//!
//! and use `guatiao::{ToValue, FromValue}`, which re-exports both macros
//! beside the traits they implement. That is the shape `serde` uses for
//! `serde_derive`, and it is the shape here for the same reason: which
//! crate a macro is *compiled* in is an implementation detail, and a
//! consumer that has to name it is a consumer who will get the two
//! versions out of step.
//!
//! # What it generates
//!
//! For a struct with named fields, `#[derive(ToValue)]` writes an impl
//! of `ToValue` that builds a map, and `#[derive(FromValue)]` writes the
//! reader. **One trait each way, not two.** A map is a value whose tag
//! says map, so a struct that converts to a map converts to a value, and
//! a second pair of map-shaped traits would differ from these in their
//! name and in nothing else. Nesting needs nothing extra: a field whose
//! type derives this converts exactly like any other field.
//!
//! ```ignore
//! #[derive(ToValue, FromValue)]
//! struct Connection {
//!     host: String,
//!     port: i64,
//!     #[map(rename = "tls-verify")]
//!     verify: bool,
//!     motd: Option<String>,   // omitted from the map when None
//!     #[map(skip)]
//!     cache: Vec<String>,     // never stored; Default::default() on read
//! }
//! ```
//!
//! # Writing takes an allocator
//!
//! `to_value` takes the allocator the value is built through, because an
//! owned tree carries its allocator with it rather than assuming a global
//! one. Reading takes none: it copies into ordinary Rust types.
//!
//! # Field attributes
//!
//! | attribute | effect |
//! |---|---|
//! | `#[map(rename = "...")]` | store under this key instead of the field name |
//! | `#[map(skip)]` | never store; on read, `Default::default()` |
//!
//! A skipped field must implement [`Default`]; nothing here can invent a
//! value for it, and requiring the bound at the use site is what makes
//! that a message about *your* type rather than about the expansion.
//!
//! # `Option<T>`
//!
//! `None` **omits the key**; on the way back, both an absent key and a
//! stored null read as `None`. The full argument — and what to do when
//! that distinction actually matters — is in `guatiao`'s `convert` module
//! documentation, which is the one place it should be written down.
//!
//! `Option` is recognised **syntactically**, by the last path segment. So
//! `Option<T>`, `std::option::Option<T>` and `core::option::Option<T>` are
//! all seen, a type alias for one is **not**, and a type of your own
//! actually named `Option` would be. This is what every derive in the
//! ecosystem does — a proc macro has no type information — and it is
//! written down here because the failure is otherwise mystifying.
//!
//! # Hygiene: the generated code names everything absolutely
//!
//! Every path it emits is rooted — `::guatiao::Value`,
//! `::core::option::Option` — and it assumes **no** `use` at the call
//! site. A derive that works only when the caller happens to have
//! imported the right names is a derive that fails mysteriously in the
//! one module that did not, and the person hitting it has no way to know
//! what is missing.
//!
//! The three local bindings it introduces are `__map`, `__value` and
//! `__alloc`. They cannot collide with anything of yours: the only tokens
//! of yours that reach the same scope are field names behind `self.`.
//!
//! # How this is tested
//!
//! The entry points below are one-line wrappers over ordinary functions
//! taking and returning [`proc_macro2::TokenStream`]. That is not
//! decoration: `proc_macro::TokenStream` can only be constructed inside a
//! real macro expansion, so a crate that used it throughout could not
//! call its own expander from a `#[test]`. Because the work sits on the
//! `proc_macro2` type, the **error messages this derive produces are unit
//! tested by their text** — see `src/expand.rs` — rather than merely
//! asserted to be "some compile error".
//!
//! The companion half lives in `guatiao`: `compile_fail` doctests there
//! prove the unsupported shapes genuinely fail to compile, which is the
//! half a token-level test cannot show.

// This crate has no C ABI and no reason for `unsafe` anywhere in it, so
// the guard is crate-level rather than per-module -- strictly stronger
// than the per-module arrangement `guatiao` needs, and it needs that one
// only because its `ffi` module cannot compile under an enclosing
// `forbid`.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod expand;

use proc_macro::TokenStream;

use crate::expand::{Derive, expand};

/// Derives `guatiao::ToValue` for a struct with named fields.
///
/// The value it builds is a map, keyed by field name.
///
/// See the [crate documentation](crate) for the field attributes and the
/// `Option<T>` rule.
#[proc_macro_derive(ToValue, attributes(map))]
pub fn derive_to_value(input: TokenStream) -> TokenStream {
    expand(Derive::ToValue, input.into()).into()
}

/// Derives `guatiao::Schema` for a struct with named fields: what the
/// type needs to be configured, as a value a consumer can read.
///
/// The kind of each option comes from the field's **type**, which already
/// states it -- `u16` is an integer between 0 and 65535 -- and everything
/// else comes from the declaration: the key from `#[map(rename = "...")]`
/// or the field name, `required` from the type not being `Option<T>`, and
/// the help text from the field's own doc comment.
///
/// | attribute | effect |
/// |---|---|
/// | `#[schema(label = "...")]` | the name a person sees |
/// | `#[schema(help = "...")]` | overrides the doc comment |
/// | `#[schema(section = "...")]` | the section id this option belongs to |
/// | `#[schema(order = N)]` | sort position within its section |
/// | `#[schema(advanced)]` | hide unless asked for |
/// | `#[schema(sensitive)]` | a passphrase or a key: never log it |
/// | `#[schema(default = <expr>)]` | the value a consumer starts from |
///
/// `#[map(rename)]` and `#[map(skip)]` are read here too, so the schema
/// and the value agree about keys by construction rather than by care.
///
/// See the [crate documentation](crate) for the rest.
#[proc_macro_derive(Schema, attributes(map, schema))]
pub fn derive_schema(input: TokenStream) -> TokenStream {
    expand(Derive::Schema, input.into()).into()
}

/// Derives `guatiao::FromValue` for a struct with named fields.
///
/// See the [crate documentation](crate) for the field attributes and the
/// `Option<T>` rule.
#[proc_macro_derive(FromValue, attributes(map))]
pub fn derive_from_value(input: TokenStream) -> TokenStream {
    expand(Derive::FromValue, input.into()).into()
}
