// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A form's flat store as a value a [`ValueSource`](crate::ValueSource) reads.

use alloc::collections::BTreeMap;
use alloc::string::String;

use coqui_builder::Description;
use guatiao::schema::SchemaRef;
use guatiao::{Alloc, Map, Value, ValueError};
use guatiao_intake::unflatten;

use crate::schema::schema_in;

/// The value a form's flat `key -> text` store holds, read against the schema made from
/// `description` (see [`schema_in`]): [`unflatten`] per top-level field.
///
/// Every scalar is the text the store holds, which the leaf's own parser then reads; a
/// duration is one stored text (`30s`). An untagged enum's structured arm is a whole value
/// in a form, not keys below the enum, so a store holds its text side only. The description
/// must be a group.
///
/// # Errors
///
/// [`ValueError`] when an allocation fails or the description is not a group.
pub fn form_source(
    description: &Description,
    store: &BTreeMap<String, String>,
    alloc: Alloc,
) -> Result<Value, ValueError> {
    let schema = schema_in(description, alloc)?;
    let root = SchemaRef::new(&schema).ok_or(ValueError::WrongKind)?;
    let mut out = Map::new_in(alloc);
    for field in root.fields() {
        if let Some(value) = unflatten(alloc, field, store) {
            out.set_in(field.key(), value, alloc)?;
        }
    }
    Ok(out.into())
}
