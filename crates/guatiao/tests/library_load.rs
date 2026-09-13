// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A real library, loaded from disk, offering a real provider.
//!
//! Everything else in this suite runs inside one artifact. This one does
//! not: `hello_library` is built as a `cdylib`, mapped at run time, asked
//! what it offers, and called through a function table — which is the
//! only way to find out whether the envelope actually works.
//!
//! # The claim being tested
//!
//! **A tree built by the library's allocator can be extended and freed by
//! the host, which never names that allocator.** Every owned container
//! records the allocator that made it, so the host's own copy of `set`
//! grows the map through the library's allocator without being told to,
//! and dropping it frees every block back to the same place. The
//! library's counter going back to where it started is that claim,
//! measured.
//!
//! If that is false, the design does not work and nothing else in the
//! envelope matters.

#![cfg(feature = "load")]

use std::path::PathBuf;

use guatiao::ReadValue;
use guatiao::library::{KeyError, Provider, Registry, Skipped, Subject};
use guatiao::schema::read::SchemaRef;
use guatiao::schema::validate_map;
use guatiao::value::status::Status;
use guatiao::value::types::Value;

use hello_library::GreeterVtable;

/// Where cargo put the example library.
///
/// Found from this test binary's own path rather than from an environment
/// variable: an integration test with no build script gets no `OUT_DIR`,
/// and shelling out to `cargo build` from inside a cargo run fights the
/// same target directory lock. The example is a dev-dependency, so it is
/// already built and already beside us by the time this runs.
fn library_path() -> PathBuf {
    let exe = std::env::current_exe().expect("a test binary knows its own path");
    let deps = exe.parent().expect("the test binary is in a directory");
    let name = format!(
        "{}hello_library{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );

    // `deps/` first, which is where a dev-dependency's cdylib lands, then
    // the profile directory beside it.
    for dir in [
        deps.to_path_buf(),
        deps.parent().map(PathBuf::from).unwrap_or_default(),
    ] {
        let candidate = dir.join(&name);
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!(
        "{name} is not beside {} or its parent. It is a dev-dependency of this crate, so \
         `cargo test` should have built it.",
        deps.display()
    );
}

/// Reads a greeter's table the way the crate documents: check the size the
/// library compiled it at against a frozen floor, then project each field
/// through the raw pointer.
///
/// Never builds a `&GreeterVtable`. A reference asserts the whole struct
/// is readable, which is exactly what a library compiled before the newest
/// slot existed does not give you — and the point of the size check is to
/// work with such a library rather than to reject it.
fn read_greeter(
    ptr: *const std::ffi::c_void,
    size: usize,
) -> (
    unsafe extern "C" fn(*mut std::ffi::c_void, *const Value, *mut Value) -> Status,
    Option<unsafe extern "C" fn(*mut std::ffi::c_void) -> i64>,
) {
    assert!(
        size >= GreeterVtable::floor(),
        "a table of {size} bytes is below the greeter floor of {}",
        GreeterVtable::floor()
    );
    let table = ptr as *const GreeterVtable;

    // SAFETY: the size check above established that `greet` is present,
    // and the mapping it lives in is never unloaded.
    let greet = unsafe { std::ptr::addr_of!((*table).greet).read() }
        .expect("a greeter that declares no greet slot is not a greeter");

    // The appended slot, read only when the library was compiled with it.
    let outstanding = if size >= GreeterVtable::outstanding_end() {
        // SAFETY: the guard established the field is present.
        unsafe { std::ptr::addr_of!((*table).outstanding).read() }
    } else {
        None
    };
    (greet, outstanding)
}

#[test]
fn a_library_on_disk_offers_a_provider_a_host_can_use() {
    let mut registry = Registry::new("guatiao-tests", env!("CARGO_PKG_VERSION"));
    let path = library_path();

    let loaded = registry
        .load_file(&path)
        .expect("the example library loads")
        .loaded()
        .expect("it is a library and it did not decline this host");
    assert_eq!(loaded.id, "hello_library");
    assert_eq!(loaded.providers, 2, "a greeter and an almanac");

    // The appended `meta` slot, read out of the library's own image.
    // `ProviderInfo::meta` is null here and `LibraryInfo::meta` is not,
    // so this pins both answers — an absent slot and a present one are
    // different outcomes, not one untested path.
    let meta = loaded.meta.expect("the library declares metadata");
    assert_eq!(
        meta.get("built-with").and_then(Value::as_str),
        Some("hello_library"),
        "a key the host was never told about crosses intact"
    );
    assert_eq!(
        meta.get("greeting-language").and_then(Value::as_str),
        Some("en")
    );
    assert!(
        meta.get("a-key-nobody-declared").is_none(),
        "a key that is not there is absent, not an error"
    );

    let provider = registry
        .provider("hello_library_greeter")
        .expect("the provider it registered");
    assert_eq!(provider.display_name(), "Hello");
    assert!(
        provider.meta().is_none(),
        "this provider declares none, and null is how it says so"
    );
    assert_eq!(provider.from(), path);

    // One provider, two kinds. It answers to each of them and is the same
    // provider both times — which is the whole reason a kind is a list.
    assert_eq!(provider.kinds(), ["greeter", "writer"]);
    assert!(provider.supports("greeter") && provider.supports("writer"));
    assert!(!provider.supports("codec"));
    for kind in ["greeter", "writer"] {
        let mut found = registry.providers(kind);
        assert_eq!(
            found.next().map(Provider::id),
            Some("hello_library_greeter"),
            "it answers to {kind}"
        );
        assert!(found.next().is_none());
    }
    assert_eq!(
        registry.providers("nothing-of-this-kind").count(),
        0,
        "a kind nobody offers is empty rather than an error"
    );

    // And one that serves no kind at all, which no capability question
    // finds and a lookup by name does.
    let almanac = registry
        .provider("hello_library_almanac")
        .expect("a provider with no kind is still a provider");
    assert!(almanac.kinds().is_empty());
    assert!(almanac.config_schema().is_none());
}

/// A provider's version is its library's unless it says otherwise.
#[test]
fn a_provider_versions_with_its_library_or_says_so() {
    let mut registry = Registry::new("guatiao-tests", env!("CARGO_PKG_VERSION"));
    let loaded = registry
        .load_file(&library_path())
        .unwrap()
        .loaded()
        .unwrap();
    let library_version = loaded.version;

    assert_eq!(
        registry
            .provider("hello_library_greeter")
            .map(Provider::version),
        Some(library_version),
        "declaring nothing means moving with the library"
    );
    assert_eq!(
        registry
            .provider("hello_library_almanac")
            .map(Provider::version),
        Some("1.0.0"),
        "and a provider whose contract froze says so"
    );
    assert_ne!(
        library_version, "1.0.0",
        "the test is vacuous if the two agree by accident"
    );
}

/// The same library twice is a SKIP naming where it came from, not an
/// error — because a host's search path and the directory beside its
/// executable are routinely the same place.
#[test]
fn the_same_library_twice_is_skipped_not_refused() {
    let mut registry = Registry::new("guatiao-tests", env!("CARGO_PKG_VERSION"));
    let path = library_path();
    registry.load_file(&path).unwrap().loaded().unwrap();

    let again = registry.load_file(&path).expect("a repeat is not an error");
    match again.skipped() {
        Some(Skipped::AlreadyLoaded { from }) => assert_eq!(from, &path),
        other => panic!("expected an already-loaded skip, got {other:?}"),
    }
    assert_eq!(registry.loaded().len(), 1);
    assert_eq!(
        registry.all().len(),
        2,
        "and nothing was registered a second time"
    );
}

/// The provider declared what configuration it takes, and the host checks
/// a configuration against that declaration having never heard of this
/// library before.
#[test]
fn the_host_validates_a_configuration_against_the_librarys_own_schema() {
    let mut registry = Registry::new("guatiao-tests", env!("CARGO_PKG_VERSION"));
    registry
        .load_file(&library_path())
        .unwrap()
        .loaded()
        .unwrap();
    let provider = registry.provider("hello_library_greeter").unwrap();

    let declared = provider
        .config_schema()
        .expect("the greeter declares a schema");
    let schema = SchemaRef::new(declared).expect("a schema is a map");
    let name = schema.find("name").expect("it declares `name`");
    assert!(name.is_required());
    assert_eq!(name.help(), "Who to greet.");

    let mut config = Value::map();
    config.set("name", Value::string("ana")).unwrap();
    assert_eq!(validate_map(schema, &config), Ok(()));

    let mut wrong = Value::map();
    wrong.set("nonesuch", Value::bool(true)).unwrap();
    assert!(
        validate_map(schema, &wrong).is_err(),
        "a key the schema does not declare is refused"
    );
}

/// **The claim the whole design rests on.**
///
/// The library builds a map through its own allocator and hands it over.
/// The host appends to it with its own copy of `set` — which grows the
/// map through the allocator recorded inside it, the library's, without
/// the host ever naming it — and then frees the whole thing by dropping
/// it. The library's own counter is back where it started.
#[test]
fn a_tree_the_library_built_is_extended_and_freed_by_the_host() {
    let mut registry = Registry::new("guatiao-tests", env!("CARGO_PKG_VERSION"));
    registry
        .load_file(&library_path())
        .unwrap()
        .loaded()
        .unwrap();
    let provider = registry.provider("hello_library_greeter").unwrap();

    let (ptr, size) = provider.vtable();
    let (greet, outstanding) = read_greeter(ptr, size);
    let outstanding = outstanding.expect("this library was built with the appended slot");

    // SAFETY: the slot came from a table whose declared size covers it.
    let before = unsafe { outstanding(provider.ctx()) };

    let mut config = Value::map();
    config.set("name", Value::string("ana")).unwrap();

    let mut out = Value::absent();
    // SAFETY: `config` is a well-formed value, `out` is a writable node,
    // and the table's size check established that `greet` is present. On
    // success `greet`'s contract is that it wrote an owned tree, whose
    // buffers came from an allocator inside a library that is never
    // unloaded -- which is what makes the `drop` at the end of this test
    // sound.
    let status = unsafe { greet(provider.ctx(), &config, &mut out) };
    assert_eq!(status, Status::GUATIAO_OK);

    let mut greeting = out;
    assert_eq!(
        greeting.get("greeting").ok_or_missing().unwrap().try_into(),
        Ok("hello, ana"),
        "the library read the configuration the host built"
    );

    assert!(
        unsafe { outstanding(provider.ctx()) } > before,
        "the library's allocator did the work"
    );

    // THE POINT: the host extends a tree it did not allocate, using its
    // own copy of `set`, and never names the library's allocator.
    greeting
        .set("seen_by", Value::string("the host"))
        .expect("the host can add to a map the library built");
    assert_eq!(
        greeting.get("seen_by").ok_or_missing().unwrap().try_into(),
        Ok("the host")
    );

    drop(greeting);
    // SAFETY: as above.
    assert_eq!(
        unsafe { outstanding(provider.ctx()) },
        before,
        "every block the library's allocator handed out came back, including the one the \
         HOST asked for when it appended a key"
    );
}

/// A library with no entry symbol is not a library, and saying so is an
/// answer rather than a failure — which is what lets a host hand a loader
/// every library in a directory without filtering by name first.
#[test]
fn a_library_that_is_not_one_of_ours_is_reported_rather_than_failing() {
    let mut registry = Registry::new("guatiao-tests", env!("CARGO_PKG_VERSION"));

    // This test binary's own directory holds the libraries cargo built
    // for the run. Any of them that is not our library is a fine stand-in
    // for "some other library that happens to be here".
    let exe = std::env::current_exe().unwrap();
    let deps = exe.parent().unwrap();
    let ours = format!(
        "{}hello_library{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );

    let mut checked = 0usize;
    for entry in std::fs::read_dir(deps).unwrap().flatten() {
        let path = entry.path();
        if path
            .extension()
            .is_none_or(|e| e != std::env::consts::DLL_EXTENSION)
        {
            continue;
        }
        if path.file_name().is_some_and(|n| n == ours.as_str()) {
            continue;
        }
        if let Ok(answer) = registry.load_file(&path) {
            assert_eq!(
                answer.skipped(),
                Some(&Skipped::NoEntrySymbol),
                "{} is not a guatiao library and must not register anything",
                path.display()
            );
            checked += 1;
        }
        if checked >= 1 {
            break;
        }
    }

    // Not an assertion that such a file exists: on a machine where the
    // only library beside the test binary is the library itself, there is
    // nothing to check and nothing wrong with that.
    if checked == 0 {
        eprintln!("no non-library library was beside the test binary; nothing to check");
    }
}

/// A host names what it loads, and by default that name is the id.
#[test]
fn a_host_files_providers_under_a_key_it_chooses() {
    let mut registry = Registry::new("guatiao-tests", env!("CARGO_PKG_VERSION"));
    let path = library_path();
    registry.load_file(&path).unwrap().loaded().unwrap();

    assert_eq!(registry.key_template().as_str(), "%id");
    assert_eq!(registry.library_key_template().as_str(), "%id");
    let provider = registry
        .provider("hello_library_greeter")
        .expect("the default key is `%id`");
    let version = provider.version().to_string();
    assert_eq!(provider.key(), "hello_library_greeter");
    assert_eq!(provider.library(), "hello_library");
    assert_eq!(registry.loaded()[0].key.as_str(), Some("hello_library"));

    assert_eq!(registry.providers_of("hello_library_greeter").count(), 1);
    assert_eq!(
        registry.providers_of("nonesuch").count(),
        0,
        "an id nobody offers is empty rather than an error"
    );

    // A host that wants versions apart says so, and what is already loaded
    // is re-keyed rather than having to be loaded again.
    registry
        .keyed_by("%id@%version")
        .expect("one provider cannot collide with itself");
    let key = format!("hello_library_greeter@{version}");
    assert_eq!(
        registry.provider(&key).map(Provider::id),
        Some("hello_library_greeter")
    );
    assert!(
        registry.provider("hello_library_greeter").is_none(),
        "the old key is not also kept"
    );

    // A template naming something no descriptor has is refused where it is
    // written, not at the next load.
    assert_eq!(
        registry.keyed_by("%id@%revision").unwrap_err(),
        KeyError::UnknownField {
            name: "revision".to_string(),
            subject: Subject::Provider,
        }
    );
}

/// The library key is the other half of the same knob: `%id` means one
/// build of a library at a time, `%id@%version` means as many as are
/// there.
#[test]
fn a_host_decides_how_many_builds_of_one_library_it_will_hold() {
    let path = library_path();

    // Under the default, the second is the same library and is skipped.
    let mut one = Registry::new("guatiao-tests", env!("CARGO_PKG_VERSION"));
    one.load_file(&path).unwrap().loaded().unwrap();
    assert!(matches!(
        one.load_file(&path).unwrap().skipped(),
        Some(Skipped::AlreadyLoaded { .. })
    ));

    // Under `%id@%version` the key carries the version — and the same file
    // twice is still the same file, caught by path before anything loads.
    let mut apart = Registry::new("guatiao-tests", env!("CARGO_PKG_VERSION"));
    apart
        .libraries_keyed_by("%id@%version")
        .expect("an empty registry cannot collide");
    let loaded = apart.load_file(&path).unwrap().loaded().unwrap();
    let expected = format!("hello_library@{}", loaded.version);
    assert_eq!(loaded.key.as_str(), Some(expected.as_str()));
    assert!(matches!(
        apart.load_file(&path).unwrap().skipped(),
        Some(Skipped::AlreadyLoaded { .. })
    ));

    // A library has no display name, so a library template cannot name one.
    assert!(
        Registry::new("guatiao-tests", "1.0")
            .libraries_keyed_by("%id-%name")
            .is_err(),
        "a library has no display name"
    );
}
