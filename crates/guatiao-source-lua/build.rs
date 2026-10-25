// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Built as a module, the cdylib leaves Lua's symbols to the interpreter
//! that loads it; macOS's linker refuses that unless told.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let module = std::env::var_os("CARGO_FEATURE_MODULE").is_some();
    let macos = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos");
    if module && macos {
        println!("cargo:rustc-cdylib-link-arg=-undefined");
        println!("cargo:rustc-cdylib-link-arg=dynamic_lookup");
    }
}
