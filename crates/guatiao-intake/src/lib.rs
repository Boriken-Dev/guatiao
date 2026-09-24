// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The README is this crate's introduction, so it IS the crate
// documentation rather than a second copy of it -- and including it makes
// its example a doctest, so a README that drifted from the API is a red
// test rather than something a reader finds out by pasting it.
#![doc = include_str!("../README.md")]
//!
//! # A form is a value that names fields by path
//!
//! It is written with the keys in [`vocab`] and it never repeats the
//! schema. What a schema already says beside a field — its `title`,
//! `description`, `x-section`, `x-order`, `x-advanced`, `x-sensitive` —
//! stays there. A form adds only what no field can say about itself:
//!
//! - **what a section is called**, and the order sections come in;
//! - **which control to draw** and what an empty one shows;
//! - **when a field is shown**, for what a variant cannot express.
//!
//! The half a field CAN say about itself is written beside the schema, by
//! [`FormBuilder`] and [`FormFieldBuilder`] in [`declare`]: they extend
//! `guatiao`'s builders with `section`, `order` and `advanced`. Those are
//! opinions about how to organise controls, which is why they live here
//! and not in the crate carrying the value model.
//!
//! With the `derive` feature, `#[derive(Form)]` writes a type's default
//! screen from `#[form(..)]` beside `#[derive(Schema)]`, as an
//! [`impl Screen`](Screen).
//!
//! Because it is a value, a C, Python or Dart consumer reads one by walking
//! a map, and `guatiao-serde` writes one out in any format.
//!
//! ```
//! use guatiao::schema::read::SchemaRef;
//! use guatiao::schema::{FieldBuilder, KindBuilder, SchemaBuilder};
//! use guatiao_intake::{
//!     Form, FormBuilder, FormFieldBuilder, FormRef, Hints, Section, check, is_visible, layout,
//! };
//!
//! let schema = SchemaBuilder::new()
//!     .field(
//!         FieldBuilder::new("host", KindBuilder::string())
//!             .section("net")     // the FIELD's hint, not the kind's
//!             .required(),
//!     )
//!     .field(FieldBuilder::new("port", KindBuilder::int_range(1, 65535)).section("net").order(1))
//!     .field(FieldBuilder::new("verify", KindBuilder::bool()))
//!     .field(FieldBuilder::new("ca", KindBuilder::string()))
//!     .finish()?;
//!
//! let form = Form::new()
//!     .section(Section::new("net").label("Network"))
//!     .field("port", Hints::new().widget("number"))
//!     .field("ca", Hints::new().placeholder("/etc/ssl/ca.pem").visible_when("verify", true))
//!     .finish()?;
//!
//! let (s, f) = (SchemaRef::new(&schema).unwrap(), FormRef::new(&form).unwrap());
//! check(s, f)?;
//!
//! // The default section first, then "Network", with `port` ordered ahead.
//! let groups = layout(s, f);
//! assert!(groups[0].section.is_none());
//! assert_eq!(groups[1].section.unwrap().label(), "Network");
//! let net: Vec<&str> = groups[1].fields.iter().map(|p| p.field.key()).collect();
//! assert_eq!(net, ["port", "host"]);
//!
//! // `ca` shows only while `verify` holds true.
//! let mut values = guatiao::Map::new();
//! values.set("verify", false)?;
//! assert!(!is_visible(s, f, "ca", &values.into())?);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # What is code and what is data
//!
//! Reading a form needs nothing from here. What does need code is the
//! judgement — [`check`], [`layout`] and [`is_visible`] — because two
//! renderers that each derived those rules would disagree about one form.
//! The same three are exported to C in [`exports`].

// NO `forbid(unsafe_code)` HERE: it would bind `exports` too. Every other
// module carries it, and `tests/unsafe_stays_in_exports.rs` checks that.
#![deny(missing_docs)]

mod build;
// The form a schema implies, for when nobody wrote one. A `//` comment,
// never a `///`.
pub mod create;
// The presentation hints written beside a schema, extending `guatiao`'s
// builders through its `Extras` hook. A `//` comment, never a `///`.
pub mod declare;
// The `extern "C"` surface. Always compiled: a surface that appears only
// when somebody remembers a flag is one a C caller cannot rely on.
pub mod exports;
// The same surface for the flat projection. A `//` comment, never a
// `///`.
pub mod exports_flat;
// Projecting a value onto flat `key -> text` storage, and resolving a
// path against a schema. A `//` comment, never a `///`.
pub mod flat;
mod judge;
// Naming one place inside a value, as text: `agent[1].name[home].host`.
// A `//` comment, never a `///`.
pub mod path;
mod read;
mod screen;
mod texts;
pub mod vocab;

pub use build::{Form, Hints, Section};
pub use create::{for_field, for_schema, form_for};
pub use declare::{FormBuilder, FormField, FormFieldBuilder};
pub use flat::{SEPARATOR, flatten, is_sensitive, keys, resolve, resolve_in, split, unflatten};
/// `#[derive(Form)]`, behind the `derive` feature: a type's default
/// screen from `#[form(..)]` on the type and its fields, as an
/// `impl Screen`. A macro and a type live in different namespaces, so
/// it shares its name with the builder it builds with.
#[cfg(feature = "derive")]
pub use guatiao_derive::Form;
pub use judge::{FormError, Group, Placed, check, is_visible, layout};
pub use read::{Condition, FormRef, HintsRef, SectionRef};
pub use screen::Screen;
pub use texts::validate_texts;
