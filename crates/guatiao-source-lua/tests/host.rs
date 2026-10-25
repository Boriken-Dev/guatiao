// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Lua as a host: `examples/lua_host/host.lua` run in a Lua state with
//! `require "guatiao"`, calling greeters written in Rust, C and Python
//! through a registry.
//!
//! The Rust greeter is the workspace's `derived_greeter`, found in
//! `GUATIAO_LIBRARY` (a directory: `cargo build --workspace
//! --all-features`, then `target/debug`); the C one is built here; the
//! Python ones come through the guatiao-source-py bridge `GUATIAO_PY_BRIDGE`
//! names. Each one missing is a stated skip.

extern crate alloc;

use alloc::rc::Rc;
use core::cell::RefCell;
use std::path::PathBuf;
use std::process::Command;

use guatiao_source_lua::mlua::prelude::*;

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

/// The greeter written in C, built into this test's directory.
fn c_greeter() -> Option<PathBuf> {
    let cc = find_compiler()?;
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(library_name("c_greeter"));
    let mut build = Command::new(cc);
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

/// Runs the example host in `lua` with `args`, answering what it printed.
fn run_host(lua: &Lua, args: &[String]) -> Result<String, String> {
    let printed = Rc::new(RefCell::new(String::new()));
    let sink = Rc::clone(&printed);
    let print = lua
        .create_function(move |_, items: LuaMultiValue| {
            let line: Vec<String> = items
                .iter()
                .map(|v| v.to_string().unwrap_or_default())
                .collect();
            let mut out = sink.borrow_mut();
            out.push_str(&line.join("\t"));
            out.push('\n');
            Ok(())
        })
        .expect("print");
    lua.globals().set("print", print).expect("set");
    let module = guatiao_source_lua::module(lua).expect("the module");
    // What `require "guatiao"` answers once the module is installed; Luau
    // has no `package` to preload it through.
    let require = lua
        .create_function(move |_, name: String| {
            if name == "guatiao" {
                Ok(module.clone())
            } else {
                Err(LuaError::runtime(format!("module '{name}' not found")))
            }
        })
        .expect("require");
    lua.globals().set("require", require).expect("set");
    let arg = lua.create_sequence_from(args.iter().cloned()).expect("arg");
    lua.globals().set("arg", arg).expect("set");
    let script = repo().join("examples/lua_host/host.lua");
    let ran = lua
        .load(std::fs::read_to_string(&script).expect("the script"))
        .set_name("@host.lua")
        .exec();
    let printed = printed.borrow().clone();
    ran.map(|()| printed.clone())
        .map_err(|e| format!("{e}\nprinted:\n{printed}"))
}

#[test]
fn a_lua_host_calls_greeters_written_in_rust_c_and_python() {
    let mut args = vec![
        repo()
            .join("examples/greeter_kind/include/greeter_kind.kinds.json")
            .display()
            .to_string(),
    ];
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
        }
        _ => {
            println!(
                "skipped: GUATIAO_PY_BRIDGE names no guatiao-source-py build for the Python greeters"
            )
        }
    }
    let lua = Lua::new();
    let printed = run_host(&lua, &args).unwrap_or_else(|e| panic!("{e}"));
    println!("{printed}");
    assert!(printed.contains("all checks passed"), "{printed}");
    for line in expect {
        assert!(printed.contains(line), "{line} missing:\n{printed}");
    }
}
