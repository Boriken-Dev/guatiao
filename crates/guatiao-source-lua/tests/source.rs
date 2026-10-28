// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The source from a Rust host: the Lua greeters called through the
//! kinds' traits, a library edited and served afresh, one deleted and
//! retired, and runaway code stopped at its limits while the host carries
//! on.
//!
//! The source is served two ways: in this process, as a `BridgeSource` a
//! registry holds, and as this crate's own file through
//! `add_source_file`, which `cargo test` builds beside the tests.

#![cfg(feature = "source")]

extern crate alloc;

use alloc::sync::Arc;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use greeter_kind::{Conversation, Counter, Greeter, Listener};
use guatiao::library::{Candidate, Host, LocalLibrary, ProviderError, Registry, Source};
use guatiao::value::convert::TryAsRef;
use guatiao::{List, Map, Status};
use guatiao_bridge::BridgeSource;
use guatiao_libffi::Libffi;
use guatiao_source_lua::LuaEngine;

#[path = "../../guatiao/tests/support/c_compiler.rs"]
mod c_compiler;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn kinds_file() -> String {
    repo()
        .join("examples/greeter_kind/include/greeter_kind.kinds.json")
        .display()
        .to_string()
}

fn library_name(base: &str) -> String {
    format!(
        "{}{base}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    )
}

/// This crate's own file, beside the test binary's directory.
fn source_file() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let debug = exe.parent()?.parent()?;
    let path = debug.join(library_name("guatiao_source_lua"));
    path.is_file().then_some(path)
}

fn texts(items: &[&str]) -> List {
    let mut list = List::new();
    for item in items {
        list.push(*item);
    }
    list
}

fn config(dir: &Path, limits: Option<Map>) -> Map {
    let mut config = Map::new();
    config.set("dirs", texts(&[&dir.display().to_string()]));
    config.set("kinds", texts(&[&kinds_file()]));
    if let Some(limits) = limits {
        config.set("limits", limits);
    }
    config
}

/// The source a registry holds, shared with the test.
struct Held(Arc<BridgeSource<LuaEngine>>);

impl Source for Held {
    fn list(&self, host: Host) -> Result<Vec<Candidate>, ProviderError> {
        self.0.list(host)
    }
    fn open(&self, host: Host, id: &str) -> Result<LocalLibrary<'static>, ProviderError> {
        self.0.open(host, id)
    }
    fn close(&self, id: &str, stamp: &str) {
        self.0.close(id, stamp);
    }
}

fn serving(config: &Map) -> Registry {
    let source = Arc::new(
        BridgeSource::<LuaEngine>::pinned(config, "lua-test")
            .expect("Lua starts")
            .with_mechanism(Arc::new(Libffi)),
    );
    let mut registry = Registry::new("lua-test-host", "1.0");
    registry.add_source("lua", Held(source)).expect("added");
    registry
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("guatiao-lua-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// A library of one module: `code` is its file, `id` its name.
fn put(dir: &Path, id: &str, manifest_extra: &str, code: &str) {
    std::fs::write(
        dir.join(format!("{id}.guatiao.toml")),
        format!("type = \"lua\"\nmodule = \"{id}\"\nkinds = [\"greeter\", \"counter\"]\n{manifest_extra}"),
    )
    .expect("written");
    std::fs::write(dir.join(format!("{id}.lua")), code).expect("written");
}

/// The example greeter under another module name, in a directory of its
/// own.
fn copy_greeter(dir: &Path, id: &str) -> PathBuf {
    let code =
        std::fs::read_to_string(repo().join("examples/lua_provider/lua_greeter/lua_greeter.lua"))
            .expect("the example");
    put(dir, id, "", &code);
    dir.join(format!("{id}.lua"))
}

fn greeter(registry: &Registry, key: &str) -> Arc<dyn Greeter> {
    registry
        .offer::<dyn Greeter>(key)
        .unwrap_or_else(|| panic!("{key} is served"))
        .expect("as the trait")
        .shared()
        .expect("an instance")
}

fn greeting(answer: &Map) -> &str {
    answer
        .get("greeting")
        .and_then(TryAsRef::<str>::try_as_ref)
        .unwrap_or("")
}

struct Heard(Arc<Mutex<Vec<String>>>);

impl Listener for Heard {
    fn heard(&mut self, what: &str) {
        self.0.lock().unwrap().push(what.to_string());
    }
}

impl Drop for Heard {
    fn drop(&mut self) {
        self.0.lock().unwrap().push("<destroyed>".to_string());
    }
}

/// Every slot of the example greeter, through the kinds' traits.
fn calls_every_slot(registry: &Registry) {
    let hello = greeter(registry, "lua_greeter_hello");
    assert_eq!(
        greeting(&hello.greet("ana").expect("greeted")),
        "hello, ana"
    );

    let refused = hello.greet("").unwrap_err();
    assert_eq!(refused.status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(refused.message(), "nobody to greet");
    let failed = hello.greet("panic").unwrap_err();
    assert_eq!(failed.status, Status::GUATIAO_ERR_INTERNAL);
    assert!(
        failed.message().contains("a provider bug, on request"),
        "{}",
        failed.message()
    );

    // Not written in Lua: the kind's default runs, through `greet`.
    assert_eq!(hello.shout("cy"), "HELLO, CY");

    let mut config = Map::new();
    assert!(hello.check(&mut config).expect("checked"));
    assert_eq!(
        config.get("name").and_then(TryAsRef::<str>::try_as_ref),
        Some("ana")
    );
    assert_eq!(
        config
            .get("checked_by")
            .and_then(TryAsRef::<str>::try_as_ref),
        Some("Lua")
    );

    let mut named = Map::new();
    named.set("name", "di");
    assert_eq!(
        greeting(&hello.greet_from(named).expect("greeted")),
        "hello, di"
    );

    let heard = Arc::new(Mutex::new(Vec::new()));
    let mut lent = Heard(Arc::clone(&heard));
    hello.greet_into("ed", &mut lent).expect("told");
    assert_eq!(*heard.lock().unwrap(), ["hello, ed"]);
    drop(lent);

    let heard = Arc::new(Mutex::new(Vec::new()));
    let mut talk: guatiao::library::Object<dyn Conversation> = hello
        .start(Heard(Arc::clone(&heard)).into_object())
        .expect("a conversation Lua holds");
    talk.say("one").expect("said");
    talk.say("two").expect("said");
    assert_eq!(talk.say("").unwrap_err().message(), "nothing to say");
    assert_eq!(talk.turns(), 2);
    let mut buf = [0u8; 32];
    let n = talk.read(&mut buf) as usize;
    assert_eq!(&buf[..n], b"one two");
    assert_eq!(*heard.lock().unwrap(), ["one", "two"]);
    drop(talk);
    assert_eq!(
        *heard.lock().unwrap(),
        ["one", "two", "<destroyed>"],
        "the listener went with the conversation"
    );

    let counter = registry
        .offer::<dyn Counter>("lua_greeter_hello")
        .expect("served")
        .expect("as the trait")
        .shared()
        .expect("an instance");
    let first = counter.count();
    assert_eq!(counter.count(), first + 1);

    let polite = greeter(registry, "lua_greeter_polite");
    assert_eq!(
        greeting(&polite.greet("fa").expect("greeted")),
        "good day, fa"
    );
    assert_eq!(
        polite.greet("").unwrap_err().message(),
        "nobody to greet, politely"
    );

    // From another thread: queued to the Lua state's.
    let from_another = {
        let hello = Arc::clone(&hello);
        std::thread::spawn(move || hello.greet("go").map(|m| greeting(&m).to_string()))
            .join()
            .expect("joined")
    };
    assert_eq!(from_another.expect("greeted"), "hello, go");
}

#[test]
fn a_rust_host_calls_the_lua_greeters() {
    let mut registry = serving(&config(&repo().join("examples/lua_provider"), None));
    let changed = registry.refresh();
    assert!(changed.refused.is_empty(), "{changed:?}");
    assert_eq!(changed.loaded.len(), 1, "{changed:?}");
    assert_eq!(
        registry.loaded()[0].version,
        "1.0.0",
        "read from `_VERSION`"
    );
    calls_every_slot(&registry);
    assert!(
        registry.refresh().is_empty(),
        "nothing changed, nothing done"
    );
}

#[test]
fn the_source_file_serves_them_too() {
    let Some(file) = source_file() else {
        println!("skipped: this crate's file is not built beside the tests");
        return;
    };
    let mut registry = Registry::new("lua-file-host", "1.0");
    let name = registry
        .add_source_file(&file, &config(&repo().join("examples/lua_provider"), None))
        .unwrap_or_else(|e| panic!("the source was refused: {e}"));
    let changed = registry.refresh();
    assert!(changed.refused.is_empty(), "{changed:?}");
    assert!(changed.loaded.iter().all(|s| s.source == name));
    calls_every_slot(&registry);
    // A scan names the file a source, not a library.
    let mut scanner = Registry::new("scanner", "1.0");
    let report =
        guatiao::library::scan_dir(&mut scanner, file.parent().unwrap()).expect("readable");
    assert!(
        report.skipped.iter().any(|(path, why)| path == &file
            && matches!(why, guatiao::library::Skipped::Source { serves } if serves == "lua")),
        "{report:?}"
    );
}

#[test]
fn an_edit_serves_new_code_and_a_deleted_library_retires() {
    let dir = scratch("edit");
    let file = copy_greeter(&dir, "lua_edited");
    let mut registry = serving(&config(&dir, None));
    assert_eq!(registry.refresh().loaded.len(), 1);
    let old = greeter(&registry, "lua_edited_hello");
    assert_eq!(greeting(&old.greet("ana").expect("greeted")), "hello, ana");

    std::thread::sleep(core::time::Duration::from_millis(50));
    let code = std::fs::read_to_string(&file).expect("read");
    std::fs::write(
        &file,
        code.replace("local GREETING = \"hello\"", "local GREETING = \"howdy\"")
            .replace("_VERSION = \"1.0.0\"", "_VERSION = \"1.0.1\""),
    )
    .expect("edited");
    let second = registry.refresh();
    assert_eq!(second.loaded.len(), 1, "{second:?}");
    assert_eq!(second.retired.len(), 1, "{second:?}");
    let new = greeter(&registry, "lua_edited_hello");
    assert_eq!(greeting(&new.greet("ana").expect("greeted")), "howdy, ana");
    assert_eq!(
        greeting(&old.greet("ana").expect("still greets")),
        "hello, ana",
        "the old build answers the instance taken from it"
    );
    assert!(registry.collect().is_empty(), "the old build is still held");
    drop(old);
    assert_eq!(
        registry.collect().len(),
        1,
        "the old build closed once let go"
    );

    drop(new);
    std::fs::remove_file(dir.join("lua_edited.guatiao.toml")).expect("removed");
    let third = registry.refresh();
    assert_eq!(third.retired.len(), 1, "{third:?}");
    assert_eq!(registry.collect().len(), 1, "closed");
    assert!(registry.offer::<dyn Greeter>("lua_edited_hello").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

fn limits(pairs: &[(&str, u64)]) -> Map {
    let mut map = Map::new();
    for (k, v) in pairs {
        map.set(k, *v);
    }
    map
}

const SPIN: &str = r#"
local Spin = { kinds = { "greeter" } }
function Spin:greet(name)
  if name == "loop" then while true do end end
  if name == "stubborn" then while true do pcall(function() while true do end end) end end
  if name == "deep" then local function down(n) return 1 + down(n + 1) end return down(1) end
  if name == "grow" then local s = string.rep("x", 67108864) return { greeting = s } end
  return { greeting = "calm, " .. name }
end
return { _VERSION = "1.0.0", providers = { spin = Spin } }
"#;

#[test]
fn runaway_code_is_stopped_at_its_limits_and_the_host_carries_on() {
    let dir = scratch("limits");
    put(&dir, "lua_spin", "", SPIN);
    let mut registry = serving(&config(
        &dir,
        Some(limits(&[
            ("operations", 200_000),
            ("memory", 16 * 1024 * 1024),
            ("depth", 150),
        ])),
    ));
    let changed = registry.refresh();
    assert!(changed.refused.is_empty(), "{changed:?}");
    let spin = greeter(&registry, "lua_spin_spin");

    let stopped = spin.greet("loop").unwrap_err();
    assert_eq!(stopped.status, Status::GUATIAO_ERR_INTERNAL);
    assert_eq!(
        stopped.message(),
        "stopped at its operations limit (200000)"
    );

    let stubborn = spin.greet("stubborn").unwrap_err();
    assert_eq!(
        stubborn.message(),
        "stopped at its operations limit (200000)",
        "a pcall around the stopped code does not keep it running"
    );

    let deep = spin.greet("deep").unwrap_err();
    assert_eq!(deep.message(), "stopped at its depth limit (150)");

    let grown = spin.greet("grow").unwrap_err();
    assert_eq!(
        grown.status,
        Status::GUATIAO_ERR_ALLOC,
        "{}",
        grown.message()
    );

    assert_eq!(
        greeting(&spin.greet("after").expect("the state carries on")),
        "calm, after"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn time_stops_a_call_too() {
    let dir = scratch("time");
    put(&dir, "lua_slow", "", SPIN);
    let mut registry = serving(&config(&dir, Some(limits(&[("time_ms", 50)]))));
    assert!(registry.refresh().refused.is_empty());
    let started = std::time::Instant::now();
    let stopped = greeter(&registry, "lua_slow_spin")
        .greet("loop")
        .unwrap_err();
    assert_eq!(stopped.message(), "stopped at its time_ms limit (50)");
    assert!(started.elapsed() < core::time::Duration::from_secs(5));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_library_that_does_not_load_is_refused_in_its_words() {
    let dir = scratch("refused");
    put(
        &dir,
        "lua_loops",
        "version = \"1.0.0\"\n",
        "while true do end",
    );
    put(&dir, "lua_broken", "version = \"1.0.0\"\n", "return {");
    put(&dir, "lua_unversioned", "", "return { providers = {} }");
    put(
        &dir,
        "lua_mismatch",
        "version = \"2.0.0\"\n",
        "return { _VERSION = \"1.0.0\", providers = {} }",
    );
    put(
        &dir,
        "lua_lacking",
        "version = \"1.0.0\"\n",
        "return { providers = { p = { kinds = { 'greeter' } } } }",
    );
    put(
        &dir,
        "lua_nokinds",
        "version = \"1.0.0\"\n",
        "return { providers = { p = {} } }",
    );
    let mut registry = serving(&config(&dir, Some(limits(&[("operations", 100_000)]))));
    let changed = registry.refresh();
    assert!(changed.loaded.is_empty(), "{changed:?}");
    let reason = |id: &str| -> String {
        changed
            .refused
            .iter()
            .find(|r| r.id.as_deref() == Some(id))
            .map(|r| r.reason.clone())
            .unwrap_or_else(|| panic!("{id} refused: {changed:?}"))
    };
    assert!(
        reason("lua_loops").contains("operations limit"),
        "{}",
        reason("lua_loops")
    );
    assert!(
        reason("lua_broken").contains("lua_broken"),
        "{}",
        reason("lua_broken")
    );
    assert!(
        reason("lua_unversioned").contains("_VERSION"),
        "{}",
        reason("lua_unversioned")
    );
    assert!(
        reason("lua_mismatch").contains("2.0.0"),
        "{}",
        reason("lua_mismatch")
    );
    assert!(
        reason("lua_lacking").contains("`greet`"),
        "{}",
        reason("lua_lacking")
    );
    assert!(
        reason("lua_nokinds").contains("names no `kinds`"),
        "{}",
        reason("lua_nokinds")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_c_host_calls_the_lua_greeter() {
    let Some(file) = source_file() else {
        println!("skipped: this crate's file is not built beside the tests");
        return;
    };
    let Some(cc) = c_compiler::find() else {
        println!("skipped: no C compiler");
        return;
    };
    let Some(guatiao_dir) = std::env::var_os("GUATIAO_LIBRARY").map(PathBuf::from) else {
        println!(
            "skipped: GUATIAO_LIBRARY names no guatiao build (with `load`) for the C host to link"
        );
        return;
    };
    let exe = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(if cfg!(windows) {
        "lua_c_host.exe"
    } else {
        "lua_c_host"
    });
    let mut compile = c_compiler::command(&cc);
    compile
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror"])
        .arg("-I")
        .arg(repo().join("crates/guatiao/include"))
        .arg("-I")
        .arg(repo().join("examples/greeter_kind/include"))
        .arg("-o")
        .arg(&exe)
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/c_host.c"));
    if cfg!(windows) {
        compile.arg(guatiao_dir.join("guatiao.dll.lib"));
    } else {
        compile
            .arg(format!("-L{}", guatiao_dir.display()))
            .arg("-lguatiao")
            .arg(format!("-Wl,-rpath,{}", guatiao_dir.display()));
    }
    let compiled = compile.output().expect("the compiler runs");
    assert!(
        compiled.status.success(),
        "the C host did not build:\n{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let mut run = std::process::Command::new(&exe);
    run.arg(&file)
        .arg(repo().join("examples/lua_provider"))
        .arg(kinds_file());
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
        stdout.contains("lua_greeter_hello greets: hello, ana"),
        "{stdout}"
    );
}
