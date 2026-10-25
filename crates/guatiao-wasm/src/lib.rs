// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! WebAssembly and guatiao.
//!
//! Built as a cdylib, this crate is a guatiao **source** whose libraries
//! are WebAssembly modules compiled from any language: a host in any
//! language adds it with `add_source_file` and calls providers that run
//! sandboxed by wasmtime, with no access to files, the network or the
//! host's memory, stopped at their limits.
//!
//! A module speaks **guatiao ABI 1** ([`abi`]): a few exports and imports,
//! every value crossing as wire bytes in the module's memory. Guests in
//! Rust use `guatiao-wasm-guest`; in Go, the `guest-go` package.
//!
//! The rlib is the engine ([`WasmEngine`]) for a Rust host that serves it
//! through its own `guatiao_bridge::BridgeSource`, and [`WireBytes`], the
//! conversion every value crosses by.

#![deny(unsafe_code)]

extern crate alloc;

pub mod abi;
pub mod budget;
mod convert;
mod engine;
// The `extern "C"` surface: `guatiao_source`. The one module with `unsafe`.
#[allow(unsafe_code)]
mod exports;
mod instance;
mod meter;
mod section;
mod wasi;

pub use convert::WireBytes;
pub use engine::{WasmEngine, WasmLibrary};
