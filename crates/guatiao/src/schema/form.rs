// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The presentation half: what a person is SHOWN.
//!
//! Its own file because it is its own question. [`super::build`] says what
//! a value **is** — its kind, its bounds, whether it is required, what it
//! defaults to. This says how to **show** one, and a reader who cares
//! about only one of those two now has only one file to read.
//!
//! **Presentation is optional and substance is not.** Every key here may
//! be missing and the schema is still correct and still usable; a consumer
//! with no user interface ignores all of them. Never make a validation or
//! type decision depend on one.
//!
//! # Two traits, because the audience differs
//!
//! [`FormBuilder`] is what every builder has: a label, some help, which
//! section it sits in. [`FormFieldBuilder`] is what only a **field** has:
//! an order among its siblings, whether it hides behind a disclosure,
//! whether it is a secret. A schema has no order among siblings and an arm
//! is not a secret, so granting them those would be a trait handing out
//! methods that mean nothing.
//!
//! # A schema still does not know about forms
//!
//! There is no way to DECLARE a section, here or anywhere in this crate: a
//! section exists only to group controls on a screen, so what one is
//! called belongs to whatever draws it. [`FormBuilder::section`] says
//! which section a thing belongs to, because that is a hint carried
//! alongside it.

#![forbid(unsafe_code)]

use super::build::{ArmBuilder, FieldBuilder, SchemaBuilder, put};
use super::vocab;
use crate::value::alloc::Alloc;
use crate::value::mutate::ValueError;
use crate::value::types::Value;

/// What a person is shown about anything a schema declares.
///
/// ```
/// use guatiao::schema::{FieldBuilder, FormBuilder, KindBuilder, SchemaBuilder};
///
/// let schema = SchemaBuilder::new()
///     .label("Connection")
///     .help("Where to connect, and how.")
///     .field(
///         FieldBuilder::new("port", KindBuilder::int_range(1, 65535))
///             .label("Port")
///             .section("net"),
///     )
///     .finish()
///     .expect("a schema this small does not exhaust an allocator");
/// ```
pub trait FormBuilder: Sized {
    /// Sets one presentation key.
    ///
    /// One of the two things an implementer writes; everything else here
    /// is provided over it.
    fn presentation(&mut self, key: &str, value: Result<Value, ValueError>);

    /// The allocator this builder is using, so a provided method can build
    /// a value with it rather than reaching for the crate's own — which
    /// would put half of a schema built into a host's arena somewhere
    /// else.
    fn form_alloc(&self) -> Alloc;

    /// A short human label.
    #[must_use]
    fn label(mut self, label: &str) -> Self {
        let alloc = self.form_alloc();
        self.presentation(vocab::LABEL, Value::string_in(alloc, label));
        self
    }

    /// Longer human help: a sentence under the control, or a tooltip.
    #[must_use]
    fn help(mut self, help: &str) -> Self {
        let alloc = self.form_alloc();
        self.presentation(vocab::HELP, Value::string_in(alloc, help));
        self
    }

    /// Which section this belongs to, by whatever id the thing drawing the
    /// form groups by.
    ///
    /// Naming a section nothing declared is not an error: a consumer that
    /// does not know it puts the field wherever it puts the ungrouped
    /// ones, which is the rule the rest of this vocabulary already has for
    /// something it does not recognise.
    #[must_use]
    fn section(mut self, section: &str) -> Self {
        let alloc = self.form_alloc();
        self.presentation(vocab::SECTION, Value::string_in(alloc, section));
        self
    }
}

/// What a person is shown about one **field**, specifically.
///
/// Separate from [`FormBuilder`] because a schema has no position among
/// siblings and an arm of a variant is not a secret. A trait granting
/// those to everything would be handing out methods that mean nothing on
/// two of its three implementers.
pub trait FormFieldBuilder: FormBuilder {
    /// Where it sits among its siblings.
    ///
    /// Declaration position, which a consumer may use or ignore. Nothing
    /// here sorts by it.
    #[must_use]
    fn order(mut self, order: i64) -> Self {
        let alloc = self.form_alloc();
        self.presentation(vocab::ORDER, Value::int_in(alloc, order));
        self
    }

    /// Hidden behind a disclosure by default.
    #[must_use]
    fn advanced(mut self) -> Self {
        self.presentation(vocab::ADVANCED, Ok(Value::bool(true)));
        self
    }

    /// A secret: masked in a form, and not somewhere to put in a log.
    ///
    /// A hint about how to SHOW it, which is why it is here. What a store
    /// does about it is that store's own decision — this says only that
    /// somebody declared it one.
    #[must_use]
    fn sensitive(mut self) -> Self {
        self.presentation(vocab::SENSITIVE, Ok(Value::bool(true)));
        self
    }
}

impl FormBuilder for SchemaBuilder {
    fn presentation(&mut self, key: &str, value: Result<Value, ValueError>) {
        put(&mut self.state, key, value);
    }

    fn form_alloc(&self) -> Alloc {
        self.alloc
    }
}

impl FormBuilder for FieldBuilder {
    fn presentation(&mut self, key: &str, value: Result<Value, ValueError>) {
        put(&mut self.state, key, value);
    }

    fn form_alloc(&self) -> Alloc {
        self.alloc
    }
}

impl FormBuilder for ArmBuilder {
    fn presentation(&mut self, key: &str, value: Result<Value, ValueError>) {
        put(&mut self.state, key, value);
    }

    fn form_alloc(&self) -> Alloc {
        self.alloc
    }
}

/// Only a field has a position, a disclosure or a secret.
impl FormFieldBuilder for FieldBuilder {}
