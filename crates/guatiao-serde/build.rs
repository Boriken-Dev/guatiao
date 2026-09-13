// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Renders this crate's C header, the way the core crate renders its own.
//!
//! The header is **committed**, because a consumer in C or Python reads it
//! without a Rust toolchain. Committed and generated at once means it can
//! go stale, so the render happens here on every build and a test compares
//! the committed file against it.
//!
//! # Refreshing the committed copy
//!
//! ```text
//! GUATIAO_WRITE_HEADER=1 cargo build -p guatiao-serde --features c-header,json,toml,yaml
//! ```
//!
//! **Render it with every format on.** The declarations are behind the
//! same features as the functions, so a render with `json` alone would
//! commit a header that silently lacks the TOML and YAML pair.

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

    let config = cbindgen::Config::from_file(crate_dir.join("cbindgen.toml"))
        .expect("cbindgen.toml is committed beside this script");

    let generated = match cbindgen::generate_with_config(&crate_dir, config) {
        Ok(g) => g,
        // Warn rather than fail the build over a header the consumer may
        // not want; the comparison test is what goes red.
        Err(e) => {
            println!("cargo::warning=cbindgen could not render the header: {e}");
            return;
        }
    };

    let rendered = out_dir.join("guatiao_serde.h");
    generated.write_to_file(&rendered);
    println!(
        "cargo::rustc-env=GUATIAO_SERDE_GENERATED_HEADER={}",
        rendered.display()
    );

    if std::env::var_os("GUATIAO_WRITE_HEADER").is_some() {
        generated.write_to_file(crate_dir.join("include").join("guatiao_serde.h"));
    }
}
