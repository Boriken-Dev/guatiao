// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! JavaScript and guatiao.
//!
//! This file is a guatiao **source** whose libraries are ES modules: a
//! host in any language adds it with `add_source_file` and calls
//! providers written in JavaScript, run by QuickJS built into the file.
//! A provider is written as the Node binding's are: a class extending
//! `Provider` from `guatiao/provide`, implementing the kinds it lists.
//!
//! The rlib is [`JsEngine`] (the bridge engine) and [`JsValues`] (guatiao's
//! values as a context's, checked against the shared vectors).

#![deny(unsafe_code)]

extern crate alloc;

// Reading the memory QuickJS lends: the one `unsafe` besides the export.
#[allow(unsafe_code)]
mod bytes;
mod engine;
mod errors;
// The `extern "C"` surface: `guatiao_source`.
#[allow(unsafe_code)]
mod exports;
// Entering a context while a frame of the runtime is on the stack.
#[allow(unsafe_code)]
mod frame;
mod host;
mod limits;
mod modules;
mod prelude;
mod served;
mod values;

pub use engine::JsEngine;
pub use rquickjs;
pub use values::{Conversion, JsValues};
