// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A source written in Rust and exported with `guatiao::source!`
//! (`examples/source_library`), added by file from a Rust host and from a
//! host written in C.

#![cfg(all(feature = "provider", feature = "load"))]

use std::path::PathBuf;
use std::process::Command;

#[path = "support/clang_target.rs"]
mod clang_target;

use greeter_kind::Greeter;
use guatiao::library::{Registry, SourceError};
use guatiao::value::convert::TryAsRef;
use guatiao::{Map, Status};

/// A library built beside this test binary, as a dev-dependency.
fn beside(base: &str) -> Option<(PathBuf, PathBuf)> {
    let exe = std::env::current_exe().ok()?;
    let deps = exe.parent()?;
    let name = format!(
        "{}{base}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    [deps.to_path_buf(), deps.parent()?.to_path_buf()]
        .into_iter()
        .map(|d| (d.join(&name), d))
        .find(|(p, _)| p.is_file())
        .map(|(p, d)| (d, p))
}

fn config(greetings: &[(&str, &str)]) -> Map {
    let mut words = Map::new();
    for (id, word) in greetings {
        words.set(id, *word);
    }
    let mut config = Map::new();
    config.set("greetings", words);
    config
}

#[test]
fn a_rust_host_adds_a_source_written_in_rust() {
    let (dir, source) = beside("source_library").expect("built as a dev-dependency");
    let mut scanner = Registry::new("scanner", "1.0");
    let report = guatiao::library::scan_dir(&mut scanner, &dir).expect("a readable directory");
    assert!(
        report.skipped.iter().any(|(path, why)| path == &source
            && matches!(why, guatiao::library::Skipped::Source { serves } if serves == "greeters")),
        "a scan names it a source and never loads it: {report:?}"
    );

    let mut registry = Registry::new("rust-host", "1.0");
    registry
        .add_source_file(&source, &config(&[("hi", "hello"), ("yo", "hey")]))
        .expect("the source starts");
    let changed = registry.refresh();
    assert!(changed.refused.is_empty(), "{changed:?}");
    let mut ids: Vec<&str> = changed.loaded.iter().map(|s| s.id.as_str()).collect();
    ids.sort();
    assert_eq!(ids, ["hi", "yo"]);

    let greeter = registry
        .offer::<dyn Greeter>("yo_greeter")
        .expect("served")
        .expect("as the trait")
        .shared()
        .expect("an instance");
    let answer = greeter.greet("bo").expect("greeted");
    assert_eq!(
        answer.get("greeting").and_then(TryAsRef::<str>::try_as_ref),
        Some("hey, bo")
    );
    assert_eq!(greeter.greet("").unwrap_err().message(), "nobody to greet");
    drop(greeter);

    // Retired, then closed once nothing holds it: the parts go back.
    registry.retire("hi").expect("served");
    registry.collect();
    assert!(registry.provider("hi_greeter").is_none());
    assert!(
        registry.refresh().is_empty(),
        "nothing changed on the shelf"
    );
}

#[test]
fn a_source_refusing_its_configuration_is_a_refusal_in_its_words() {
    let (_, source) = beside("source_library").expect("built as a dev-dependency");
    let mut registry = Registry::new("rust-host", "1.0");
    let refused = registry.add_source_file(&source, &Map::new()).unwrap_err();
    let SourceError::Refused {
        status, message, ..
    } = refused
    else {
        panic!("{refused}");
    };
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(message, "`greetings` is missing or not a map");
}

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

#[test]
fn a_c_host_adds_a_source_written_in_rust() {
    let Some(cc) = find_compiler() else {
        println!("skipped: no C compiler found");
        return;
    };
    let Some((dir, core)) = beside("guatiao") else {
        println!("skipped: the guatiao shared library is not beside this test binary");
        return;
    };
    let (_, source) = beside("source_library").expect("built as a dev-dependency");
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = dir.join(if cfg!(windows) {
        "guatiao_source_host.exe"
    } else {
        "guatiao_source_host"
    });
    let mut compile = Command::new(&cc);
    compile.args(clang_target::args(&cc));
    compile
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror"])
        .arg("-I")
        .arg(manifest.join("include"))
        .arg("-I")
        .arg(manifest.join("../../examples/greeter_kind/include"))
        .arg("-o")
        .arg(&out)
        .arg(manifest.join("tests/c_consumer/source_host.c"));
    if cfg!(windows) {
        compile.arg(core.with_extension("dll.lib"));
    } else {
        compile
            .arg(format!("-L{}", dir.display()))
            .arg("-lguatiao")
            .arg("-Wl,-rpath,$ORIGIN");
    }
    let compiled = compile.output().expect("the compiler runs");
    assert!(
        compiled.status.success(),
        "the C host did not build:\n{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let run = Command::new(&out)
        .arg(&source)
        .current_dir(&dir)
        .output()
        .expect("the host was just built");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success() && stdout.contains("all checks passed"),
        "stdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
}
