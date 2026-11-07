// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A host written in C starts a channel on the echo library through the
//! headers alone, its sink called from the echo's own thread.

use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "../../../crates/guatiao/tests/support/c_compiler.rs"]
mod c_compiler;

/// A shared library built beside this test binary.
fn shared_library(stem: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let deps = exe.parent()?;
    let name = format!(
        "{}{stem}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    [deps.to_path_buf(), deps.join("..")]
        .into_iter()
        .map(|dir| dir.join(&name))
        .find(|path| path.is_file())
}

/// A skip, unless `GUATIAO_REQUIRE_ALL` says every check must run.
fn skip(why: &str) {
    assert!(
        std::env::var_os("GUATIAO_REQUIRE_ALL").is_none(),
        "{why}, and GUATIAO_REQUIRE_ALL is set"
    );
    println!("skipped: {why}");
}

#[test]
fn a_c_host_starts_a_channel_on_the_echo() {
    let Some(cc) = c_compiler::find() else {
        skip("no C compiler found");
        return;
    };
    let Some(core) = shared_library("guatiao") else {
        skip(
            "the guatiao shared library is not beside this test binary; \
             run `cargo build --workspace --all-features` first",
        );
        return;
    };
    let echo = shared_library("channel_echo").expect("the package's own cdylib");
    let lib_dir = core.parent().expect("in a directory").to_path_buf();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(if cfg!(windows) {
        "channel_echo_c_host.exe"
    } else {
        "channel_echo_c_host"
    });
    let mut compile = c_compiler::command(&cc);
    compile
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror"])
        .arg("-I")
        .arg(repo.join("crates/guatiao/include"))
        .arg("-I")
        .arg(repo.join("crates/guatiao-channel/include"))
        .arg("-I")
        .arg(repo.join("examples/channel_echo/kind/include"))
        .arg("-o")
        .arg(&out)
        .arg(repo.join("examples/channel_echo/tests/c/host.c"));
    if cfg!(windows) {
        compile.arg(core.with_extension("dll.lib"));
    } else {
        compile
            .arg(format!("-L{}", lib_dir.display()))
            .arg("-lguatiao")
            .arg("-pthread")
            .arg(format!("-Wl,-rpath,{}", lib_dir.display()));
    }
    let compiled = compile.output().expect("the compiler runs");
    assert!(
        compiled.status.success(),
        "the C host did not build:\n{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let path = std::env::join_paths(core::iter::once(lib_dir.clone()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .expect("a joinable PATH");
    let run = Command::new(&out)
        .arg(&echo)
        .current_dir(&lib_dir)
        .env("PATH", path)
        .output()
        .expect("the host was just built");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success() && stdout.contains("all checks passed"),
        "stdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    println!("{}", stdout.trim());
}
