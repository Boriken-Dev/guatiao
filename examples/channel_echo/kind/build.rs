// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Renders `include/channel_echo.h`, the echo kind's table as a C
//! struct, with `c-header` on. The table is `#[guatiao::kind]` output,
//! so cbindgen expands the crate first (`[parse.expand]`), which the
//! stable compiler allows only with `RUSTC_BOOTSTRAP=1`, set around the
//! render, in a target directory of its own. guatiao's own types are
//! named as `guatiao.h` declares them, through the rename table guatiao
//! ships beside it.
//!
//! ```text
//! GUATIAO_WRITE_HEADER=1 cargo build -p channel_echo_kind --features c-header
//! ```
//!
//! writes the committed copy as well; `tests/header_is_current.rs`
//! compares the two.

fn main() {
    println!("cargo::rerun-if-changed=src");
    println!("cargo::rerun-if-changed=cbindgen.toml");
    println!("cargo::rerun-if-changed=Cargo.toml");
    println!("cargo::rerun-if-env-changed=GUATIAO_WRITE_HEADER");

    #[cfg(feature = "c-header")]
    render();
}

#[cfg(feature = "c-header")]
fn render() {
    use std::path::PathBuf;

    let crate_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets this"));
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets this"));

    // SAFETY: a build script is single-threaded here, and the variables
    // are read by the child cargo cbindgen spawns.
    unsafe {
        std::env::set_var("RUSTC_BOOTSTRAP", "1");
        std::env::set_var("CARGO_EXPAND_TARGET_DIR", out_dir.join("expand"));
    }

    let mut config = cbindgen::Config::from_file(crate_dir.join("cbindgen.toml"))
        .expect("cbindgen.toml is committed beside this script");
    let include = PathBuf::from(
        std::env::var("DEP_GUATIAO_INCLUDE").expect("guatiao says `links = \"guatiao\"`"),
    );
    let shipped = include.join("kind_rename.toml");
    println!("cargo::rerun-if-changed={}", shipped.display());
    let guatiao_names = cbindgen::Config::from_file(&shipped)
        .expect("guatiao ships its rename table beside guatiao.h");
    config.export.rename.extend(guatiao_names.export.rename);
    let generated = match cbindgen::generate_with_config(&crate_dir, config) {
        Ok(g) => g,
        Err(e) => {
            println!("cargo::warning=cbindgen could not render the echo header: {e}");
            return;
        }
    };

    let rendered = out_dir.join("channel_echo.h");
    generated.write_to_file(&rendered);
    println!(
        "cargo::rustc-env=CHANNEL_ECHO_GENERATED_HEADER={}",
        rendered.display()
    );
    if std::env::var_os("GUATIAO_WRITE_HEADER").is_some() {
        generated.write_to_file(crate_dir.join("include").join("channel_echo.h"));
    }
}
