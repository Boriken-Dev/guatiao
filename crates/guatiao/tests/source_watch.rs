// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A watched directory, refreshed as library files are added and removed.
//!
//! Its own test binary: it maps a copy of `hello_library` and closes it,
//! which a test sharing a binary with another mapping of it would not
//! observe.

#![cfg(feature = "load")]

use std::path::PathBuf;

use guatiao::library::{Registry, ScanRules, SourceError};

/// The `hello_library` artifact cargo built beside this test binary.
fn hello_library() -> PathBuf {
    let exe = std::env::current_exe().expect("a test binary knows its path");
    let deps = exe.parent().expect("it is in a directory");
    let name = format!(
        "{}hello_library{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    [
        deps.to_path_buf(),
        deps.parent().map(Into::into).unwrap_or_default(),
    ]
    .into_iter()
    .map(|d| d.join(&name))
    .find(|p| p.exists())
    .expect("hello_library is built as a dev-dependency")
}

fn library_name(base: &str) -> String {
    format!(
        "{}{base}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    )
}

#[test]
fn a_watched_directory_follows_files_added_and_removed() {
    let dir =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("watched-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a directory to watch");
    let aside = dir.with_extension("aside");
    let _ = std::fs::remove_dir_all(&aside);
    std::fs::create_dir_all(&aside).expect("a directory beside it");

    let mut registry = Registry::new("watch-tests", "1.0");
    registry
        .watch_dir(&dir, ScanRules::default())
        .expect("the first watch of it");
    assert!(matches!(
        registry.watch_dir(&dir, ScanRules::default()),
        Err(SourceError::Duplicate { .. })
    ));
    assert!(
        registry.refresh().is_empty(),
        "an empty directory changes nothing"
    );

    // A file that appears is loaded; a text file with a library's
    // extension is passed over, once.
    let copy = dir.join(library_name("hello_watched"));
    std::fs::copy(hello_library(), &copy).expect("a copy of the library");
    std::fs::write(dir.join(library_name("not_a_library")), b"plain text").unwrap();
    let added = registry.refresh();
    assert_eq!(added.loaded.len(), 1, "{added:?}");
    assert_eq!(added.loaded[0].key, "hello_library");
    assert_eq!(added.skipped.len(), 1);
    assert!(registry.provider("hello_library_greeter").is_some());
    assert!(registry.refresh().is_empty(), "nothing changed");

    // A file that leaves is retired, and closed once nothing holds it. A
    // mapped image cannot be deleted on Windows, so it is moved aside.
    std::fs::rename(&copy, aside.join(library_name("hello_watched"))).expect("moved out");
    let removed = registry.refresh();
    assert_eq!(removed.retired.len(), 1, "{removed:?}");
    assert_eq!(removed.retired[0].key, "hello_library");
    assert!(registry.provider("hello_library_greeter").is_none());
    assert!(registry.loaded().is_empty());
    assert_eq!(
        registry.collect(),
        ["hello_library"],
        "closed: nothing held it"
    );

    // Back under another name: loaded again.
    std::fs::copy(hello_library(), dir.join(library_name("hello_again"))).expect("another copy");
    let again = registry.refresh();
    assert_eq!(again.loaded.len(), 1, "{again:?}");
    assert!(registry.provider("hello_library_greeter").is_some());
}
