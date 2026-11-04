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

use guatiao_source_lua::mlua::prelude::*;

#[path = "../../guatiao/tests/support/c_compiler.rs"]
mod c_compiler;

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

/// The greeter written in C, built into this test's directory.
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
            // The bridge imports python3.dll from the Python installation,
            // found on PATH.
            args.push("--import-search".into());
            args.push("path".into());
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

/// A source's note reaches a Lua host's refresh once: the source written in
/// Rust (`examples/source_library`, in `GUATIAO_LIBRARY`) tells its
/// configuration's `note`.
#[test]
fn a_lua_host_is_told_a_sources_note_once() {
    let source = std::env::var_os("GUATIAO_LIBRARY")
        .map(|dir| PathBuf::from(dir).join(library_name("source_library")))
        .filter(|file| file.is_file());
    let Some(source) = source else {
        println!(
            "skipped: GUATIAO_LIBRARY names no directory holding source_library (build the workspace)"
        );
        return;
    };
    let lua = Lua::new();
    let module = guatiao_source_lua::module(&lua).expect("the module");
    lua.globals().set("guatiao", module).expect("set");
    lua.globals()
        .set("source", source.display().to_string())
        .expect("set");
    let said: String = lua
        .load(
            r#"
            local reg = guatiao.registry("lua-notes", "0.1")
            local name = reg:add_source_file(source, {
                greetings = {hi = "hello"},
                note = "the shelf is dusty, says Rust",
            })
            local first = reg:refresh()
            assert(#first.notes == 1, "one note")
            local note = first.notes[1]
            assert(note.source == name, "by its source")
            assert(note.subject == "", "about the whole source")
            assert(note.detail.said == "by Rust", "with its detail")
            assert(reg:refresh().notes == nil, "told once")
            reg:close()
            return note.text
            "#,
        )
        .eval()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(said, "the shelf is dusty, says Rust");
}

/// This crate's own file, beside the test binary's directory: `cargo
/// build` puts it there.
fn source_file() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let debug = exe.parent()?.parent()?;
    let path = debug.join(library_name("guatiao_source_lua"));
    path.is_file().then_some(path)
}

/// A Lua host builds instances of a Lua provider from a configuration
/// and asks whether one can run, through this crate's file as a source.
#[test]
fn a_lua_host_builds_instances_and_asks_availability() {
    let Some(source) = source_file() else {
        println!("skipped: the source's own file is not built (cargo build first)");
        return;
    };
    let dir =
        std::env::temp_dir().join(format!("guatiao-lua-host-instances-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    std::fs::write(
        dir.join("lua_conf.guatiao"),
        "type = \"lua\"\nmodule = \"lua_conf\"\nkinds = [\"greeter\"]\nversion = \"1.0.0\"\n",
    )
    .expect("written");
    std::fs::write(
        dir.join("lua_conf.lua"),
        r#"
local Word = {
  kinds = { "greeter" },
  config = {
    type = "object",
    properties = { word = { type = "string" } },
    required = { "word" },
    additionalProperties = false,
  },
}
Word.__index = Word
function Word:new(config)
  if config.word == "nobody" then return nil, "`nobody` greets nobody" end
  return setmetatable({ word = config.word }, Word)
end
function Word:greet(name) return { greeting = (self.word or "hello") .. ", " .. name } end

local Down = { kinds = { "greeter" } }
function Down:available() return "the greeter is unplugged" end
function Down:greet(name) return { greeting = name } end

return { providers = { word = Word, down = Down } }
"#,
    )
    .expect("written");

    let kinds = repo()
        .join("examples/greeter_kind/include/greeter_kind.kinds.json")
        .display()
        .to_string();
    let lua = Lua::new();
    let module = guatiao_source_lua::module(&lua).expect("the module");
    lua.globals().set("guatiao", module).expect("set");
    lua.globals()
        .set("source", source.display().to_string())
        .expect("set");
    lua.globals()
        .set("dir", dir.display().to_string())
        .expect("set");
    lua.globals().set("kinds", kinds).expect("set");
    let said: String = lua
        .load(
            r#"
            guatiao.kinds(kinds)
            local reg = guatiao.registry("lua-instances", "0.1")
            reg:add_source_file(source, { dirs = { dir }, kinds = { kinds } })
            local changed = reg:refresh()
            assert(#changed.refused == 0, changed.refused[1] and changed.refused[1].reason)

            local howdy = assert(reg:create("lua_conf_word", "greeter", { word = "howdy" }))
            local hey = assert(reg:create("lua_conf_word", guatiao.kind("greeter"), { word = "hey" }))
            assert(howdy:greet("ana").greeting == "howdy, ana")
            assert(hey:greet("bo").greeting == "hey, bo")

            local none, err = reg:create("lua_conf_word", "greeter", { word = "nobody" })
            assert(none == nil and err.status == "bad_value", "refused by the provider")
            assert(err.message == "`nobody` greets nobody", err.message)
            local bad, why = reg:create("lua_conf_word", "greeter", { wrod = "x" })
            assert(bad == nil and why.status == "bad_value", "refused by the schema")

            local up = reg:available("lua_conf_word")
            assert(up == true, "available without asking")
            local ok, reason = reg:available("lua_conf_down")
            assert(ok == false, "unavailable")

            howdy = nil
            collectgarbage()
            assert(hey:greet("cy").greeting == "hey, cy", "the other instance stays")
            return reason
            "#,
        )
        .eval()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(said, "the greeter is unplugged");
    drop(lua);
    let _ = std::fs::remove_dir_all(&dir);
}
