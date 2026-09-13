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
//! A status says a configuration was refused; it cannot say which field
//! or what would have been accepted. So the detail is written through an
//! optional out-parameter as an ordinary map, which the caller already
//! knows how to read.
//!
//! It carries the field's **key** and what *would* have been accepted,
//! and deliberately never the value that was refused: a field may be
//! marked sensitive, and an error that quotes its input is one that
//! eventually logs a passphrase.

#![allow(non_camel_case_types)]

use std::ptr;

use std::collections::BTreeMap;

use crate::schema::ValidationError;
use crate::schema::flat;
use crate::schema::read::{FieldRef, SchemaRef};
use crate::value::alloc::{Alloc, Allocator};
use crate::value::status::Status;
use crate::value::types::{Str, Value};

/// Whether `config` is a value `schema` accepts.
///
/// `out_error` may be null, and receives a map carrying the field's key
/// and what would have been accepted — never the value that was refused,
/// because a field may be marked sensitive and an error type that
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

/// The field governing a flat key, or null.
///
/// Follows one level of projection, so `auth.password` answers the arm
/// field's own field rather than the `auth` field. Every per-field flag
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
            Some(field) => field.as_value() as *const Value,
            None => ptr::null(),
        }
    })
}

/// The flat keys one field projects onto, as a list of strings.
///
/// # Safety
///
/// `field` addresses a well-formed field value and `out` writable
/// storage for one value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_schema_flat_keys(
    field: *const Value,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    if field.is_null() || out.is_null() {
        return Status::GUATIAO_ERR_NULL;
    }
    super::guard(|| {
        // SAFETY: the caller's contract.
        let Some(field) = FieldRef::new(unsafe { &*field }) else {
            return Status::GUATIAO_ERR_WRONG_KIND;
        };
        // SAFETY: as above.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_BAD_VALUE;
        };
        let mut list = Value::list_in(alloc);
        for key in flat::keys(field) {
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
/// `GUATIAO_ERR_WRONG_KIND` when the field is not a variant or the value
/// is not a map, which is the same "it does not apply" the Rust side
/// reports as `false`.
///
/// # Safety
///
/// Every non-null pointer addresses what its type says, and `out`
/// addresses writable storage for one value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_schema_flatten(
    field: *const Value,
    value: *const Value,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    if field.is_null() || value.is_null() || out.is_null() {
        return Status::GUATIAO_ERR_NULL;
    }
    super::guard(|| {
        // SAFETY: the caller's contract.
        let (field, value) = unsafe { (&*field, &*value) };
        let Some(field) = FieldRef::new(field) else {
            return Status::GUATIAO_ERR_WRONG_KIND;
        };
        // SAFETY: as above.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_BAD_VALUE;
        };

        let mut store = BTreeMap::new();
        if !flat::flatten(field, value, &mut store) {
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
    field: *const Value,
    flat: *const Value,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    if field.is_null() || flat.is_null() || out.is_null() {
        return Status::GUATIAO_ERR_NULL;
    }
    super::guard(|| {
        // SAFETY: the caller's contract.
        let (field, flat) = unsafe { (&*field, &*flat) };
        let Some(field) = FieldRef::new(field) else {
            return Status::GUATIAO_ERR_WRONG_KIND;
        };
        let Some(store) = store_of(flat) else {
            return Status::GUATIAO_ERR_WRONG_KIND;
        };
        // SAFETY: as above.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_BAD_VALUE;
        };
        let Some(built) = flat::unflatten(alloc, field, &store) else {
            return Status::GUATIAO_ERR_WRONG_KIND;
        };
        // SAFETY: `out` is writable by contract.
        unsafe { ptr::write(out, built) };
        Status::GUATIAO_OK
    })
}

// --- exporting a type's own schema --------------------------------------

/// The body every arm of [`export_schema`](crate::export_schema) expands
/// to.
///
/// Written once here rather than three times in the macro: an arm that
/// only chooses a symbol name should not also be a place the unwind
/// policy or the out-parameter contract could drift.
///
/// # Safety
///
/// `alloc` is null or addresses a complete allocator vtable that outlives
/// the tree written through `out`, and `out` addresses writable storage
/// for one value.
pub unsafe fn schema_export<T: crate::schema::Schema>(
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    if out.is_null() {
        return Status::GUATIAO_ERR_NULL;
    }
    super::guard(|| {
        // SAFETY: the caller's contract on `alloc`.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_BAD_VALUE;
        };
        let Ok(schema) = T::schema(alloc) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: `out` is writable by contract, and the tree MOVES into
        // it rather than being copied, so nothing here frees what the
        // caller now owns.
        unsafe { ptr::write(out, schema) };
        Status::GUATIAO_OK
    })
}

/// Exports a type's schema as a C symbol.
///
/// A schema describes **any** value being passed — a record, a batch of
/// metadata, a set of capabilities — and not every schema belongs to a
/// provider or came out of a builder. A library with a type it can
/// describe says so to a caller that has no Rust, without writing the
/// wrapper by hand and without pretending to be a provider.
///
/// ```ignore
/// #[derive(guatiao::Schema)]
/// struct Connection { host: String, port: u16 }
///
/// guatiao::export_schema!(Connection, "connection");
/// ```
///
/// emits, in a crate called `acme-net`:
///
/// ```c
/// guatiao_status acme_net_connection_schema(const guatiao_alloc *alloc,
///                                           guatiao_value *out);
/// ```
///
/// # The symbol carries your crate's name
///
/// A C namespace is flat and shared with everything the caller already
/// links, and `connection_schema` is a name two libraries will both want.
/// So the default prefix is the **calling crate's** name, read at the
/// expansion site, with `-` written as `_` the way Cargo already writes it
/// for a library target.
///
/// # Two builds side by side
///
/// Loading two versions of one library into a process needs their symbols
/// to differ, so how much of the version appears is an argument:
///
/// ```ignore
/// guatiao::export_schema!(Connection, "connection", version = major);
/// // -> acme_net_v1_connection_schema
/// guatiao::export_schema!(Connection, "connection", version = minor);
/// // -> acme_net_v1_2_connection_schema
/// guatiao::export_schema!(Connection, "connection", version = patch);
/// // -> acme_net_v1_2_3_connection_schema
/// ```
///
/// **`major` is the one to reach for**, because it is what C already
/// models: a soname is `libfoo.so.<major>`, major versions are presumed
/// ABI incompatible and minor ones presumed compatible, and the real
/// filename carries major.minor so builds coexist on disk. A symbol that
/// changed on every minor bump would break exactly what that convention
/// protects.
///
/// `minor` and `patch` are for when that is not enough — two builds semver
/// calls compatible that still must not share a symbol, and **anything
/// below 1.0**, where the major is always 0 and `major` separates nothing.
/// Cargo treats 0.y as the compatibility unit and C has no equivalent, so
/// a pre-1.0 library keeping two builds apart wants `minor` at least.
///
/// # Any other shape
///
/// `symbol` takes an expression, not just a literal, so `concat!` composes
/// whatever the host expects — including the `@` an ELF consumer may look
/// up, which no C identifier could hold:
///
/// ```ignore
/// guatiao::export_schema!(
///     Connection,
///     symbol = concat!("acme_net@v", env!("CARGO_PKG_VERSION"), "_connection")
/// );
/// ```
///
/// That is legal in an export table and unusable from a C header, which is
/// the right trade when the host resolves it by name at run time rather
/// than linking against it.
///
/// `versioned` uses the **major** version and nothing else, which is what
/// a C library already does: a soname is `libfoo.so.<major>`, because
/// major versions are presumed ABI incompatible and minor ones presumed
/// compatible. The real filename carries major.minor so builds coexist on
/// disk, while the linking identity stays major — and a symbol that
/// changed on every minor bump would break exactly what that convention
/// protects.
///
/// **Below 1.0 this separates nothing**, since the major is always 0.
/// That is a real gap rather than an oversight: Cargo treats 0.y as the
/// compatibility unit and C has no equivalent, so the two cannot both be
/// honoured in one name. A pre-1.0 library that genuinely needs two
/// schemas in one process should name them itself with `symbol = `, the
/// way a C library hand-picks `_v2` when a signature changes.
///
/// # What the caller gets
///
/// An **owned** tree, built through the allocator it passed, which it
/// frees with `guatiao_value_free`. A null allocator is
/// `GUATIAO_ERR_BAD_VALUE`, a null `out` is `GUATIAO_ERR_NULL`, and a
/// schema that cannot be built is `GUATIAO_ERR_ALLOC`.
///
/// # Not the provider path
///
/// A provider hands its schema over through `ProviderInfo::config`, a
/// pointer to a value it keeps alive for as long as it is loaded — no
/// symbol and no allocator. This is for the other case, and the two do not
/// overlap.
#[macro_export]
macro_rules! export_schema {
    ($ty:ty, $label:literal) => {
        $crate::export_schema!(
            @emit $ty,
            ::core::concat!(::core::env!("CARGO_PKG_NAME"), "_", $label, "_schema")
        );
    };
    ($ty:ty, $label:literal, version = major) => {
        $crate::export_schema!(
            @emit $ty,
            ::core::concat!(
                ::core::env!("CARGO_PKG_NAME"),
                "_v",
                ::core::env!("CARGO_PKG_VERSION_MAJOR"),
                "_",
                $label,
                "_schema"
            )
        );
    };
    ($ty:ty, $label:literal, version = minor) => {
        $crate::export_schema!(
            @emit $ty,
            ::core::concat!(
                ::core::env!("CARGO_PKG_NAME"),
                "_v",
                ::core::env!("CARGO_PKG_VERSION_MAJOR"),
                "_",
                ::core::env!("CARGO_PKG_VERSION_MINOR"),
                "_",
                $label,
                "_schema"
            )
        );
    };
    ($ty:ty, $label:literal, version = patch) => {
        $crate::export_schema!(
            @emit $ty,
            ::core::concat!(
                ::core::env!("CARGO_PKG_NAME"),
                "_v",
                ::core::env!("CARGO_PKG_VERSION_MAJOR"),
                "_",
                ::core::env!("CARGO_PKG_VERSION_MINOR"),
                "_",
                ::core::env!("CARGO_PKG_VERSION_PATCH"),
                "_",
                $label,
                "_schema"
            )
        );
    };
    ($ty:ty, symbol = $symbol:expr) => {
        $crate::export_schema!(@emit $ty, $symbol);
    };
    (@emit $ty:ty, $name:expr) => {
        // An anonymous const, so two invocations in one crate do not
        // collide on the Rust-side name. The symbol is what matters and
        // `export_name` sets that independently.
        const _: () = {
            /// The schema of a type this library describes.
            ///
            /// # Safety
            ///
            /// `alloc` is null or a complete allocator vtable outliving
            /// the tree, and `out` is writable storage for one value.
            #[unsafe(export_name = $name)]
            pub unsafe extern "C" fn schema_export(
                alloc: *const $crate::value::alloc::Allocator,
                out: *mut $crate::Value,
            ) -> $crate::Status {
                // SAFETY: forwarded from this function's own contract.
                unsafe { $crate::exports::schema::schema_export::<$ty>(alloc, out) }
            }
        };
    };
}
