// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The C compiler a test uses, set to build for that test's own target.
//!
//! Every test in this repository that compiles C or C++ includes this file
//! with `#[path]`, from any of its workspaces.

#![allow(
    dead_code,
    reason = "each test that includes this file uses only part of it"
)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// The first C compiler that runs: clang, the default LLVM install path,
/// cc, gcc.
///
/// clang comes first because it runs from a plain shell on Windows, where
/// `cl.exe` needs a developer environment.
pub fn find() -> Option<PathBuf> {
    for name in ["clang", r"C:\Program Files\LLVM\bin\clang.exe", "cc", "gcc"] {
        let path = PathBuf::from(name);
        // A bare name is looked up on PATH; a missing absolute path is
        // skipped rather than run.
        if path.is_absolute() && !path.exists() {
            continue;
        }
        if Command::new(&path).arg("--version").output().is_ok() {
            return Some(path);
        }
    }
    None
}

/// A command running `compiler`, given `--target=<arch>-pc-windows-msvc`
/// when it is a clang driver on Windows MSVC.
///
/// clang builds for its own host by default. An x64 test running emulated
/// on ARM64 would get an ARM64 program or library, which cannot link this
/// test's x64 artifacts or load into its process.
pub fn command(compiler: &Path) -> Command {
    let mut command = Command::new(compiler);
    let clang = compiler.file_stem().is_some_and(|stem| {
        stem.to_string_lossy()
            .to_ascii_lowercase()
            .starts_with("clang")
    });
    if cfg!(all(windows, target_env = "msvc")) && clang {
        let arch = match std::env::consts::ARCH {
            "x86" => "i686",
            arch => arch,
        };
        command.arg(format!("--target={arch}-pc-windows-msvc"));
    }
    command
}
