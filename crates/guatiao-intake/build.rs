// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Renders this crate's C header, the way the other crates render theirs.
//!
//! The header is **committed**, because a consumer in C or Python reads it
//! without a Rust toolchain. Committed and generated at once means it can
//! go stale, so the render happens here on every build and a test compares
//! the committed file against it.
//!
//! # Refreshing the committed copy
//!
//! ```text
//! GUATIAO_WRITE_HEADER=1 cargo build -p guatiao-intake --features c-header
//! ```

fn main() {
    // Where the committed header is, for a consumer whose own header or
    // build includes it: readable in that consumer's build script as
    // `DEP_GUATIAO_INTAKE_INCLUDE`, because `Cargo.toml` says
    // `links = "guatiao-intake"` -- the arrangement `guatiao` itself has.
    let include = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("include");
    println!("cargo::metadata=include={}", include.display());
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

    let rendered = out_dir.join("guatiao_intake.h");
    generated.write_to_file(&rendered);
    println!(
        "cargo::rustc-env=GUATIAO_INTAKE_GENERATED_HEADER={}",
        rendered.display()
    );

    if std::env::var_os("GUATIAO_WRITE_HEADER").is_some() {
        generated.write_to_file(crate_dir.join("include").join("guatiao_intake.h"));
    }
}
