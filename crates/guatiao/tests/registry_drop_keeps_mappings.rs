// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A registry gives up its mappings without closing them: dropped, or
//! passing over a file whose entry point already ran. A file that is a
//! second handle to an image it already holds is the exception, and is
//! closed.
//!
//! A test binary of its own, because each is measured on a library's
//! first mapping: a second loader in the same process would hold the
//! image open and make "still mapped" the answer whatever the registry
//! did. Each test maps copies of its own, in a directory of its own.

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
        offer
            .boxed()
            .expect("the default greeter is its own instance")
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

/// A directory of this process's own beside the build, named `<name>-<pid>`.
///
/// Those of earlier runs are removed first: a library a run kept mapped
/// cannot be deleted on Windows while that run is alive, so each run
/// clears what the ones before it left.
fn scratch(name: &str) -> PathBuf {
    let source = library_path("hello_library");
    let parent = source.parent().expect("the library is in a directory");
    let ours = format!("{name}-{}", std::process::id());
    if let Ok(entries) = std::fs::read_dir(parent) {
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let file_name = file_name.to_string_lossy();
            if file_name.starts_with(&format!("{name}-")) && file_name != ours {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    let dir = parent.join(ours);
    std::fs::create_dir_all(&dir).expect("a scratch directory beside the build");
    dir
}

/// The same library at a second path is passed over by its key after its
/// entry point ran, and its image stays mapped: the counter reads that one
/// entry call rather than a fresh image's zero.
#[test]
fn a_library_skipped_by_key_keeps_its_mapping() {
    let source = library_path("hello_library");
    let dir = scratch("skip-by-key");
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
    drop(registry);
    // Works where a mapped file can be unlinked; on Windows the next run
    // removes it.
    let _ = std::fs::remove_dir_all(&dir);
}

/// A hard link to a loaded library, passed over by its key, leaves no
/// second reference to the image behind: on Linux the loader answers the
/// link with the image it already mapped, and once the library is
/// unloaded a fresh load enters it once. macOS never unmaps a closed
/// library, so there is nothing to measure there.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_hard_link_passed_over_does_not_pin_the_image() {
    let source = library_path("hello_library");
    let dir = scratch("hard-link");
    let first = dir.join(source.file_name().expect("a file name"));
    let link = dir.join(format!(
        "{}linked_hello{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    std::fs::copy(&source, &first).expect("a copy of its own");
    std::fs::hard_link(&first, &link).expect("a hard link to the copy");

    let mut registry = Registry::new("link-tests", "1.0");
    assert!(matches!(
        registry.load_file(&first).expect("the copy loads"),
        Loading::Loaded(_)
    ));
    let skipped = registry
        .load_file(&link)
        .expect("the link is an answer, not an error");
    assert!(matches!(
        skipped.skipped(),
        Some(Skipped::AlreadyLoaded { .. })
    ));
    // SAFETY: nothing was taken from the library: no tree, no table, no
    // instance, and no handle of this test's own is open.
    unsafe { registry.unload("hello_library") }.expect("nothing is outstanding");

    assert!(matches!(
        registry.load_file(&first).expect("it loads again"),
        Loading::Loaded(_)
    ));
    assert_eq!(
        entry_calls(&first),
        1,
        "the image went away at the unload, so its statics started again"
    );
    drop(registry);
    // Works where a mapped file can be unlinked; on Windows the next run
    // removes it.
    let _ = std::fs::remove_dir_all(&dir);
}
