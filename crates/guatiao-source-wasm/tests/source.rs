// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The source from a Rust host, over the example greeters written in Rust
//! and in Go: both served by one source and called through the kinds'
//! traits, a library rebuilt and served afresh, one deleted and retired,
//! a panic contained, a call back into a busy library refused, and the
//! source's own file serving them.
//!
//! The guests are built by the tests (`wasm32-unknown-unknown` for Rust,
//! Go 1.24 for Go); a missing toolchain skips, and fails under
//! `GUATIAO_REQUIRE_ALL=1`.

mod common;

extern crate alloc;

use alloc::sync::Arc;
use std::path::Path;
use std::sync::Mutex;

use common::*;
use greeter_kind::{Conversation, Counter, Greeter, Listener, Wide};
use guatiao::library::Registry;
use guatiao::value::convert::TryAsRef;
use guatiao::{Map, Status};

#[path = "../../guatiao/tests/support/c_compiler.rs"]
mod c_compiler;

/// Every slot of a example greeter's providers, through the kinds' traits.
fn calls_every_slot(registry: &Registry, module: &str, checked_by: &str) {
    let hello = greeter(registry, &format!("{module}_hello"));
    assert_eq!(
        greeting(&hello.greet("ana").expect("greeted")),
        "hello, ana"
    );

    let refused = hello.greet("").unwrap_err();
    assert_eq!(refused.status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(refused.message(), "nobody to greet");

    // Not written in the guest: the kind's default runs, through `greet`.
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
        Some(checked_by)
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
        .expect("a conversation the guest holds");
    talk.say("one").expect("said");
    talk.say("two").expect("said");
    assert_eq!(talk.say("").unwrap_err().message(), "nothing to say");
    assert_eq!(talk.turns(), 2);
    let mut buf = [0u8; 32];
    let n = talk.read(&mut buf) as usize;
    assert_eq!(&buf[..n], b"one two");
    let mut small = [0u8; 4];
    assert_eq!(talk.read(&mut small), 4, "as much as fits");
    assert_eq!(&small, b"one ");
    assert_eq!(*heard.lock().unwrap(), ["one", "two"]);
    drop(talk);
    assert_eq!(
        *heard.lock().unwrap(),
        ["one", "two", "<destroyed>"],
        "the listener went with the conversation"
    );

    let counter = registry
        .offer::<dyn Counter>(&format!("{module}_hello"))
        .expect("served")
        .expect("as the trait")
        .shared()
        .expect("an instance");
    let first = counter.count();
    assert_eq!(counter.count(), first + 1);

    let polite = greeter(registry, &format!("{module}_polite"));
    assert_eq!(
        greeting(&polite.greet("fa").expect("greeted")),
        "good day, fa"
    );
    assert_eq!(
        polite.greet("").unwrap_err().message(),
        "nobody to greet, politely"
    );

    let wide = registry
        .offer::<dyn Wide>(&format!("{module}_wide"))
        .expect("served")
        .expect("as the trait")
        .shared()
        .expect("an instance");
    for x in [i64::MIN, -1, 0, 1 << 53, i64::MAX] {
        assert_eq!(wide.echo_i64(x).expect("echoed"), x);
    }
    for x in [0, 1 << 63, u64::MAX] {
        assert_eq!(wide.echo_u64(x).expect("echoed"), x);
    }

    // From other threads at once: each waits for the library's lock.
    let threads: Vec<_> = (0..4)
        .map(|i| {
            let hello = Arc::clone(&hello);
            std::thread::spawn(move || {
                (0..25)
                    .map(|_| {
                        hello
                            .greet(&format!("t{i}"))
                            .map(|m| greeting(&m).to_string())
                    })
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    for (i, t) in threads.into_iter().enumerate() {
        for answer in t.join().expect("joined") {
            assert_eq!(answer.expect("greeted"), format!("hello, t{i}"));
        }
    }
}

fn both() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    Some((rust_example()?, go_example()?))
}

#[test]
fn both_guests_are_served_by_one_source_and_answer_the_same_calls() {
    let Some((rust, go)) = both() else {
        return;
    };
    let mut registry = serving(&config(&[&rust, &go], None));
    let changed = registry.refresh();
    assert!(changed.refused.is_empty(), "{changed:?}");
    assert_eq!(changed.loaded.len(), 2, "{changed:?}");
    for loaded in registry.loaded() {
        assert_eq!(loaded.version, "1.0.0", "{}", loaded.id);
    }
    calls_every_slot(&registry, "wasm_greeter_rust", "Rust, in wasm");
    calls_every_slot(&registry, "wasm_greeter_go", "Go, in wasm");
    assert!(
        registry.refresh().is_empty(),
        "nothing changed, nothing done"
    );
}

#[test]
fn the_source_file_serves_them_too() {
    let Some((rust, go)) = both() else {
        return;
    };
    let Some(file) = source_file() else {
        println!("skipped: this crate's file is not built beside the tests");
        return;
    };
    let mut registry = Registry::new("wasm-file-host", "1.0");
    let name = registry
        .add_source_file(&file, &config(&[&rust, &go], None))
        .unwrap_or_else(|e| panic!("the source was refused: {e}"));
    let changed = registry.refresh();
    assert!(changed.refused.is_empty(), "{changed:?}");
    assert_eq!(changed.loaded.len(), 2);
    assert!(changed.loaded.iter().all(|s| s.source == name));
    calls_every_slot(&registry, "wasm_greeter_rust", "Rust, in wasm");
    calls_every_slot(&registry, "wasm_greeter_go", "Go, in wasm");
    // A scan names the file a source, not a library.
    let mut scanner = Registry::new("scanner", "1.0");
    let report =
        guatiao::library::scan_dir(&mut scanner, file.parent().unwrap()).expect("readable");
    assert!(
        report.skipped.iter().any(|(path, why)| path == &file
            && matches!(why, guatiao::library::Skipped::Source { serves } if serves == "wasm")),
        "{report:?}"
    );
}

/// A library in a directory of its own: `module` copied there with a
/// manifest naming `version` when given.
fn put(dir: &Path, id: &str, module: &Path, version: Option<&str>) {
    let version = version
        .map(|v| format!("version = \"{v}\"\n"))
        .unwrap_or_default();
    std::fs::write(
        dir.join(format!("{id}.guatiao.toml")),
        format!("type = \"wasm\"\nmodule = \"{id}\"\nkinds = [\"greeter\", \"counter\", \"wide\"]\n{version}"),
    )
    .expect("written");
    std::fs::copy(module, dir.join(format!("{id}.wasm"))).expect("copied");
}

fn rebuilt_is_served(name: &str, first: &Path, edited: &Path, version: Option<&str>) {
    let dir = scratch(name);
    let id = format!("wasm_{name}");
    put(&dir, &id, first, version);
    let mut registry = serving(&config(&[&dir], None));
    assert_eq!(registry.refresh().loaded.len(), 1);
    let key = format!("{id}_hello");
    let old = greeter(&registry, &key);
    assert_eq!(greeting(&old.greet("ana").expect("greeted")), "hello, ana");

    std::thread::sleep(core::time::Duration::from_millis(50));
    std::fs::copy(edited, dir.join(format!("{id}.wasm"))).expect("the rebuilt module");
    let second = registry.refresh();
    assert_eq!(second.loaded.len(), 1, "{second:?}");
    assert_eq!(second.retired.len(), 1, "{second:?}");
    let new = greeter(&registry, &key);
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
    std::fs::remove_file(dir.join(format!("{id}.guatiao.toml"))).expect("removed");
    let third = registry.refresh();
    assert_eq!(third.retired.len(), 1, "{third:?}");
    assert_eq!(registry.collect().len(), 1, "closed");
    assert!(registry.offer::<dyn Greeter>(&key).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_rust_guest_rebuilt_is_served_afresh_and_a_deleted_one_retires() {
    let Some(example) = rust_example() else {
        return;
    };
    let edited_target = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/guest-edited");
    let Some(edited) = build_rust_guest(&edited_target, Some("howdy")) else {
        return;
    };
    let first = example.join("target/wasm32-unknown-unknown/release/wasm_greeter_rust.wasm");
    rebuilt_is_served("edited_rust", &first, &edited, None);
}

#[test]
fn a_go_guest_rebuilt_is_served_afresh_and_a_deleted_one_retires() {
    let Some(example) = go_example() else {
        return;
    };
    let out = scratch("go-howdy").join("howdy.wasm");
    let Some(edited) = build_go_guest(&out, Some("howdy")) else {
        return;
    };
    rebuilt_is_served(
        "edited_go",
        &example.join("wasm_greeter_go.wasm"),
        &edited,
        Some("1.0.0"),
    );
}

#[test]
fn a_panic_is_contained_and_named() {
    let Some((rust, go)) = both() else {
        return;
    };
    let mut registry = serving(&config(&[&rust, &go], None));
    assert_eq!(registry.refresh().loaded.len(), 2);

    // Rust: the module aborts, its panic named; the next call meets a new
    // instance, with its state made afresh.
    let hello = greeter(&registry, "wasm_greeter_rust_hello");
    let counter = registry
        .offer::<dyn Counter>("wasm_greeter_rust_hello")
        .expect("served")
        .expect("as the trait")
        .shared()
        .expect("an instance");
    assert_eq!(counter.count(), 1);
    assert_eq!(counter.count(), 2);
    let failed = hello.greet("panic").unwrap_err();
    assert_eq!(failed.status, Status::GUATIAO_ERR_INTERNAL);
    // The panic's location is a path as the guest was built: `\` on Windows.
    let words = failed.message().replace('\\', "/");
    assert!(
        words.contains("trapped") && words.contains(": a provider bug, on request (src/lib.rs:"),
        "{words}"
    );
    assert_eq!(greeting(&hello.greet("bo").expect("greeted")), "hello, bo");
    assert_eq!(counter.count(), 1, "the instance was made afresh");

    // Go: the panic is recovered in the guest, which carries on.
    let hello = greeter(&registry, "wasm_greeter_go_hello");
    let counter = registry
        .offer::<dyn Counter>("wasm_greeter_go_hello")
        .expect("served")
        .expect("as the trait")
        .shared()
        .expect("an instance");
    assert_eq!(counter.count(), 1);
    let failed = hello.greet("panic").unwrap_err();
    assert_eq!(failed.status, Status::GUATIAO_ERR_INTERNAL);
    assert_eq!(failed.message(), "panic: a provider bug, on request");
    assert_eq!(counter.count(), 2, "the same instance");
}

/// Calls back into the greeter it listens to.
struct Echo(Arc<dyn Greeter>, Arc<Mutex<Vec<String>>>);

impl Listener for Echo {
    fn heard(&mut self, what: &str) {
        let back = match self.0.greet("again") {
            Ok(m) => greeting(&m).to_string(),
            Err(e) => format!("{:?}: {}", e.status, e.message()),
        };
        self.1.lock().unwrap().push(format!("{what} / {back}"));
    }
}

#[test]
fn a_call_back_into_a_busy_library_is_refused_and_another_library_answers() {
    let Some((rust, go)) = both() else {
        return;
    };
    let mut registry = serving(&config(&[&rust, &go], None));
    assert_eq!(registry.refresh().loaded.len(), 2);
    let rust_hello = greeter(&registry, "wasm_greeter_rust_hello");
    let go_hello = greeter(&registry, "wasm_greeter_go_hello");

    let heard = Arc::new(Mutex::new(Vec::new()));
    let mut same = Echo(Arc::clone(&rust_hello), Arc::clone(&heard));
    rust_hello.greet_into("ana", &mut same).expect("told");
    let mut other = Echo(Arc::clone(&go_hello), Arc::clone(&heard));
    rust_hello.greet_into("bo", &mut other).expect("told");
    let heard = heard.lock().unwrap();
    assert!(heard[0].starts_with("hello, ana / "), "{heard:?}");
    assert!(heard[0].contains("BUSY"), "{heard:?}");
    assert_eq!(heard[1], "hello, bo / hello, again");
    drop(heard);
    // The library carries on.
    assert_eq!(
        greeting(&rust_hello.greet("cy").expect("greeted")),
        "hello, cy"
    );
}

#[test]
fn a_go_guest_runs_under_an_operations_limit_that_covers_its_start() {
    let Some(go) = go_example() else {
        return;
    };
    let mut tight = serving(&config(&[&go], Some(limits(&[("operations", 100_000)]))));
    let changed = tight.refresh();
    assert!(changed.loaded.is_empty());
    assert!(
        changed.refused[0]
            .reason
            .contains("operations limit (100000)"),
        "Go's runtime starts in about 430 000: {changed:?}"
    );
    let mut room = serving(&config(
        &[&go],
        Some(limits(&[
            ("operations", 5_000_000),
            ("time_ms", 2_000),
            ("memory", 256 << 20),
        ])),
    ));
    assert!(room.refresh().refused.is_empty());
    let hello = greeter(&room, "wasm_greeter_go_hello");
    assert_eq!(
        greeting(&hello.greet("ana").expect("greeted")),
        "hello, ana"
    );
}

#[test]
fn a_c_host_calls_the_wasm_greeters() {
    let Some((rust, go)) = both() else {
        return;
    };
    let Some(file) = source_file() else {
        println!("skipped: this crate's file is not built beside the tests");
        return;
    };
    let Some(cc) = c_compiler::find() else {
        println!("skipped: no C compiler");
        return;
    };
    let Some(guatiao_dir) = std::env::var_os("GUATIAO_LIBRARY").map(std::path::PathBuf::from)
    else {
        println!(
            "skipped: GUATIAO_LIBRARY names no guatiao build (with `load`) for the C host to link"
        );
        return;
    };
    let exe = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(if cfg!(windows) {
        "wasm_c_host.exe"
    } else {
        "wasm_c_host"
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
    run.arg(&file).arg(kinds_file()).arg(&rust).arg(&go);
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
        stdout.contains("wasm_greeter_go_hello greets: hello, ana"),
        "{stdout}"
    );
}
