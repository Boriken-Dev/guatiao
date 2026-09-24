// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The presentation half, written beside a schema: what a person is SHOWN.
//!
//! `guatiao`'s builders say what a value **is** — its kind, its bounds,
//! whether it is required, and JSON Schema's own `title` and
//! `description`. These two traits extend them with what a **screen**
//! wants: which section a field sits in, where among its siblings,
//! whether it hides behind a disclosure. They live here because that is
//! an opinion about how to organise controls, and the crate that carries
//! the value model holds none.
//!
//! Every key they write is `x-` prefixed, and every one is optional:
//! **presentation is optional and substance is not**. A schema with none
//! of them is correct and usable, a consumer with no user interface
//! ignores all of them, and nothing about validation may ever depend on
//! one. They are written through [`Extras`], the hook `guatiao` offers
//! for exactly this, so a hint lands in the same allocator as the schema
//! carrying it and its failure is collected the same way.
//!
//! # Two traits, because the audience differs
//!
//! [`FormBuilder`] is what every builder has: which section it sits in.
//! [`FormFieldBuilder`] is what only a **field** has — an order among its
//! siblings, and whether it hides behind a disclosure. A schema has no
//! position among siblings and an arm of a variant is not disclosed
//! separately, so granting them those would be a trait handing out
//! methods that mean nothing on two of its three implementers.
//!
//! `sensitive` is in neither: it stays on [`FieldBuilder`] itself,
//! because "never print this value" is obeyed by a log and a crash dump
//! as much as by a form.
//!
//! # A schema still does not know about forms
//!
//! There is no way to DECLARE a section here either — that is
//! [`Section`](crate::Section), in the form document. A section exists to
//! group controls on a screen, so what one is *called* belongs with the
//! rest of the drawing. [`FormBuilder::section`] says only which section
//! a thing belongs to, which is a hint carried alongside it; naming one
//! nothing declares is not an error.

#![forbid(unsafe_code)]

use guatiao::schema::vocab;
use guatiao::schema::{ArmBuilder, Extras, FieldBuilder, SchemaBuilder};
use guatiao::value::types::{Number, Text, Value};

/// Where a schema, a field or an arm is shown.
///
/// ```
/// use guatiao::schema::{FieldBuilder, KindBuilder, SchemaBuilder};
/// use guatiao_form::FormBuilder;
///
/// let schema = SchemaBuilder::new()
///     .title("Connection")
///     .description("Where to connect, and how.")
///     .field(
///         FieldBuilder::new("port", KindBuilder::int_range(1, 65535))
///             .title("Port")
///             .section("net"),
///     )
///     .finish()
///     .expect("a schema this small does not exhaust an allocator");
/// ```
pub trait FormBuilder: Extras {
    /// Which section this belongs to, by whatever id the thing drawing
    /// the form groups by.
    ///
    /// Naming a section nothing declared is not an error: a consumer that
    /// does not know it puts the field wherever it puts the ungrouped
    /// ones, which is the rule the rest of this vocabulary already has
    /// for something it does not recognise.
    #[must_use]
    fn section(self, section: &str) -> Self {
        let alloc = self.extra_alloc();
        self.extra(
            vocab::X_SECTION,
            Text::new_in(alloc, section).map(Value::from),
        )
    }
}

/// What is shown about one **field**, specifically.
///
/// Separate from [`FormBuilder`] because a schema has no position among
/// siblings and an arm of a variant is not disclosed on its own. A trait
/// granting those to everything would be handing out methods that mean
/// nothing on two of its three implementers.
pub trait FormFieldBuilder: FormBuilder {
    /// Where it sits among its siblings.
    ///
    /// Declaration position, which a consumer may use or ignore.
    /// [`layout`](crate::layout) sorts by it; nothing else does.
    #[must_use]
    fn order(self, order: i64) -> Self {
        let alloc = self.extra_alloc();
        self.extra(
            vocab::X_ORDER,
            Number::new_in(alloc, &order.to_string()).map(Value::from),
        )
    }

    /// Hidden behind a disclosure by default.
    #[must_use]
    fn advanced(self) -> Self {
        self.extra(vocab::X_ADVANCED, Ok(Value::from(true)))
    }
}

impl FormBuilder for SchemaBuilder {}
impl FormBuilder for FieldBuilder {}
impl FormBuilder for ArmBuilder {}

/// Only a field has a position among siblings, or a disclosure.
impl FormFieldBuilder for FieldBuilder {}
