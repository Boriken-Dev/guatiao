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
//! Whichever one enables the `c-exports` feature. A host that is already
//! a shared library enables it and exports them itself; a consumer with
//! no such host builds a thin `cdylib` that does nothing else. Two copies
//! in one process are harmless, because every one of these is pure over
//! the structs plus the allocator pointer the tree carries — there is no
//! process-global state for the copies to disagree about.
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

pub mod merge;
pub mod schema;
pub mod value;

/// Runs a boundary body, converting a panic into a status rather than an
/// abort, and forgetting the payload rather than dropping it.
///
/// One copy, used by every `extern "C"` body in the crate, so the two
/// halves of the boundary cannot drift apart on the one rule that keeps a
/// host alive.
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
