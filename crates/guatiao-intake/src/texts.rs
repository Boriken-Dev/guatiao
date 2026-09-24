// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Checking a store of text against a schema.
//!
//! A front end that holds `key -> text` and nothing else -- a command
//! line, a query string, an INI file, a form's fields -- asks a different
//! question from one holding a value: every entry is text, so the check
//! is per key and the schema is what says which key means what.
//!
//! `guatiao::schema::validate_text` answers for ONE text against one
//! field, which needs no path and stays there. This one takes a store,
//! which means it takes the path language, which is why it is here.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use guatiao::schema::ValidationError;
use guatiao::schema::read::SchemaRef;
use guatiao::schema::validate::validate_text;

/// A refusal naming the field and what would have been accepted.
///
/// The same shape `guatiao::schema::validate` builds, written here
/// because the variant's fields are public and the helper is not.
fn bad(key: &str, expected: impl Into<String>) -> ValidationError {
    ValidationError::BadValue {
        key: key.to_string(),
        expected: expected.into(),
    }
}

/// Whether every entry of `values` is accepted by `schema`.
///
/// An undeclared key is an error rather than something to ignore. Silently
/// dropping a misspelled field is how somebody ends up convinced a
/// setting does nothing.
///
/// A dotted key is a tagged field's payload, checked against the arm the
/// store selects -- see [`crate::flat::resolve_in`]. The flat spelling
/// carries the discriminant only under the field's own key, so this is
/// the one place the text form can say what the nested form says: a
/// field belonging to an unselected arm is refused, not ignored.
pub fn validate_texts(
    schema: SchemaRef<'_>,
    values: &BTreeMap<String, String>,
) -> Result<(), ValidationError> {
    for (key, value) in values {
        let field = crate::flat::resolve_in(schema, key, |k| values.get(k).cloned())?;
        validate_text(field, value)?;
    }
    for field in schema.fields() {
        if field.is_required() && !values.contains_key(field.key()) {
            return Err(bad(field.key(), "a value — it is required"));
        }
    }
    Ok(())
}
