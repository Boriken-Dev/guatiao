// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Lua and guatiao.
//!
//! Built as the **source** (the default), this file is a guatiao source
//! whose libraries are Lua modules: a host in any language adds it with
//! `add_source_file` and calls providers written in Lua. Lua is built
//! into it from its C sources.
//!
//! Built as a **module** (`--no-default-features --features
//! module,lua54`), the same Lua side is what `bindings/lua/guatiao`
//! offers a Lua interpreter: `require "guatiao"` for values, kinds and a
//! registry.
//!
//! The rlib is the Lua side for Rust: [`LuaValues`] (guatiao's values as
//! a Lua state's, checked against the shared vectors), [`helpers`] and
//! [`module`] (the `guatiao` table a library or a host requires).

#![deny(unsafe_code)]

extern crate alloc;

#[cfg(all(feature = "source", feature = "module"))]
compile_error!(
    "`source` and `module` are exclusive: a source carries its Lua, a module uses its host's"
);
#[cfg(all(feature = "source", feature = "luajit"))]
compile_error!("LuaJIT serves no source: its operations limit does not hold for compiled code");
#[cfg(all(feature = "module", feature = "luau"))]
compile_error!("Luau has no loadable modules");

mod calls;
#[cfg(feature = "source")]
mod engine;
// The `extern "C"` surface: `guatiao_source`. The one module with `unsafe`.
#[cfg(feature = "source")]
#[allow(unsafe_code)]
mod exports;
mod host;
#[cfg(feature = "source")]
mod limits;
mod served;
mod values;

pub use calls::{BufferView, Handle, KindView, Refusal};
#[cfg(feature = "source")]
pub use engine::LuaEngine;
pub use host::{RegistryView, helpers, module};
pub use mlua;
pub use values::{Absent, Bytes, Exact, LuaValues};
