// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Checking, laying out and evaluating a form from C — or from Python
//! through `ctypes`, or anything else that can call a C function.
//!
//! # Reading a form needs nothing from here
//!
//! A form is a value, so a C consumer walks one with `guatiao.h`'s own
//! helpers. These three are the judgement, which is the part two
//! implementations would get differently:
//!
//! ```c
//! guatiao_status guatiao_form_check(const guatiao_value *schema, const guatiao_value *form,
//!                                   const guatiao_alloc *alloc, guatiao_value *out_error);
//! guatiao_status guatiao_form_layout(const guatiao_value *schema, const guatiao_value *form,
//!                                    const guatiao_alloc *alloc, guatiao_value *out);
//! guatiao_status guatiao_form_is_visible(const guatiao_value *schema, const guatiao_value *form,
//!                                        guatiao_str key, const guatiao_value *values, bool *out);
//! ```
//!
//! # A layout is keys, not copies
//!
//! `guatiao_form_layout` answers **grouping and order** and nothing else:
//! a list of `{ "section": <the section's map, or null>, "fields": [key, …] }`.
//! Everything else about a field the caller already holds — its schema
//! through `guatiao_schema_resolve`, its hints in the form it passed in —
//! so copying either would only give it a second thing to keep in step.
//!
//! # The shape of every function here
//!
//! The work is the safe Rust API; these convert pointers and call it, and a
//! panic becomes `GUATIAO_ERR_INTERNAL` rather than unwinding into C.

#![allow(non_camel_case_types)]

use std::panic::{AssertUnwindSafe, catch_unwind};

use guatiao::ToValue;
use guatiao::schema::read::SchemaRef;
use guatiao::value::alloc::{Alloc, Allocator};
use guatiao::value::status::Status;
use guatiao::value::types::{Str, Value};

use crate::judge::{FormError, check, is_visible, layout};
use crate::read::FormRef;

/// The key a layout group's section is under.
const GROUP_SECTION: &str = "section";
/// The key a layout group's field keys are under.
const GROUP_FIELDS: &str = "fields";

/// Runs a boundary body, turning a panic into a status. The payload is
/// forgotten rather than dropped: dropping one can panic, and a second
/// panic in an `extern "C"` body is the abort this exists to avoid.
fn guard(body: impl FnOnce() -> Status) -> Status {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(status) => status,
        Err(payload) => {
            std::mem::forget(payload);
            Status::GUATIAO_ERR_INTERNAL
        }
    }
}

/// A schema and a form, viewed, or the status saying which was wrong.
///
/// # Safety
///
/// Each pointer is null or addresses a well-formed value that outlives
/// the call.
unsafe fn views<'a>(
    schema: *const Value,
    form: *const Value,
) -> Result<(SchemaRef<'a>, FormRef<'a>), Status> {
    if schema.is_null() || form.is_null() {
        return Err(Status::GUATIAO_ERR_NULL);
    }
    // SAFETY: the caller's contract.
    let (schema, form) = unsafe { (&*schema, &*form) };
    let schema = SchemaRef::new(schema).ok_or(Status::GUATIAO_ERR_WRONG_KIND)?;
    let form = FormRef::new(form).ok_or(Status::GUATIAO_ERR_WRONG_KIND)?;
    Ok((schema, form))
}

/// A key's text, or the status saying why not.
///
/// # Safety
///
/// `key` is a readable view for the call.
unsafe fn key_text<'a>(key: Str) -> Result<&'a str, Status> {
    if key.len > 0 && key.ptr.is_null() {
        return Err(Status::GUATIAO_ERR_NULL);
    }
    let bytes = if key.len == 0 {
        &[][..]
    } else {
        // SAFETY: the caller's contract; a zero length never reads.
        unsafe { std::slice::from_raw_parts(key.ptr, key.len) }
    };
    std::str::from_utf8(bytes).map_err(|_| Status::GUATIAO_ERR_BAD_VALUE)
}

/// Whether `form` fits `schema`.
///
/// `GUATIAO_ERR_BAD_VALUE` when it does not, and `out_error` — which may be
/// null — receives a map: `kind` (`malformed`, `unknown_field`,
/// `duplicate_section`, `condition_refused` or `cyclic_condition`), `at`,
/// whichever of `path`, `id`, `field` and `expected` apply, and a
/// `message`. It never carries a value the form compares with.
///
/// # Safety
///
/// Every non-null pointer addresses what its type says, and `out_error`
/// addresses writable storage for one value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_form_check(
    schema: *const Value,
    form: *const Value,
    alloc: *const Allocator,
    out_error: *mut Value,
) -> Status {
    guard(|| {
        // SAFETY: forwarded from this function's contract.
        let (schema, form) = match unsafe { views(schema, form) } {
            Ok(v) => v,
            Err(status) => return status,
        };
        let Err(e) = check(schema, form) else {
            return Status::GUATIAO_OK;
        };
        if !out_error.is_null()
            // SAFETY: the caller's contract on `alloc`.
            && let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) })
            && let Some(detail) = describe(&e, alloc)
        {
            // SAFETY: `out_error` is writable by contract, and the tree
            // moves into it.
            unsafe { out_error.write(detail) };
        }
        Status::GUATIAO_ERR_BAD_VALUE
    })
}

/// The failure as a map a C caller already knows how to read.
fn describe(error: &FormError, alloc: Alloc) -> Option<Value> {
    let mut out = Value::map_in(alloc);
    let mut text = |key: &str, text: &str| -> Option<()> {
        out.set(key, Value::string_in(alloc, text).ok()?).ok()
    };
    match error {
        FormError::Malformed { at, expected } => {
            text("kind", "malformed")?;
            text("at", at)?;
            text("expected", expected)?;
        }
        FormError::UnknownField { at, path } => {
            text("kind", "unknown_field")?;
            text("at", at)?;
            text("path", path)?;
        }
        FormError::DuplicateSection { id } => {
            text("kind", "duplicate_section")?;
            text("id", id)?;
        }
        FormError::ConditionRefused {
            at,
            field,
            expected,
        } => {
            text("kind", "condition_refused")?;
            text("at", at)?;
            text("field", field)?;
            text("expected", expected)?;
        }
        FormError::CyclicCondition { path } => {
            text("kind", "cyclic_condition")?;
            text("path", path)?;
        }
    }
    text("message", &error.to_string())?;
    Some(out)
}

/// The schema's fields grouped into sections and put in order.
///
/// Writes a list of `{ "section": <map or null>, "fields": [key, …] }` to
/// `out`, built through `alloc` and freed with `guatiao_value_free`. The
/// rules are `guatiao_form::layout`'s: declared sections in order, the
/// default section first unless the form places it, explicit `x-order`
/// ahead of declaration order, empty groups left out.
///
/// # Safety
///
/// Every non-null pointer addresses what its type says, and `out`
/// addresses writable storage for one value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_form_layout(
    schema: *const Value,
    form: *const Value,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    if out.is_null() {
        return Status::GUATIAO_ERR_NULL;
    }
    guard(|| {
        // SAFETY: forwarded from this function's contract.
        let (schema, form) = match unsafe { views(schema, form) } {
            Ok(v) => v,
            Err(status) => return status,
        };
        // SAFETY: the caller's contract on `alloc`.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_BAD_VALUE;
        };
        let Some(built) = layout_value(schema, form, alloc) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: `out` is writable by contract, and the tree moves into it.
        unsafe { out.write(built) };
        Status::GUATIAO_OK
    })
}

fn layout_value(schema: SchemaRef<'_>, form: FormRef<'_>, alloc: Alloc) -> Option<Value> {
    let mut groups = Value::list_in(alloc);
    for group in layout(schema, form) {
        let section = match group.section {
            // A copy of the section as the form wrote it, annotations and
            // all, so the caller reads it with the same code it reads the
            // form with.
            Some(section) => section.as_value().to_value(alloc).ok()?,
            None => Value::null(),
        };
        let mut keys = Value::list_in(alloc);
        for placed in &group.fields {
            keys.push(Value::string_in(alloc, placed.field.key()).ok()?)
                .ok()?;
        }
        let mut entry = Value::map_in(alloc);
        entry.set(GROUP_SECTION, section).ok()?;
        entry.set(GROUP_FIELDS, keys).ok()?;
        groups.push(entry).ok()?;
    }
    Some(groups)
}

/// Whether the field under `key` is shown, given the `values` entered so
/// far. Writes the answer through `out`.
///
/// The rules are `guatiao_form::is_visible`'s: no condition is shown; a
/// condition is met when the field it reads is itself shown and holds the
/// value; a field holding nothing reads as its schema default.
///
/// # Safety
///
/// Every non-null pointer addresses what its type says, `key` is a readable
/// view, and `out` addresses writable storage for one `bool`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_form_is_visible(
    schema: *const Value,
    form: *const Value,
    key: Str,
    values: *const Value,
    out: *mut bool,
) -> Status {
    if out.is_null() || values.is_null() {
        return Status::GUATIAO_ERR_NULL;
    }
    guard(|| {
        // SAFETY: forwarded from this function's contract.
        let (schema, form) = match unsafe { views(schema, form) } {
            Ok(v) => v,
            Err(status) => return status,
        };
        // SAFETY: as above.
        let key = match unsafe { key_text(key) } {
            Ok(k) => k,
            Err(status) => return status,
        };
        // SAFETY: checked non-null; the caller's contract for the rest.
        let values = unsafe { &*values };
        // SAFETY: `out` is writable by contract.
        unsafe { out.write(is_visible(schema, form, key, values)) };
        Status::GUATIAO_OK
    })
}
