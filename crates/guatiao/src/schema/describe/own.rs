// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What this crate's own containers accept, as field types: whatever the
//! kind they are holds.

#![forbid(unsafe_code)]

use super::Schema;
use crate::schema::build::KindBuilder;
use crate::value::alloc::Alloc;
use crate::value::types::{Buffer, List, Map, Number, Text, Value};

/// Any value: `{}`, which accepts null as well, so `Option<Value>`
/// describes the same.
impl Schema for Value {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::any_in(alloc)
    }
}

/// An object whose keys are data and whose values are anything: what a
/// `BTreeMap<String, Value>` describes.
impl Schema for Map {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::map_of_in(alloc, KindBuilder::any_in(alloc))
    }
}

/// A list of anything: what a `Vec<Value>` describes.
impl Schema for List {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::list_in(alloc, KindBuilder::any_in(alloc))
    }
}

impl Schema for Text {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::string_in(alloc)
    }
}

/// A number, kept as the exact text that wrote it.
impl Schema for Number {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::float_in(alloc)
    }
}

/// The crate's bytes kind.
impl Schema for Buffer {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::bytes_in(alloc)
    }
}
