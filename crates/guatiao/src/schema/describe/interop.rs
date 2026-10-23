// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What another crate's open tables accept, as field types: a table is
//! an open object, as [`Map`](crate::Map) is, and a value of either
//! format is any value, as [`Value`](crate::Value) is.

#![forbid(unsafe_code)]

use super::Schema;
use crate::schema::build::KindBuilder;
use crate::value::alloc::Alloc;

/// An object whose keys are data and whose values are anything.
#[cfg(feature = "toml")]
impl Schema for toml::Table {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::map_of_in(alloc, KindBuilder::any_in(alloc))
    }
}

/// Any value: `{}`.
#[cfg(feature = "toml")]
impl Schema for toml::Value {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::any_in(alloc)
    }
}

/// An object whose keys are data and whose values are anything.
#[cfg(feature = "serde_json")]
impl Schema for serde_json::Map<String, serde_json::Value> {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::map_of_in(alloc, KindBuilder::any_in(alloc))
    }
}

/// Any value: `{}`.
#[cfg(feature = "serde_json")]
impl Schema for serde_json::Value {
    fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::any_in(alloc)
    }
}
