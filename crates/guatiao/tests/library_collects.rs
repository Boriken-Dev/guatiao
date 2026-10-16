// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A retired library is closed once nothing uses it. Its own binary: it
//! measures `hello_library`'s first mapping, which a second loader
//! anywhere in the process would answer for instead.

#![cfg(feature = "load")]

use std::path::PathBuf;

use guatiao::library::Registry;

fn library_path() -> PathBuf {
    let exe = std::env::current_exe().expect("a test binary knows its own path");
    let deps = exe.parent().expect("the test binary is in a directory");
    let name = format!(
        "{}hello_library{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    [
        deps.to_path_buf(),
        deps.parent().map(PathBuf::from).unwrap_or_default(),
    ]
    .into_iter()
    .map(|dir| dir.join(&name))
    .find(|p| p.is_file())
    .expect("cargo built the dev-dependency cdylib")
}

/// How many times the mapping at `path` has been entered, read through a
/// mapping of this test's own that is closed before it returns.
fn entry_calls(path: &std::path::Path) -> u64 {
    // SAFETY: the file is a library this test just had the loader map,
    // so mapping it again runs no initialiser that has not already run;
    // the symbol is this example's own and takes no arguments.
    unsafe {
        let library = libloading::Library::new(path).expect("it is already mapped");
        let count = library
            .get::<unsafe extern "C" fn() -> u64>(b"hello_library_entry_calls\0")
            .expect("the example exports its entry counter");
        let n = count();
        drop(library);
        n
    }
}

/// Retired with a handle held, the library stays; once the handle is
/// dropped, `collect` closes it and names it, and a fresh load maps it
/// anew where the loader really unmaps.
#[test]
fn a_retired_library_is_collected_once_its_last_handle_goes() {
    let path = library_path();
    let mut registry = Registry::new("collect-tests", "1.0");
    registry
        .load_file(&path)
        .expect("the example library loads")
        .loaded()
        .expect("it accepts this host");
    assert_eq!(entry_calls(&path), 1, "one entry call, this one");

    let held = registry
        .provider("hello_library_greeter")
        .expect("filed under its id")
        .view()
        .clone();
    registry.retire("hello_library").expect("it is loaded");
    assert!(
        registry.collect().is_empty(),
        "a handle holds the lease, so nothing is closed"
    );
    assert!(held.vtable_size() > 0, "and the handle still reads");

    drop(held);
    assert_eq!(registry.collect(), ["hello_library"]);
    assert!(registry.collect().is_empty(), "closed once");

    registry
        .load_file(&path)
        .expect("it loads again")
        .loaded()
        .expect("it accepts this host");
    let after = entry_calls(&path);
    println!("entry calls after a collect and a fresh load: {after}");
    if cfg!(target_os = "macos") {
        // macOS keeps a closed image mapped, its statics with it.
        return;
    }
    assert_eq!(
        after, 1,
        "the image went away, so its statics started again"
    );
}
