// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What dates and times accept: text with the `format` JSON Schema names
//! for it, or a pattern where its format would require an offset.

#![forbid(unsafe_code)]

use std::time::SystemTime;

use super::Schema;
use crate::schema::build::KindBuilder;
use crate::value::alloc::Alloc;
use crate::value::time::{Date, DateTime, Time, Timestamp};

/// `HH:MM:SS` and up to nine fractional digits.
const TIME: &str = r"([01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9](\.[0-9]{1,9})?";

/// `format: date`, RFC 3339's `full-date`.
impl Schema for Date {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::string_in(alloc).format("date")
    }
}

/// A pattern: JSON Schema's `format: time` requires an offset, which a
/// time of day does not have.
impl Schema for Time {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::string_in(alloc).pattern(&alloc::format!("^{TIME}$"))
    }
}

/// A pattern, as for [`Time`]: `format: date-time` requires an offset. A
/// pattern cannot know a month's length; the reader refuses `02-30`.
impl Schema for DateTime {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::string_in(alloc).pattern(&alloc::format!(
            "^[0-9]{{4}}-(0[1-9]|1[0-2])-(0[1-9]|[12][0-9]|3[01])[Tt]{TIME}$"
        ))
    }
}

/// `format: date-time`, RFC 3339's `date-time`.
impl Schema for Timestamp {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::string_in(alloc).format("date-time")
    }
}

/// A [`Timestamp`]'s.
impl Schema for SystemTime {
    fn kind(alloc: Alloc) -> KindBuilder {
        Timestamp::kind(alloc)
    }
}
