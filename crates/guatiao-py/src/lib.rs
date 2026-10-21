// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A guatiao **source** of libraries written in Python.
//!
//! A host adds this file with `add_source_file` and a configuration naming
//! where Python libraries are described; each is a manifest
//! (`*.guatiao-py.toml`) naming a module, which is the library as the
//! Python binding's `guatiao.provide` reads one. A refresh lists the manifests without
//! importing anything, opens a new or changed one by importing it into the
//! one interpreter this file keeps for the program, and closes one the
//! registry let go by dropping its modules, so the next refresh imports the
//! code as it now is.
//!
//! The interpreter is initialised when the source is added, or joined when
//! the host is itself Python, and never finalised.

#![deny(unsafe_code)]

extern crate alloc;

mod bridge;
// The `extern "C"` surface: the source entry point and its three slots,
// and finding this file's own path. The one module with `unsafe`.
#[allow(unsafe_code)]
mod exports;
mod manifest;
