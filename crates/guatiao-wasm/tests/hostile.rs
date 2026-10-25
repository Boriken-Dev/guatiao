// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Guests that push on the host rather than on themselves: a stack overflow
//! on a thread with little stack, answers that would swell when decoded,
//! tables that add up and maps that cost the square of their keys. What
//! could take the process down runs in a child process.

mod common;

extern crate alloc;

use alloc::sync::Arc;
use core::time::Duration;
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

use common::*;
use greeter_kind::{Greeter, Listener};
use guatiao::library::Registry;
use guatiao::value::wire;
use guatiao::{List, Map, Status, Value};

fn escaped(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("\\{b:02x}")).collect()
}

fn leb(mut n: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let byte = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(byte);
            return out;
        }
        out.push(byte | 0x80);
    }
}

/// The header of a node with `tag` and a length or count of `n`.
fn header(tag: u8, n: u64) -> Vec<u8> {
    let mut out = vec![tag];
    out.extend(leb(n));
    out
}

/// A map of `n` keys, each to null.
fn wide_map(n: usize) -> Vec<u8> {
    let mut map = Map::new();
    for i in 0..n {
        map.set(&format!("k{i:06}"), Value::null());
    }
    wire::to_bytes(&Value::from(map)).expect("encoded")
}

/// Stores `bytes` at `at`, one instruction a byte.
fn store(at: u32, bytes: &[u8]) -> String {
    bytes
        .iter()
        .enumerate()
        .map(|(i, b)| format!("(i32.store8 (i32.const {}) (i32.const {b}))", at + i as u32))
        .collect()
}

/// Where the 40 000-key map sits in the guest's memory.
const BIG_MAP_AT: u32 = 65536;
/// Where the 4000-key map sits, after it.
const MAP4K_AT: u32 = 7 << 16;
/// Where a large answer's header ends and its body starts.
const BODY_AT: u32 = 4400;

/// A guest whose providers do what their names say, each serving
/// `greeter` (`greet` and `check`).
fn guest() -> String {
    assert!(BIG_MAP_AT as usize + wide_map(40_000).len() <= MAP4K_AT as usize);
    assert!(MAP4K_AT as usize + wide_map(4000).len() <= 8 << 16);
    let providers: [(&str, String); 13] = [
        (
            "ok",
            "(call $answer (i32.const 4096) (global.get $ok_len)) (return (i32.const 0))".into(),
        ),
        (
            "deep",
            "(drop (call $deep (i32.const 0))) (return (i32.const 0))".into(),
        ),
        ("nulls", {
            let h = header(6, 15_000_000);
            format!(
                "(drop (memory.grow (i32.const 248)))
                 (memory.fill (i32.const {BODY_AT}) (i32.const 1) (i32.const 15000000))
                 {}
                 (call $answer (i32.const {}) (i32.const {}))
                 (return (i32.const 0))",
                store(BODY_AT - h.len() as u32, &h),
                BODY_AT - h.len() as u32,
                h.len() + 15_000_000
            )
        }),
        ("bytes70", {
            let n = 70u64 << 20;
            let h = header(5, n);
            format!(
                "(drop (memory.grow (i32.const 1279)))
                 {}
                 (call $answer (i32.const {}) (i32.const {}))
                 (return (i32.const 0))",
                store(BODY_AT - h.len() as u32, &h),
                BODY_AT - h.len() as u32,
                h.len() as u64 + n
            )
        }),
        ("bigmap", {
            format!(
                "(call $answer (i32.const {BIG_MAP_AT}) (i32.const {}))
                 (return (i32.const 0))",
                wide_map(40_000).len()
            )
        }),
        ("bigback", {
            format!(
                "(local.set $i (call $write_back (i32.const 0) (i32.const {BIG_MAP_AT}) (i32.const {})))
                 (if (local.get $i) (then (call $answer (i32.const 4200) (i32.const 3)) (return (local.get $i))))
                 (call $answer (i32.const 4300) (i32.const 2))
                 (return (i32.const 0))",
                wide_map(40_000).len()
            )
        }),
        ("sixteen", {
            let n = 16u64 << 20;
            let h = header(5, n);
            format!(
                "(drop (memory.grow (i32.const 260)))
                 {}
                 (call $answer (i32.const {}) (i32.const {}))
                 (return (i32.const 0))",
                store(BODY_AT - h.len() as u32, &h),
                BODY_AT - h.len() as u32,
                h.len() as u64 + n
            )
        }),
        (
            "random",
            "(drop (memory.grow (i32.const 40)))
             (local.set $i (call $random_get (i32.const 1048576) (i32.const 2097152)))
             (if (local.get $i) (then (call $answer (i32.const 4200) (i32.const 3)) (return (local.get $i))))
             (call $answer (i32.const 4096) (global.get $ok_len)) (return (i32.const 0))"
                .into(),
        ),
        (
            "iovs",
            "(local.set $i (call $fd_write (i32.const 2) (i32.const 16384) (i32.const 5000) (i32.const 4600)))
             (if (local.get $i) (then (call $answer (i32.const 4200) (i32.const 3)) (return (local.get $i))))
             (call $answer (i32.const 4096) (global.get $ok_len)) (return (i32.const 0))"
                .into(),
        ),
        (
            "spin",
            "(loop $forever (br $forever)) (return (i32.const 0))".into(),
        ),
        (
            "fill",
            "(drop (memory.grow (i32.const 1016)))
             (loop $forever
               (memory.fill (i32.const 0) (i32.const 0) (i32.const 67108864))
               (br $forever))
             (return (i32.const 0))"
                .into(),
        ),
        (
            "randoms",
            "(drop (memory.grow (i32.const 40)))
             (loop $forever
               (drop (call $random_get (i32.const 1048576) (i32.const 1048576)))
               (br $forever))
             (return (i32.const 0))"
                .into(),
        ),
        ("map4k", {
            format!(
                "(call $answer (i32.const {MAP4K_AT}) (i32.const {}))
                 (return (i32.const 0))",
                wide_map(4000).len()
            )
        }),
    ];
    let mut list = List::new();
    for (id, _) in &providers {
        let mut slots = List::new();
        slots.push("greet");
        slots.push("check");
        let mut serves = Map::new();
        serves.set("greeter", slots);
        let mut p = Map::new();
        p.set("id", *id);
        p.set("serves", serves);
        list.push(p);
    }
    let mut d = Map::new();
    d.set("providers", list);
    let declared = wire::to_bytes(&Value::from(d)).expect("encoded");
    let mut ok = Map::new();
    ok.set("greeting", "ok");
    let ok = wire::to_bytes(&Value::from(ok)).expect("encoded");
    let dispatch: String = providers
        .iter()
        .enumerate()
        .map(|(k, (_, body))| {
            format!("(if (i32.eq (local.get $t) (i32.const {k})) (then {body}))\n")
        })
        .collect();
    format!(
        r#"(module
  (import "guatiao" "answer" (func $answer (param i32 i32)))
  (import "guatiao" "write_back" (func $write_back (param i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "random_get" (func $random_get (param i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_write" (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 8)
  (global $ok_len i32 (i32.const {ok_len}))
  (data (i32.const 1024) "{decl}")
  (data (i32.const 4096) "{ok}")
  (data (i32.const 4200) "why")
  (data (i32.const 4300) "\02\01")
  (data (i32.const {BIG_MAP_AT}) "{big}")
  (data (i32.const {MAP4K_AT}) "{map4k}")
  (func (export "guatiao_guest_abi") (result i32) (i32.const 1))
  (func (export "guatiao_guest_alloc") (param i32) (result i32) (i32.const 8192))
  (func (export "guatiao_guest_declare") (result i32)
    (call $answer (i32.const 1024) (i32.const {decl_len}))
    (i32.const 0))
  (func $deep (param i32) (result i32)
    (i32.add (call $deep (i32.add (local.get 0) (i32.const 1))) (i32.const 1)))
  (func (export "guatiao_guest_call") (param $object i32) (param $target i64) (param $req i32) (param $len i32) (result i32)
    (local $t i32) (local $i i32)
    (local.set $t (i32.wrap_i64 (local.get $target)))
    {dispatch}
    (i32.const 13))
)"#,
        decl = escaped(&declared),
        decl_len = declared.len(),
        ok = escaped(&ok),
        ok_len = ok.len(),
        big = escaped(&wide_map(40_000)),
        map4k = escaped(&wide_map(4000)),
    )
}

/// The guest above, as library `pushy` in a directory of its own.
fn pushy(name: &str) -> std::path::PathBuf {
    let dir = scratch(name);
    let bytes = wat::parse_str(guest()).expect("the guest parses");
    std::fs::write(
        dir.join("pushy.guatiao-wasm.toml"),
        "module = \"pushy\"\nkinds = [\"greeter\"]\nversion = \"1.0.0\"\n",
    )
    .expect("written");
    std::fs::write(dir.join("pushy.wasm"), bytes).expect("written");
    dir
}

fn greet(registry: &Registry, key: &str) -> Result<String, (Status, String)> {
    greeter(registry, key)
        .greet("ana")
        .map(|m| greeting(&m).to_string())
        .map_err(|e| (e.status, e.message().to_string()))
}

/// Runs `f` on a thread of `stack` bytes, answering what it answered.
fn on_small_thread<T: Send + 'static>(stack: usize, f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(stack)
        .spawn(f)
        .expect("the thread starts")
        .join()
        .expect("joined")
}

// --- a guest's stack and the host's -------------------------------------------

#[test]
fn a_guest_recursing_on_a_small_thread_is_stopped_not_the_host() {
    const NAME: &str = "a_guest_recursing_on_a_small_thread_is_stopped_not_the_host";
    if let Some(role) = child_role() {
        let stack: usize = role.parse().expect("a stack size");
        let dir = pushy(&format!("stack-{stack}"));
        let mut registry = serving(&config(&[&dir], None));
        assert!(registry.refresh().refused.is_empty());
        let deep = greeter(&registry, "pushy_deep");
        let ok = greeter(&registry, "pushy_ok");
        let outcome = on_small_thread(stack, move || {
            let stopped = deep
                .greet("ana")
                .map(|_| ())
                .map_err(|e| e.message().to_string());
            let after = ok.greet("bo").map(|m| greeting(&m).to_string());
            (stopped, after)
        });
        println!("STOPPED: {:?}", outcome.0);
        println!("AFTER: {:?}", outcome.1);
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    for stack in [256 << 10, 128 << 10] {
        let (ok, out) = in_child(NAME, &stack.to_string());
        assert!(
            ok && out.contains("STOPPED: Err(") && out.contains("call stack exhausted"),
            "a {stack}-byte thread:\n{out}"
        );
        assert!(out.contains("AFTER: Ok(\"ok\")"), "{out}");
    }
}

/// Calls another library's greeter from inside a call, and keeps what it
/// answered.
struct Calls(Arc<dyn Greeter>, Arc<Mutex<Vec<String>>>);

impl Listener for Calls {
    fn heard(&mut self, what: &str) {
        let inner = match self.0.greet("x") {
            Ok(m) => greeting(&m).to_string(),
            Err(e) => e.message().to_string(),
        };
        self.1.lock().unwrap().push(format!("{what} / {inner}"));
    }
}

#[test]
fn nested_libraries_on_a_small_thread_each_get_their_stack() {
    const NAME: &str = "nested_libraries_on_a_small_thread_each_get_their_stack";
    if child_role().is_some() {
        let Some(rust) = rust_example() else {
            return;
        };
        let dir = pushy("nested");
        let mut registry = serving(&config(&[&rust, &dir], None));
        assert!(registry.refresh().refused.is_empty());
        let outer = greeter(&registry, "wasm_greeter_rust_hello");
        let deep = greeter(&registry, "pushy_deep");
        let ok = greeter(&registry, "pushy_ok");
        let heard = Arc::new(Mutex::new(Vec::new()));
        let kept = Arc::clone(&heard);
        let told = on_small_thread(256 << 10, move || {
            let mut first = Calls(deep, Arc::clone(&kept));
            let a = outer
                .greet_into("ana", &mut first)
                .map_err(|e| e.message().to_string());
            let mut second = Calls(ok, kept);
            let b = outer
                .greet_into("bo", &mut second)
                .map_err(|e| e.message().to_string());
            (a, b)
        });
        println!("TOLD: {told:?}");
        println!("HEARD: {:?}", heard.lock().unwrap());
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    if rust_example().is_none() {
        return;
    }
    let (ok, out) = in_child(NAME, "nested");
    assert!(ok, "{out}");
    assert!(out.contains("TOLD: (Ok(()), Ok(()))"), "{out}");
    assert!(
        out.contains("hello, ana / ") && out.contains("call stack exhausted"),
        "{out}"
    );
    assert!(out.contains("hello, bo / ok"), "{out}");
}

// --- what the host decodes ------------------------------------------------------

/// What follows `label` on its line of a child's output (a test's own name
/// may come first on that line).
fn after<'a>(out: &'a str, label: &str) -> &'a str {
    let at = out
        .find(label)
        .unwrap_or_else(|| panic!("no `{label}`: {out}"));
    out[at + label.len()..].lines().next().unwrap_or("").trim()
}

/// The process's peak resident memory, in KiB, where Linux says.
fn peak_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

#[test]
fn an_answer_that_would_swell_on_the_host_is_refused_before_it_is_decoded() {
    const NAME: &str = "an_answer_that_would_swell_on_the_host_is_refused_before_it_is_decoded";
    if child_role().is_some() {
        let dir = pushy("swell");
        let mut registry = serving(&config(&[&dir], Some(limits(&[("memory", 16 << 20)]))));
        assert!(registry.refresh().refused.is_empty());
        let before = peak_kib();
        let started = Instant::now();
        let outcome = greet(&registry, "pushy_nulls");
        println!("OUTCOME: {outcome:?}");
        println!("TOOK: {:?}", started.elapsed());
        if let (Some(before), Some(after)) = (before, peak_kib()) {
            println!("GREW: {} KiB", after.saturating_sub(before));
        }
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    let (ok, out) = in_child(NAME, "swell");
    assert!(ok, "{out}");
    assert!(
        out.contains("OUTCOME: Err((GUATIAO_ERR_ALLOC,") && out.contains("more than"),
        "15 million nulls in 15 MiB: {out}"
    );
    if peak_kib().is_some() {
        let grew: u64 = after(&out, "GREW: ")
            .trim_end_matches(" KiB")
            .parse()
            .expect("a number");
        assert!(
            grew < 128 << 10,
            "the host grew {grew} KiB decoding it: {out}"
        );
    }
}

#[test]
fn without_a_memory_limit_the_host_still_takes_a_bounded_answer() {
    let dir = pushy("bounded");
    let mut registry = serving(&config(&[&dir], None));
    assert!(registry.refresh().refused.is_empty());
    let (status, words) = greet(&registry, "pushy_bytes70").unwrap_err();
    assert_eq!(status, Status::GUATIAO_ERR_ALLOC, "{words}");
    assert!(words.contains("more than"), "{words}");
    assert_eq!(greet(&registry, "pushy_ok").as_deref(), Ok("ok"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_map_of_forty_thousand_keys_is_refused_at_once_in_an_answer_and_a_write_back() {
    let dir = pushy("bigmap");
    let mut registry = serving(&config(
        &[&dir],
        // Enough for the copy (a unit per 8 bytes), not for the decoding.
        Some(limits(&[("operations", 100_000), ("time_ms", 100)])),
    ));
    assert!(registry.refresh().refused.is_empty());
    let started = Instant::now();
    let (status, words) = greet(&registry, "pushy_bigmap").unwrap_err();
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE, "{words}");
    assert!(words.contains("40000 keys"), "{words}");
    let mut config = Map::new();
    let refused = greeter(&registry, "pushy_bigback")
        .check(&mut config)
        .unwrap_err();
    assert_ne!(refused.status, Status::GUATIAO_OK);
    assert!(config.is_empty(), "nothing written back");
    let took = started.elapsed();
    assert!(took < Duration::from_secs(2), "took {took:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

// --- what the host's memory holds for a guest ----------------------------------

fn put_wat(dir: &Path, id: &str, text: &str) {
    std::fs::write(
        dir.join(format!("{id}.guatiao-wasm.toml")),
        format!("module = \"{id}\"\nkinds = [\"greeter\"]\nversion = \"1.0.0\"\n"),
    )
    .expect("written");
    std::fs::write(
        dir.join(format!("{id}.wasm")),
        wat::parse_str(text).unwrap_or_else(|e| panic!("{id}: {e}")),
    )
    .expect("written");
}

#[test]
fn tables_count_against_the_memory_limit_and_their_number_is_bounded() {
    let dir = scratch("tables");
    let good = guest();
    let anchor = "(memory (export";
    put_wat(
        &dir,
        "two_big",
        &good.replace(
            anchor,
            &format!("(table 100000 funcref)\n(table 100000 funcref)\n{anchor}"),
        ),
    );
    put_wat(
        &dir,
        "many",
        &good.replace(
            anchor,
            &format!("{}{anchor}", "(table 1 funcref)\n".repeat(40)),
        ),
    );
    put_wat(&dir, "plain", &good);
    let mut limited = serving(&config(&[&dir], Some(limits(&[("memory", 2 << 20)]))));
    let changed = limited.refresh();
    let reason = |id: &str| {
        changed
            .refused
            .iter()
            .find(|r| r.id.as_deref() == Some(id))
            .map(|r| r.reason.clone())
            .unwrap_or_else(|| panic!("{id} refused: {changed:?}"))
    };
    assert!(
        reason("two_big").contains("memory limit"),
        "{}",
        reason("two_big")
    );
    assert!(reason("many").contains("table"), "{}", reason("many"));
    assert!(
        changed.loaded.iter().any(|l| l.id == "plain"),
        "{changed:?}"
    );
    let mut free = serving(&config(&[&dir], None));
    let changed = free.refresh();
    assert!(
        changed
            .refused
            .iter()
            .any(|r| r.id.as_deref() == Some("many")),
        "forty tables without a limit: {changed:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn without_a_memory_limit_tables_are_still_bounded() {
    let dir = scratch("huge-table");
    let good = guest();
    let anchor = "(memory (export";
    put_wat(
        &dir,
        "huge_table",
        &good.replace(anchor, &format!("(table 20000000 funcref)\n{anchor}")),
    );
    put_wat(
        &dir,
        "growing",
        &good
            .replace(anchor, &format!("(table $t 1 funcref)\n{anchor}"))
            .replace(
                "(local.set $t (i32.wrap_i64 (local.get $target)))",
                "(local.set $t (i32.wrap_i64 (local.get $target)))
                 (drop (table.grow $t (ref.null func) (i32.const 20000000)))",
            ),
    );
    let mut registry = serving(&config(&[&dir], None));
    let changed = registry.refresh();
    let refused = changed
        .refused
        .iter()
        .find(|r| r.id.as_deref() == Some("huge_table"))
        .unwrap_or_else(|| panic!("a table of 20 million entries: {changed:?}"));
    assert!(
        refused.reason.contains("tables would take more than"),
        "{}",
        refused.reason
    );
    let (status, words) = greet(&registry, "growing_ok").unwrap_err();
    assert_eq!(status, Status::GUATIAO_ERR_ALLOC, "{words}");
    assert!(words.contains("tables would take more than"), "{words}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_module_past_the_size_bound_is_refused_unread() {
    let dir = scratch("huge");
    std::fs::write(
        dir.join("huge.guatiao-wasm.toml"),
        "module = \"huge\"\nkinds = [\"greeter\"]\nversion = \"1.0.0\"\n",
    )
    .expect("written");
    let file = std::fs::File::create(dir.join("huge.wasm")).expect("created");
    file.set_len(300 << 20).expect("a sparse file");
    drop(file);
    let mut registry = serving(&config(&[&dir], None));
    let changed = registry.refresh();
    let why = &changed.refused[0].reason;
    assert!(why.contains("larger than"), "{why}");
    let _ = std::fs::remove_dir_all(&dir);
}
