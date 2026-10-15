// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A registry gives up its mappings without closing them: dropped, or
//! passing over a file whose entry point already ran.
//!
//! A test binary of its own, because both are measured on a library's
//! first mapping: a second loader in the same process would hold the
//! image open and make "still mapped" the answer whatever the registry
//! did.

#![cfg(feature = "load")]

use std::path::{Path, PathBuf};

use greeter_kind::Greeter;
use guatiao::library::{Loading, Registry, Skipped};

/// Where cargo put an example library, found from this binary's own path.
fn library_path(basename: &str) -> PathBuf {
    let exe = std::env::current_exe().expect("a test binary knows its own path");
    let deps = exe.parent().expect("the test binary is in a directory");
    let name = format!(
        "{}{basename}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    for dir in [
        deps.to_path_buf(),
        deps.parent().map(PathBuf::from).unwrap_or_default(),
    ] {
        let candidate = dir.join(&name);
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!("{name} is not beside {}", deps.display());
}

/// A provider kept as the trait outlives the registry it came from.
///
/// The registry's drop would otherwise close `derived_greeter`, the only
/// mapping of it in this process, and the call below would jump into
/// unmapped memory.
#[test]
fn a_provider_kept_past_its_registry_still_answers() {
    let greeter: Box<dyn Greeter> = {
        let mut registry = Registry::new("drop-tests", "1.0");
        registry
            .load_file(&library_path("derived_greeter"))
            .expect("the derived library loads")
            .loaded()
            .expect("it accepts this host");
        let offer = registry
            .offers::<dyn Greeter>()
            .find(|o| o.id() == "derived_greeter_hello")
            .expect("the default greeter is offered");
        offer.boxed()
    };
    let answer = greeter.greet("kept").expect("the greeter answers");
    assert!(answer.get("greeting").is_some(), "a greeting came back");
}

/// How many times the mapping at `path` has been entered, read through a
/// handle of this test's own, closed before it returns.
fn entry_calls(path: &Path) -> u64 {
    // SAFETY: the file is a copy of the example library, whose initialisers
    // do nothing; the symbol is its own and takes no arguments.
    unsafe {
        let library = libloading::Library::new(path).expect("the copy maps");
        let count = library
            .get::<unsafe extern "C" fn() -> u64>(b"hello_library_entry_calls\0")
            .expect("the example exports its entry counter");
        let n = count();
        drop(library);
        n
    }
}

/// The same library at a second path is passed over by its key after its
/// entry point ran, and its image stays mapped: the counter reads that one
/// entry call rather than a fresh image's zero.
#[test]
fn a_library_skipped_by_key_keeps_its_mapping() {
    let source = library_path("hello_library");
    let dir = source
        .parent()
        .expect("the library is in a directory")
        .join(format!("skip-by-key-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch directory beside the build");
    let first = dir.join(source.file_name().expect("a file name"));
    let second = dir.join(format!(
        "{}second_hello{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    std::fs::copy(&source, &first).expect("a first copy");
    std::fs::copy(&source, &second).expect("a second copy");

    let mut registry = Registry::new("skip-tests", "1.0");
    assert!(matches!(
        registry.load_file(&first).expect("the first copy loads"),
        Loading::Loaded(_)
    ));
    let skipped = registry
        .load_file(&second)
        .expect("the second copy is an answer, not an error");
    assert!(matches!(
        skipped.skipped(),
        Some(Skipped::AlreadyLoaded { .. })
    ));
    assert_eq!(
        entry_calls(&second),
        1,
        "the skipped copy's entry ran once, and its image is the one still mapped"
    );
}
