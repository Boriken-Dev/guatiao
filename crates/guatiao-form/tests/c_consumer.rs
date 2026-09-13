// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A C program that checks, lays out and evaluates a form.
//!
//! It compiles C against the committed header, links the artifact, and runs
//! it — the only way to find out whether the exported surface is usable by
//! the audience it exists for.
//!
//! # Skipping
//!
//! With no C compiler, or with the shared libraries not built, this prints
//! a stated skip and passes: a missing toolchain is an environment fact.
//! The header-drift check never skips.

use std::path::PathBuf;
use std::process::Command;

/// The C compilers worth trying, in order.
fn find_compiler() -> Option<PathBuf> {
    for name in ["clang", r"C:\Program Files\LLVM\bin\clang.exe", "cc", "gcc"] {
        let path = PathBuf::from(name);
        if Command::new(&path)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
        {
            return Some(path);
        }
    }
    None
}

/// The directory holding a built shared library, and that library.
///
/// `cargo test` does not build a cdylib for the package under test, so
/// this answers `None` rather than panicking and the caller states the
/// skip.
fn shared_library(stem: &str) -> Option<(PathBuf, PathBuf)> {
    let exe = std::env::current_exe().ok()?;
    let deps = exe.parent()?;
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
            return Some((dir, candidate));
        }
    }
    None
}

/// **The committed header is what this build rendered.** Never skips.
#[test]
#[cfg(feature = "c-header")]
fn the_committed_header_is_what_this_build_rendered() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let committed = std::fs::read_to_string(manifest.join("include/guatiao_form.h"))
        .expect("the header is committed");
    let fresh = std::fs::read_to_string(env!("GUATIAO_FORM_GENERATED_HEADER"))
        .expect("build.rs rendered one");

    assert_eq!(
        first_difference(&committed, &fresh),
        None,
        "the committed header is not what this build renders. Regenerate it:\n  \
         GUATIAO_WRITE_HEADER=1 cargo build -p guatiao-form --features c-header"
    );
}

#[cfg(feature = "c-header")]
fn first_difference(a: &str, b: &str) -> Option<String> {
    for (n, (x, y)) in a.lines().zip(b.lines()).enumerate() {
        if x != y {
            return Some(format!("line {}: committed {x:?} vs fresh {y:?}", n + 1));
        }
    }
    (a.lines().count() != b.lines().count()).then(|| {
        format!(
            "a different number of lines: committed {}, fresh {}",
            a.lines().count(),
            b.lines().count()
        )
    })
}

/// A C program checks, lays out and evaluates a form.
#[test]
fn a_c_program_checks_lays_out_and_evaluates_a_form() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source = manifest.join("tests/c_consumer/form_consumer.c");
    assert!(
        source.exists(),
        "the C source is missing: {}",
        source.display()
    );

    let Some(cc) = find_compiler() else {
        println!(
            "skipped: no C compiler found (tried clang, the default LLVM install path, cc and gcc)."
        );
        return;
    };
    // Two libraries: the value model's and this crate's. The header
    // includes `guatiao.h`, so both include directories are needed too.
    let (Some((dir, ours)), Some((_, core))) =
        (shared_library("guatiao_form"), shared_library("guatiao"))
    else {
        println!(
            "skipped: the shared libraries are not beside this test binary. \
             `cargo test` does not build a cdylib for the package under test; \
             run `cargo build --workspace --all-features` first."
        );
        return;
    };

    let out = dir.join(if cfg!(windows) {
        "guatiao_form_consumer.exe"
    } else {
        "guatiao_form_consumer"
    });

    let mut compile = Command::new(&cc);
    compile
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror"])
        .arg("-I")
        .arg(manifest.join("include"))
        .arg("-I")
        .arg(manifest.join("../guatiao/include"))
        .arg("-o")
        .arg(&out)
        .arg(&source);
    if cfg!(windows) {
        compile
            .arg(ours.with_extension("dll.lib"))
            .arg(core.with_extension("dll.lib"));
    } else {
        compile
            .arg(format!("-L{}", dir.display()))
            .arg("-lguatiao_form")
            .arg("-lguatiao")
            .arg("-Wl,-rpath,$ORIGIN");
    }

    let compiled = compile.output().expect("the compiler runs");
    assert!(
        compiled.status.success(),
        "the C consumer did not compile or link.\nIf the compile succeeded and \
         the LINK failed, a symbol is missing from the artifact — which is the \
         thing this test exists to catch.\nstderr:\n{}",
        String::from_utf8_lossy(&compiled.stderr)
    );

    let run = Command::new(&out)
        .current_dir(&dir)
        .output()
        .expect("the consumer was just built");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success(),
        "the C consumer failed its own checks.\nstdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        stdout.contains("all checks passed"),
        "the consumer exited 0 without reporting success.\nstdout:\n{stdout}"
    );
    println!("{}", stdout.trim());
}
