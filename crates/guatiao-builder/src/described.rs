// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A described type as a guatiao value, schema and field.

use alloc::string::ToString;

use coqui_builder::{Describe, Inspect};
use guatiao::schema::KindBuilder;
use guatiao::{Alloc, FromValue, MapError, Schema, ToValue, Value, ValueError};

use crate::convert::{self, FromError};
use crate::schema;

/// A type that derives coqui-builder's `Describe`, held by guatiao.
///
/// It writes the value [`to_value`](crate::to_value) writes, reads the one
/// [`from_value`](crate::from_value) reads, and describes itself with
/// [`kind_in`](crate::kind_in) of its description, so it can be a field of a
/// guatiao-derived struct, the type of a provider's configuration, or the argument of a
/// kind.
///
/// `FromValue::KINDS` is left at any kind: a described type cannot be an arm of an enum told
/// apart by kind.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Described<T>(pub T);

impl<T: Describe + Inspect> ToValue for Described<T> {
    fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
        convert::to_value_in(&self.0, alloc)
    }
}

impl<T: Describe> FromValue for Described<T> {
    fn from_value(value: &Value) -> Result<Described<T>, MapError> {
        convert::from_value(value).map(Described).map_err(map_error)
    }
}

impl<T: Describe> Schema for Described<T> {
    fn kind(alloc: Alloc) -> KindBuilder {
        schema::kind_in(&T::description(), alloc)
    }
}

/// A refusal as guatiao reports one: under the key it names, never quoting the input.
pub(crate) fn map_error(error: FromError) -> MapError {
    match error {
        FromError::Missing(missing) if missing.keys().is_empty() => {
            MapError::bad_value("every input the type needs")
        }
        FromError::Missing(missing) => {
            let keys: alloc::vec::Vec<_> = missing.keys().iter().map(ToString::to_string).collect();
            MapError::missing(&keys.join(", "))
        }
        FromError::Fill(error) => {
            let refused =
                MapError::bad_value(alloc::format!("a value this type reads ({})", error.kind()));
            let at = convert::at_of(&error);
            if at.is_empty() {
                refused
            } else {
                refused.under(&at)
            }
        }
    }
}
