// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The `extern "C"` surface: everything this crate can do, reachable by a
//! caller that cannot link Rust.
//!
//! # What is here and what is not
//!
//! **If it is public and it is not a Rust convenience, it is here.**
//! Reading a value needs nothing from this module — a value is a plain
//! struct and the header's `static inline` helpers walk one with no call
//! into any library — but everything that is real work is: building and
//! freeing a tree, layering two of them, and checking one against a
//! schema.
//!
//! What is deliberately absent is the half that only means something in
//! Rust: `Drop`, which frees a tree when a binding ends and has no
//! meaning to a caller that has no bindings; the conversion traits, which
//! convert *Rust* types; and the builders, which are a typed way to
//! assemble a value a C caller assembles directly. A C caller frees with
//! `guatiao_value_free` — the same walk `Drop` runs.
//!
//! # Which artifact exports them
//!
//! **Every `cdylib` built from this crate, always.** These are not behind
//! a feature: this is an FFI library, and a surface that appears only
//! when somebody remembers a flag is one a C caller cannot rely on.
//!
//! That means a plugin `cdylib` exports them too, alongside its own entry
//! symbol. Two copies in one process are harmless, and that is a property
//! of what these functions are rather than luck: every one is pure over
//! the structs plus the allocator pointer the tree carries, so there is
//! no process-global state for the copies to disagree about. A tree
//! allocated by one copy frees correctly through another, because the
//! allocator travels with it.
//!
//! # The unwind discipline
//!
//! These stay `extern "C"` rather than `extern "C-unwind"`. A Rust panic
//! escaping `extern "C"` aborts, which is *defined* and debuggable; a
//! panic loose in a Dart or C++ runtime is the case with no guarantees at
//! all. Aborting is the floor, though, not the goal, so every body is
//! wrapped in a guard and converted to a status: a configuration
//! library that kills a host application because a key was malformed is
//! not shippable.
//!
//! Dropping the caught payload can itself panic, which would land back in
//! the abort case, so the payload is forgotten rather than dropped.
//!
//! # Why one directory
//!
//! `unsafe` lives in three places in this crate and no others: the value
//! model, which writes raw pointers by definition; the loader, which maps
//! a library; and here. Scattering boundary functions beside the modules
//! they wrap would have added an exemption per module to
//! `tests/forbid_unsafe_per_module.rs` and turned "an auditor's scope is
//! a directory" into a list nobody reads.
//!
//! # What a caller must guarantee, and what is checked
//!
//! Every pointer is null-checked here. What cannot be checked is that a
//! non-null pointer really addresses a well-formed value — that is the
//! caller's side of the contract, stated in the header.

#![allow(non_camel_case_types)]

use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::value::status::Status;
use crate::value::types::Str;

// Loading needs `libloading`, and the `load` feature exists so a library
// AUTHOR takes no dependency at all. The artifact a host links turns it
// on. A `//` comment, never a `///`.
#[cfg(feature = "load")]
pub mod library;
pub mod merge;
pub mod schema;
pub mod value;

/// Runs a boundary body, converting a panic into a status rather than an
/// abort, and forgetting the payload rather than dropping it.
///
/// One copy, used by every `extern "C"` body in the crate, so the two
/// halves of the boundary cannot drift apart on the one rule that keeps a
/// host alive.
/// A borrowed `&str` from a view, or a status saying why not.
///
/// # Safety
///
/// `s` is a view whose `len` bytes are readable for the call.
pub(crate) unsafe fn as_str<'a>(s: Str) -> Result<&'a str, Status> {
    if s.len == 0 {
        return Ok("");
    }
    if s.ptr.is_null() {
        return Err(Status::GUATIAO_ERR_NULL);
    }
    // SAFETY: the caller guarantees `len` readable bytes at `ptr`.
    let bytes = unsafe { std::slice::from_raw_parts(s.ptr, s.len) };
    std::str::from_utf8(bytes).map_err(|_| Status::GUATIAO_ERR_BAD_VALUE)
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
            // As in `guard`: dropping the payload can panic, and a second
            // panic inside an `extern "C"` body is an abort.
            std::mem::forget(payload);
            fallback
        }
    }
}

pub(crate) fn guard(body: impl FnOnce() -> Status) -> Status {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(s) => s,
        Err(payload) => {
            // Dropping a panic payload can panic, and a second panic
            // inside an `extern "C"` body is the abort this whole
            // arrangement exists to avoid.
            std::mem::forget(payload);
            Status::GUATIAO_ERR_INTERNAL
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

pub(crate) use entry;
