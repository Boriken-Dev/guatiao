// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What every guatiao bridge to a language runtime shares.
//!
//! A **bridge** makes libraries written in another language (Python, Lua,
//! JavaScript, Rhai, WebAssembly) providers any guatiao host loads, in any
//! language: it is a guatiao source (`Registry::add_source_file`) whose
//! libraries are that language's modules. Every bridge needs the same
//! machinery around its runtime, and this crate is it:
//!
//! - [`Engine`]: what the runtime implements (start, open, call, close),
//!   and its [`ThreadModel`];
//! - the engine's thread, for a runtime that lives on one;
//! - [`manifest`]: libraries described without running them, and their
//!   stamps;
//! - [`BridgeSource`]: the glue from an engine to guatiao's `Source`;
//! - [`Kinds`]: kinds known from their descriptions, and providers an
//!   engine declares served through tables built from them, once the
//!   source is given a mechanism ([`BridgeSource::with_mechanism`]; the
//!   crate `guatiao-libffi` is one);
//! - [`Limits`] and [`BridgeError`]: what a library's code may use, and
//!   what each refusal crosses as;
//! - [`Convert`]: an engine's values both ways, and the shared vectors;
//! - [`Held`] and [`PreparedArg`]: a library's code calling a host's object,
//!   its handle and its arguments, the engine reading only its own values.
//!
//! A language crate holds only its engine and its value conversion, and
//! exports the source with `guatiao::source!`:
//!
//! ```ignore
//! fn start(_host: Host, config: &Map) -> Result<BridgeSource<Lua>, ProviderError> {
//!     Ok(BridgeSource::pinned(config, "lua")?)
//! }
//! guatiao::source!(serves = "lua", make = start);
//! ```

#![forbid(unsafe_code)]

extern crate alloc;

mod calling;
mod config;
mod convert;
mod described;
mod engine;
mod error;
mod kinds;
pub mod manifest;
mod source;
mod thread;

pub use calling::{Held, PreparedArg, Reading};
pub use config::{Budget, Config, Limits, text_list};
pub use convert::{Convert, conformance};
pub use engine::{Call, CallArg, Engine, Opened, Provided, Reply, Target, ThreadModel};
pub use error::{BridgeError, Limit};
pub use kinds::Kinds;
pub use manifest::{Found, Manifest, ManifestFormat};
pub use source::BridgeSource;
