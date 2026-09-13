// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Compiles and runs `c_consumer/consumer.c` against the committed header.
//!
//! # Why a C compiler and not another Rust test
//!
//! Rust asserting its own layout proves nothing about the header, and the
//! header is what a foreign consumer compiles against. The two come from
//! different machinery — one from `size_of`, the other from a generator —
//! so only a C compiler can tell you they agree.
//!
//! It also catches the class a byte-comparison of the header cannot: a
//! header that no C compiler will accept byte-compares perfectly against
//! the last equally broken one. This crate's generator emitted exactly
//! that before the ordering fix, and this is the test that would have
//! caught it.
//!
//! # Skipping
//!
//! With no C compiler the COMPILE test prints a stated skip and passes: a
//! missing toolchain is an environment fact, and failing over it would
//! make the suite red on machines where nothing is wrong.
//!
//! The header-drift test below never skips. `build.rs` renders the header
//! on every build, so the comparison needs nothing installed — and a
//! check that can skip is off on exactly the machine that needed it.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The C compilers worth trying, in order.
///
/// clang first, and not by taste: on the machine this was written for it
/// works from a plain shell with no environment setup, while `cl.exe` is
/// not on `PATH` and needs a `vcvarsall` dance first.
fn find_compiler() -> Option<(PathBuf, Kind)> {
    let candidates: [(&str, Kind); 4] = [
        ("clang", Kind::Posix),
        (r"C:\Program Files\LLVM\bin\clang.exe", Kind::Posix),
        ("cc", Kind::Posix),
        ("gcc", Kind::Posix),
    ];
    for (name, kind) in candidates {
        let path = PathBuf::from(name);
        // A bare name is looked up on PATH by the OS; an absolute path we
        // check ourselves so a missing install is not an exec failure.
        if path.is_absolute() && !path.exists() {
            continue;
        }
        if Command::new(&path).arg("--version").output().is_ok() {
            return Some((path, kind));
        }
    }
    None
}

#[derive(Clone, Copy)]
enum Kind {
    Posix,
}

#[test]
fn the_c_consumer_reads_a_literal_tree_with_nothing_linked() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source = manifest.join("tests/c_consumer/consumer.c");
    let include = manifest.join("include");
    assert!(
        source.exists(),
        "the C source is missing: {}",
        source.display()
    );

    let Some((cc, kind)) = find_compiler() else {
        println!(
            "skipped: no C compiler found (tried clang, the default LLVM \
             install path, cc and gcc). The C consumer is what proves the \
             generated header is usable from C, so this is worth having on \
             at least one machine."
        );
        return;
    };

    // Output beside the other test artefacts rather than in the source
    // tree: a test that writes into the repository makes the working tree
    // dirty for everyone who runs it.
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(if cfg!(windows) {
        "guatiao_c_consumer.exe"
    } else {
        "guatiao_c_consumer"
    });

    let Kind::Posix = kind;
    let compile = Command::new(&cc)
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror"])
        .arg("-I")
        .arg(&include)
        .arg("-o")
        .arg(&out)
        .arg(&source)
        .output()
        .expect("the compiler answered --version, so it should run");

    assert!(
        compile.status.success(),
        "the generated header did not compile as C.\n\
         This is the failure a byte-comparison of the header cannot catch: a \
         header no compiler accepts compares perfectly against the last \
         equally broken one.\n\
         command: {} -std=c11 -Wall -Wextra -Werror -I {} -o {} {}\n\
         stdout:\n{}\nstderr:\n{}",
        cc.display(),
        include.display(),
        out.display(),
        source.display(),
        String::from_utf8_lossy(&compile.stdout),
        String::from_utf8_lossy(&compile.stderr),
    );

    let run = Command::new(&out)
        .output()
        .expect("the consumer was just built");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success(),
        "the C consumer failed its own checks.\nstdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stderr),
    );
    assert!(
        stdout.contains("all checks passed"),
        "the consumer exited 0 without reporting success, which means it did \
         not reach the end.\nstdout:\n{stdout}"
    );
    println!("{}", stdout.trim());
}

/// The committed header is what this build rendered.
///
/// Without this, an edit to the Rust that nobody regenerated ships a
/// header describing a different ABI — and the consumer that finds out is
/// somebody else's.
///
/// `build.rs` renders into `OUT_DIR` on every build and hands the path
/// over in `GUATIAO_GENERATED_HEADER`, so this compares two files and has
/// no skip in it. A check that can skip is a check that is off on exactly
/// the machine that needed it.
// Gated on the feature because `build.rs` only renders under it, so
// `GUATIAO_GENERATED_HEADER` only exists there. Not the same kind of
// conditional as a tool on PATH: CI runs `--all-features`, so this
// runs on every push rather than on whichever machine happened to
// have cbindgen installed.
#[cfg(feature = "c-header")]
#[test]
fn the_committed_header_is_what_this_build_rendered() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let committed = manifest.join("include/guatiao.h");

    let text = std::fs::read_to_string(&committed).expect("the header is committed");
    assert!(
        !text.contains('\r'),
        "the committed header must be LF-only: it is a generated artefact, and \
         line endings that depend on where it was generated make every \
         regeneration a diff"
    );

    let rendered = env!("GUATIAO_GENERATED_HEADER");
    let fresh = std::fs::read_to_string(rendered).expect("build.rs rendered the header");

    assert_eq!(
        first_difference(&text, &fresh),
        None,
        "the committed header is not what this build renders. Regenerate it:\n  \
         GUATIAO_WRITE_HEADER=1 cargo build -p guatiao --features c-header"
    );
    assert_eq!(
        text.len(),
        fresh.len(),
        "the two agree line by line but differ in length, so one ends early"
    );
}

/// The first line where two texts differ, as a message, or `None`.
///
/// A whole-file diff in an assertion message is unreadable; the line that
/// first disagrees is what tells you what changed.
fn first_difference(a: &str, b: &str) -> Option<String> {
    for (n, (la, lb)) in a.lines().zip(b.lines()).enumerate() {
        if la != lb {
            return Some(format!("line {}: committed {la:?} vs fresh {lb:?}", n + 1));
        }
    }
    let (na, nb) = (a.lines().count(), b.lines().count());
    if na != nb {
        return Some(format!("committed has {na} lines, fresh has {nb}"));
    }
    None
}

/// A guard against the scan looking in the wrong place.
#[test]
fn the_c_sources_this_test_depends_on_exist() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    for rel in [
        "tests/c_consumer/consumer.c",
        "include/guatiao.h",
        "cbindgen.toml",
    ] {
        assert!(
            manifest.join(rel).exists(),
            "{rel} is missing, so the tests above would skip or pass by seeing nothing"
        );
    }
}
