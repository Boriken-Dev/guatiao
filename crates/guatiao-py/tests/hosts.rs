// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The bridge from a Rust host and from a C host: each adds it as a source
//! and calls a greeter written in Python.
//!
//! The bridge is this crate's cdylib, which `cargo test` does not build:
//! `cargo build` first, or these state a skip. The C host also links a
//! guatiao library built with `load`, from `GUATIAO_LIBRARY`.

use std::path::{Path, PathBuf};
use std::process::Command;

use guatiao::library::Registry;
use guatiao::value::convert::TryAsRef;
use guatiao::{List, Map};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn library_name(base: &str) -> String {
    format!(
        "{}{base}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    )
}

/// This crate's cdylib, beside the test binary's directory.
fn bridge() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let debug = exe.parent()?.parent()?;
    let path = debug.join(library_name("guatiao_py"));
    path.is_file().then_some(path)
}

fn text_list(items: &[PathBuf]) -> List {
    let mut list = List::new();
    for item in items {
        list.push(item.to_string_lossy().as_ref());
    }
    list
}

fn config() -> Map {
    let mut config = Map::new();
    config.set(
        "dirs",
        text_list(&[repo().join("examples/python_provider")]),
    );
    config.set(
        "paths",
        text_list(&[
            repo().join("bindings/python/guatiao/src"),
            repo().join("examples/greeter_kind/python"),
        ]),
    );
    config
}

#[test]
fn a_rust_host_serves_python_libraries() {
    let Some(bridge) = bridge() else {
        println!("skipped: the bridge is not built beside this test; `cargo build` first");
        return;
    };
    let mut registry = Registry::new("rust-host", "1.0");
    let name = registry
        .add_source_file(&bridge, &config())
        .unwrap_or_else(|e| panic!("the bridge was refused: {e}"));
    let changed = registry.refresh();
    assert!(changed.refused.is_empty(), "{changed:?}");
    let mut ids: Vec<&str> = changed.loaded.iter().map(|s| s.id.as_str()).collect();
    ids.sort();
    assert_eq!(ids, ["py_greeter", "py_tally"]);
    assert!(changed.loaded.iter().all(|s| s.source == name));
    let provider = registry.provider("py_greeter_hello").expect("served");
    assert_eq!(provider.kinds(), ["greeter", "counter"]);
    // The same file is no library: a scan names it a source.
    let dir = bridge.parent().unwrap();
    let mut scanner = Registry::new("scanner", "1.0");
    let report = guatiao::library::scan_dir(&mut scanner, dir).expect("a readable directory");
    assert!(
        report.skipped.iter().any(|(path, why)| path == &bridge
            && matches!(why, guatiao::library::Skipped::Source { serves } if serves == "python")),
        "{report:?}"
    );
    // A refresh with nothing changed changes nothing.
    assert!(registry.refresh().is_empty());
    let _ = TryAsRef::<str>::try_as_ref(&guatiao::Value::from("unused"));
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
fn a_c_host_serves_python_libraries() {
    let Some(bridge) = bridge() else {
        println!("skipped: the bridge is not built beside this test; `cargo build` first");
        return;
    };
    let Some(cc) = find_compiler() else {
        println!("skipped: no C compiler");
        return;
    };
    let Some(named) = std::env::var_os("GUATIAO_LIBRARY").map(PathBuf::from) else {
        println!(
            "skipped: GUATIAO_LIBRARY names no guatiao build (with `load`) for the C host to link"
        );
        return;
    };
    // A directory holds guatiao's own library; a file is a library carrying
    // the core surface under a name of its own, linked as it is.
    let (guatiao_dir, core_file) = if named.is_dir() {
        (named, None)
    } else {
        let dir = named.parent().map(Path::to_path_buf).unwrap_or_default();
        (dir, Some(named))
    };
    let out_dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let exe = out_dir.join(if cfg!(windows) {
        "c_host.exe"
    } else {
        "c_host"
    });
    let mut compile = Command::new(&cc);
    compile
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror"])
        .arg("-I")
        .arg(repo().join("crates/guatiao/include"))
        .arg("-I")
        .arg(repo().join("examples/greeter_kind/include"))
        .arg("-o")
        .arg(&exe)
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/c_host.c"));
    match (&core_file, cfg!(windows)) {
        (None, true) => {
            compile.arg(guatiao_dir.join("guatiao.dll.lib"));
        }
        (Some(file), true) => {
            let mut import = file.clone().into_os_string();
            import.push(".lib");
            compile.arg(import);
        }
        (None, false) => {
            compile
                .arg(format!("-L{}", guatiao_dir.display()))
                .arg("-lguatiao");
        }
        (Some(file), false) => {
            compile.arg(file);
        }
    }
    if !cfg!(windows) {
        compile.arg(format!("-Wl,-rpath,{}", guatiao_dir.display()));
    }
    let compiled = compile.output().expect("the compiler runs");
    assert!(
        compiled.status.success(),
        "the C host did not build:\n{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let mut run = Command::new(&exe);
    run.arg(&bridge)
        .arg(repo().join("examples/python_provider"))
        .arg(repo().join("bindings/python/guatiao/src"))
        .arg(repo().join("examples/greeter_kind/python"));
    if cfg!(windows) {
        // Where Windows finds guatiao.dll for the host it just linked.
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut dirs = vec![guatiao_dir.clone()];
        dirs.extend(std::env::split_paths(&path));
        run.env("PATH", std::env::join_paths(dirs).expect("paths join"));
    }
    let ran = run.output().expect("the C host runs");
    let stdout = String::from_utf8_lossy(&ran.stdout);
    assert!(
        ran.status.success() && stdout.contains("all checks passed"),
        "stdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    assert!(
        stdout.contains("py_greeter_hello greets: hello, ana"),
        "{stdout}"
    );
}
