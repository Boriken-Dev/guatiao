// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Guests that push on the host rather than on themselves: a stack overflow
//! on a thread with little stack, answers that would swell when decoded,
//! tables that add up, a wide map taken whole, host work a loop of imports
//! asks for, two libraries calling each other from two
//! threads, a ticker that should sleep while nothing runs, and a time
//! limit that is the clock's, not the ticker's. What could
//! take the process down runs in a child process; what could hang runs
//! under a timeout.

mod common;

extern crate alloc;

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::path::Path;
use std::sync::{Barrier, Mutex, mpsc};
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

/// Keys in the wide map.
const WIDE: usize = 50_000;

/// A map of `n` keys, each to null, encoded by hand: building a wide
/// `Map` key by key is slow.
fn wide_map(n: usize) -> Vec<u8> {
    let mut out = header(7, n as u64);
    for i in 0..n {
        let key = format!("k{i:06}");
        out.extend(leb(key.len() as u64));
        out.extend_from_slice(key.as_bytes());
        out.push(1);
    }
    out
}

/// Stores `bytes` at `at`, one instruction a byte.
fn store(at: u32, bytes: &[u8]) -> String {
    bytes
        .iter()
        .enumerate()
        .map(|(i, b)| format!("(i32.store8 (i32.const {}) (i32.const {b}))", at + i as u32))
        .collect()
}

/// Where the wide map sits in the guest's memory.
const BIG_MAP_AT: u32 = 65536;
/// Where a large answer's header ends and its body starts.
const BODY_AT: u32 = 4400;

/// A guest whose providers do what their names say, each serving
/// `greeter` (`greet` and `check`).
fn guest() -> String {
    assert!(BIG_MAP_AT as usize + wide_map(WIDE).len() <= 8 << 16);
    let providers: [(&str, String); 12] = [
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
                wide_map(WIDE).len()
            )
        }),
        ("bigback", {
            format!(
                "(local.set $i (call $write_back (i32.const 0) (i32.const {BIG_MAP_AT}) (i32.const {})))
                 (if (local.get $i) (then (call $answer (i32.const 4200) (i32.const 3)) (return (local.get $i))))
                 (call $answer (i32.const 4300) (i32.const 2))
                 (return (i32.const 0))",
                wide_map(WIDE).len()
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
        big = escaped(&wide_map(WIDE)),
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
fn a_wide_map_is_taken_in_an_answer_and_a_write_back_charged_by_its_nodes() {
    let dir = pushy("bigmap");
    // Copying the map costs about 56 000 units (one per 8 bytes) and
    // decoding it 50 001 (one a node), each way.
    let mut registry = serving(&config(&[&dir], Some(limits(&[("operations", 300_000)]))));
    assert!(registry.refresh().refused.is_empty());
    assert_eq!(greet(&registry, "pushy_bigmap").as_deref(), Ok(""));
    let mut config = Map::new();
    let checked = greeter(&registry, "pushy_bigback")
        .check(&mut config)
        .map_err(|e| (e.status, e.message().to_string()));
    assert_eq!(checked, Ok(true));
    assert_eq!(config.len(), WIDE);
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
fn growing_past_a_module_maximum_fails_in_the_guest_and_counts_nothing() {
    let dir = scratch("grow-max");
    put_wat(
        &dir,
        "capped",
        &guest()
            .replace(
                "(memory (export \"memory\") 8)",
                "(table $t 1 10 funcref)\n(memory (export \"memory\") 8 16)",
            )
            .replace(
                "(local.set $t (i32.wrap_i64 (local.get $target)))",
                "(local.set $t (i32.wrap_i64 (local.get $target)))
                 (loop $again
                   (if (i32.ne (table.grow $t (ref.null func) (i32.const 100000)) (i32.const -1))
                     (then (unreachable)))
                   (if (i32.ne (memory.grow (i32.const 100)) (i32.const -1))
                     (then (unreachable)))
                   (local.set $i (i32.add (local.get $i) (i32.const 1)))
                   (br_if $again (i32.lt_u (local.get $i) (i32.const 100))))
                 (local.set $i (i32.const 0))",
            ),
    );
    let mut registry = serving(&config(&[&dir], Some(limits(&[("memory", 2 << 20)]))));
    assert!(registry.refresh().refused.is_empty());
    // Each grow is past the module's maximum: -1 to the guest, and nothing
    // the limit counts, however many times it asks.
    for _ in 0..3 {
        assert_eq!(greet(&registry, "capped_ok").as_deref(), Ok("ok"));
    }
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

// --- host work a guest asks for -------------------------------------------------

#[test]
fn host_work_counts_against_the_operations_limit() {
    let dir = pushy("host-work");
    let mut registry = serving(&config(
        &[&dir],
        Some(limits(&[("operations", 2000), ("memory", 32 << 20)])),
    ));
    assert!(registry.refresh().refused.is_empty());
    let (status, words) = greet(&registry, "pushy_sixteen").unwrap_err();
    assert_eq!(status, Status::GUATIAO_ERR_INTERNAL, "{words}");
    assert_eq!(words, "stopped at its operations limit (2000)");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Calls `key`'s `greet` on a thread of its own, and answers what it
/// answered, or `None` when it has not answered within `within`: a guest
/// the host does not stop runs on, and the test fails rather than hangs.
fn greet_within(
    registry: &Registry,
    key: &str,
    within: Duration,
) -> Option<Result<String, (Status, String)>> {
    let greeter = greeter(registry, key);
    let (told, answer) = mpsc::channel();
    std::thread::spawn(move || {
        let outcome = greeter
            .greet("ana")
            .map(|m| greeting(&m).to_string())
            .map_err(|e| (e.status, e.message().to_string()));
        let _ = told.send(outcome);
    });
    answer.recv_timeout(within).ok()
}

#[test]
fn bulk_instructions_count_against_the_operations_limit_by_their_length() {
    let dir = pushy("bulk");
    let mut registry = serving(&config(&[&dir], Some(limits(&[("operations", 1_000_000)]))));
    assert!(registry.refresh().refused.is_empty());
    // Without the charge each 64 MiB fill costs a few units: hours of
    // filling before a million.
    let outcome = greet_within(&registry, "pushy_fill", Duration::from_secs(10))
        .expect("stopped well within 10 s");
    assert_eq!(
        outcome,
        Err((
            Status::GUATIAO_ERR_INTERNAL,
            "stopped at its operations limit (1000000)".to_string()
        ))
    );
    assert_eq!(greet(&registry, "pushy_ok").as_deref(), Ok("ok"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn randomness_counts_against_the_operations_limit_by_its_length() {
    let dir = pushy("randoms");
    let mut registry = serving(&config(&[&dir], Some(limits(&[("operations", 1_000_000)]))));
    assert!(registry.refresh().refused.is_empty());
    let outcome = greet_within(&registry, "pushy_randoms", Duration::from_secs(10))
        .expect("stopped well within 10 s");
    assert_eq!(
        outcome,
        Err((
            Status::GUATIAO_ERR_INTERNAL,
            "stopped at its operations limit (1000000)".to_string()
        ))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn large_wasi_requests_are_refused_to_the_guest() {
    let dir = pushy("wasi-sizes");
    let mut registry = serving(&config(&[&dir], None));
    assert!(registry.refresh().refused.is_empty());
    for key in ["pushy_random", "pushy_iovs"] {
        let (_, words) = greet(&registry, key).unwrap_err();
        assert!(
            words.contains("status 28"),
            "{key}: EINVAL, not served: {words}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// --- two libraries, two threads ----------------------------------------------------

/// Waits for the other thread to be inside its library, then calls the
/// other library, and keeps its own until the other thread has tried too.
struct Crossing {
    other: Arc<dyn Greeter>,
    both_inside: Arc<Barrier>,
    both_tried: Arc<Barrier>,
    heard: Arc<Mutex<Vec<String>>>,
}

impl Listener for Crossing {
    fn heard(&mut self, _what: &str) {
        self.both_inside.wait();
        let answer = match self.other.greet("x") {
            Ok(m) => greeting(&m).to_string(),
            Err(e) => format!("{:?}", e.status),
        };
        self.heard.lock().unwrap().push(answer);
        self.both_tried.wait();
    }
}

#[test]
fn libraries_calling_each_other_from_two_threads_answer_busy_not_deadlock() {
    let (Some(rust), Some(go)) = (rust_example(), go_example()) else {
        return;
    };
    let mut registry = serving(&config(&[&rust, &go], None));
    assert_eq!(registry.refresh().loaded.len(), 2);
    let r = greeter(&registry, "wasm_greeter_rust_hello");
    let g = greeter(&registry, "wasm_greeter_go_hello");
    let both_inside = Arc::new(Barrier::new(2));
    let both_tried = Arc::new(Barrier::new(2));
    let heard = Arc::new(Mutex::new(Vec::new()));
    let (done, finished) = mpsc::channel();
    for (into, other) in [(Arc::clone(&r), Arc::clone(&g)), (g, r)] {
        let mut listener = Crossing {
            other,
            both_inside: Arc::clone(&both_inside),
            both_tried: Arc::clone(&both_tried),
            heard: Arc::clone(&heard),
        };
        let done = done.clone();
        std::thread::spawn(move || {
            let told = into.greet_into("ana", &mut listener).is_ok();
            let _ = done.send(told);
        });
    }
    for _ in 0..2 {
        let told = finished
            .recv_timeout(Duration::from_secs(20))
            .expect("both calls answer: no deadlock");
        assert!(told);
    }
    let heard = heard.lock().unwrap();
    assert_eq!(*heard, ["GUATIAO_ERR_BUSY", "GUATIAO_ERR_BUSY"]);
}

// --- the ticker ---------------------------------------------------------------------

/// How often the epoch thread was switched to, where Linux says.
fn ticker_switches() -> Option<u64> {
    for task in std::fs::read_dir("/proc/self/task").ok()?.flatten() {
        let comm = std::fs::read_to_string(task.path().join("comm")).unwrap_or_default();
        if comm.trim() == "guatiao-wasm-ep" {
            let status = std::fs::read_to_string(task.path().join("status")).ok()?;
            let line = status
                .lines()
                .find(|l| l.starts_with("voluntary_ctxt_switches"))?;
            return line.split_whitespace().nth(1)?.parse().ok();
        }
    }
    None
}

#[test]
fn the_epoch_ticker_sleeps_while_no_call_runs() {
    const NAME: &str = "the_epoch_ticker_sleeps_while_no_call_runs";
    if child_role().is_some() {
        let dir = pushy("ticker");
        let mut registry = serving(&config(&[&dir], Some(limits(&[("time_ms", 50)]))));
        assert!(registry.refresh().refused.is_empty());
        assert_eq!(greet(&registry, "pushy_ok").as_deref(), Ok("ok"));
        std::thread::sleep(Duration::from_millis(20));
        let before = ticker_switches();
        std::thread::sleep(Duration::from_millis(300));
        let after = ticker_switches();
        if let (Some(before), Some(after)) = (before, after) {
            println!("IDLE SWITCHES: {}", after - before);
        }
        let started = Instant::now();
        let (_, words) = greet(&registry, "pushy_spin").unwrap_err();
        println!("SPIN: {words} in {:?}", started.elapsed());
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    let (ok, out) = in_child(NAME, "ticker");
    assert!(ok, "{out}");
    assert!(
        out.contains("SPIN: stopped at its time_ms limit (50)"),
        "{out}"
    );
    if std::path::Path::new("/proc/self/task").is_dir() {
        let switches: u64 = after(&out, "IDLE SWITCHES: ").parse().expect("a number");
        assert!(
            switches < 5,
            "the ticker woke {switches} times in 300 ms idle"
        );
    }
}

#[test]
fn a_time_limit_is_the_milliseconds_of_the_call_alone() {
    let dir = pushy("clock");
    put_wat(&dir, "other", &guest());
    let mut registry = serving(&config(&[&dir], Some(limits(&[("time_ms", 300)]))));
    assert!(registry.refresh().refused.is_empty());
    let stopped = Err((
        Status::GUATIAO_ERR_INTERNAL,
        "stopped at its time_ms limit (300)".to_string(),
    ));
    let spin = |registry: &Registry| {
        let started = Instant::now();
        let outcome = greet(registry, "pushy_spin");
        (started.elapsed(), outcome)
    };
    // Alone: no later than the clock says, however long the ticker's
    // sleeps run (Windows' default timer wakes every 15.6 ms).
    let (took, outcome) = spin(&registry);
    assert_eq!(outcome, stopped);
    assert!(took < Duration::from_millis(900), "alone: {took:?}");
    // With calls starting in another library meanwhile: no sooner either.
    let other = greeter(&registry, "other_ok");
    let stop = Arc::new(AtomicBool::new(false));
    let calls = {
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            let mut n = 0u64;
            while !stop.load(Ordering::Relaxed) {
                other.greet("ana").expect("answers");
                n += 1;
            }
            n
        })
    };
    let (took, outcome) = spin(&registry);
    stop.store(true, Ordering::Relaxed);
    let n = calls.join().expect("joined");
    assert_eq!(outcome, stopped);
    assert!(
        took >= Duration::from_millis(290),
        "with {n} calls meanwhile: {took:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
