// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Guests written by hand in the text format, for what a compiler does not
//! emit: a trap, a loop with no end, memory grown past the limit, a stack
//! overflowed, an answer outside the guest's memory, a status guatiao does
//! not define, and modules that are not guatiao libraries. Each is
//! contained: the call or the library is refused in words, and the host
//! carries on.

mod common;

extern crate alloc;

use std::path::Path;

use common::*;
use guatiao::value::wire;
use guatiao::{List, Map, Status, Value};
use guatiao_bridge::BridgeSource;
use guatiao_source_wasm::WasmEngine;

/// What each provider does when its `greet` is called, by its position.
const BEHAVIOURS: [&str; 10] = [
    "count",
    "trap",
    "spin",
    "grow",
    "deep",
    "stray",
    "status",
    "garbage",
    "write_back",
    "stranger",
];

fn escaped(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("\\{b:02x}")).collect()
}

fn declaration(kinds: &[&str]) -> Vec<u8> {
    let mut providers = List::new();
    for name in BEHAVIOURS {
        let mut serves = Map::new();
        for kind in kinds {
            let mut slots = List::new();
            slots.push("greet");
            serves.set(kind, slots);
        }
        let mut p = Map::new();
        p.set("id", name);
        p.set("serves", serves);
        providers.push(p);
    }
    let mut d = Map::new();
    d.set("providers", providers);
    wire::to_bytes(&Value::from(d)).expect("encoded")
}

/// A guest answering `{"greeting": "<n>"}`, `n` its calls so far, from
/// provider 0, and misbehaving as `BEHAVIOURS` says from the others.
fn guest(kinds: &[&str]) -> String {
    let declared = declaration(kinds);
    let mut answer = Map::new();
    answer.set("greeting", "0");
    let answer = wire::to_bytes(&Value::from(answer)).expect("encoded");
    let digit = 4096 + answer.len() - 1;
    format!(
        r#"(module
  (import "guatiao" "answer" (func $answer (param i32 i32)))
  (import "guatiao" "write_back" (func $write_back (param i32 i32 i32) (result i32)))
  (import "guatiao" "object_call" (func $object_call (param i64 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (global $calls (mut i32) (i32.const 0))
  (data (i32.const 1024) "{decl}")
  (data (i32.const 4096) "{answer}")
  (data (i32.const 4200) "why")
  (func (export "guatiao_guest_abi") (result i32) (i32.const 1))
  (func (export "guatiao_guest_alloc") (param i32) (result i32) (i32.const 8192))
  (func (export "guatiao_guest_declare") (result i32)
    (call $answer (i32.const 1024) (i32.const {decl_len}))
    (i32.const 0))
  (func $deep (param i32) (result i32)
    (i32.add (call $deep (i32.add (local.get 0) (i32.const 1))) (i32.const 1)))
  (func (export "guatiao_guest_call") (param $object i32) (param $target i64) (param $req i32) (param $len i32) (result i32)
    (local $t i32)
    (local.set $t (i32.wrap_i64 (local.get $target)))
    (if (i32.eq (local.get $t) (i32.const 1)) (then unreachable))
    (if (i32.eq (local.get $t) (i32.const 2)) (then (loop $forever (br $forever))))
    (if (i32.eq (local.get $t) (i32.const 3)) (then (drop (memory.grow (i32.const 2000)))))
    (if (i32.eq (local.get $t) (i32.const 4)) (then (drop (call $deep (i32.const 0)))))
    (if (i32.eq (local.get $t) (i32.const 5))
      (then (call $answer (i32.const 65530) (i32.const 64)) (return (i32.const 0))))
    (if (i32.eq (local.get $t) (i32.const 6))
      (then (call $answer (i32.const 4200) (i32.const 3)) (return (i32.const 42))))
    (if (i32.eq (local.get $t) (i32.const 7))
      (then (call $answer (i32.const 4200) (i32.const 3)) (return (i32.const 0))))
    (if (i32.eq (local.get $t) (i32.const 8))
      (then (call $answer (i32.const 4200) (i32.const 3))
            (return (call $write_back (i32.const 0) (i32.const 4096) (i32.const {answer_len})))))
    (if (i32.eq (local.get $t) (i32.const 9))
      (then (call $answer (i32.const 4200) (i32.const 3))
            (return (call $object_call (i64.const 77) (i32.const 4096) (i32.const {answer_len})))))
    (global.set $calls (i32.add (global.get $calls) (i32.const 1)))
    (i32.store8 (i32.const {digit}) (i32.add (i32.const 48) (global.get $calls)))
    (call $answer (i32.const 4096) (i32.const {answer_len}))
    (i32.const 0))
)"#,
        decl = escaped(&declared),
        decl_len = declared.len(),
        answer = escaped(&answer),
        answer_len = answer.len(),
    )
}

/// A library of one module, `bytes`, under `id`.
fn put(dir: &Path, id: &str, bytes: &[u8], manifest_extra: &str) {
    std::fs::write(
        dir.join(format!("{id}.guatiao.toml")),
        format!("type = \"wasm\"\nmodule = \"{id}\"\nkinds = [\"greeter\"]\n{manifest_extra}"),
    )
    .expect("written");
    std::fs::write(dir.join(format!("{id}.wasm")), bytes).expect("written");
}

fn put_wat(dir: &Path, id: &str, text: &str, manifest_extra: &str) {
    let bytes = wat::parse_str(text).unwrap_or_else(|e| panic!("{id}: {e}"));
    put(dir, id, &bytes, manifest_extra);
}

const VERSION: &str = "version = \"1.0.0\"\n";

fn greeted(registry: &guatiao::library::Registry, key: &str) -> Result<String, (Status, String)> {
    greeter(registry, key)
        .greet("ana")
        .map(|m| greeting(&m).to_string())
        .map_err(|e| (e.status, e.message().to_string()))
}

#[test]
fn a_trap_is_contained_and_the_library_made_afresh() {
    let dir = scratch("trap");
    put_wat(&dir, "trapping", &guest(&["greeter"]), VERSION);
    let mut registry = serving(&config(&[&dir], None));
    let changed = registry.refresh();
    assert!(changed.refused.is_empty(), "{changed:?}");
    assert_eq!(greeted(&registry, "trapping_count").as_deref(), Ok("1"));
    assert_eq!(greeted(&registry, "trapping_count").as_deref(), Ok("2"));
    let (status, words) = greeted(&registry, "trapping_trap").unwrap_err();
    assert_eq!(status, Status::GUATIAO_ERR_INTERNAL);
    assert!(
        words.starts_with("trapping trapped") && words.contains("unreachable"),
        "{words}"
    );
    assert_eq!(
        greeted(&registry, "trapping_count").as_deref(),
        Ok("1"),
        "the next call meets an instance made afresh"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn runaway_code_is_stopped_at_its_limits_and_the_host_carries_on() {
    let dir = scratch("limits");
    put_wat(&dir, "runaway", &guest(&["greeter"]), VERSION);
    let mut registry = serving(&config(
        &[&dir],
        Some(limits(&[("operations", 1_000_000), ("memory", 16 << 20)])),
    ));
    assert!(registry.refresh().refused.is_empty());

    let (status, words) = greeted(&registry, "runaway_spin").unwrap_err();
    assert_eq!(status, Status::GUATIAO_ERR_INTERNAL);
    assert_eq!(words, "stopped at its operations limit (1000000)");

    let (status, words) = greeted(&registry, "runaway_grow").unwrap_err();
    assert_eq!(status, Status::GUATIAO_ERR_ALLOC);
    assert_eq!(words, "stopped at its memory limit (16777216)");

    let (status, words) = greeted(&registry, "runaway_deep").unwrap_err();
    assert_eq!(status, Status::GUATIAO_ERR_INTERNAL);
    assert!(words.contains("call stack exhausted"), "{words}");

    assert_eq!(greeted(&registry, "runaway_count").as_deref(), Ok("1"));

    // Unlimited, the same memory grows.
    let mut free = serving(&config(&[&dir], None));
    assert!(free.refresh().refused.is_empty());
    assert_eq!(greeted(&free, "runaway_grow").as_deref(), Ok("1"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn time_stops_a_call_too() {
    let dir = scratch("time");
    put_wat(&dir, "slow", &guest(&["greeter"]), VERSION);
    let mut registry = serving(&config(&[&dir], Some(limits(&[("time_ms", 50)]))));
    assert!(registry.refresh().refused.is_empty());
    let started = std::time::Instant::now();
    let (status, words) = greeted(&registry, "slow_spin").unwrap_err();
    assert_eq!(status, Status::GUATIAO_ERR_INTERNAL);
    assert_eq!(words, "stopped at its time_ms limit (50)");
    assert!(started.elapsed() < core::time::Duration::from_secs(5));
    assert_eq!(greeted(&registry, "slow_count").as_deref(), Ok("1"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn what_a_guest_answers_wrong_is_refused_in_words() {
    let dir = scratch("wrong");
    put_wat(&dir, "wrong", &guest(&["greeter"]), VERSION);
    let mut registry = serving(&config(&[&dir], None));
    assert!(registry.refresh().refused.is_empty());

    let (status, words) = greeted(&registry, "wrong_stray").unwrap_err();
    assert_eq!(status, Status::GUATIAO_ERR_INTERNAL);
    assert!(words.contains("outside its memory"), "{words}");

    let (status, words) = greeted(&registry, "wrong_status").unwrap_err();
    assert_eq!(status, Status::GUATIAO_ERR_INTERNAL);
    assert_eq!(
        words,
        "wrong answered status 42, which guatiao does not define: why"
    );

    let (status, words) = greeted(&registry, "wrong_garbage").unwrap_err();
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
    assert!(words.contains("not a value"), "{words}");

    let (status, _) = greeted(&registry, "wrong_write_back").unwrap_err();
    assert_eq!(
        status,
        Status::GUATIAO_ERR_WRONG_KIND,
        "`greet`'s name is not lent: nothing to write back"
    );

    let (status, _) = greeted(&registry, "wrong_stranger").unwrap_err();
    assert_eq!(
        status,
        Status::GUATIAO_ERR_GONE,
        "handle 77 was never passed"
    );

    assert_eq!(greeted(&registry, "wrong_count").as_deref(), Ok("1"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_module_that_is_not_a_guatiao_library_is_refused_before_it_runs() {
    let dir = scratch("shapes");
    let good = guest(&["greeter"]);
    put(&dir, "not_wasm", b"hello", VERSION);
    put(&dir, "a_component", b"\0asm\x0d\0\x01\0", VERSION);
    let cases: [(&str, String, &str); 11] = [
        (
            "no_call",
            good.replace("(export \"guatiao_guest_call\")", ""),
            VERSION,
        ),
        (
            "wrong_alloc",
            good.replace(
                "(func (export \"guatiao_guest_alloc\") (param i32)",
                "(func (export \"guatiao_guest_alloc\") (param i64)",
            ),
            VERSION,
        ),
        (
            "imports_env",
            good.replace("(memory (export", "(import \"env\" \"x\" (func))\n  (memory (export"),
            VERSION,
        ),
        (
            "imports_files",
            good.replace(
                "(memory (export",
                "(import \"wasi_snapshot_preview1\" \"path_open\" (func (param i32 i32 i32 i32 i32 i64 i64 i32 i32) (result i32)))\n  (memory (export",
            ),
            VERSION,
        ),
        (
            "abi_two",
            good.replace("(result i32) (i32.const 1))", "(result i32) (i32.const 2))"),
            VERSION,
        ),
        (
            "start_loops",
            good.replace(
                "(memory (export",
                "(func (export \"_initialize\") (loop $l (br $l)))\n  (memory (export",
            ),
            VERSION,
        ),
        (
            "declare_traps",
            good.replace(
                "(call $answer (i32.const 1024)",
                "unreachable (call $answer (i32.const 1024)",
            ),
            VERSION,
        ),
        (
            "declare_garbage",
            good.replace(
                "(call $answer (i32.const 1024) (i32.const",
                "(call $answer (i32.const 4200) (i32.const 3)) (drop (i32.const",
            ),
            VERSION,
        ),
        ("unlisted_kind", guest(&["greeter", "counter"]), VERSION),
        (
            "version_mismatch",
            good.replace("(memory (export", "(@custom \"guatiao.version\" \"2.0.0\")\n  (memory (export"),
            VERSION,
        ),
        ("unversioned", good.clone(), ""),
    ];
    for (id, text, extra) in &cases {
        put_wat(&dir, id, text, extra);
    }
    // The version alone, read from the module.
    put_wat(
        &dir,
        "versioned",
        &good.replace(
            "(memory (export",
            "(@custom \"guatiao.version\" \"3.1.4\")\n  (memory (export",
        ),
        "",
    );
    let mut registry = serving(&config(&[&dir], Some(limits(&[("operations", 1_000_000)]))));
    let changed = registry.refresh();
    assert_eq!(
        changed
            .loaded
            .iter()
            .map(|l| l.id.as_str())
            .collect::<Vec<_>>(),
        ["versioned"],
        "{changed:?}"
    );
    assert_eq!(registry.loaded()[0].version, "3.1.4");
    let reason = |id: &str| -> String {
        changed
            .refused
            .iter()
            .find(|r| r.id.as_deref() == Some(id))
            .map(|r| r.reason.clone())
            .unwrap_or_else(|| panic!("{id} refused: {changed:?}"))
    };
    let expect = [
        ("not_wasm", "is not a WebAssembly module"),
        ("a_component", "is not a WebAssembly module"),
        ("no_call", "exports no `guatiao_guest_call`"),
        ("wrong_alloc", "`guatiao_guest_alloc` is `(i64) -> (i32)`"),
        ("imports_env", "imports `env.x`"),
        (
            "imports_files",
            "imports `wasi_snapshot_preview1.path_open`",
        ),
        ("abi_two", "speaks guatiao ABI 2"),
        ("start_loops", "operations limit (1000000)"),
        ("declare_traps", "trapped"),
        ("declare_garbage", "the declaration is not a value"),
        (
            "unlisted_kind",
            "serves `counter`, which the manifest does not list",
        ),
        (
            "version_mismatch",
            "states version 2.0.0, and its manifest 1.0.0",
        ),
        ("unversioned", "states no version"),
    ];
    for (id, words) in expect {
        let why = reason(id);
        assert!(why.contains(words), "{id}: {why}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_limit_wasm_cannot_keep_is_refused_at_start() {
    let mut config = Map::new();
    config.set("limits", limits(&[("depth", 100)]));
    let Err(refused) = BridgeSource::<WasmEngine>::shared(&config) else {
        panic!("a source with a `depth` limit started");
    };
    assert!(refused.to_string().contains("`depth`"), "{refused}");
}
