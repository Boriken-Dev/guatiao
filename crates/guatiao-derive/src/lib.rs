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
//! reader. An enum is accepted in two shapes:
//!
//! | declaration | value | schema kind |
//! |---|---|---|
//! | unit variants only | the variant's name, as text | a string `enum` |
//! | `#[map(tag = "k")]` on the enum | a map: the name under `k`, then the variant's fields | a tagged variant |
//!
//! An enum whose variants carry fields and that names no tag is refused.
//! The key that tells two variants apart is a wire-format decision, and it
//! is yours to make rather than this crate's. **One trait each way, not two.** A map is a value whose tag
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
//! On an enum, `#[map(tag = "...")]` names the key a variant's name is
//! stored under; on a variant, `#[map(rename = "...")]` changes the name
//! stored. A variant cannot be skipped — a value of it would have no way to
//! be written.
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
//! `::core::option::Option`, and `::std::vec::Vec` for the one list a
//! schema builds — and it assumes **no** `use` at the call site. A derive
//! that works only when the caller happens to have imported the right
//! names is a derive that fails mysteriously in the one module that did
//! not, and the person hitting it has no way to know what is missing.
//!
//! The local bindings it introduces are `__alloc`, `__map`, `__value`,
//! `__field`, `__fields`, `__arms`, `__tag`, `__e`, and `__f0`, `__f1`, …
//! for the fields of a variant. They cannot collide with anything of
//! yours: the only tokens of yours that reach the same scope are field
//! names behind `self.`, and a variant's fields are bound to the
//! generated `__fN` names precisely so that a field called `__map` cannot
//! shadow one of these.
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
#![deny(missing_docs)]

mod expand;
#[cfg(feature = "form")]
mod form;
#[cfg(feature = "provider")]
mod kind;
#[cfg(feature = "provider")]
mod provider;

use proc_macro::TokenStream;

use crate::expand::{Derive, expand};

/// Derives `guatiao::ToValue` for a struct with named fields, an enum of
/// unit variants, or an enum with `#[map(tag = "...")]`.
///
/// A struct builds a map keyed by field name; a unit enum, its variant's
/// name; a tagged enum, a map holding the name under the tag beside the
/// variant's fields.
///
/// See the [crate documentation](crate) for the field attributes and the
/// `Option<T>` rule.
///
/// `#[schema(...)]` is **accepted and ignored** here. It is registered so
/// that a type deriving only this one still compiles with the attribute on
/// its fields; what it means is `Schema`'s business.
#[proc_macro_derive(ToValue, attributes(map, schema))]
pub fn derive_to_value(input: TokenStream) -> TokenStream {
    expand(Derive::ToValue, input.into()).into()
}

/// Derives `guatiao::Schema` for a struct with named fields or an enum:
/// the values the type accepts, described as a value a consumer can read.
///
/// A unit enum describes itself as a choice and a tagged enum as a
/// variant. **A doc comment fills the most descriptive slot the thing
/// has**: a field or an arm has a description as well as a title, so its
/// doc comment is the description; a choice has only a label, so its doc
/// comment is the label.
///
/// The kind of each option comes from the field's **type**, which already
/// states it -- `u16` is an integer between 0 and 65535 -- and everything
/// else comes from the declaration: the key from `#[map(rename = "...")]`
/// or the field name, `required` from the type not being `Option<T>`, and
/// the description from the field's own doc comment.
///
/// | attribute | effect |
/// |---|---|
/// | `#[schema(title = "...")]` | the name a person sees |
/// | `#[schema(description = "...")]` | overrides the doc comment |
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

/// Derives `guatiao::FromValue` for a struct with named fields, an enum of
/// unit variants, or an enum with `#[map(tag = "...")]`.
///
/// A name no variant declares is `MapError::BadValue`, naming the
/// alternatives and never the value it was given.
///
/// See the [crate documentation](crate) for the field attributes and the
/// `Option<T>` rule.
///
/// `#[schema(...)]` is **accepted and ignored** here. It is registered so
/// that a type deriving only this one still compiles with the attribute on
/// its fields; what it means is `Schema`'s business.
#[proc_macro_derive(FromValue, attributes(map, schema))]
pub fn derive_from_value(input: TokenStream) -> TokenStream {
    expand(Derive::FromValue, input.into()).into()
}

/// Declares a provider kind: a trait the host and a library both compile
/// against, with the `repr(C)` function table, the shims and the proxy
/// generated beside it. Reached as `#[guatiao::kind]` with the `provider`
/// feature.
///
/// `#[kind]` names the kind after the trait in snake case (`Greeter` →
/// `"greeter"`, `SessionBackend` → `"session_backend"`);
/// `#[kind(name = "...")]` names it explicitly.
///
/// The trait names `Send + Sync`, and every method takes `&self`.
/// Arguments: integers, floats, `bool`, `&str`, `&[u8]`, `&Value`,
/// `Option<&Value>`, `&Map`, or any other type by value through
/// `ToValue`. Returns: `()`, the scalars, `Value`, `Map`, `List`, `String`,
/// or any other type through `FromValue`; each optionally inside
/// `Result<_, ProviderError>`, which a method that converts must use. A
/// method with a default body is an appended slot; a required method may
/// not follow one.
#[cfg(feature = "provider")]
#[proc_macro_attribute]
pub fn kind(attr: TokenStream, item: TokenStream) -> TokenStream {
    kind::expand(attr.into(), item.into()).into()
}

/// Makes a type that implements kind traits a provider, with every piece
/// of glue generated: one table per kind, the instance, the descriptor.
/// Reached as `guatiao::Provider` with the `provider` feature.
///
/// `#[provider(Greeter, Counter)]` names the kinds and defaults the rest;
/// the long form is `#[provider(kinds(..), id = "..", name = "..",
/// version = "..", config = T, new = path, new_with_host = path,
/// available = path)]`. `id` defaults to `{package}_{type}` in snake case,
/// `name` to the type's ident, `version` to empty (the library's), the
/// instance to `Default`.
#[cfg(feature = "provider")]
#[proc_macro_derive(Provider, attributes(provider))]
pub fn derive_provider(input: TokenStream) -> TokenStream {
    provider::expand(input.into()).into()
}

/// Derives `guatiao_form::Screen`: the type's default screen, as a form
/// value beside its schema. Reached as `guatiao_form::Form` with that
/// crate's `derive` feature.
///
/// On the type, `#[form(section(id = "..", label = "..", help = ".."))]`,
/// repeated in display order. On a field, `#[form(widget = "..",
/// placeholder = "..", visible_when(field = "..", equals = <value>),
/// nested)]`; `nested` composes the field type's own hints under
/// `<key>.`. Keys follow `#[map(rename)]`; which section a field is in
/// stays `#[schema(section)]`'s.
#[cfg(feature = "form")]
#[proc_macro_derive(Form, attributes(form))]
pub fn derive_form(input: TokenStream) -> TokenStream {
    form::expand(input.into()).into()
}
