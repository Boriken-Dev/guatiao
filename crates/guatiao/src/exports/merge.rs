// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Layering, callable from C.
//!
//! Everything this crate does that a caller cannot do by reading structs
//! is reachable across the boundary, and merging is the largest of those:
//! it is real work over two trees, and a C or Dart consumer that had to
//! reimplement it would get one of the five surprising behaviours wrong.
//!
//! # What crosses, and in what shape
//!
//! The mode is a plain `uint32_t` **checked on arrival**, not an enum
//! parameter: handing an out-of-range integer to a Rust enum is undefined
//! behaviour immediately, and the C side of a boundary is exactly where
//! an out-of-range integer comes from. The sub-options are a bitmask for
//! the same reason a `bool` is not passed by value anywhere else here —
//! only two of its 256 bit patterns are valid, and a boundary is where
//! the other 254 arrive.
//!
//! Overrides are an array of `{path, mode}` rather than a map, because a
//! caller assembling a handful of them in C should not have to build a
//! container first.
//!
//! # The error carries its detail as a VALUE
//!
//! A status says a merge failed; it cannot say that `tls.verify` was a
//! map on one side and a string on the other. So the failure is written
//! through an optional out-parameter as an ordinary map — which the
//! caller already knows how to read, and which needs no second error
//! vocabulary in the header.

#![allow(non_camel_case_types)]

use std::ptr;

use super::{entry, out};
use crate::value::alloc::{Alloc, Allocator};
use crate::value::merge::{MergeError, MergeMode, MergeOptions, MergeOverrides};
use crate::value::status::Status;
use crate::value::types::{Number, Str, Value};

/// Shallow: top-level keys replace, nested maps are not recursed into.
pub const GUATIAO_MERGE_SIMPLE: u32 = 1;
/// Recursive maps, lists extended with unique items.
pub const GUATIAO_MERGE_DEEP: u32 = 2;
/// Recursive maps, wholesale list replacement. The default.
pub const GUATIAO_MERGE_SUBSTITUTE: u32 = 3;

/// `mergelists`: [`GUATIAO_MERGE_DEEP`] merges map elements of a list by
/// position rather than appending them.
pub const GUATIAO_MERGE_OPT_MERGELISTS: u32 = 1;

/// One per-path mode override: what the *declarer* of an option knows
/// that whoever merges two maps does not.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MergeOverride {
    /// The dotted path this governs, matched exactly.
    pub path: Str,
    /// One of the `GUATIAO_MERGE_*` mode constants.
    pub mode: u32,
}

/// The mode a `uint32_t` names, or `None` for one nobody declared.
///
/// **An explicit match, never a transmute.** Handing an integer outside
/// the declared set to a Rust enum is undefined behaviour at the moment
/// it happens, not at the moment it is read, and this is the one function
/// standing between a foreign caller and that.
fn mode_of(raw: u32) -> Option<MergeMode> {
    match raw {
        GUATIAO_MERGE_SIMPLE => Some(MergeMode::Simple),
        GUATIAO_MERGE_DEEP => Some(MergeMode::Deep),
        GUATIAO_MERGE_SUBSTITUTE => Some(MergeMode::Substitute),
        _ => None,
    }
}

/// The number a mode is written as, for an error value a caller reads.
fn mode_number(mode: MergeMode) -> i64 {
    match mode {
        MergeMode::Simple => GUATIAO_MERGE_SIMPLE as i64,
        MergeMode::Deep => GUATIAO_MERGE_DEEP as i64,
        _ => GUATIAO_MERGE_SUBSTITUTE as i64,
    }
}

/// Builds the error value a caller reads the detail out of.
///
/// Best effort: if the allocator refuses while building the explanation,
/// the caller still gets the status. An explanation that could not be
/// built is not worth turning a reportable failure into a different one.
fn describe(error: &MergeError, alloc: Alloc) -> Option<Value> {
    let mut out = Value::map_in(alloc);
    match error {
        MergeError::Kind {
            path,
            earlier,
            later,
            mode,
        } => {
            out.set("path", Value::string_in(alloc, path).ok()?).ok()?;
            out.set(
                "mode",
                Number::new_in(alloc, &mode_number(*mode).to_string()).ok()?,
            )
            .ok()?;
            if let Some(tag) = earlier {
                out.set(
                    "earlier",
                    Number::new_in(alloc, &u32::from(*tag).to_string()).ok()?,
                )
                .ok()?;
            }
            if let Some(tag) = later {
                out.set(
                    "later",
                    Number::new_in(alloc, &u32::from(*tag).to_string()).ok()?,
                )
                .ok()?;
            }
        }
        MergeError::Build(_) => {
            out.set("path", Value::string_in(alloc, "").ok()?).ok()?;
        }
    }
    out.set("message", Value::string_in(alloc, &error.to_string()).ok()?)
        .ok()?;
    Some(out)
}

impl From<&MergeError> for Status {
    fn from(e: &MergeError) -> Status {
        match e {
            MergeError::Kind { .. } => Status::GUATIAO_ERR_WRONG_KIND,
            MergeError::Build(inner) => Status::from(*inner),
        }
    }
}

/// Merges `later` into `earlier`, later winning, and writes a new tree
/// through `out`.
///
/// Neither input is touched. The result is built entirely through
/// `alloc`, which is also the allocator it frees through, so the caller
/// releases it with `guatiao_value_free` and nothing else.
///
/// `overrides` may be null when `overrides_len` is zero. `out_error` may
/// be null, and receives a map describing the disagreement when the merge
/// fails on one.
///
/// Both `out` and a non-null `out_error` are written the absent marker on
/// entry, so a failed call leaves each ABSENT rather than untouched — a
/// caller that reads one back after a failure reads what the call
/// produced, not what its own local happened to contain.
///
/// A null or unusable allocator is `GUATIAO_ERR_ALLOC`.
///
/// # Safety
///
/// Every non-null pointer addresses what its type says, `out` addresses
/// writable storage for one value that does not already hold one the
/// caller still owns, and `overrides` addresses `overrides_len` entries.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_merge(
    mode: u32,
    earlier: *const Value,
    later: *const Value,
    alloc: *const Allocator,
    options: u32,
    overrides: *const MergeOverride,
    overrides_len: usize,
    out: *mut Value,
    out_error: *mut Value,
) -> Status {
    out!(out, out_error);
    entry!(earlier, later, out => {
        let Some(mode) = mode_of(mode) else {
            return Status::GUATIAO_ERR_BAD_VALUE;
        };
        // SAFETY: the caller's contract.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };

        let mut table = MergeOverrides::new();
        if overrides_len > 0 {
            if overrides.is_null() {
                return Status::GUATIAO_ERR_NULL;
            }
            for i in 0..overrides_len {
                // SAFETY: the caller declared `overrides_len` entries.
                let entry = unsafe { overrides.add(i).read() };
                let Some(entry_mode) = mode_of(entry.mode) else {
                    return Status::GUATIAO_ERR_BAD_VALUE;
                };
                // SAFETY: a view whose `len` bytes are readable.
                let Some(path) = (unsafe { crate::library::raw::str_of(entry.path) }) else {
                    return Status::GUATIAO_ERR_BAD_VALUE;
                };
                table.set(path, entry_mode);
            }
        }

        let sub = MergeOptions::new().with_mergelists(options & GUATIAO_MERGE_OPT_MERGELISTS != 0);

        // SAFETY: both are well-formed values by the caller's contract.
        let (a, b) = unsafe { (&*earlier, &*later) };
        match mode.merge_with(a, b, alloc, sub, Some(&table)) {
            Ok(merged) => {
                // SAFETY: `out` is writable, and the tree stops being
                // Rust's here.
                unsafe { ptr::write(out, merged) };
                Status::GUATIAO_OK
            }
            Err(e) => {
                if !out_error.is_null()
                    && let Some(detail) = describe(&e, alloc)
                {
                    // SAFETY: `out_error` is writable by contract.
                    unsafe { ptr::write(out_error, detail) };
                }
                Status::from(&e)
            }
        }
    })
}
