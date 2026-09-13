// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Checking a value against a schema, from C.
//!
//! # Reading a schema needs nothing from here
//!
//! A schema **is a value**, written with the keys in
//! [`crate::schema::vocab`], so a C consumer walks one with the header's
//! own helpers and never calls into this library to do it. That is the
//! whole reason the schema is data rather than a set of typed structs.
//!
//! What does need code is the judgement: deciding whether a
//! configuration is one the schema accepts means walking two trees
//! together and applying rules about bounds, arms and required fields.
//! Reimplementing that on the far side of a boundary is how two
//! implementations of the same schema start disagreeing about the same
//! configuration.
//!
//! # The failure crosses as a VALUE
//!
//! A status says a configuration was refused; it cannot say which option
//! or what would have been accepted. So the detail is written through an
//! optional out-parameter as an ordinary map, which the caller already
//! knows how to read.
//!
//! It carries the option's **key** and what *would* have been accepted,
//! and deliberately never the value that was refused: an option may be
//! marked sensitive, and an error that quotes its input is one that
//! eventually logs a passphrase.

#![allow(non_camel_case_types)]

use std::ptr;

use crate::schema::ValidationError;
use crate::schema::read::SchemaRef;
use crate::value::alloc::{Alloc, Allocator};
use crate::value::status::Status;
use crate::value::types::Value;

/// Whether `config` is a value `schema` accepts.
///
/// `out_error` may be null, and receives a map carrying the option's key
/// and what would have been accepted — never the value that was refused,
/// because an option may be marked sensitive and an error type that
/// quotes its input is one that eventually logs a passphrase.
///
/// # Safety
///
/// Every non-null pointer addresses what its type says, and `out_error`
/// addresses writable storage for one value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_schema_validate(
    schema: *const Value,
    config: *const Value,
    alloc: *const Allocator,
    out_error: *mut Value,
) -> Status {
    if schema.is_null() || config.is_null() {
        return Status::GUATIAO_ERR_NULL;
    }
    super::guard(|| {
        // SAFETY: the caller's contract.
        let (schema, config) = unsafe { (&*schema, &*config) };
        let Some(schema) = SchemaRef::new(schema) else {
            return Status::GUATIAO_ERR_WRONG_KIND;
        };
        let Err(e) = crate::schema::validate_map(schema, config) else {
            return Status::GUATIAO_OK;
        };

        if !out_error.is_null()
            // SAFETY: the caller's contract.
            && let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) })
            && let Some(detail) = describe_validation(&e, alloc)
        {
            // SAFETY: `out_error` is writable by contract.
            unsafe { ptr::write(out_error, detail) };
        }
        Status::GUATIAO_ERR_BAD_VALUE
    })
}

/// The validation failure as a value, in the same shape as the merge's.
fn describe_validation(error: &ValidationError, alloc: Alloc) -> Option<Value> {
    let mut out = Value::map_in(alloc);
    match error {
        ValidationError::UnknownOption { key, known } => {
            out.set("key", Value::string_in(alloc, key).ok()?).ok()?;
            let mut list = Value::list_in(alloc);
            for name in known {
                list.push(Value::string_in(alloc, name).ok()?).ok()?;
            }
            out.set("known", list).ok()?;
        }
        ValidationError::BadValue { key, expected } => {
            out.set("key", Value::string_in(alloc, key).ok()?).ok()?;
            out.set("expected", Value::string_in(alloc, expected).ok()?)
                .ok()?;
        }
    }
    out.set("message", Value::string_in(alloc, &error.to_string()).ok()?)
        .ok()?;
    Some(out)
}
