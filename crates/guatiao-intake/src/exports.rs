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
//! guatiao_status guatiao_intake_check(const guatiao_value *schema, const guatiao_value *form,
//!                                   const guatiao_alloc *alloc, guatiao_value *out_error);
//! guatiao_status guatiao_intake_layout(const guatiao_value *schema, const guatiao_value *form,
//!                                    const guatiao_alloc *alloc, guatiao_value *out);
//! guatiao_status guatiao_intake_is_visible(const guatiao_value *schema, const guatiao_value *form,
//!                                        guatiao_str key, const guatiao_value *values, bool *out);
//! ```
//!
//! # A layout is keys, not copies
//!
//! `guatiao_intake_layout` answers **grouping and order** and nothing else:
//! a list of `{ "section": <the section's map, or null>, "fields": [key, …] }`.
//! Everything else about a field the caller already holds — its schema
//! through `guatiao_intake_resolve`, its hints in the form it passed in —
//! so copying either would only give it a second thing to keep in step.
//!
//! # The shape of every function here
//!
//! **Null checks first, then the guard, then the safe Rust API.** Every
//! pointer a call needs is checked before anything else happens, in one
//! place per function, so `GUATIAO_ERR_NULL` never depends on how far a
//! body got; the work is the safe API, and a panic becomes
//! `GUATIAO_ERR_INTERNAL` rather than unwinding into C.
//!
//! # Statuses
//!
//! `GUATIAO_ERR_NULL` for a null pointer the call needs,
//! `GUATIAO_ERR_WRONG_KIND` when the schema or the form is not a map,
//! `GUATIAO_ERR_ALLOC` for an allocator that is null or cannot allocate,
//! `GUATIAO_ERR_NOT_FOUND` for a key the schema does not declare,
//! `GUATIAO_ERR_BAD_VALUE` when a form does not fit its schema, and
//! `GUATIAO_ERR_INTERNAL` if a panic was caught.

#![allow(non_camel_case_types)]

use std::panic::{AssertUnwindSafe, catch_unwind};

use guatiao::ToValue;
use guatiao::schema::read::SchemaRef;
use guatiao::value::alloc::{Alloc, Allocator};
use guatiao::value::status::Status;
use guatiao::value::types::{List, Map, Str, Text, Value};

use crate::judge::{FormError, check, is_visible, layout};
use crate::read::FormRef;

/// The key a layout group's section is under.
const GROUP_SECTION: &str = "section";
/// The key a layout group's field keys are under.
const GROUP_FIELDS: &str = "fields";

/// Runs a boundary body, turning a panic into a status. The payload is
/// forgotten rather than dropped: dropping one can panic, and a second
/// panic in an `extern "C"` body is the abort this exists to avoid.
pub(crate) fn guard(body: impl FnOnce() -> Status) -> Status {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(status) => status,
        Err(payload) => {
            std::mem::forget(payload);
            Status::GUATIAO_ERR_INTERNAL
        }
    }
}

/// [`guard`] for a body that answers something other than a status.
///
/// A function returning a POINTER cannot report an internal failure as a
/// status, so it reports it the only way its signature allows: by handing
/// back `fallback`, which for a pointer is null.
pub(crate) fn guard_with<T>(fallback: T, body: impl FnOnce() -> T) -> T {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(v) => v,
        Err(payload) => {
            std::mem::forget(payload);
            fallback
        }
    }
}

/// Null-checks the pointers a boundary function must have, then runs its
/// body under [`guard`].
macro_rules! entry {
    ($($p:ident),* $(,)? => $body:expr) => {{
        $(if $p.is_null() { return Status::GUATIAO_ERR_NULL; })*
        $crate::exports::guard(|| $body)
    }};
}

/// Writes the absent marker through every non-null out-pointer named.
///
/// **The first statement of every function that has one**, before any
/// check and before [`entry!`], so a caller reading an out-parameter
/// after a failure reads ABSENT rather than whatever it happened to
/// contain.
macro_rules! out {
    ($($p:ident),* $(,)?) => {
        $(if !$p.is_null() {
            // SAFETY: checked non-null, and by the caller's contract it
            // addresses writable storage for one value that does not
            // already hold one the caller still owns.
            unsafe { ::std::ptr::write($p, ::guatiao::Value::absent()) };
        })*
    };
}

pub(crate) use {entry, out};

/// A schema and a form, viewed, or the status saying which was wrong.
///
/// # Safety
///
/// Each pointer is non-null -- every caller checks that first, which is
/// the one shape at this boundary -- and addresses a well-formed value
/// that outlives the call.
unsafe fn views<'a>(
    schema: *const Value,
    form: *const Value,
) -> Result<(SchemaRef<'a>, FormRef<'a>), Status> {
    // SAFETY: the caller's contract.
    let (schema, form) = unsafe { (&*schema, &*form) };
    let schema = SchemaRef::new(schema).ok_or(Status::GUATIAO_ERR_WRONG_KIND)?;
    let form = FormRef::new(form).ok_or(Status::GUATIAO_ERR_WRONG_KIND)?;
    Ok((schema, form))
}

/// Whether `form` fits `schema`.
///
/// `GUATIAO_ERR_BAD_VALUE` when it does not, and `out_error` — which may be
/// null — receives a map: `kind` (`malformed`, `unknown_field`,
/// `duplicate_section`, `condition_refused` or `cyclic_condition`), `at`,
/// whichever of `path`, `id`, `field` and `expected` apply, and a
/// `message`. It never carries a value the form compares with.
///
/// `GUATIAO_ERR_ALLOC` when `out_error` was asked for and `alloc` cannot
/// build it: the verdict is known, and the allocator is what to fix
/// first. Pass a null `out_error` for the verdict alone.
///
/// # Safety
///
/// Every non-null pointer addresses what its type says, and `out_error`
/// addresses writable storage for one value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_intake_check(
    schema: *const Value,
    form: *const Value,
    alloc: *const Allocator,
    out_error: *mut Value,
) -> Status {
    // `out_error` is the one optional pointer here: a caller that only
    // wants the verdict passes null and gets no detail.
    if schema.is_null() || form.is_null() {
        return Status::GUATIAO_ERR_NULL;
    }
    guard(|| {
        // SAFETY: forwarded from this function's contract.
        let (schema, form) = match unsafe { views(schema, form) } {
            Ok(v) => v,
            Err(status) => return status,
        };
        let Err(e) = check(schema, form) else {
            return Status::GUATIAO_OK;
        };
        if out_error.is_null() {
            return Status::GUATIAO_ERR_BAD_VALUE;
        }
        // SAFETY: the caller's contract on `alloc`.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            // The verdict is known and the detail cannot be built, so the
            // allocator is what the caller has to fix first.
            return Status::GUATIAO_ERR_ALLOC;
        };
        let Some(detail) = describe(&e, alloc) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: `out_error` is writable by contract, and the tree moves
        // into it.
        unsafe { out_error.write(detail) };
        Status::GUATIAO_ERR_BAD_VALUE
    })
}

/// The failure as a map a C caller already knows how to read.
fn describe(error: &FormError, alloc: Alloc) -> Option<Value> {
    let mut out = Map::new_in(alloc);
    let mut text = |key: &str, text: &str| -> Option<()> {
        out.set(key, Text::new_in(alloc, text).map(Value::from).ok()?)
            .ok()
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
    Some(out.into())
}

/// The schema's fields grouped into sections and put in order.
///
/// Writes a list of `{ "section": <map or null>, "fields": [key, …] }` to
/// `out`, built through `alloc` and freed with `guatiao_value_free`. The
/// rules are `guatiao_intake::layout`'s: declared sections in order, the
/// default section first unless the form places it, explicit `x-order`
/// ahead of declaration order, empty groups left out.
///
/// # Safety
///
/// Every non-null pointer addresses what its type says, and `out`
/// addresses writable storage for one value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_intake_layout(
    schema: *const Value,
    form: *const Value,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    if schema.is_null() || form.is_null() || out.is_null() {
        return Status::GUATIAO_ERR_NULL;
    }
    guard(|| {
        // SAFETY: forwarded from this function's contract.
        let (schema, form) = match unsafe { views(schema, form) } {
            Ok(v) => v,
            Err(status) => return status,
        };
        // A null or incomplete allocator is `GUATIAO_ERR_ALLOC`: the
        // caller passed something, and what it passed cannot allocate.
        // SAFETY: the caller's contract on `alloc`.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
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
    let mut groups = List::new_in(alloc);
    for group in layout(schema, form) {
        let section = match group.section {
            // A copy of the section as the form wrote it, annotations and
            // all, so the caller reads it with the same code it reads the
            // form with.
            Some(section) => section.as_value().to_value(alloc).ok()?,
            None => Value::null(),
        };
        let mut keys = List::new_in(alloc);
        for placed in &group.fields {
            keys.push(
                Text::new_in(alloc, placed.field.key())
                    .map(Value::from)
                    .ok()?,
            )
            .ok()?;
        }
        let mut entry = Map::new_in(alloc);
        entry.set(GROUP_SECTION, section).ok()?;
        entry.set(GROUP_FIELDS, keys).ok()?;
        groups.push(entry).ok()?;
    }
    Some(groups.into())
}

/// The form a schema implies, for when nobody wrote one.
///
/// An object is a form: the schema already says which section each field
/// belongs to, so a renderer with no form still has one to draw. What
/// comes back carries one section per distinct `x-section` in
/// first-appearance order, **ids only** -- what a section is called is a
/// form's business and a schema has no opinion -- plus each member's own
/// form where there is something in it. A schema that groups nothing
/// gives an empty map, which is a complete form.
///
/// `out` receives a value the **caller owns** and frees with
/// `guatiao_value_free`.
///
/// # Safety
///
/// Every non-null pointer addresses what its type says, and `out`
/// addresses writable storage for one value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_intake_for_schema(
    schema: *const Value,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    out!(out);
    entry!(schema, out => {
        // SAFETY: the caller's contract.
        let Some(schema) = SchemaRef::new(unsafe { &*schema }) else {
            return Status::GUATIAO_ERR_WRONG_KIND;
        };
        // SAFETY: as above.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        match crate::create::for_schema(schema, alloc) {
            Ok(form) => {
                // SAFETY: `out` is writable by contract, and the tree
                // MOVES into it rather than being copied.
                unsafe { ::std::ptr::write(out, form) };
                Status::GUATIAO_OK
            }
            Err(e) => Status::from(e),
        }
    })
}

/// The form to show the field at `path` with: **the one the form
/// assigns, or the one its schema implies**.
///
/// The question a renderer asks. Assignment wins, because somebody wrote
/// it down; where nobody did, the member's schema still groups its
/// fields.
///
/// **A field with no form and no members is not an error**: `out` is left
/// ABSENT and the status is `GUATIAO_OK`, which is what a lookup that
/// found nothing answers everywhere else here. So is a path naming no
/// field.
///
/// `out` receives a value the **caller owns**, whichever way it was
/// reached.
///
/// # Safety
///
/// Every non-null pointer addresses what its type says, `path` is a
/// readable view, and `out` addresses writable storage for one value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_intake_form_for(
    form: *const Value,
    schema: *const Value,
    path: Str,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    out!(out);
    entry!(form, schema, out => {
        // SAFETY: the caller's contract.
        let (Some(form), Some(schema)) = (
            FormRef::new(unsafe { &*form }),
            SchemaRef::new(unsafe { &*schema }),
        ) else {
            return Status::GUATIAO_ERR_WRONG_KIND;
        };
        // SAFETY: as above.
        let Ok(path) = ::std::str::from_utf8(path.into()) else {
            return Status::GUATIAO_ERR_BAD_VALUE;
        };
        // SAFETY: as above.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        match crate::create::form_for(form, schema, path, alloc) {
            // Nothing to show it with is an answer, not a failure: the
            // slot keeps the absent marker `out!` wrote.
            None => Status::GUATIAO_OK,
            Some(Ok(found)) => {
                // SAFETY: as above; the tree moves into `out`.
                unsafe { ::std::ptr::write(out, found) };
                Status::GUATIAO_OK
            }
            Some(Err(e)) => Status::from(e),
        }
    })
}

/// Whether the field under `key` is shown, given the `values` entered so
/// far. Writes the answer through `out`.
///
/// The rules are `guatiao_intake::is_visible`'s: no condition is shown; a
/// condition is met when the field it reads is itself shown and holds the
/// value; a field holding nothing reads as its schema default.
///
/// `GUATIAO_ERR_NOT_FOUND` when `key` -- or a key a condition reads --
/// names no field the schema declares, and `out` is then not written.
///
/// # Safety
///
/// Every non-null pointer addresses what its type says, `key` is a readable
/// view, and `out` addresses writable storage for one `bool`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_intake_is_visible(
    schema: *const Value,
    form: *const Value,
    key: Str,
    values: *const Value,
    out: *mut bool,
) -> Status {
    if schema.is_null() || form.is_null() || values.is_null() || out.is_null() {
        return Status::GUATIAO_ERR_NULL;
    }
    guard(|| {
        // SAFETY: forwarded from this function's contract.
        let (schema, form) = match unsafe { views(schema, form) } {
            Ok(v) => v,
            Err(status) => return status,
        };
        // SAFETY: as above.
        let key = match std::str::from_utf8(key.into()) {
            Ok(k) => k,
            Err(e) => return e.into(),
        };
        // SAFETY: checked non-null; the caller's contract for the rest.
        let values = unsafe { &*values };
        let Ok(shown) = is_visible(schema, form, key, values) else {
            // The only failure the judgement has here: a key naming no
            // field. `out` stays untouched, so a caller that ignored the
            // status cannot read an answer that was never given.
            return Status::GUATIAO_ERR_NOT_FOUND;
        };
        // SAFETY: `out` is writable by contract.
        unsafe { out.write(shown) };
        Status::GUATIAO_OK
    })
}
