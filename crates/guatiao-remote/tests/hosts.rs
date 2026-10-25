// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Hosts in Rust and in C calling a remote Rust greeter and a remote C
//! greeter: the server is `guatiao-remote-server --stdio` in a child
//! process, loading `derived_greeter` and the core's C greeter. Each test
//! skips, saying why, without a C compiler or the workspace's libraries.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::procs;
use greeter_kind::Greeter;
use guatiao::Status;
use guatiao::library::Registry;
use guatiao::value::convert::TryAsRef;
use guatiao_remote::transport::child::ChildCommand;
use guatiao_remote::{LinkOptions, RemoteSource, Trust};

fn kinds_json() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/greeter_kind/include/greeter_kind.kinds.json")
}

fn server() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_guatiao-remote-server"))
}

/// The two greeters' libraries, or why the test cannot run.
fn greeters() -> Option<(PathBuf, PathBuf, PathBuf)> {
    let Some(cc) = procs::compiler() else {
        println!("skipped: no C compiler for the C greeter");
        return None;
    };
    let Some(derived) = procs::library("derived_greeter") else {
        println!("skipped: derived_greeter is not built; build the workspace first");
        return None;
    };
    // Built once: the tests run at once, and two compilers writing one
    // library collide.
    static BUILT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    let c = BUILT
        .get_or_init(|| {
            let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("remote_hosts");
            std::fs::create_dir_all(&dir).unwrap();
            procs::c_greeter(&cc, &dir)
        })
        .clone();
    Some((derived, c, cc))
}

fn greeting(map: &guatiao::Map) -> String {
    map.get("greeting")
        .and_then(TryAsRef::<str>::try_as_ref)
        .unwrap_or_default()
        .to_string()
}

#[test]
fn a_rust_host_calls_a_remote_rust_and_a_remote_c_greeter() {
    let Some((derived, c, _)) = greeters() else {
        return;
    };
    let child = ChildCommand::new(server())
        .args(["--stdio", "--kinds"])
        .arg(kinds_json())
        .arg("--load")
        .arg(&derived)
        .arg("--load")
        .arg(&c);
    let source = RemoteSource::new(Box::new(child), LinkOptions::new(Trust::Local));
    let mut registry = Registry::new("host", "1.0");
    registry.add_source("child", source.clone()).unwrap();
    let refreshed = registry.refresh();
    assert!(refreshed.refused.is_empty(), "{refreshed:?}");
    assert_eq!(refreshed.loaded.len(), 2, "{refreshed:?}");

    let rust = registry
        .offer::<dyn Greeter>("derived_greeter_hello")
        .expect("the Rust greeter")
        .unwrap()
        .shared()
        .unwrap();
    assert_eq!(greeting(&rust.greet("ana").unwrap()), "hello, ana");
    let c_greeter = registry
        .offer::<dyn Greeter>("c_greeter")
        .expect("the C greeter")
        .unwrap()
        .shared()
        .unwrap();
    assert_eq!(
        greeting(&c_greeter.greet("bo").unwrap()),
        "hello from C, bo"
    );
    let e = c_greeter.greet("").unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(e.message(), "nobody to greet, says C");
    // The C greeter leaves `shout` out: the trait's default runs here.
    assert_eq!(c_greeter.shout("cy"), "HELLO FROM C, CY");
}

#[test]
fn a_c_host_calls_a_remote_rust_and_a_remote_c_greeter() {
    let Some((derived, c, cc)) = greeters() else {
        return;
    };
    let (Some(remote), Some(guatiao)) =
        (procs::library("guatiao_remote"), procs::library("guatiao"))
    else {
        println!("skipped: guatiao or guatiao_remote's library is not built");
        return;
    };
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = guatiao.parent().unwrap().to_path_buf();
    let exe = dir.join(format!(
        "guatiao_remote_host{}",
        std::env::consts::EXE_SUFFIX
    ));
    let mut cmd = Command::new(&cc);
    cmd.args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-I"])
        .arg(here.join("../guatiao/include"))
        .arg("-I")
        .arg(here.join("../../examples/greeter_kind/include"))
        .arg("-o")
        .arg(&exe)
        .arg(here.join("tests/c/remote_host.c"));
    if cfg!(windows) {
        cmd.arg(guatiao.with_extension("dll.lib"));
    } else {
        cmd.arg(format!("-L{}", dir.display()))
            .arg("-lguatiao")
            .arg("-Wl,-rpath,$ORIGIN");
    }
    let (ok, said) = procs::run(&mut cmd);
    assert!(ok, "the C host did not compile:\n{said}");

    let (ok, out) = procs::run(
        Command::new(&exe)
            .arg(&remote)
            .arg(server())
            .arg(kinds_json())
            .arg(&derived)
            .arg(&c)
            .arg("--")
            .arg("derived_greeter_hello")
            .arg("c_greeter"),
    );
    assert!(ok, "{out}");
    for line in [
        "libraries served: 2",
        "derived_greeter_hello greets: hello, ana",
        "c_greeter greets: hello from C, ana",
        "all checks passed",
    ] {
        assert!(out.contains(line), "missing `{line}` in:\n{out}");
    }
}
