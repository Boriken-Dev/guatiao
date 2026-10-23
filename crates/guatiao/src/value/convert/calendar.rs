// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Dates and times: [`Date`], [`Time`], [`DateTime`] and [`Timestamp`]
//! cross as their RFC 3339 text, and `SystemTime` as a [`Timestamp`]'s.

#![forbid(unsafe_code)]

use super::Kinds;
use alloc::borrow::Cow;
use alloc::string::ToString;
use std::time::SystemTime;

use super::{FromValue, MapError, MapKey, ToValue};
use crate::schema::build::KindBuilder;
use crate::schema::describe::Schema;
use crate::value::alloc::Alloc;
use crate::value::error::ValueError;
use crate::value::time::{Date, DateTime, Time, Timestamp};
use crate::value::types::{Text, Value};

/// The text a refusal names: what was expected and where.
fn refused(what: &str, e: ValueError) -> MapError {
    match e {
        ValueError::Unparsed { at, expected } => {
            MapError::bad_value(alloc::format!("{what}: {expected} at byte {at}"))
        }
        _ => MapError::bad_value(what.to_string()),
    }
}

macro_rules! calendar {
    ($($t:ty => $what:literal),* $(,)?) => {$(
        /// Its canonical text.
        impl ToValue for $t {
            fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
                Ok(Text::new_in(alloc, &self.to_string())?.into())
            }
        }

        /// Text its `FromStr` reads; a refusal names the byte it stopped
        /// at and what it expected there.
        impl FromValue for $t {
            const KINDS: Kinds = Kinds::STRING;
            fn from_value(value: &Value) -> Result<$t, MapError> {
                <&str>::try_from(value)?.parse().map_err(|e| refused($what, e))
            }
        }

        /// Its canonical text. Two keys naming one instant (`Z` and
        /// `+00:00`) read as one key, and the map reader refuses them.
        impl MapKey for $t {
            fn to_key(&self) -> Result<Cow<'_, str>, ValueError> {
                Ok(Cow::Owned(self.to_string()))
            }

            fn from_key(key: &str) -> Result<$t, MapError> {
                key.parse().map_err(|e| refused(concat!("a key that is ", $what), e))
            }

            fn key_kind(alloc: Alloc) -> Option<KindBuilder> {
                Some(<$t as Schema>::kind(alloc))
            }
        }
    )*};
}

calendar!(
    Date => "an RFC 3339 full-date",
    Time => "a time of day, HH:MM:SS",
    DateTime => "a date and time with no offset",
    Timestamp => "an RFC 3339 date-time",
);

/// A [`Timestamp`]'s text: RFC 3339 in UTC, `2026-10-21T13:45:00.5Z`. A
/// time outside the years 0000 to 9999 is [`ValueError::Unrepresentable`].
impl ToValue for SystemTime {
    fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
        Timestamp::try_from(*self)?.to_value_in(alloc)
    }
}

/// What a [`Timestamp`] reads, kept to the clock's resolution: 100 ns on
/// Windows.
impl FromValue for SystemTime {
    const KINDS: Kinds = Kinds::STRING;
    fn from_value(value: &Value) -> Result<SystemTime, MapError> {
        SystemTime::try_from(Timestamp::from_value(value)?)
            .map_err(|_| MapError::bad_value("a date-time this platform's clock can hold"))
    }
}
