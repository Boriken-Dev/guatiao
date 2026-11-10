// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs, missing_debug_implementations)]

extern crate alloc;

mod convert;
mod described;
mod form;
mod schema;
mod source;
pub mod text;
mod view;
mod with;

pub use convert::{FromError, from_value, from_value_over, to_value, to_value_in};
pub use described::Described;
pub use form::form_source;
pub use schema::{DURATION_PATTERN, kind_in, schema, schema_in};
pub use source::{ValueSource, partial};
pub use view::ValueView;

/// What `with_module!` names, so the crate that calls it needs no dependency of its own.
#[doc(hidden)]
pub mod __private {
    pub use coqui_builder::Describe;
    pub use guatiao::schema::KindBuilder;
    pub use guatiao::{Alloc, MapError, Value, ValueError};

    /// [`crate::from_value`] with its refusal as guatiao reports one.
    ///
    /// # Errors
    ///
    /// The refusal, under the key it names.
    pub fn from_value<T: Describe>(value: &Value) -> Result<T, MapError> {
        crate::from_value(value).map_err(crate::described::map_error)
    }
}
