// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The target a clang found on `PATH` must build for, shared by the tests
//! that compile C or C++ against this crate.

use std::path::Path;

/// `--target=<arch>-pc-windows-msvc` for a clang driver on Windows MSVC,
/// and nothing for any other compiler or platform.
///
/// clang builds for its own host by default. An x64 test running emulated
/// on ARM64 then gets an ARM64 program or library, which cannot link this
/// test's x64 artifacts or load into its process.
pub fn args(compiler: &Path) -> Vec<String> {
    let clang = compiler.file_stem().is_some_and(|stem| {
        stem.to_string_lossy()
            .to_ascii_lowercase()
            .starts_with("clang")
    });
    if !(cfg!(all(windows, target_env = "msvc")) && clang) {
        return Vec::new();
    }
    let arch = match std::env::consts::ARCH {
        "x86" => "i686",
        arch => arch,
    };
    vec![format!("--target={arch}-pc-windows-msvc")]
}
