// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Guests that push on the host rather than on themselves: a stack overflow
//! on a thread with little stack. What could take the process down runs in
//! a child process.

mod common;

extern crate alloc;

use alloc::sync::Arc;
use std::sync::Mutex;

use common::*;
use greeter_kind::{Greeter, Listener};
use guatiao::value::wire;
use guatiao::{List, Map, Value};

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
