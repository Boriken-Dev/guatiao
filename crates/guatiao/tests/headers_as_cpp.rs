// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The three committed C headers, compiled as C++.
//!
//! C++ refuses things C accepts (implicit `void *` conversions, designated
//! initialisers before C++20, enum conversions), so a header that passes
//! the C consumers can still break every C++ caller. This compiles
//! `guatiao.h`, `guatiao_serde.h` and `guatiao_intake.h` each alone and
//! all three together as C++11, C++17 and C++20 with warnings as errors,
//! then compiles, links and runs `cpp_consumer/all_three.cpp`, which
//! calls one function from each.
//!
//! # The compiler
//!
//! `GUATIAO_CXX` names one, and then it must work. Without it the first
//! of clang++, the default LLVM install path, c++ and g++ that answers
//! `--version` is used. A driver named `cl` or `clang-cl` takes MSVC
//! flags (`/W4 /WX`) and needs the Visual Studio environment; `cl` has no
//! C++11 mode, so its oldest leg is C++14.
//!
//! # Skipping
//!
//! With no C++ compiler, or with the shared libraries not built (`cargo
//! test` builds no cdylib for the package under test; run `cargo build
//! --workspace --all-features` first), a test prints a stated skip and
//! passes: a missing toolchain is an environment fact.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// How a driver spells its options.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Driver {
    /// g++, clang++, Apple clang.
    Gnu,
    /// clang-cl: MSVC options, clang's language modes.
    ClangCl,
    /// cl.exe.
    Msvc,
}

impl Driver {
    fn of(path: &Path) -> Self {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        match stem.as_str() {
            "cl" => Driver::Msvc,
            "clang-cl" => Driver::ClangCl,
            _ => Driver::Gnu,
        }
    }

    /// The language mode for a standard named `11`, `17` or `20`.
    fn standard(self, year: u32) -> String {
        match self {
            Driver::Gnu => format!("-std=c++{year}"),
            Driver::ClangCl => format!("/clang:-std=c++{year}"),
            Driver::Msvc => format!("/std:c++{}", year.max(14)),
        }
    }

    fn warnings(self) -> &'static [&'static str] {
        match self {
            Driver::Gnu => &["-Wall", "-Wextra", "-Werror"],
            Driver::ClangCl | Driver::Msvc => &["/nologo", "/W4", "/WX", "/EHsc"],
        }
    }

    fn define(self, name: &str) -> String {
        match self {
            Driver::Gnu => format!("-D{name}"),
            _ => format!("/D{name}"),
        }
    }

    fn include(self, dir: &Path) -> OsString {
        let mut arg = OsString::from(if self == Driver::Gnu { "-I" } else { "/I" });
        arg.push(dir);
        arg
    }
}

const STANDARDS: [u32; 3] = [11, 17, 20];

/// The modes a run used, as the driver spells them.
fn modes(driver: Driver) -> String {
    STANDARDS.map(|year| driver.standard(year)).join(" ")
}

fn find_compiler() -> Option<(PathBuf, Driver)> {
    if let Some(named) = std::env::var_os("GUATIAO_CXX") {
        let path = PathBuf::from(named);
        let driver = Driver::of(&path);
        // cl has no `--version`; running it with no input prints its banner.
        let probe = match driver {
            Driver::Msvc => Command::new(&path).output(),
            _ => Command::new(&path).arg("--version").output(),
        };
        assert!(
            probe.is_ok(),
            "GUATIAO_CXX names {}, which does not run",
            path.display()
        );
        return Some((path, driver));
    }
    for name in [
        "clang++",
        r"C:\Program Files\LLVM\bin\clang++.exe",
        "c++",
        "g++",
    ] {
        let path = PathBuf::from(name);
        if Command::new(&path)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
        {
            return Some((path, Driver::Gnu));
        }
    }
    None
}

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the crate sits in the workspace's crates directory")
        .to_path_buf()
}

fn include_dirs() -> [PathBuf; 3] {
    let crates = crates_dir();
    [
        crates.join("guatiao/include"),
        crates.join("guatiao-serde/include"),
        crates.join("guatiao-intake/include"),
    ]
}

/// The options every compile here shares: the mode, warnings as errors,
/// serde's TOML and YAML declarations on (they are guarded), and the
/// three include directories.
fn common_args(cmd: &mut Command, driver: Driver, year: u32) {
    cmd.arg(driver.standard(year))
        .args(driver.warnings())
        .arg(driver.define("GUATIAO_SERDE_TOML"))
        .arg(driver.define("GUATIAO_SERDE_YAML"));
    for dir in include_dirs() {
        cmd.arg(driver.include(&dir));
    }
}

fn skip_no_compiler() {
    println!(
        "skipped: no C++ compiler found (GUATIAO_CXX is unset; tried clang++, \
         the default LLVM install path, c++ and g++). This is the test that \
         proves the C headers are usable from C++."
    );
}

#[test]
fn every_header_compiles_as_cpp_alone_and_together() {
    let Some((cxx, driver)) = find_compiler() else {
        skip_no_compiler();
        return;
    };

    let work = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("headers_as_cpp");
    std::fs::create_dir_all(&work).expect("the test's scratch directory");

    let units: [(&str, &[&str]); 4] = [
        ("guatiao", &["guatiao.h"]),
        ("guatiao_serde", &["guatiao_serde.h"]),
        ("guatiao_intake", &["guatiao_intake.h"]),
        (
            "all_three",
            &["guatiao.h", "guatiao_serde.h", "guatiao_intake.h"],
        ),
    ];

    let mut failures = Vec::new();
    for (stem, headers) in units {
        let source = work.join(format!("{stem}.cpp"));
        let text: String = headers
            .iter()
            .map(|h| format!("#include \"{h}\"\n"))
            .collect();
        std::fs::write(&source, text).expect("write the translation unit");

        for year in STANDARDS {
            let object = work.join(format!("{stem}_{year}.o"));
            let mut cmd = Command::new(&cxx);
            common_args(&mut cmd, driver, year);
            match driver {
                Driver::Gnu => cmd.arg("-c").arg(&source).arg("-o").arg(&object),
                _ => {
                    let mut out = OsString::from("/Fo");
                    out.push(&object);
                    cmd.arg("/c").arg(&source).arg(out)
                }
            };
            let compiled = cmd.current_dir(&work).output().expect("the compiler runs");
            if !compiled.status.success() {
                failures.push(format!(
                    "{stem}.cpp under {}:\n{}{}",
                    driver.standard(year),
                    String::from_utf8_lossy(&compiled.stdout),
                    String::from_utf8_lossy(&compiled.stderr)
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "a committed header does not compile as C++ with {}. Fix it through \
         the crate's cbindgen.toml or the Rust declarations and regenerate; \
         never by hand.\n\n{}",
        cxx.display(),
        failures.join("\n")
    );
    println!(
        "{} translation units compiled under {} with {}",
        units.len(),
        modes(driver),
        cxx.display()
    );
}

/// The directory holding a built shared library, and that library.
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

#[test]
fn a_cpp_program_calls_each_header_and_links() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/cpp_consumer/all_three.cpp");
    assert!(
        source.exists(),
        "the C++ source is missing: {}",
        source.display()
    );

    let Some((cxx, driver)) = find_compiler() else {
        skip_no_compiler();
        return;
    };
    let (Some((dir, core)), Some((_, serde)), Some((_, intake))) = (
        shared_library("guatiao"),
        shared_library("guatiao_serde"),
        shared_library("guatiao_intake"),
    ) else {
        println!(
            "skipped: the three shared libraries are not beside this test \
             binary. `cargo test` does not build a cdylib for the package \
             under test; run `cargo build --workspace --all-features` first."
        );
        return;
    };

    // Objects go to a directory of their own; the program goes beside the
    // libraries, where Windows finds them at run time.
    let work = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("headers_as_cpp_link");
    std::fs::create_dir_all(&work).expect("the test's scratch directory");

    for year in STANDARDS {
        let program = dir.join(format!(
            "guatiao_cpp_all_three_{year}{}",
            std::env::consts::EXE_SUFFIX
        ));
        let mut cmd = Command::new(&cxx);
        common_args(&mut cmd, driver, year);
        match driver {
            Driver::Gnu => {
                cmd.arg(&source).arg("-o").arg(&program);
                if cfg!(windows) {
                    for lib in [&serde, &intake, &core] {
                        cmd.arg(lib.with_extension("dll.lib"));
                    }
                } else {
                    cmd.arg(format!("-L{}", dir.display()))
                        .args(["-lguatiao_serde", "-lguatiao_intake", "-lguatiao"])
                        .arg(if cfg!(target_os = "macos") {
                            "-Wl,-rpath,@loader_path"
                        } else {
                            "-Wl,-rpath,$ORIGIN"
                        });
                }
            }
            _ => {
                let mut exe = OsString::from("/Fe");
                exe.push(&program);
                cmd.arg(&source).arg(exe);
                for lib in [&serde, &intake, &core] {
                    cmd.arg(lib.with_extension("dll.lib"));
                }
            }
        }

        let compiled = cmd.current_dir(&work).output().expect("the compiler runs");
        assert!(
            compiled.status.success(),
            "the C++ program did not compile or link under {} with {}.\n\
             If the compile succeeded and the LINK failed, a symbol is missing \
             from an artifact.\nstdout:\n{}\nstderr:\n{}",
            driver.standard(year),
            cxx.display(),
            String::from_utf8_lossy(&compiled.stdout),
            String::from_utf8_lossy(&compiled.stderr)
        );

        let run = Command::new(&program)
            .current_dir(&dir)
            .output()
            .expect("the program was just built");
        let stdout = String::from_utf8_lossy(&run.stdout);
        assert!(
            run.status.success() && stdout.contains("all checks passed"),
            "the C++ program built under {} failed its own checks.\n\
             stdout:\n{stdout}\nstderr:\n{}",
            driver.standard(year),
            String::from_utf8_lossy(&run.stderr)
        );
    }
    println!(
        "linked and ran under {} with {}",
        modes(driver),
        cxx.display()
    );
}

/// A guard against the tests above passing by finding nothing.
#[test]
fn the_sources_this_test_depends_on_exist() {
    for dir in include_dirs() {
        assert!(dir.is_dir(), "{} is missing", dir.display());
    }
    let crates = crates_dir();
    for rel in [
        "guatiao/include/guatiao.h",
        "guatiao-serde/include/guatiao_serde.h",
        "guatiao-intake/include/guatiao_intake.h",
        "guatiao/tests/cpp_consumer/all_three.cpp",
    ] {
        assert!(crates.join(rel).is_file(), "{rel} is missing");
    }
}
