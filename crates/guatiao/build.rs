// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Renders the C header, so it cannot drift from the Rust that declares
//! it.
//!
//! The header is **committed**, because a consumer in C, C++ or Dart
//! reads it out of the repository without a Rust toolchain. Committed and
//! generated at once means it can go stale, so the render happens here on
//! every build and `tests/c_consumer.rs` compares the committed file
//! against it. An edit to the Rust that is not regenerated fails the
//! suite rather than shipping a header describing a different ABI.
//!
//! # Refreshing the committed copy
//!
//! ```text
//! GUATIAO_WRITE_HEADER=1 cargo build -p guatiao --features c-exports
//! ```
//!
//! Writing into the source tree is opt-in because a build script writing
//! outside `OUT_DIR` breaks a read-only or vendored checkout. The default
//! path renders into `OUT_DIR` and touches nothing.
//!
//! # Why this only runs under `c-exports`
//!
//! The header declares the exported ABI, and `cbindgen` is an optional
//! **build**-dependency enabled by that same feature. A default build of
//! this crate pulls in nothing, which is a property worth keeping: a
//! consumer that only reads a value should not compile a code generator.

fn main() {
    // Cheap and unconditional, so a source edit re-renders even when the
    // feature is off and the render is skipped.
    println!("cargo::rerun-if-changed=src");
    println!("cargo::rerun-if-changed=cbindgen.toml");
    println!("cargo::rerun-if-changed=Cargo.toml");
    println!("cargo::rerun-if-env-changed=GUATIAO_WRITE_HEADER");

    #[cfg(feature = "c-exports")]
    render();
}

#[cfg(feature = "c-exports")]
fn render() {
    use std::path::PathBuf;

    let crate_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets this"));
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets this"));

    let config = cbindgen::Config::from_file(crate_dir.join("cbindgen.toml"))
        .expect("cbindgen.toml is committed beside this script");

    let generated = match cbindgen::generate_with_config(&crate_dir, config) {
        Ok(g) => g,
        // A build script that fails the build over a header the consumer
        // may not even want is the wrong trade: say so loudly and let the
        // comparison test be what goes red.
        Err(e) => {
            println!("cargo::warning=cbindgen could not render the header: {e}");
            return;
        }
    };

    // `OUT_DIR` always, so the comparison test has something to read
    // without a `cbindgen` binary on `PATH`.
    let rendered = out_dir.join("guatiao.h");
    generated.write_to_file(&rendered);
    println!(
        "cargo::rustc-env=GUATIAO_GENERATED_HEADER={}",
        rendered.display()
    );

    // The committed copy, only when asked.
    if std::env::var_os("GUATIAO_WRITE_HEADER").is_some() {
        generated.write_to_file(crate_dir.join("include").join("guatiao.h"));
    }
}
