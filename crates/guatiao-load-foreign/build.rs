// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Builds the helper program from `helper/helper.c` into `OUT_DIR`, where
//! `src/helper.rs` embeds it. Only for Linux on x86_64 and aarch64; the crate
//! is empty elsewhere.
//!
//! `GUATIAO_FOREIGN_HELPER_CC` names the C compiler, which must target glibc
//! on the target's architecture (default `cc`, and only when cargo builds on
//! that architecture); `none` builds the crate with no helper of its own, so a
//! host names one in its settings.

use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

include!("helper/flags.rs");

fn main() {
    println!("cargo:rustc-check-cfg=cfg(guatiao_foreign_helper)");
    println!("cargo:rerun-if-changed=helper/helper.c");
    println!("cargo:rerun-if-changed=helper/flags.rs");
    println!("cargo:rerun-if-env-changed=GUATIAO_FOREIGN_HELPER_CC");

    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if os != "linux" || !matches!(arch.as_str(), "x86_64" | "aarch64") {
        return;
    }

    let compiler = match env::var_os("GUATIAO_FOREIGN_HELPER_CC") {
        Some(name) if name == "none" => return,
        Some(name) => name,
        None => {
            let host = env::var("HOST").unwrap_or_default();
            let host_arch = host.split('-').next().unwrap_or_default();
            if host_arch != arch || !host.contains("linux") {
                panic!(
                    "the helper is a glibc program for {arch} and this build runs on {host}: \
                     set GUATIAO_FOREIGN_HELPER_CC to a C compiler targeting {arch} glibc, \
                     or to `none` to build without a helper"
                );
            }
            OsString::from("cc")
        }
    };

    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let source = manifest.join("helper").join("helper.c");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets it")).join("helper");
    let output = Command::new(&compiler)
        .args(HELPER_FLAGS)
        .arg("-o")
        .arg(&out)
        .arg(&source)
        .args(HELPER_LIBS)
        .output();
    match output {
        Ok(output) if output.status.success() => {}
        Ok(output) => panic!(
            "{} could not build the helper ({}):\n{}\nset GUATIAO_FOREIGN_HELPER_CC to a C \
             compiler targeting {arch} glibc, or to `none` to build without a helper",
            compiler.to_string_lossy(),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ),
        Err(error) => panic!(
            "{} could not be run to build the helper: {error}; set GUATIAO_FOREIGN_HELPER_CC \
             to a C compiler targeting {arch} glibc, or to `none` to build without a helper",
            compiler.to_string_lossy()
        ),
    }
    println!("cargo:rustc-cfg=guatiao_foreign_helper");
}
