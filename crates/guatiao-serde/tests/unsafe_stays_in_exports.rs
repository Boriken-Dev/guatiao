// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `unsafe` lives in `exports.rs` and nowhere else in this crate.
//!
//! `#![forbid(unsafe_code)]` binds child modules, so the crate root cannot
//! carry it without binding the exports too. Every other module carries it
//! itself, and this is what keeps that true when a module is added.
//!
//! The same check `guatiao-intake` has, and the one `lib.rs` cites: the
//! arrangement is only a claim until something reads the files.

use std::fs;
use std::path::PathBuf;

#[test]
fn every_module_but_the_exports_forbids_unsafe() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut checked = 0;
    let mut missing = Vec::new();

    for entry in fs::read_dir(&src).expect("the source directory is readable") {
        let path = entry.expect("a directory entry").path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let name = path
            .file_name()
            .expect("a file has a name")
            .to_string_lossy()
            .into_owned();
        let text = fs::read_to_string(&path).expect("a source file is readable");
        let forbids = text
            .lines()
            .any(|line| line.trim() == "#![forbid(unsafe_code)]");
        checked += 1;

        match name.as_str() {
            // The root cannot forbid: the attribute would bind `exports`.
            // The exports cannot forbid: they are the boundary.
            "lib.rs" | "exports.rs" => assert!(
                !forbids,
                "{name} forbids unsafe, which would stop the C surface compiling"
            ),
            _ if !forbids => missing.push(name),
            _ => {}
        }
    }

    // `de`, `exports`, `lib`, `ser`, `text` -- every one of them, so a
    // pass here is not a pass over an empty directory.
    assert!(
        checked >= 5,
        "only {checked} source files were checked, so this proved little"
    );
    assert!(
        missing.is_empty(),
        "these modules do not carry `#![forbid(unsafe_code)]`: {missing:?}"
    );
}
