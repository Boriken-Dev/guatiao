// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A library written in C, loaded and called as a Rust trait.
//!
//! `tests/c_consumer/greeter_in_c.c` fills the table `greeter_kind.h`
//! renders for `#[guatiao::kind] trait Greeter`, offers it through the
//! envelope, and links nothing. This test compiles it into a shared
//! library, loads it through a registry, and calls it through the proxy
//! the kind attribute generated -- the same one a Rust library's table is
//! called through. A kind that cannot be written in C is not a C ABI;
//! this is the check.

#![cfg(all(feature = "provider", feature = "load"))]

use std::path::{Path, PathBuf};
use std::process::Command;

use greeter_kind::{Greeter, GreeterVtable, Listener};
use guatiao::library::{Kind, Registry};
use guatiao::{Status, Value};

/// The C compilers worth trying, in order (as `c_consumer.rs`).
fn find_compiler() -> Option<PathBuf> {
    for name in ["clang", r"C:\Program Files\LLVM\bin\clang.exe", "cc", "gcc"] {
        let path = PathBuf::from(name);
        if path.is_absolute() && !path.exists() {
            continue;
        }
        if Command::new(&path).arg("--version").output().is_ok() {
            return Some(path);
        }
    }
    None
}

/// Builds the C greeter into a shared library beside the test artefacts,
/// or says why it could not.
fn build_c_greeter(cc: &Path) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source = manifest.join("tests/c_consumer/greeter_in_c.c");
    let guatiao_include = manifest.join("include");
    let kind_include = manifest.join("../../examples/greeter_kind/include");
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "{}c_greeter{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    let compile = Command::new(cc)
        .args(["-shared", "-std=c11", "-Wall", "-Wextra", "-Werror"])
        .arg("-I")
        .arg(&guatiao_include)
        .arg("-I")
        .arg(&kind_include)
        .arg("-o")
        .arg(&out)
        .arg(&source)
        .output()
        .expect("the compiler answered --version, so it should run");
    assert!(
        compile.status.success(),
        "the C greeter did not compile against the rendered kind header.\n\
         stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&compile.stdout),
        String::from_utf8_lossy(&compile.stderr),
    );
    out
}

#[test]
fn a_greeter_written_in_c_is_called_as_the_trait() {
    let Some(cc) = find_compiler() else {
        println!(
            "skipped: no C compiler found (tried clang, the default LLVM install path, \
             cc and gcc). This is the test that proves a kind is a C ABI, so it is \
             worth having on at least one machine."
        );
        return;
    };
    let library = build_c_greeter(&cc);

    let mut registry = Registry::new("c-library-tests", env!("CARGO_PKG_VERSION"));
    let loaded = registry
        .load_file(&library)
        .expect("a shared library the compiler just wrote")
        .loaded()
        .expect("the C entry accepted this host")
        .id;
    assert_eq!(loaded, "c_greeter_library");

    // 1. The C table is an offer of the Rust trait: its hash is the one
    // the header carried, its required slot is filled.
    let offer = registry
        .offer::<dyn Greeter>("c_greeter")
        .expect("filed under its id")
        .expect("a table the header described validates");
    assert_eq!(offer.display_name(), "Greeter, in C");
    assert_eq!(offer.version(), "1.0");
    assert_eq!(offer.remote().size(), std::mem::size_of::<GreeterVtable>());

    // 2. A call crosses into C and back: the answer is a map C built
    // through its own allocator, read here and freed here -- through
    // that allocator, which the value carries.
    let answer = offer.greet("ana").expect("the C greeter answers");
    assert_eq!(
        answer.get("greeting").and_then(Value::as_str),
        Some("hello from C, ana")
    );
    drop(answer);

    // 3. A ProviderError crosses out of C with its message.
    let e = offer.greet("").unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(e.message(), "nobody to greet, says C");

    // 4. The slots C left null are appended ones: the proxy runs the
    // trait's default bodies -- `shout` through C's own `greet`, `start`
    // refusing as a greeter that holds no conversations.
    assert_eq!(offer.shout("bo"), "HELLO FROM C, BO");
    let e = offer
        .start(NoListener.into_object())
        .expect_err("the default body refuses");
    assert_eq!(e.status, Status::GUATIAO_ERR_NULL);

    // 5. The hash C wrote is the literal the attribute wrote, on both
    // sides of the boundary.
    assert_eq!(<dyn Greeter as Kind>::FLOOR_HASH, GreeterVtable::FLOOR_HASH);
}

/// A listener nobody will hear from: what `start`'s default body drops.
struct NoListener;

impl Listener for NoListener {
    fn heard(&mut self, _what: &str) {}
}
