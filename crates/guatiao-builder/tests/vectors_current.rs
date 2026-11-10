// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The two vector files both libraries are held to are current with coqui-builder's copies,
//! found through `cargo metadata` at the revision this crate depends on.
//!
//! `differences.json` is byte for byte. `attributes.json` differs in the cases listed below,
//! each in the keys named: the guatiao columns and expectations a guatiao derive answers for
//! itself. A listed difference that has gone is as much a failure as an unlisted one.

extern crate alloc;

use alloc::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

/// Cases of `attributes.json` that differ from coqui-builder's copy, and in which keys.
const ALLOWED: &[(&str, &[&str])] = &[
    ("as_text on a list is per item", &["guatiao"]),
    ("as_text on an optional list is per item", &["guatiao"]),
    ("an Option with a written None", &["guatiao"]),
    ("none needs a default", &["guatiao"]),
    ("none is for an Option", &["guatiao"]),
    ("none beside as_text", &["guatiao"]),
    ("a generic enum", &["guatiao"]),
    ("a default makes a key optional", &["expect"]),
    ("other keeps a name no variant has", &["expect"]),
    (
        "with names a module that converts the field (a list of protocol names)",
        &["guatiao_item"],
    ),
    (
        "with names a module that converts bytes held as text (a protocol name)",
        &["guatiao_item"],
    ),
];

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

#[test]
fn differences_json_is_a_byte_copy() {
    let ours = read(
        [
            env!("CARGO_MANIFEST_DIR"),
            "../guatiao/tests/vectors/differences.json",
        ]
        .iter()
        .collect(),
    );
    let theirs = read(coqui_builder().join("tests/vectors/differences.json"));
    assert!(
        ours == theirs,
        "crates/guatiao/tests/vectors/differences.json is not coqui-builder's copy"
    );
}

#[test]
fn attributes_json_differs_only_where_listed() {
    let ours: Value = serde_json::from_str(&read(
        [
            env!("CARGO_MANIFEST_DIR"),
            "../guatiao-derive/tests/vectors/attributes.json",
        ]
        .iter()
        .collect(),
    ))
    .expect("JSON");
    let theirs: Value = serde_json::from_str(&read(
        coqui_builder().join("crates/coqui-builder-derive/tests/vectors/attributes.json"),
    ))
    .expect("JSON");
    assert_eq!(ours["about"], theirs["about"], "the about text");
    let (ours, theirs) = (
        ours["cases"].as_array().expect("cases"),
        theirs["cases"].as_array().expect("cases"),
    );
    let names = |cases: &[Value]| -> Vec<String> {
        cases
            .iter()
            .map(|c| c["name"].as_str().expect("a name").to_owned())
            .collect()
    };
    assert_eq!(names(ours), names(theirs), "the cases and their order");

    let mut differing = BTreeSet::new();
    for (mine, other) in ours.iter().zip(theirs) {
        let name = mine["name"].as_str().expect("a name");
        let (a, b) = (
            mine.as_object().expect("an object"),
            other.as_object().expect("an object"),
        );
        let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
        let found: BTreeSet<&str> = keys
            .into_iter()
            .filter(|k| a.get(*k) != b.get(*k))
            .map(String::as_str)
            .collect();
        let allowed: BTreeSet<&str> = ALLOWED
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, keys)| keys.iter().copied().collect())
            .unwrap_or_default();
        assert_eq!(found, allowed, "case {name:?}: keys that differ | allowed");
        if !found.is_empty() {
            differing.insert(name.to_owned());
        }
    }
    assert_eq!(differing.len(), ALLOWED.len(), "every listed case differs");
}
