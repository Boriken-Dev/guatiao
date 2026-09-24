// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The flat projection, for a caller that cannot link Rust.
//!
//! Moved here with the projection itself: a schema describes a struct,
//! and naming a place inside one is this library's business.

use std::collections::BTreeMap;
use std::ptr;

use guatiao::schema::read::{FieldRef, SchemaRef};
use guatiao::value::alloc::{Alloc, Allocator};
use guatiao::value::convert::TryAsRef;
use guatiao::value::status::Status;
use guatiao::value::types::{List, Map, Str, Text, Value};

use super::exports::{entry, guard_with, out};

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
    for entry in TryAsRef::<Map>::try_as_ref(v).map(Map::entries)? {
        out.insert(
            entry.key().to_string(),
            TryAsRef::<str>::try_as_ref(entry.value())?.to_string(),
        );
    }
    Some(out)
}

/// The same, back into a value.
fn store_into(alloc: Alloc, store: &BTreeMap<String, String>) -> Option<Value> {
    let mut out = Map::new_in(alloc);
    for (k, v) in store {
        out.set(k, Text::new_in(alloc, v).map(Value::from).ok()?)
            .ok()?;
    }
    Some(out.into())
}

/// The field a key names, following one level of projection.
///
/// The three exports below all need it, and doing it once is what keeps
/// them agreeing about what `auth.password` means.
///
/// # Safety
///
/// `schema` is non-null and addresses a well-formed value, and `key` is a
/// readable view.
unsafe fn field_at<'a>(schema: *const Value, key: Str) -> Option<FieldRef<'a>> {
    // SAFETY: the caller's contract.
    let schema = SchemaRef::new(unsafe { &*schema })?;
    // SAFETY: as above.
    let key = std::str::from_utf8(key.into()).ok()?;
    crate::flat::resolve(schema, key)
}

/// The field governing a flat key, or null.
///
/// Follows one level of projection, so `auth.password` answers the arm
/// field's own field rather than the `auth` field. Every per-field flag
/// a caller wants — required, advanced, sensitive, the title — is read
/// off the value this hands back, so the boundary needs one lookup rather
/// than one export per flag.
///
/// The result **borrows from `schema`** and is valid for as long as it is.
///
/// # Safety
///
/// `schema` addresses a well-formed value, and `key` a readable view.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_intake_resolve(schema: *const Value, key: Str) -> *const Value {
    if schema.is_null() {
        return ptr::null();
    }
    guard_with(ptr::null(), || {
        // SAFETY: the caller's contract.
        let Some(schema) = SchemaRef::new(unsafe { &*schema }) else {
            return ptr::null();
        };
        // SAFETY: as above.
        let Ok(key) = std::str::from_utf8(key.into()) else {
            return ptr::null();
        };
        match crate::flat::resolve(schema, key) {
            Some(field) => field.as_value() as *const Value,
            None => ptr::null(),
        }
    })
}

/// The flat keys one field projects onto, as a list of strings.
///
/// Takes the schema and a key rather than a field, because **a field's
/// name is not inside the field**: it is the key it is filed under in
/// `properties`, so a bare pointer to a field's schema cannot say what it
/// is called. Same for the three below.
///
/// `out` is written the absent marker on entry, so a failed call leaves
/// it ABSENT. A null or unusable allocator is `GUATIAO_ERR_ALLOC`.
///
/// # Safety
///
/// `schema` addresses a well-formed value, `key` a readable view, and
/// `out` writable storage for one value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_intake_flat_keys(
    schema: *const Value,
    key: Str,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    out!(out);
    entry!(schema, out => {
        // SAFETY: the caller's contract.
        let Some(field) = (unsafe { field_at(schema, key) }) else {
            return Status::GUATIAO_ERR_WRONG_KIND;
        };
        // SAFETY: as above.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        let mut list = List::new_in(alloc);
        for key in crate::flat::keys(field) {
            let Ok(item) = Text::new_in(alloc, &key).map(Value::from) else {
                return Status::GUATIAO_ERR_ALLOC;
            };
            if list.push(item).is_err() {
                return Status::GUATIAO_ERR_ALLOC;
            }
        }
        // SAFETY: `out` is writable by contract, and the tree moves into
        // it rather than being copied.
        unsafe { ptr::write(out, list.into()) };
        Status::GUATIAO_OK
    })
}

/// Writes a tagged value into a flat store of `key -> text`.
///
/// `GUATIAO_ERR_WRONG_KIND` when the key names no field, the field is not
/// a variant, or the value is not a map — all of which are the same "it
/// does not apply" the Rust side reports as `false`.
///
/// `out` is written the absent marker on entry, so a failed call leaves
/// it ABSENT. A null or unusable allocator is `GUATIAO_ERR_ALLOC`.
///
/// # Safety
///
/// Every non-null pointer addresses what its type says, `key` is a
/// readable view, and `out` addresses writable storage for one value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_intake_flatten(
    schema: *const Value,
    key: Str,
    value: *const Value,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    out!(out);
    entry!(schema, value, out => {
        // SAFETY: the caller's contract.
        let value = unsafe { &*value };
        // SAFETY: as above.
        let Some(field) = (unsafe { field_at(schema, key) }) else {
            return Status::GUATIAO_ERR_WRONG_KIND;
        };
        // SAFETY: as above.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };

        let mut store = BTreeMap::new();
        if !crate::flat::flatten(field, value, &mut store) {
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
/// The reverse of [`guatiao_intake_flatten`], and the round trip is what
/// makes the projection usable: a front end reads text, hands it back, and
/// gets the value the schema describes.
///
/// `out` is written the absent marker on entry, so a failed call leaves
/// it ABSENT. A null or unusable allocator is `GUATIAO_ERR_ALLOC`.
///
/// # Safety
///
/// As for [`guatiao_intake_flatten`]. `flat` is a map whose values are all
/// strings; one that is not answers `GUATIAO_ERR_WRONG_KIND`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_intake_unflatten(
    schema: *const Value,
    key: Str,
    flat: *const Value,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    out!(out);
    entry!(schema, flat, out => {
        // SAFETY: the caller's contract.
        let flat = unsafe { &*flat };
        // SAFETY: as above.
        let Some(field) = (unsafe { field_at(schema, key) }) else {
            return Status::GUATIAO_ERR_WRONG_KIND;
        };
        let Some(store) = store_of(flat) else {
            return Status::GUATIAO_ERR_WRONG_KIND;
        };
        // SAFETY: as above.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        let Some(built) = crate::flat::unflatten(alloc, field, &store) else {
            return Status::GUATIAO_ERR_WRONG_KIND;
        };
        // SAFETY: `out` is writable by contract.
        unsafe { ptr::write(out, built) };
        Status::GUATIAO_OK
    })
}
