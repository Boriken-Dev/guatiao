// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Guests written by hand in the text format, for what a compiler does not
//! emit: a trap, a loop with no end, memory grown past the limit, a stack
//! overflowed, an answer outside the guest's memory, a status guatiao does
//! not define, and modules that are not guatiao libraries. Each is
//! contained: the call or the library is refused in words, and the host
//! carries on. And a guest whose calls into a host object are refused
//! before they run, keeping the objects it passed.

mod common;

extern crate alloc;

use std::path::Path;

use common::*;
use guatiao::library::{Object, ProviderError};
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
        dir.join(format!("{id}.guatiao")),
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

/// A host object a guest calls, handed to it by the host.
#[guatiao::kind(object)]
pub trait Echo: Send {
    /// Answers the text back.
    fn say(&mut self, what: &str) -> Result<String, ProviderError>;
    /// Takes `other` over, and answers `times`.
    fn adopt(&mut self, other: Object<dyn Echo>, times: u8) -> Result<i64, ProviderError>;
    /// Takes both over.
    fn both(&mut self, a: Object<dyn Echo>, b: Object<dyn Echo>) -> Result<i64, ProviderError>;
}

/// What the guest serves: it is lent one echo and handed another.
#[guatiao::kind]
pub trait Relay: Send + Sync {
    /// Passes `other` to slots of `target` in calls refused before they
    /// run, then calls `other`: what it said.
    fn refused(
        &self,
        target: &mut dyn Echo,
        other: Object<dyn Echo>,
    ) -> Result<String, ProviderError>;
}

/// An echo recording what it was told, and when it is destroyed.
struct Parrot(alloc::sync::Arc<std::sync::Mutex<Vec<String>>>);

impl Echo for Parrot {
    fn say(&mut self, what: &str) -> Result<String, ProviderError> {
        self.0.lock().unwrap().push(what.to_string());
        Ok(format!("echo {what}"))
    }
    fn adopt(&mut self, other: Object<dyn Echo>, times: u8) -> Result<i64, ProviderError> {
        drop(other);
        Ok(i64::from(times))
    }
    fn both(&mut self, a: Object<dyn Echo>, b: Object<dyn Echo>) -> Result<i64, ProviderError> {
        drop((a, b));
        Ok(2)
    }
}

impl Drop for Parrot {
    fn drop(&mut self) {
        self.0.lock().unwrap().push("<destroyed>".to_string());
    }
}

/// A guest serving `relay`: `target` arrives as handle 1 and `other` as
/// handle 2. It calls `target.adopt(other, "twice")` and
/// `target.both(other, other)`, each to be refused as a bad value (another
/// status comes back as status 100 more), then `other.say("still here")`,
/// lets `other` go, and answers what `say` answered, or its refusal.
fn relaying_guest() -> String {
    let mut serves = Map::new();
    serves.set("relay", texts(&["refused"]));
    let mut provider = Map::new();
    provider.set("id", "relay");
    provider.set("serves", serves);
    let mut providers = List::new();
    providers.push(provider);
    let mut declared = Map::new();
    declared.set("providers", providers);
    let declared = wire::to_bytes(&Value::from(declared)).expect("encoded");
    let request = |method: &str, args: &[Value]| {
        let mut list = List::new();
        list.push(method);
        for arg in args {
            list.push(arg.clone());
        }
        wire::to_bytes(&Value::from(list)).expect("encoded")
    };
    let other = Value::from(2u64);
    let adopt = request("adopt", &[other.clone(), Value::from("twice")]);
    let both = request("both", &[other.clone(), other]);
    let say = request("say", &[Value::from("still here")]);
    let bad = u32::from(Status::GUATIAO_ERR_BAD_VALUE);
    format!(
        r#"(module
  (import "guatiao" "answer" (func $answer (param i32 i32)))
  (import "guatiao" "object_call" (func $object_call (param i64 i32 i32) (result i32)))
  (import "guatiao" "result" (func $result (param i32 i32) (result i32)))
  (import "guatiao" "drop" (func $drop (param i64)))
  (memory (export "memory") 1)
  (data (i32.const 1024) "{declared}")
  (data (i32.const 2048) "{adopt}")
  (data (i32.const 2560) "{both}")
  (data (i32.const 3072) "{say}")
  (func (export "guatiao_guest_abi") (result i32) (i32.const 1))
  (func (export "guatiao_guest_alloc") (param i32) (result i32) (i32.const 8192))
  (func (export "guatiao_guest_declare") (result i32)
    (call $answer (i32.const 1024) (i32.const {declared_len}))
    (i32.const 0))
  (func $said (param $status i32) (result i32)
    (call $answer (i32.const 4096) (call $result (i32.const 4096) (i32.const 1024)))
    (local.get $status))
  (func $refused (param $status i32) (result i32)
    (if (result i32) (i32.eq (local.get $status) (i32.const {bad}))
      (then (i32.const 0))
      (else (call $said (i32.add (i32.const 100) (local.get $status))))))
  (func (export "guatiao_guest_call") (param i32 i64 i32 i32) (result i32)
    (local $s i32)
    (local.set $s (call $refused (call $object_call (i64.const 1) (i32.const 2048) (i32.const {adopt_len}))))
    (if (local.get $s) (then (return (local.get $s))))
    (local.set $s (call $refused (call $object_call (i64.const 1) (i32.const 2560) (i32.const {both_len}))))
    (if (local.get $s) (then (return (local.get $s))))
    (local.set $s (call $said (call $object_call (i64.const 2) (i32.const 3072) (i32.const {say_len}))))
    (call $drop (i64.const 2))
    (local.get $s))
)"#,
        declared = escaped(&declared),
        declared_len = declared.len(),
        adopt = escaped(&adopt),
        adopt_len = adopt.len(),
        both = escaped(&both),
        both_len = both.len(),
        say = escaped(&say),
        say_len = say.len(),
    )
}

#[test]
fn a_call_refused_before_it_runs_leaves_the_guests_objects_alive() {
    let dir = scratch("refused");
    let kinds = dir.join("relay.kinds.json");
    std::fs::write(
        &kinds,
        format!(
            "{{\"kinds\":[{},{}]}}",
            EchoVtable::description(),
            RelayVtable::description()
        ),
    )
    .expect("written");
    let bytes = wat::parse_str(relaying_guest()).expect("the guest parses");
    std::fs::write(dir.join("relaying.wasm"), bytes).expect("written");
    std::fs::write(
        dir.join("relaying.guatiao"),
        format!("type = \"wasm\"\nmodule = \"relaying\"\nkinds = [\"relay\"]\n{VERSION}"),
    )
    .expect("written");
    let mut config = config(&[&dir], None);
    config.set("kinds", texts(&[&kinds.display().to_string()]));
    let mut registry = serving(&config);
    let changed = registry.refresh();
    assert!(changed.refused.is_empty(), "{changed:?}");
    let relay: alloc::sync::Arc<dyn Relay> = registry
        .offer::<dyn Relay>("relaying_relay")
        .expect("served")
        .expect("as the trait")
        .shared()
        .expect("an instance");

    let mut target = Parrot(Default::default());
    let seen = alloc::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let other = Parrot(alloc::sync::Arc::clone(&seen)).into_object();
    let said = relay
        .refused(&mut target, other)
        .map_err(|e| (e.status, e.message().to_string()));
    assert_eq!(said.as_deref(), Ok("echo still here"));
    assert_eq!(
        *seen.lock().unwrap(),
        ["still here", "<destroyed>"],
        "the refused calls left the object to the guest, which let it go after"
    );
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

/// The schema the configured guest's providers declare: `{"word": text}`.
fn word_schema() -> Map {
    let mut word = Map::new();
    word.set("type", "string");
    let mut properties = Map::new();
    properties.set("word", word);
    let mut schema = Map::new();
    schema.set("type", "object");
    schema.set("properties", properties);
    schema.set("required", texts(&["word"]));
    schema.set("additionalProperties", false);
    schema
}

/// A guest whose providers are built from a configuration: `tuned` (also
/// callable as itself, answering how many instances were destroyed),
/// `only` (instances only) and `down` (never available; its own `greet`
/// traps). An instance's `greet` answers the configuration it was built
/// from; a configuration over 1000 bytes is refused. Without `create`,
/// the module exports no `guatiao_guest_create`.
fn configured_guest(create: bool) -> String {
    let mut providers = List::new();
    for (id, config, only, asks) in [
        ("tuned", true, false, true),
        ("only", true, true, false),
        ("down", false, false, true),
    ] {
        let mut slots = List::new();
        slots.push("greet");
        let mut serves = Map::new();
        serves.set("greeter", slots);
        let mut p = Map::new();
        p.set("id", id);
        p.set("serves", serves);
        if config {
            p.set("config", word_schema());
        }
        if only {
            p.set("instances_only", true);
        }
        if asks {
            p.set("may_be_unavailable", true);
        }
        providers.push(p);
    }
    let mut d = Map::new();
    d.set("providers", providers);
    let declared = wire::to_bytes(&Value::from(d)).expect("encoded");
    let mut answer = Map::new();
    answer.set("greeting", "0");
    let answer = wire::to_bytes(&Value::from(answer)).expect("encoded");
    let digit = 4096 + answer.len() - 1;
    let unplugged = wire::to_bytes(&Value::from("unplugged")).expect("encoded");
    let ids: Vec<Vec<u8>> = (1..=9i64)
        .map(|i| wire::to_bytes(&Value::from(i)).expect("encoded"))
        .collect();
    let id_len = ids[0].len();
    assert!(ids.iter().all(|i| i.len() == id_len));
    let create = if create {
        format!(
            r#"(func (export "guatiao_guest_create") (param $p i64) (param $cfg i32) (param $len i32) (result i32)
    (if (i32.gt_u (local.get $len) (i32.const 1000))
      (then (call $answer (i32.const 4300) (i32.const 8)) (return (i32.const 7))))
    (global.set $made (i32.add (global.get $made) (i32.const 1)))
    (memory.copy (i32.add (i32.const 16384) (i32.mul (global.get $made) (i32.const 1024)))
                 (local.get $cfg) (local.get $len))
    (i32.store (i32.add (i32.const 12288) (i32.mul (global.get $made) (i32.const 4))) (local.get $len))
    (call $answer (i32.add (i32.const 5000) (i32.mul (i32.sub (global.get $made) (i32.const 1)) (i32.const {id_len})))
                  (i32.const {id_len}))
    (i32.const 0))"#
        )
    } else {
        String::new()
    };
    format!(
        r#"(module
  (import "guatiao" "answer" (func $answer (param i32 i32)))
  (memory (export "memory") 1)
  (global $made (mut i32) (i32.const 0))
  (global $destroyed (mut i32) (i32.const 0))
  (data (i32.const 1024) "{decl}")
  (data (i32.const 4096) "{answer}")
  (data (i32.const 4200) "{unplugged}")
  (data (i32.const 4300) "too long")
  (data (i32.const 5000) "{ids}")
  (func (export "guatiao_guest_abi") (result i32) (i32.const 1))
  (func (export "guatiao_guest_alloc") (param i32) (result i32) (i32.const 8192))
  (func (export "guatiao_guest_declare") (result i32)
    (call $answer (i32.const 1024) (i32.const {decl_len}))
    (i32.const 0))
  {create}
  (func (export "guatiao_guest_destroy") (param $i i64)
    (global.set $destroyed (i32.add (global.get $destroyed) (i32.const 1))))
  (func (export "guatiao_guest_available") (param $p i64) (result i32)
    (if (i64.eq (local.get $p) (i64.const 2))
      (then (call $answer (i32.const 4200) (i32.const {unplugged_len}))))
    (i32.const 0))
  (func (export "guatiao_guest_call") (param $object i32) (param $target i64) (param $req i32) (param $len i32) (result i32)
    (local $k i32)
    (if (i32.eq (local.get $object) (i32.const 2))
      (then
        (local.set $k (i32.wrap_i64 (local.get $target)))
        (call $answer (i32.add (i32.const 16384) (i32.mul (local.get $k) (i32.const 1024)))
                      (i32.load (i32.add (i32.const 12288) (i32.mul (local.get $k) (i32.const 4)))))
        (return (i32.const 0))))
    (if (i64.eq (local.get $target) (i64.const 2)) (then unreachable))
    (i32.store8 (i32.const {digit}) (i32.add (i32.const 48) (global.get $destroyed)))
    (call $answer (i32.const 4096) (i32.const {answer_len}))
    (i32.const 0))
)"#,
        decl = escaped(&declared),
        decl_len = declared.len(),
        answer = escaped(&answer),
        answer_len = answer.len(),
        unplugged = escaped(&unplugged),
        unplugged_len = unplugged.len(),
        ids = ids.iter().map(|i| escaped(i)).collect::<String>(),
    )
}

fn word(answer: &Map) -> &str {
    answer
        .get("word")
        .and_then(guatiao::value::convert::TryAsRef::<str>::try_as_ref)
        .unwrap_or("")
}

fn configuration(word: &str) -> Value {
    let mut config = Map::new();
    config.set("word", word);
    Value::from(config)
}

#[test]
fn a_guest_builds_instances_from_a_configuration_and_says_whether_it_runs() {
    use greeter_kind::Greeter;
    use guatiao::library::kind::InstantiateError;

    let dir = scratch("configured");
    put_wat(&dir, "configured", &configured_guest(true), VERSION);
    let mut registry = serving(&config(&[&dir], None));
    let changed = registry.refresh();
    assert!(changed.refused.is_empty(), "{changed:?}");

    let offer = registry
        .offer::<dyn Greeter>("configured_tuned")
        .expect("served")
        .expect("as the trait");
    assert!(offer.builds_instances());
    assert!(offer.config_schema().is_some(), "the schema is offered");
    let itself = offer.default_instance().expect("callable as itself");
    assert_eq!(greeting(&itself.greet("ana").expect("greeted")), "0");

    let howdy = offer.instantiate(&configuration("howdy")).expect("built");
    let hey = offer.instantiate(&configuration("hey")).expect("built");
    assert_eq!(word(&howdy.greet("ana").expect("greeted")), "howdy");
    assert_eq!(word(&hey.greet("bo").expect("greeted")), "hey");

    // Checked against the schema before the guest is asked.
    let mut misspelled = Map::new();
    misspelled.set("wrod", "x");
    assert!(matches!(
        offer.instantiate(&Value::from(misspelled)),
        Err(InstantiateError::Invalid(_))
    ));
    // The guest's refusal crosses in its words.
    match offer.instantiate(&configuration(&"x".repeat(2000))) {
        Err(InstantiateError::Refused(e)) => {
            assert_eq!(e.status, Status::GUATIAO_ERR_BAD_VALUE);
            assert_eq!(e.message(), "too long");
        }
        Ok(_) => panic!("built from a configuration the guest refuses"),
        Err(other) => panic!("{other:?}"),
    }

    drop(howdy);
    assert_eq!(
        greeting(&itself.greet("ana").expect("greeted")),
        "1",
        "dropping the instance reached the guest once"
    );
    assert_eq!(word(&hey.greet("cy").expect("greeted")), "hey");

    let only = registry
        .offer::<dyn Greeter>("configured_only")
        .expect("served")
        .expect("as the trait");
    assert!(only.default_instance().is_none(), "instances only");
    let built = only.instantiate(&configuration("yo")).expect("built");
    assert_eq!(word(&built.greet("di").expect("greeted")), "yo");

    let provider = |key: &str| registry.provider(key).expect("registered");
    assert_eq!(provider("configured_tuned").available(), Ok(()));
    assert_eq!(provider("configured_down").available(), Err("unplugged"));
    assert_eq!(
        provider("configured_only").available(),
        Ok(()),
        "never asked"
    );

    // A trap discards the guest's store, and the instances with it.
    let down = greeter(&registry, "configured_down");
    let stopped = down.greet("ana").unwrap_err();
    assert_eq!(stopped.status, Status::GUATIAO_ERR_INTERNAL);
    let gone = hey.greet("ed").unwrap_err();
    assert_eq!(gone.status, Status::GUATIAO_ERR_GONE, "{}", gone.message());
    drop((hey, built, itself, down));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_configuration_declared_without_create_refuses_the_library() {
    let dir = scratch("uncreated");
    put_wat(&dir, "uncreated", &configured_guest(false), VERSION);
    let mut registry = serving(&config(&[&dir], None));
    let changed = registry.refresh();
    let words = format!("{:?}", changed.refused);
    assert!(
        words.contains("`tuned` declares `config`") && words.contains("guatiao_guest_create"),
        "{words}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
