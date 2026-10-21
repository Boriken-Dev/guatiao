// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What the two durations accept: the form each writes.

#![forbid(unsafe_code)]

use core::time::Duration;

use super::Schema;
use crate::schema::build::KindBuilder;
use crate::schema::vocab;
use crate::value::alloc::Alloc;
use crate::value::span::Span;

/// `[secs, nanos]`: a tuple of a `u64` and an integer from 0 to
/// 999 999 999. The other spellings a read accepts are not declared: a
/// schema describes what is written.
impl Schema for Duration {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::tuple_in(
            alloc,
            alloc::vec![
                u64::kind(alloc),
                KindBuilder::int_in(alloc)
                    .number(vocab::MINIMUM, "0")
                    .number(vocab::MAXIMUM, "999999999"),
            ],
        )
    }
}

/// `format: duration`, JSON Schema's name for an RFC 3339 appendix A
/// (ISO 8601) duration, which the canonical text is.
impl Schema for Span {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::string_in(alloc).format("duration")
    }
}
