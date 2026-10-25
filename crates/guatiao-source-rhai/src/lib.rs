// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Rhai and guatiao.
//!
//! This file is a guatiao **source** whose libraries are Rhai scripts: a
//! host in any language adds it with `add_source_file` and calls
//! providers written in Rhai. Rhai is pure Rust and built into it; the
//! scripts run under the limits the host sets.
//!
//! The rlib is the same engine for a Rust host ([`RhaiEngine`], served
//! through `guatiao_bridge::BridgeSource`) and [`RhaiValues`], guatiao's
//! values as Rhai's, checked against the shared vectors.

#![deny(unsafe_code)]

extern crate alloc;

mod calls;
mod engine;
// The `extern "C"` surface: `guatiao_source`. The one module with `unsafe`.
#[allow(unsafe_code)]
mod exports;
mod helpers;
mod limits;
mod values;

pub use calls::{Handle, Refusal};
pub use engine::RhaiEngine;
pub use rhai;
pub use values::{Absent, Exact, RhaiValues};
