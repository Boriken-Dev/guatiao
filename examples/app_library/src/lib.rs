// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! An application's own C library, carrying guatiao's C surfaces.
//!
//! rustc exports a dependency's `#[no_mangle]` functions from a `cdylib`,
//! so linking the three crates is all it takes: this one file exports
//! `guatiao_*`, `guatiao_json_*` and `guatiao_intake_*` beside the
//! application's own [`myapp_version`], and a binding told to use it
//! serves every surface from it. Built for `wasm32-unknown-unknown`, it is
//! the one module a web page loads.

use guatiao as _;
use guatiao_intake as _;
use guatiao_serde as _;

/// A function of the application's own, exported beside guatiao's.
#[unsafe(no_mangle)]
pub extern "C" fn myapp_version() -> u32 {
    1
}
