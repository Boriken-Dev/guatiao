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

use std::collections::BTreeMap;

use crate::schema::ValidationError;
use crate::schema::flat;
use crate::schema::read::{OptionRef, SchemaRef};
use crate::value::alloc::{Alloc, Allocator};
use crate::value::status::Status;
use crate::value::types::{Str, Value};

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

// --- the flat projection ------------------------------------------------
//
// A front end that only has `key -> text` — an INI file, a web form, a
// command line — is the consumer least likely to be written in Rust, so
// this is the half of the schema API that most needed a boundary.
//
// **The store is an ordinary map of strings.** Not a new type: a C caller
// builds one with `guatiao_map_set` and frees it with
// `guatiao_value_free`, like everything else it holds.

/// A map of `key -> text` as a `BTreeMap`, or `None` if any value is not
/// a string.
///
/// Strict about the kind rather than stringifying whatever it finds: a
/// flat store holds text by definition, and a caller that put a number in
/// one has made a mistake worth hearing about at the boundary instead of
/// two layers in.
fn store_of(v: &Value) -> Option<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for entry in v.entries()? {
        out.insert(
            entry.key_str()?.to_string(),
            entry.value().as_str()?.to_string(),
        );
    }
    Some(out)
}

/// The same, back into a value.
fn store_into(alloc: Alloc, store: &BTreeMap<String, String>) -> Option<Value> {
    let mut out = Value::map_in(alloc);
    for (k, v) in store {
        out.set(k, Value::string_in(alloc, v).ok()?).ok()?;
    }
    Some(out)
}

/// The option governing a flat key, or null.
///
/// Follows one level of projection, so `auth.password` answers the arm
/// field's own option rather than the `auth` option. Every per-option flag
/// a caller wants — required, advanced, sensitive, the label — is read
/// off the value this hands back, so the boundary needs one lookup rather
/// than one export per flag.
///
/// The result **borrows from `schema`** and is valid for as long as it is.
///
/// # Safety
///
/// `schema` addresses a well-formed value, and `key` a readable view.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_schema_resolve(schema: *const Value, key: Str) -> *const Value {
    if schema.is_null() {
        return ptr::null();
    }
    super::guard_with(ptr::null(), || {
        // SAFETY: the caller's contract.
        let Some(schema) = SchemaRef::new(unsafe { &*schema }) else {
            return ptr::null();
        };
        // SAFETY: as above.
        let Ok(key) = (unsafe { super::as_str(key) }) else {
            return ptr::null();
        };
        match flat::resolve(schema, key) {
            Some(option) => option.as_value() as *const Value,
            None => ptr::null(),
        }
    })
}

/// The flat keys one option projects onto, as a list of strings.
///
/// # Safety
///
/// `option` addresses a well-formed option value and `out` writable
/// storage for one value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_schema_flat_keys(
    option: *const Value,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    if option.is_null() || out.is_null() {
        return Status::GUATIAO_ERR_NULL;
    }
    super::guard(|| {
        // SAFETY: the caller's contract.
        let Some(option) = OptionRef::new(unsafe { &*option }) else {
            return Status::GUATIAO_ERR_WRONG_KIND;
        };
        // SAFETY: as above.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_BAD_VALUE;
        };
        let mut list = Value::list_in(alloc);
        for key in flat::keys(option) {
            let Ok(item) = Value::string_in(alloc, &key) else {
                return Status::GUATIAO_ERR_ALLOC;
            };
            if list.push(item).is_err() {
                return Status::GUATIAO_ERR_ALLOC;
            }
        }
        // SAFETY: `out` is writable by contract, and the tree moves into
        // it rather than being copied.
        unsafe { ptr::write(out, list) };
        Status::GUATIAO_OK
    })
}

/// Writes a tagged value into a flat store of `key -> text`.
///
/// `GUATIAO_ERR_WRONG_KIND` when the option is not a variant or the value
/// is not a map, which is the same "it does not apply" the Rust side
/// reports as `false`.
///
/// # Safety
///
/// Every non-null pointer addresses what its type says, and `out`
/// addresses writable storage for one value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_schema_flatten(
    option: *const Value,
    value: *const Value,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    if option.is_null() || value.is_null() || out.is_null() {
        return Status::GUATIAO_ERR_NULL;
    }
    super::guard(|| {
        // SAFETY: the caller's contract.
        let (option, value) = unsafe { (&*option, &*value) };
        let Some(option) = OptionRef::new(option) else {
            return Status::GUATIAO_ERR_WRONG_KIND;
        };
        // SAFETY: as above.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_BAD_VALUE;
        };

        let mut store = BTreeMap::new();
        if !flat::flatten(option, value, &mut store) {
            return Status::GUATIAO_ERR_WRONG_KIND;
        }
        let Some(flat) = store_into(alloc, &store) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: `out` is writable by contract.
        unsafe { ptr::write(out, flat) };
        Status::GUATIAO_OK
    })
}

/// Rebuilds a tagged value from a flat store.
///
/// The reverse of [`guatiao_schema_flatten`], and the round trip is what
/// makes the projection usable: a front end reads text, hands it back, and
/// gets the value the schema describes.
///
/// # Safety
///
/// As for [`guatiao_schema_flatten`]. `flat` is a map whose values are all
/// strings; one that is not answers `GUATIAO_ERR_WRONG_KIND`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_schema_unflatten(
    option: *const Value,
    flat: *const Value,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    if option.is_null() || flat.is_null() || out.is_null() {
        return Status::GUATIAO_ERR_NULL;
    }
    super::guard(|| {
        // SAFETY: the caller's contract.
        let (option, flat) = unsafe { (&*option, &*flat) };
        let Some(option) = OptionRef::new(option) else {
            return Status::GUATIAO_ERR_WRONG_KIND;
        };
        let Some(store) = store_of(flat) else {
            return Status::GUATIAO_ERR_WRONG_KIND;
        };
        // SAFETY: as above.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_BAD_VALUE;
        };
        let Some(built) = flat::unflatten(alloc, option, &store) else {
            return Status::GUATIAO_ERR_WRONG_KIND;
        };
        // SAFETY: `out` is writable by contract.
        unsafe { ptr::write(out, built) };
        Status::GUATIAO_OK
    })
}
