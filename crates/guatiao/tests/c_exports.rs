// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The exported functions, called from a C program that LINKS them.
//!
//! `c_consumer.rs` proves the header is readable with nothing linked,
//! which is the headline claim. This proves the other half: that the
//! functions a consumer has to link are actually in an artifact's export
//! table and actually callable across the boundary.
//!
//! It is a different axis from every other test here. `library_load`
//! exercises descriptors, an entry symbol and a loader; this exercises
//! none of them — just the C ABI, from C.
//!
//! # What it links
//!
//! `guatiao` itself. The crate is `crate-type = ["rlib", "cdylib"]`, so
//! one package is both the Rust library and the artifact a C consumer
//! links; an rlib exports nothing, and the cdylib is where the
//! `#[unsafe(no_mangle)]` functions actually land.
//!
//! **`cargo test` does not build a cdylib for the package under test**,
//! only for a dev-dependency, so this needs a `cargo build` first and
//! says so when the artifact is missing. CI does the build.
//!
//! # Skipping
//!
//! With no C compiler this prints a stated skip and passes: a missing
//! toolchain is an environment fact, and failing over it would make the
//! suite red on machines where nothing is wrong. The header-drift check
//! in `c_consumer.rs` is the one that never skips.

use std::path::PathBuf;
use std::process::Command;

/// The C compilers worth trying, in order. See `c_consumer.rs`.
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

/// The directory holding the built `guatiao` shared library.
///
/// Found from this test binary's own path for the reason `library_load`
/// explains: an integration test gets no `OUT_DIR`, and shelling out to
/// cargo from inside a cargo run fights the same target-directory lock.
/// The cdylib is built by `cargo build`, not by `cargo test`, so this
/// answers `None` rather than panicking and the caller states the skip.
fn shared_library() -> Option<(PathBuf, PathBuf)> {
    let exe = std::env::current_exe().expect("a test binary knows its own path");
    let deps = exe.parent().expect("the test binary is in a directory");
    let name = format!(
        "{}guatiao{}",
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

#[test]
fn a_c_program_builds_and_frees_a_tree_through_the_exports() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source = manifest.join("tests/c_consumer/exports_consumer.c");
    let include = manifest.join("include");
    assert!(
        source.exists(),
        "the C source is missing: {}",
        source.display()
    );

    let Some(cc) = find_compiler() else {
        println!(
            "skipped: no C compiler found (tried clang, the default LLVM \
             install path, cc and gcc). This is what proves the exported \
             symbols are callable from C, so it is worth having on at least \
             one machine."
        );
        return;
    };

    let Some((dir, dll)) = shared_library() else {
        println!(
            "skipped: the guatiao shared library is not beside this test \
             binary. `cargo test` does not build a cdylib for the package \
             under test; run `cargo build -p guatiao --all-features` first, \
             which is what CI does."
        );
        return;
    };

    // Built INTO the directory holding the library, so the loader finds it
    // beside the executable and no PATH or rpath has to be arranged.
    let out = dir.join(if cfg!(windows) {
        "guatiao_exports_consumer.exe"
    } else {
        "guatiao_exports_consumer"
    });

    let mut compile = Command::new(&cc);
    compile
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror"])
        .arg("-I")
        .arg(&include)
        .arg("-o")
        .arg(&out)
        .arg(&source);

    if cfg!(windows) {
        // Link against the import library cargo emits beside the DLL.
        let implib = dll.with_extension("dll.lib");
        assert!(
            implib.is_file(),
            "no import library beside {}: expected {}",
            dll.display(),
            implib.display()
        );
        compile.arg(&implib);
    } else {
        // `-L` so the linker finds it now, and an rpath of `$ORIGIN` so
        // the program finds it at run time without an environment
        // variable, which is what makes this runnable from anywhere.
        compile
            .arg(format!("-L{}", dir.display()))
            .arg("-lguatiao")
            .arg("-Wl,-rpath,$ORIGIN");
    }

    let compiled = compile
        .output()
        .expect("the compiler answered --version, so it should run");
    assert!(
        compiled.status.success(),
        "the C consumer did not compile or link against the exported \
         symbols.\nIf the compile succeeded and the LINK failed, a symbol \
         is missing from the artifact's export table — which is the thing \
         this test exists to catch.\nstderr:\n{}",
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
        String::from_utf8_lossy(&run.stderr),
    );
    assert!(
        stdout.contains("all checks passed"),
        "the consumer exited 0 without reporting success, which means it did \
         not reach the end.\nstdout:\n{stdout}"
    );
    println!("{}", stdout.trim());
}
