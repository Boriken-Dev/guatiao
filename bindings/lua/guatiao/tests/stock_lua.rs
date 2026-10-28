// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A stock Lua interpreter requires the module and hosts greeters written
//! in Rust, C, Python and Lua.
//!
//! - the interpreter: `GUATIAO_LUA` (an executable), else `lua` on PATH;
//!   it must be the Lua this module was built for;
//! - the module: this crate's build (`cargo build` first);
//! - the Rust greeter: `derived_greeter` in `GUATIAO_LIBRARY`;
//! - the C greeter: built here;
//! - the Python greeters: the guatiao-source-py build `GUATIAO_PY_BRIDGE` names;
//! - the Lua greeters: the guatiao-source-lua source `GUATIAO_LUA_SOURCE` names.
//!
//! Each one missing is a stated skip.

use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "../../../../crates/guatiao/tests/support/c_compiler.rs"]
mod c_compiler;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn library_name(base: &str) -> String {
    format!(
        "{}{base}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    )
}

fn kinds_file() -> String {
    repo()
        .join("examples/greeter_kind/include/greeter_kind.kinds.json")
        .display()
        .to_string()
}

/// The interpreter, if one runs.
fn interpreter() -> Option<PathBuf> {
    let lua = std::env::var_os("GUATIAO_LUA").map_or_else(|| PathBuf::from("lua"), PathBuf::from);
    Command::new(&lua).arg("-v").output().ok()?;
    Some(lua)
}

/// A directory holding this crate's build under the name `require` looks
/// for: `guatiao.so` or `guatiao.dll`.
fn module_dir(test: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let built = exe
        .parent()?
        .parent()?
        .join(library_name("guatiao_lua_module"));
    if !built.is_file() {
        return None;
    }
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("cpath-{test}"));
    std::fs::create_dir_all(&dir).ok()?;
    let ext = if cfg!(windows) { "dll" } else { "so" };
    std::fs::copy(&built, dir.join(format!("guatiao.{ext}"))).ok()?;
    Some(dir)
}

fn c_greeter() -> Option<PathBuf> {
    let cc = c_compiler::find()?;
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(library_name("c_greeter"));
    let mut build = c_compiler::command(&cc);
    build.args(["-shared", "-std=c11", "-Wall", "-Wextra", "-Werror"]);
    if !cfg!(windows) {
        build.arg("-fPIC");
    }
    let built = build
        .arg("-I")
        .arg(repo().join("crates/guatiao/include"))
        .arg("-I")
        .arg(repo().join("examples/greeter_kind/include"))
        .arg("-o")
        .arg(&out)
        .arg(repo().join("crates/guatiao/tests/c_consumer/greeter_in_c.c"))
        .output()
        .expect("the compiler runs");
    assert!(
        built.status.success(),
        "the C greeter did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    Some(out)
}

fn run(lua: &Path, cpath: &Path, script: &Path, args: &[String]) -> String {
    let ext = if cfg!(windows) { "dll" } else { "so" };
    let ran = Command::new(lua)
        .arg(script)
        .args(args)
        .env("LUA_CPATH", format!("{}/?.{ext};;", cpath.display()))
        .output()
        .expect("the interpreter runs");
    let stdout = String::from_utf8_lossy(&ran.stdout).into_owned();
    assert!(
        ran.status.success(),
        "stdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    stdout
}

#[test]
fn a_stock_lua_requires_the_module() {
    let name = "helpers";
    let Some(lua) = interpreter() else {
        println!("skipped: no Lua interpreter (GUATIAO_LUA, or lua on PATH)");
        return;
    };
    let Some(cpath) = module_dir(name) else {
        println!("skipped: the module is not built beside the tests; `cargo build` first");
        return;
    };
    let printed = run(
        &lua,
        &cpath,
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/helpers.lua"),
        &[kinds_file()],
    );
    assert!(printed.contains("helpers: all checks passed"), "{printed}");
}

#[test]
fn a_stock_lua_hosts_greeters_written_in_rust_c_python_and_lua() {
    let name = "host";
    let Some(lua) = interpreter() else {
        println!("skipped: no Lua interpreter (GUATIAO_LUA, or lua on PATH)");
        return;
    };
    let Some(cpath) = module_dir(name) else {
        println!("skipped: the module is not built beside the tests; `cargo build` first");
        return;
    };
    let mut args = vec![kinds_file()];
    let mut expect = Vec::new();
    match std::env::var_os("GUATIAO_LIBRARY").map(PathBuf::from) {
        Some(dir) if dir.join(library_name("derived_greeter")).is_file() => {
            args.push("--library".into());
            args.push(
                dir.join(library_name("derived_greeter"))
                    .display()
                    .to_string(),
            );
            expect.push("derived_greeter_hello greets: hello, ana");
            expect.push("derived_greeter_hello told a lent listener: hello, di");
            expect.push("derived_greeter_hello held a conversation: one");
        }
        _ => println!(
            "skipped: GUATIAO_LIBRARY names no directory holding derived_greeter (build the workspace)"
        ),
    }
    match c_greeter() {
        Some(file) => {
            args.push("--library".into());
            args.push(file.display().to_string());
            expect.push("c_greeter greets: hello from C, ana");
        }
        None => println!("skipped: no C compiler for the C greeter"),
    }
    match std::env::var_os("GUATIAO_PY_BRIDGE").map(PathBuf::from) {
        Some(bridge) if bridge.is_file() => {
            args.push("--source".into());
            args.push(bridge.display().to_string());
            args.push(format!(
                "dirs={}",
                repo().join("examples/python_provider").display()
            ));
            args.push(format!(
                "paths={};{}",
                repo().join("bindings/python/guatiao/src").display(),
                repo().join("examples/greeter_kind/python").display()
            ));
            expect.push("py_greeter_hello greets: hello, ana");
            expect.push("py_greeter_hello held a conversation: one");
        }
        _ => {
            println!(
                "skipped: GUATIAO_PY_BRIDGE names no guatiao-source-py build for the Python greeters"
            )
        }
    }
    match std::env::var_os("GUATIAO_LUA_SOURCE").map(PathBuf::from) {
        Some(source) if source.is_file() => {
            args.push("--source".into());
            args.push(source.display().to_string());
            args.push(format!(
                "dirs={}",
                repo().join("examples/lua_provider").display()
            ));
            args.push(format!("kinds={}", kinds_file()));
            expect.push("lua_greeter_hello greets: hello, ana");
            expect.push("lua_greeter_hello cannot call a Lua table back from its thread");
            expect.push("lua_greeter_polite greets: good day, ana");
        }
        _ => {
            println!(
                "skipped: GUATIAO_LUA_SOURCE names no guatiao-source-lua build for the Lua greeters"
            )
        }
    }
    let printed = run(
        &lua,
        &cpath,
        &repo().join("examples/lua_host/host.lua"),
        &args,
    );
    println!("{printed}");
    assert!(printed.contains("all checks passed"), "{printed}");
    for line in expect {
        assert!(printed.contains(line), "{line} missing:\n{printed}");
    }
}
