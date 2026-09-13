// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Scanning a directory without running what is in it.
//!
//! The directory these tests build is the real thing in miniature: a
//! library, a shared library that is not one, a file that is not an object
//! file at all, and a file with the wrong extension.
//!
//! # What is actually proven, and what is not
//!
//! That nothing ran is hard to observe directly — a well-behaved library
//! that gets mapped by mistake behaves identically to one that never was.
//! So the discriminating case is the **junk file**: bytes that are not an
//! object file, carrying a library's extension.
//!
//! - Read as data, it fails to parse and is reported `NotExaminable`.
//! - Mapped, the operating system refuses it and it would be a
//!   `LoadError::Open` in `failed`.
//!
//! Those are different outcomes, so asserting the first is evidence the
//! scan did not take the second path.

#![cfg(feature = "load")]

use std::path::{Path, PathBuf};

use guatiao::library::{Registry, Skipped, declares_entry_symbol, scan_dir};
use guatiao::schema::SchemaRef;
use guatiao::value::alloc::Allocator;
use guatiao::value::status::Status;
use guatiao::{Alloc, Value};

/// A built artefact beside this test binary.
///
/// Everything named here is a dev-dependency or a proc-macro this crate
/// already uses, so `cargo test` has built it and put it where a test
/// binary can find it — no shelling out to cargo from inside a cargo
/// run, which would fight the same target-directory lock.
fn built(stem: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().expect("a test binary knows its own path");
    let deps = exe.parent().expect("the test binary is in a directory");
    let name = format!(
        "{}{stem}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    for dir in [
        deps.to_path_buf(),
        deps.parent().map(PathBuf::from).unwrap_or_default(),
    ] {
        let candidate = dir.join(&name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// A directory holding the four cases, built fresh for one test.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory under the target dir");
    dir
}

fn dll_name(stem: &str) -> String {
    format!(
        "{}{stem}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    )
}

fn reason_for<'a>(skipped: &'a [(PathBuf, Skipped)], file: &Path) -> Option<&'a Skipped> {
    skipped
        .iter()
        .find(|(p, _)| p.file_name() == file.file_name())
        .map(|(_, r)| r)
}

/// The export table answers the question, and the file never runs.
#[test]
fn a_shared_library_that_is_not_a_plugin_is_read_not_loaded() {
    let Some(library) = built("hello_library") else {
        println!("skipped: the example library is not built beside this test");
        return;
    };
    // The derive crate's own dynamic library: a REAL shared library,
    // exporting real symbols, none of them the entry symbol. Exactly the
    // shape of the thing a scan must pass over without running — and a
    // proc-macro is built by `cargo test` whatever else is, so this case
    // never quietly turns into a skip.
    let Some(not_a_plugin) = built("guatiao_derive") else {
        println!("skipped: the derive crate's library is not beside this test");
        return;
    };

    assert!(
        declares_entry_symbol(&library).expect("the example library parses"),
        "the example library declares the entry symbol"
    );
    assert!(
        !declares_entry_symbol(&not_a_plugin).expect("the library parses"),
        "a real shared library that exports plenty and declares no entry \
         symbol, decided by reading its export table rather than by mapping \
         it"
    );
}

/// A whole directory, sorted into the three outcomes.
#[test]
fn a_scan_reports_every_candidate_it_passed_over() {
    let Some(library) = built("hello_library") else {
        println!("skipped: the example library is not built beside this test");
        return;
    };
    let Some(not_a_plugin) = built("guatiao_derive") else {
        println!("skipped: the derive crate's library is not beside this test");
        return;
    };

    let dir = scratch("scan_reports");
    std::fs::copy(&library, dir.join(dll_name("hello_library"))).expect("copy the library");
    std::fs::copy(&not_a_plugin, dir.join(dll_name("guatiao_derive")))
        .expect("copy the proc-macro library");

    // Not an object file at all. THIS is the discriminating case: read as
    // data it fails to parse; mapped, the OS would refuse it and it would
    // be a load failure instead.
    let junk = dir.join(dll_name("nonsense"));
    std::fs::write(&junk, b"this is not an object file\n").expect("write the junk file");

    // Right bytes, wrong extension: not a candidate at all, so it is not
    // even reported. A scan answers for files it could have loaded.
    let ignored = dir.join("hello_library.notalibrary");
    std::fs::copy(&library, &ignored).expect("copy with the wrong extension");

    let mut registry = Registry::new("guatiao-tests", env!("CARGO_PKG_VERSION"));
    let report = scan_dir(&mut registry, &dir).expect("the directory is readable");

    assert_eq!(report.loaded.len(), 1, "one library loaded: {report:#?}");
    assert_eq!(
        report.loaded[0].file_name(),
        Some(std::ffi::OsStr::new(&dll_name("hello_library")))
    );
    assert!(report.failed.is_empty(), "nothing failed: {report:#?}");

    assert_eq!(
        reason_for(&report.skipped, Path::new(&dll_name("guatiao_derive"))),
        Some(&Skipped::NoEntrySymbol),
        "a real shared library with no entry symbol is REPORTED, not dropped \
         — which is what a person needs when the plugin they expected is \
         missing: {report:#?}"
    );
    assert_eq!(
        reason_for(&report.skipped, &junk),
        Some(&Skipped::NotExaminable),
        "junk bytes fail to PARSE. Had the scan mapped the file instead, the \
         operating system would have refused it and this would be a \
         `LoadError::Open` in `failed` — so this assertion is the evidence \
         that nothing was mapped speculatively: {report:#?}"
    );
    assert!(
        reason_for(&report.skipped, &ignored).is_none(),
        "a file without the platform's library extension is not a candidate, \
         so it is not reported either: {report:#?}"
    );

    // And the library that did load is usable, so the scan produced a
    // registry rather than just a report.
    assert!(registry.provider("hello_library_greeter").is_some());
}

/// An empty directory is an empty report, not an error.
#[test]
fn an_empty_directory_is_not_a_failure() {
    let dir = scratch("scan_empty");
    let mut registry = Registry::new("guatiao-tests", env!("CARGO_PKG_VERSION"));
    let report = scan_dir(&mut registry, &dir).expect("an empty directory is readable");
    assert!(report.loaded.is_empty());
    assert!(report.skipped.is_empty());
    assert!(report.failed.is_empty());

    // A directory that is not there IS an error, because the caller named
    // it and naming something that does not exist is a mistake worth
    // hearing about.
    let missing = dir.join("nowhere");
    assert!(scan_dir(&mut registry, &missing).is_err());
}

/// A schema exported by `export_schema!`, fetched and called.
///
/// The macro's whole claim is that a caller with no Rust can ask a library
/// what one of its types looks like. This is that call, made the way such
/// a caller makes it: look the symbol up by name, hand over an allocator,
/// read the tree that comes back.
///
/// Not a provider's configuration. `Greeting` is an ordinary type the
/// example library can describe — which is what a schema is for, config
/// being one of the things it describes rather than the only one.
#[test]
fn a_macro_exported_schema_is_callable_by_name() {
    let Some(library) = built("hello_library") else {
        println!("skipped: the example library is not built beside this test");
        return;
    };

    type SchemaFn = unsafe extern "C" fn(*const Allocator, *mut Value) -> Status;

    // SAFETY: mapping a library runs its initialisers, and this is the
    // example library the suite builds itself. It is never unloaded.
    let lib = unsafe { libloading::Library::new(&library) }.expect("the example library maps");

    for name in [
        // The default: the calling crate's name is the prefix.
        b"hello_library_greeting_schema\0".as_slice(),
        // A second invocation in the same crate, which is the case that
        // would collide if the macro named its Rust function.
        b"hello_library_ledger_schema\0".as_slice(),
        // The versioned arm. `minor` here because the example is a 0.x
        // crate, where the major is always 0 and separates nothing.
        b"hello_library_v0_1_ledger_schema\0".as_slice(),
    ] {
        // SAFETY: the symbol has the signature the macro emits.
        let f = unsafe { lib.get::<SchemaFn>(name) }
            .unwrap_or_else(|e| panic!("{} is not exported: {e}", String::from_utf8_lossy(name)));

        let mut out = Value::absent();
        // SAFETY: a complete allocator that outlives the tree, and
        // writable storage for one value.
        let status = unsafe { f(Alloc::rust().as_raw(), &mut out) };
        assert_eq!(
            status,
            Status::GUATIAO_OK,
            "{}",
            String::from_utf8_lossy(name)
        );

        let schema = SchemaRef::new(&out).expect("what comes back is a schema");
        assert!(
            schema.options().next().is_some(),
            "{} describes at least one option",
            String::from_utf8_lossy(name)
        );
    }

    // The greeting's own options, so this checks the SCHEMA and not just
    // that something came back.
    // SAFETY: as above.
    let f = unsafe { lib.get::<SchemaFn>(b"hello_library_greeting_schema\0") }.expect("exported");
    let mut out = Value::absent();
    // SAFETY: as above.
    assert_eq!(
        unsafe { f(Alloc::rust().as_raw(), &mut out) },
        Status::GUATIAO_OK
    );
    let schema = SchemaRef::new(&out).expect("a schema");
    let keys: Vec<&str> = schema.options().map(|o| o.key()).collect();
    assert_eq!(
        keys,
        ["greeting", "seen_by"],
        "the fields, in declaration order"
    );
    assert_eq!(
        schema.find("greeting").expect("the option").help(),
        "The text to show.",
        "the doc comment reached a caller that has no Rust"
    );

    // A null out-parameter is refused rather than written through.
    // SAFETY: passing null is the case under test.
    assert_eq!(
        unsafe { f(Alloc::rust().as_raw(), std::ptr::null_mut()) },
        Status::GUATIAO_ERR_NULL
    );

    // Never unloaded: everything it handed over points into its mapping.
    std::mem::forget(lib);
}
