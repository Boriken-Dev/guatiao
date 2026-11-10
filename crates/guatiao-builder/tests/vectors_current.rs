// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The two vector files both libraries are held to are byte for byte coqui-builder's copies,
//! found through `cargo metadata` at the revision this crate depends on.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

/// The directory of the `coqui-builder` package this crate builds against.
fn coqui_builder() -> PathBuf {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--offline"])
        .arg("--manifest-path")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .output()
        .expect("cargo metadata runs");
    assert!(output.status.success(), "cargo metadata failed");
    let metadata: Value = serde_json::from_slice(&output.stdout).expect("metadata is JSON");
    let package = metadata["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .find(|p| p["name"] == "coqui-builder")
        .expect("coqui-builder is a dependency");
    let manifest = PathBuf::from(package["manifest_path"].as_str().expect("a path"));
    manifest.parent().expect("a directory").to_path_buf()
}

fn read(path: PathBuf) -> String {
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Asserts the file at `ours` (from this repository) is the file at `theirs`.
fn assert_byte_copy(ours: &[&str], theirs: PathBuf) {
    let ours: PathBuf = core::iter::once(env!("CARGO_MANIFEST_DIR"))
        .chain(ours.iter().copied())
        .collect();
    assert!(
        read(ours.clone()) == read(theirs),
        "{} is not coqui-builder's copy",
        ours.display()
    );
}

#[test]
fn differences_json_is_a_byte_copy() {
    assert_byte_copy(
        &["..", "guatiao", "tests", "vectors", "differences.json"],
        coqui_builder().join("tests/vectors/differences.json"),
    );
}

#[test]
fn attributes_json_is_a_byte_copy() {
    assert_byte_copy(
        &[
            "..",
            "guatiao-derive",
            "tests",
            "vectors",
            "attributes.json",
        ],
        coqui_builder().join("crates/coqui-builder-derive/tests/vectors/attributes.json"),
    );
}
