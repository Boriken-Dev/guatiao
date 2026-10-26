// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `unsafe` lives in the files listed here and nowhere else in `src/`;
//! every other file forbids it, or is reached only through a module that
//! does.

use std::path::{Path, PathBuf};

/// The files allowed to hold `unsafe`, and why.
const ALLOWED: &[&str] = &[
    // `guatiao::source!` expands to the exported `guatiao_source`.
    "exports.rs",
    // The shared rings: a mapping, atomics on it, copies in and out.
    "transport/shm.rs",
    // libibverbs opened at run time: its structs, calls and registered
    // memory, behind safe types.
    "transport/rdma/verbs.rs",
    // Windows: `CancelIoEx`, ending a read another thread is blocked in.
    "transport/cancel.rs",
];

fn sources(dir: &Path, into: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, into);
        } else if path.extension().is_some_and(|e| e == "rs") {
            into.push(path);
        }
    }
}

#[test]
fn unsafe_is_only_where_it_is_listed() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&src, &mut files);
    let lib = std::fs::read_to_string(src.join("lib.rs")).unwrap();
    assert!(
        lib.contains("#![deny(unsafe_code)]"),
        "lib.rs denies unsafe"
    );
    for file in files {
        let rel = file
            .strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let text = std::fs::read_to_string(&file).unwrap();
        let has_unsafe = text.lines().any(|l| {
            let code = l.split("//").next().unwrap_or_default();
            code.contains("unsafe ") || code.contains("unsafe{")
        });
        if ALLOWED.contains(&rel.as_str()) {
            continue;
        }
        assert!(!has_unsafe, "{rel} holds `unsafe`, and is not listed");
    }
    for allowed in ALLOWED {
        assert!(src.join(allowed).is_file(), "{allowed} is listed and gone");
    }
}
