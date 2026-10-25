// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The source from a Rust host: the JavaScript greeters called through the
//! kinds' traits, a library edited and served afresh, one deleted and
//! retired, and runaway code stopped at its limits while the host carries
//! on.
//!
//! The source is served two ways: in this process, as a `BridgeSource` a
//! registry holds, and as this crate's own file through
//! `add_source_file`, which `cargo build` makes beside the tests.

extern crate alloc;

use alloc::sync::Arc;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use greeter_kind::{Conversation, Counter, Greeter, Listener, Wide};
use guatiao::library::{Candidate, Host, LocalLibrary, ProviderError, Registry, Source};
use guatiao::value::convert::TryAsRef;
use guatiao::{List, Map, Status};
use guatiao_bridge::BridgeSource;
use guatiao_libffi::Libffi;
use guatiao_source_js::JsEngine;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn kinds_file() -> String {
    repo()
        .join("examples/greeter_kind/include/greeter_kind.kinds.json")
        .display()
        .to_string()
}

/// The module `guatiao-kind` wrote, as an import specifier a library in a
/// scratch directory can use.
fn kinds_module() -> String {
    repo()
        .join("examples/greeter_kind/node/greeter_kind.js")
        .canonicalize()
        .expect("the kinds module")
        .display()
        .to_string()
        .replace('\\', "/")
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
    let path = debug.join(library_name("guatiao_source_js"));
    path.is_file().then_some(path)
}

fn texts(items: &[&str]) -> List {
    let mut list = List::new();
    for item in items {
        list.push(*item);
    }
    list
}

fn examples() -> PathBuf {
    repo().join("examples/js_provider")
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
struct Held(Arc<BridgeSource<JsEngine>>);

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
        BridgeSource::<JsEngine>::pinned(config, "js-test")
            .expect("QuickJS starts")
            .with_mechanism(Arc::new(Libffi)),
    );
    let mut registry = Registry::new("js-test-host", "1.0");
    registry.add_source("js", Held(source)).expect("added");
    registry
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("guatiao-js-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// A library of one module: `code` is its file, `id` its name. The
/// greeter kinds are one import away: `kinds()`.
fn put(dir: &Path, id: &str, manifest_extra: &str, code: &str) {
    std::fs::write(
        dir.join(format!("{id}.guatiao.toml")),
        format!(
            "type = \"js\"\nname = \"{id}\"\nmain = \"{id}.js\"\nkinds = [\"greeter\", \"counter\"]\npaths = [\".\", \"{}\"]\n{manifest_extra}",
            Path::new(&kinds_module()).parent().unwrap().display().to_string().replace('\\', "/")
        ),
    )
    .expect("written");
    std::fs::write(dir.join(format!("{id}.js")), code).expect("written");
}

/// The import of the greeter kinds a scratch library starts with.
fn preamble() -> String {
    format!(
        "import {{ BadValue }} from 'guatiao';\nimport {{ Provider }} from 'guatiao/provide';\nimport {{ Counter, Greeter }} from '{}';\n",
        kinds_module()
    )
}

/// The example greeter under another library name, in a directory of its
/// own.
fn copy_greeter(dir: &Path, id: &str) -> PathBuf {
    let code = std::fs::read_to_string(examples().join("quickjs_greeter/greeters.js"))
        .expect("the example")
        .replace(
            "'../../greeter_kind/node/greeter_kind.js'",
            &format!("'{}'", kinds_module()),
        );
    put(dir, id, "version = \"1.0.0\"\n", &code);
    dir.join(format!("{id}.js"))
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

/// Every slot of the example greeters, through the kinds' traits.
fn calls_every_slot(registry: &Registry) {
    let hello = greeter(registry, "quickjs_greeter_hello");
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

    // Not written in JavaScript: the kind's default runs, through `greet`.
    assert_eq!(hello.shout("cy"), "HELLO, CY");

    let heard = Arc::new(Mutex::new(Vec::new()));
    let mut lent = Heard(Arc::clone(&heard));
    hello.greet_into("ed", &mut lent).expect("told");
    assert_eq!(*heard.lock().unwrap(), ["hello, ed"]);
    drop(lent);

    let heard = Arc::new(Mutex::new(Vec::new()));
    let mut talk: guatiao::library::Object<dyn Conversation> = hello
        .start(Heard(Arc::clone(&heard)).into_object())
        .expect("a conversation JavaScript holds");
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
        .offer::<dyn Counter>("quickjs_greeter_hello")
        .expect("served")
        .expect("as the trait")
        .shared()
        .expect("an instance");
    let first = counter.count();
    assert_eq!(counter.count(), first + 1);

    let polite = greeter(registry, "quickjs_greeter_polite");
    assert_eq!(
        greeting(&polite.greet("fa").expect("greeted")),
        "good day, fa"
    );
    assert_eq!(
        polite.greet("").unwrap_err().message(),
        "nobody to greet, politely"
    );

    // From another thread: queued to the QuickJS thread.
    let from_another = {
        let hello = Arc::clone(&hello);
        std::thread::spawn(move || hello.greet("go").map(|m| greeting(&m).to_string()))
            .join()
            .expect("joined")
    };
    assert_eq!(from_another.expect("greeted"), "hello, go");
}

/// The tree slots, which take plain data here.
fn calls_the_tree_slots(registry: &Registry) {
    let trees = greeter(registry, "quickjs_trees_trees");
    let mut config = Map::new();
    assert!(trees.check(&mut config).expect("checked"));
    assert_eq!(
        config.get("name").and_then(TryAsRef::<str>::try_as_ref),
        Some("ana")
    );
    assert_eq!(
        config
            .get("checkedBy")
            .and_then(TryAsRef::<str>::try_as_ref),
        Some("JavaScript")
    );
    let mut named = Map::new();
    named.set("name", "di");
    assert_eq!(
        greeting(&trees.greet_from(named).expect("greeted")),
        "hello, di"
    );
    assert_eq!(
        trees.greet_from(Map::new()).unwrap_err().message(),
        "nobody to greet"
    );
}

#[test]
fn a_rust_host_calls_the_javascript_greeters() {
    let mut registry = serving(&config(&examples(), None));
    let changed = registry.refresh();
    assert!(changed.refused.is_empty(), "{changed:?}");
    assert_eq!(changed.loaded.len(), 2, "{changed:?}");
    let versions: Vec<_> = registry
        .loaded()
        .iter()
        .map(|l| l.version.clone())
        .collect();
    assert_eq!(versions, ["1.0.0", "1.0.0"], "read from package.json");
    calls_every_slot(&registry);
    calls_the_tree_slots(&registry);
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
    let mut registry = Registry::new("js-file-host", "1.0");
    let name = registry
        .add_source_file(&file, &config(&examples(), None))
        .unwrap_or_else(|e| panic!("the source was refused: {e}"));
    let changed = registry.refresh();
    assert!(changed.refused.is_empty(), "{changed:?}");
    assert!(changed.loaded.iter().all(|s| s.source == name));
    calls_every_slot(&registry);
    calls_the_tree_slots(&registry);
    // A scan names the file a source, not a library.
    let mut scanner = Registry::new("scanner", "1.0");
    let report =
        guatiao::library::scan_dir(&mut scanner, file.parent().unwrap()).expect("readable");
    assert!(
        report.skipped.iter().any(|(path, why)| path == &file
            && matches!(why, guatiao::library::Skipped::Source { serves } if serves == "js")),
        "{report:?}"
    );
}

#[test]
fn an_edit_serves_new_code_and_a_deleted_library_retires() {
    let dir = scratch("edit");
    let file = copy_greeter(&dir, "js_edited");
    let mut registry = serving(&config(&dir, None));
    assert_eq!(registry.refresh().loaded.len(), 1);
    let old = greeter(&registry, "js_edited_hello");
    assert_eq!(greeting(&old.greet("ana").expect("greeted")), "hello, ana");

    std::thread::sleep(core::time::Duration::from_millis(50));
    let code = std::fs::read_to_string(&file).expect("read");
    std::fs::write(
        &file,
        code.replace("GREETING = 'hello'", "GREETING = 'howdy'"),
    )
    .expect("edited");
    let second = registry.refresh();
    assert_eq!(second.loaded.len(), 1, "{second:?}");
    assert_eq!(second.retired.len(), 1, "{second:?}");
    let new = greeter(&registry, "js_edited_hello");
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
    std::fs::remove_file(dir.join("js_edited.guatiao.toml")).expect("removed");
    let third = registry.refresh();
    assert_eq!(third.retired.len(), 1, "{third:?}");
    assert_eq!(registry.collect().len(), 1, "closed");
    assert!(registry.offer::<dyn Greeter>("js_edited_hello").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn many_edits_leave_no_memory_behind() {
    let dir = scratch("leak");
    let file = copy_greeter(&dir, "js_leaky");
    let source = Arc::new(
        BridgeSource::<JsEngine>::pinned(&config(&dir, None), "js-leak-test")
            .expect("QuickJS starts")
            .with_mechanism(Arc::new(Libffi)),
    );
    let mut registry = Registry::new("js-leak-host", "1.0");
    registry
        .add_source("js", Held(Arc::clone(&source)))
        .expect("added");
    let code = std::fs::read_to_string(&file).expect("read");
    let used = |source: &BridgeSource<JsEngine>| source.run(|e| e.memory_used()).expect("asked");
    let mut round = |n: usize| {
        std::thread::sleep(core::time::Duration::from_millis(5));
        std::fs::write(
            &file,
            code.replace("GREETING = 'hello'", &format!("GREETING = 'hi{n}'")),
        )
        .expect("edited");
        let changed = registry.refresh();
        assert!(changed.refused.is_empty(), "{changed:?}");
        let hello = greeter(&registry, "js_leaky_hello");
        assert!(greeting(&hello.greet("a").expect("greeted")).starts_with(&format!("hi{n}")));
        let mut talk: guatiao::library::Object<dyn Conversation> = hello
            .start(Heard(Arc::new(Mutex::new(Vec::new()))).into_object())
            .expect("a conversation");
        talk.say("x").expect("said");
        drop(talk);
        drop(hello);
        registry.collect();
    };
    for n in 0..5 {
        round(n);
    }
    let settled = used(&source);
    for n in 5..60 {
        round(n);
    }
    let after = used(&source);
    println!("QuickJS holds {settled} bytes after 5 builds, {after} after 60");
    assert!(
        after <= settled + settled / 4,
        "55 more builds of the library: {settled} bytes, then {after}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_edit_to_a_module_the_library_imports_is_noticed() {
    let dir = scratch("import");
    std::fs::write(dir.join("words.js"), "export const WORD = 'one';\n").expect("written");
    put(
        &dir,
        "js_imports",
        "version = \"1.0.0\"\n",
        &format!(
            "{}import {{ WORD }} from './words.js';\nexport class Says extends Provider {{\n  static kinds = [Greeter];\n  greet(name) {{ return {{ greeting: `${{WORD}}, ${{name}}` }}; }}\n}}\n",
            preamble()
        ),
    );
    let mut registry = serving(&config(&dir, None));
    assert!(registry.refresh().refused.is_empty());
    assert_eq!(
        greeting(
            &greeter(&registry, "js_imports_says")
                .greet("a")
                .expect("greeted")
        ),
        "one, a"
    );
    std::thread::sleep(core::time::Duration::from_millis(50));
    std::fs::write(dir.join("words.js"), "export const WORD = 'two';\n").expect("edited");
    assert_eq!(registry.refresh().loaded.len(), 1);
    assert_eq!(
        greeting(
            &greeter(&registry, "js_imports_says")
                .greet("a")
                .expect("greeted")
        ),
        "two, a"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn limits(pairs: &[(&str, u64)]) -> Map {
    let mut map = Map::new();
    for (k, v) in pairs {
        map.set(k, *v);
    }
    map
}

fn spin() -> String {
    format!(
        "{}{}",
        preamble(),
        r#"
let ticks = 0;
export class Spin extends Provider {
  static kinds = [Greeter];
  greet(name) {
    if (name === 'peek') return { greeting: String(ticks) };
    if (name === 'spawn') {
      // Two chains, each job queueing two more.
      const again = () => { ticks++; Promise.resolve().then(again); Promise.resolve().then(again); };
      again();
      return { greeting: 'spawned' };
    }
    if (name === 'hang') {
      // Two reactions that never end, the second catching what it can.
      Promise.resolve().then(() => { for (;;) {} });
      Promise.resolve().then(() => { for (;;) { try { for (;;) {} } catch (e) {} } });
      return { greeting: 'queued' };
    }
    if (name === 'fill-then-catch') {
      const kept = [];
      try { for (;;) kept.push(new Array(1000).fill(1)); } catch (e) {}
      for (;;) { try { for (;;) {} } catch (e) {} }
    }
    if (name === 'throws') {
      Promise.resolve().then(() => { ticks++; });
      throw new Error('after queueing');
    }
    if (name === 'wide') return { greeting: new Array(256).fill('x'.repeat(1 << 20)) };
    if (name === 'doubling') { let a = [1]; for (let i = 0; i < 22; i++) a = [a, a]; return { greeting: a }; }
    if (name === 'holes') return { greeting: new Array(20000000) };
    if (name === 'huge') return { greeting: new Array(2 ** 32 - 1) };
    if (name === 'detached') { const a = new Uint8Array(4); a.buffer.transfer(); return { greeting: a }; }
    if (name === 'loop') while (true) {}
    if (name === 'stubborn') {
      while (true) { try { while (true) {} } catch (e) {} finally { while (true) {} } }
    }
    if (name === 'deep') { const down = (n) => 1 + down(n + 1); return { greeting: String(down(1)) }; }
    if (name === 'grow') return { greeting: 'x'.repeat(64 * 1024 * 1024) };
    if (name === 'jobs') { const again = () => { Promise.resolve().then(again); }; again(); return { greeting: 'queued' }; }
    if (name === 'flood') {
      // Many reactions that end at once, then a call that never does.
      for (let i = 0; i < 150000; i++) Promise.resolve().then(() => { ticks++; });
      for (;;) {}
    }
    if (name === 'spinners') {
      // Many reactions that never end.
      const p = Promise.resolve();
      for (let i = 0; i < 150000; i++) p.then(() => { for (;;) {} });
      for (;;) {}
    }
    if (name === 'queue-forever') {
      const p = Promise.resolve();
      for (;;) p.then(() => { for (;;) {} });
    }
    if (name === 'imports') {
      // Many dynamic imports, each a job that would reach the files.
      for (let i = 0; i < 300000; i++) import('./dep.js');
      for (;;) {}
    }
    if (name === 'import-forever') for (;;) import('./dep.js');
    if (name === 'levels') {
      let most = 0;
      const down = (n) => { most = n; down(n + 1); };
      try { down(1); } catch (e) {}
      return { greeting: String(most) };
    }
    return { greeting: `calm, ${name}` };
  }
}
"#
    )
}

#[test]
fn runaway_code_is_stopped_at_its_limits_and_the_host_carries_on() {
    let dir = scratch("limits");
    put(&dir, "js_spin", "version = \"1.0.0\"\n", &spin());
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
    let spin = greeter(&registry, "js_spin_spin");

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
        "a try around the stopped code does not keep it running"
    );

    // The reactions the call queued run after its answer, under the same
    // limits: the call is the one that was stopped.
    let started = std::time::Instant::now();
    let jobs = spin.greet("jobs").unwrap_err();
    assert_eq!(jobs.message(), "stopped at its operations limit (200000)");
    assert!(
        started.elapsed() < core::time::Duration::from_secs(5),
        "promise reactions that queue themselves are stopped"
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
        greeting(&spin.greet("after").expect("the runtime carries on")),
        "calm, after"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Runs `f` on a thread of its own, and fails if it has not answered in
/// `seconds`: a runaway script must not hang the suite with it.
fn within<T: Send + 'static>(seconds: u64, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(core::time::Duration::from_secs(seconds))
        .expect("the call ended in time")
}

#[test]
fn a_loop_that_catches_everything_is_still_stopped() {
    let dir = scratch("uncatchable");
    put(&dir, "js_catch", "version = \"1.0.0\"\n", &spin());
    let mut registry = serving(&config(
        &dir,
        Some(limits(&[
            ("operations", 2_000_000),
            ("memory", 16 * 1024 * 1024),
        ])),
    ));
    assert!(registry.refresh().refused.is_empty());
    let spin = greeter(&registry, "js_catch_spin");
    // Reactions that never end, one of them catching.
    let stopped = within(60, {
        let spin = Arc::clone(&spin);
        move || spin.greet("hang").unwrap_err()
    });
    assert_eq!(
        stopped.message(),
        "stopped at its operations limit (2000000)"
    );
    // Memory full to its limit, then a loop that catches the stop.
    let stopped = within(60, {
        let spin = Arc::clone(&spin);
        move || spin.greet("fill-then-catch").unwrap_err()
    });
    assert_eq!(
        stopped.message(),
        "stopped at its operations limit (2000000)"
    );
    assert_eq!(
        greeting(&spin.greet("after").expect("carries on")),
        "calm, after"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reactions_a_failed_call_queued_run_before_it_returns() {
    let dir = scratch("throwing");
    put(&dir, "js_throwing", "version = \"1.0.0\"\n", &spin());
    let mut registry = serving(&config(&dir, None));
    assert!(registry.refresh().refused.is_empty());
    let spin = greeter(&registry, "js_throwing_spin");
    assert!(spin.greet("throws").is_err());
    assert_eq!(
        greeting(&spin.greet("peek").expect("peeked")),
        "1",
        "the reaction ran in the call that queued it, not in the next"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn what_a_script_makes_is_bounded_as_a_whole() {
    let dir = scratch("whole");
    put(&dir, "js_whole", "version = \"1.0.0\"\n", &spin());
    let mut registry = serving(&config(
        &dir,
        Some(limits(&[("memory", 16 * 1024 * 1024), ("time_ms", 5000)])),
    ));
    assert!(registry.refresh().refused.is_empty());
    let spin = greeter(&registry, "js_whole_spin");
    for name in ["wide", "doubling"] {
        let started = std::time::Instant::now();
        let refused = spin.greet(name).unwrap_err();
        assert!(
            refused.message().contains("more than"),
            "{name}: {}",
            refused.message()
        );
        assert!(
            started.elapsed() < core::time::Duration::from_secs(3),
            "{name} took {:?}",
            started.elapsed()
        );
    }
    assert_eq!(
        greeting(&spin.greet("after").expect("carries on")),
        "calm, after"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reactions_a_stopped_call_queued_die_with_it() {
    let dir = scratch("jobs");
    put(&dir, "js_jobs", "version = \"1.0.0\"\n", &spin());
    let mut registry = serving(&config(&dir, Some(limits(&[("operations", 200_000)]))));
    assert!(registry.refresh().refused.is_empty());
    let spin = greeter(&registry, "js_jobs_spin");
    let ticks = |spin: &Arc<dyn Greeter>| -> String {
        greeting(&spin.greet("peek").expect("peeked")).to_string()
    };
    let stopped = spin.greet("spawn").unwrap_err();
    assert_eq!(
        stopped.message(),
        "stopped at its operations limit (200000)",
        "the call that left the chains running is the one stopped"
    );
    let after = ticks(&spin);
    for _ in 0..3 {
        assert_eq!(
            greeting(&spin.greet("calm").expect("answers")),
            "calm, calm"
        );
    }
    assert_eq!(
        ticks(&spin),
        after,
        "no reaction of the stopped call ran in the calls after it"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// How long ending what a stopped call left may take: a fraction of a
/// second is measured, in either build.
const ENDS_WITHIN: core::time::Duration =
    core::time::Duration::from_secs(if cfg!(debug_assertions) { 20 } else { 8 });

/// What a stopped call left queued is ended however much of it there is,
/// at a cost that does not grow with what each reaction would run, and
/// the source and its other libraries carry on.
#[test]
fn a_flood_of_reactions_a_stopped_call_queued_ends_and_the_source_carries_on() {
    let dir = scratch("flood");
    std::fs::write(dir.join("dep.js"), "export const dep = 1;\n").expect("written");
    put(
        &dir,
        "js_flood",
        "version = \"1.0.0\"
",
        &spin(),
    );
    let mut registry = serving(&config(&dir, Some(limits(&[("operations", 20_000_000)]))));
    assert!(registry.refresh().refused.is_empty());
    let spin = greeter(&registry, "js_flood_spin");
    for name in ["flood", "spinners", "imports"] {
        let started = std::time::Instant::now();
        let stopped = within(120, {
            let spin = Arc::clone(&spin);
            move || spin.greet(name).unwrap_err()
        });
        assert_eq!(
            stopped.message(),
            "stopped at its operations limit (20000000)",
            "{name}"
        );
        let took = started.elapsed();
        assert!(took < ENDS_WITHIN, "{name} took {took:?}");
        assert_eq!(
            greeting(&spin.greet("after").expect("the source still serves")),
            "calm, after",
            "{name}"
        );
    }
    assert_eq!(
        greeting(&spin.greet("peek").expect("peeked")),
        "0",
        "no reaction of the stopped call ran"
    );
    std::fs::write(dir.join("js_flood.guatiao.toml"), {
        let text = std::fs::read_to_string(dir.join("js_flood.guatiao.toml")).unwrap();
        text.replace("1.0.0", "1.0.1")
    })
    .expect("written");
    let changed = registry.refresh();
    assert!(changed.refused.is_empty(), "{changed:?}");
    let spin = greeter(&registry, "js_flood_spin");
    assert_eq!(
        greeting(&spin.greet("again").expect("refreshed")),
        "calm, again"
    );

    // Under a time limit alone a call queues as much as it can before it
    // is stopped, and all of it is ended.
    let mut registry = serving(&config(&dir, Some(limits(&[("time_ms", 200)]))));
    assert!(registry.refresh().refused.is_empty());
    let spin = greeter(&registry, "js_flood_spin");
    for name in ["queue-forever", "import-forever"] {
        let started = std::time::Instant::now();
        let stopped = within(120, {
            let spin = Arc::clone(&spin);
            move || spin.greet(name).unwrap_err()
        });
        assert_eq!(
            stopped.message(),
            "stopped at its time_ms limit (200)",
            "{name}"
        );
        let took = started.elapsed();
        assert!(took < ENDS_WITHIN, "{name} took {took:?}");
        assert_eq!(
            greeting(&spin.greet("after").expect("serves")),
            "calm, after",
            "{name}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lists_and_buffers_a_script_makes_are_bounded_by_the_memory_limit() {
    let dir = scratch("holes");
    put(&dir, "js_holes", "version = \"1.0.0\"\n", &spin());
    let mut registry = serving(&config(
        &dir,
        Some(limits(&[("memory", 16 * 1024 * 1024), ("time_ms", 50)])),
    ));
    assert!(registry.refresh().refused.is_empty());
    let spin = greeter(&registry, "js_holes_spin");
    for name in ["holes", "huge"] {
        let started = std::time::Instant::now();
        let refused = spin.greet(name).unwrap_err();
        assert!(
            refused.message().contains("items"),
            "{name}: {}",
            refused.message()
        );
        assert!(
            started.elapsed() < core::time::Duration::from_secs(2),
            "{name} took {:?}",
            started.elapsed()
        );
    }
    assert_eq!(
        greeting(&spin.greet("after").expect("carries on")),
        "calm, after"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_detached_buffer_does_not_break_the_next_library() {
    let dir = scratch("detached");
    put(&dir, "js_detached", "version = \"1.0.0\"\n", &spin());
    let mut registry = serving(&config(&dir, None));
    assert!(registry.refresh().refused.is_empty());
    let spin = greeter(&registry, "js_detached_spin");
    // The bytes of a detached array are none; the answer is still a map.
    assert!(spin.greet("detached").is_ok());
    put(&dir, "js_next", "version = \"1.0.0\"\n", &spin_code_plain());
    let changed = registry.refresh();
    assert!(
        changed.refused.is_empty() && changed.loaded.iter().any(|l| l.id == "js_next"),
        "an unrelated library opens: {changed:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn spin_code_plain() -> String {
    format!(
        "{}export class Plain extends Provider {{ static kinds = [Greeter]; greet() {{ return {{ greeting: 'plain' }}; }} }}\n",
        preamble()
    )
}

#[cfg(unix)]
#[test]
fn a_link_out_of_the_paths_does_not_lead_out() {
    let dir = scratch("links");
    let lib = dir.join("lib");
    std::fs::create_dir_all(&lib).expect("made");
    std::fs::write(dir.join("outside.js"), "export const X = 1;\n").expect("written");
    std::fs::write(lib.join("inside.js"), "export const Y = 2;\n").expect("written");
    std::os::unix::fs::symlink("../outside.js", lib.join("leads_out.js")).expect("linked");
    std::os::unix::fs::symlink("inside.js", lib.join("stays_in.js")).expect("linked");
    for (id, imports) in [("js_leads_out", "leads_out"), ("js_stays_in", "stays_in")] {
        put(
            &lib,
            id,
            "version = \"1.0.0\"\n",
            &format!(
                "{}import './{imports}.js';\nexport class P extends Provider {{ static kinds = [Greeter]; greet() {{ return {{ greeting: 'ok' }}; }} }}\n",
                preamble()
            ),
        );
    }
    let mut registry = serving(&config(&dir, None));
    let changed = registry.refresh();
    let refused: Vec<_> = changed
        .refused
        .iter()
        .filter_map(|r| r.id.as_deref())
        .collect();
    assert_eq!(refused, ["js_leads_out"], "{changed:?}");
    assert!(
        changed.refused[0].reason.contains("outside the paths"),
        "{}",
        changed.refused[0].reason
    );
    assert_eq!(changed.loaded.len(), 1, "{changed:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn one_file_is_one_module_whatever_it_is_imported_as() {
    let dir = scratch("alias");
    std::fs::write(dir.join("words.js"), "export const W = {};\n").expect("written");
    std::os::unix::fs::symlink("words.js", dir.join("alias.js")).expect("linked");
    put(
        &dir,
        "js_alias",
        "version = \"1.0.0\"\n",
        &format!(
            "{}import * as a from './words.js';\nimport * as b from './alias.js';\nexport class P extends Provider {{ static kinds = [Greeter]; greet() {{ return {{ greeting: String(a.W === b.W) }}; }} }}\n",
            preamble()
        ),
    );
    let mut registry = serving(&config(&dir, None));
    assert!(registry.refresh().refused.is_empty());
    assert_eq!(
        greeting(
            &greeter(&registry, "js_alias_p")
                .greet("x")
                .expect("greeted")
        ),
        "true"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unbounded_recursion_is_an_error_not_a_crash() {
    let dir = scratch("recursion");
    put(&dir, "js_deep", "version = \"1.0.0\"\n", &spin());
    // No limits at all: QuickJS's own stack limit answers, on a thread that
    // has room for it.
    let mut registry = serving(&config(&dir, None));
    assert!(registry.refresh().refused.is_empty());
    let deep = greeter(&registry, "js_deep_spin")
        .greet("deep")
        .unwrap_err();
    assert_eq!(deep.status, Status::GUATIAO_ERR_INTERNAL);
    assert!(deep.message().contains("call stack"), "{}", deep.message());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A depth limit bounds the script's recursion, not the work of opening a
/// library: the least one still opens one, and stops a recursion.
#[test]
fn a_library_opens_under_the_least_depth_limit() {
    let dir = scratch("least-depth");
    put(&dir, "js_least", "version = \"1.0.0\"\n", &spin());
    let mut registry = serving(&config(&dir, Some(limits(&[("depth", 1)]))));
    let changed = registry.refresh();
    assert!(changed.refused.is_empty(), "{changed:?}");
    let spin = greeter(&registry, "js_least_spin");
    let deep = spin.greet("deep").unwrap_err();
    assert_eq!(deep.message(), "stopped at its depth limit (1)");
    assert_eq!(
        greeting(&spin.greet("after").expect("carries on")),
        "calm, after"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// No depth limit gives a call more stack than having none.
#[test]
fn a_large_depth_limit_reaches_no_deeper_than_none() {
    let dir = scratch("large-depth");
    put(&dir, "js_levels", "version = \"1.0.0\"\n", &spin());
    let levels = |limits_given: Option<Map>| -> u64 {
        let mut registry = serving(&config(&dir, limits_given));
        assert!(registry.refresh().refused.is_empty());
        greeting(
            &greeter(&registry, "js_levels_spin")
                .greet("levels")
                .expect("counted"),
        )
        .parse()
        .expect("a count")
    };
    let none = levels(None);
    let large = levels(Some(limits(&[("depth", 100_000)])));
    assert!(
        large <= none,
        "depth 100000 reached {large} levels, none {none}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Opening a library is under the limits too: a module whose reactions
/// spin while it awaits forever is stopped, and the source carries on.
#[test]
fn a_module_that_never_finishes_loading_is_stopped() {
    let dir = scratch("never-loads");
    put(
        &dir,
        "js_never",
        "version = \"1.0.0\"\n",
        &format!(
            "{}{}",
            preamble(),
            r#"
const again = () => { Promise.resolve().then(again); for (;;) {} };
Promise.resolve().then(again);
await new Promise(() => {});
export class Never extends Provider { static kinds = [Greeter]; greet(name) { return { greeting: name }; } }
"#
        ),
    );
    put(&dir, "js_calm", "version = \"1.0.0\"\n", &spin());
    let mut registry = serving(&config(
        &dir,
        Some(limits(&[("operations", 2_000_000), ("time_ms", 500)])),
    ));
    let changed = within(120, move || {
        let changed = registry.refresh();
        (changed, registry)
    });
    let (changed, registry) = changed;
    let reason = changed
        .refused
        .iter()
        .find(|r| r.id.as_deref() == Some("js_never"))
        .map(|r| r.reason.clone())
        .expect("refused");
    assert!(reason.starts_with("stopped at its"), "{reason}");
    assert_eq!(
        greeting(
            &greeter(&registry, "js_calm_spin")
                .greet("x")
                .expect("served")
        ),
        "calm, x"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn time_stops_a_call_too() {
    let dir = scratch("time");
    put(&dir, "js_slow", "version = \"1.0.0\"\n", &spin());
    let mut registry = serving(&config(&dir, Some(limits(&[("time_ms", 50)]))));
    assert!(registry.refresh().refused.is_empty());
    let started = std::time::Instant::now();
    let stopped = greeter(&registry, "js_slow_spin")
        .greet("loop")
        .unwrap_err();
    assert_eq!(stopped.message(), "stopped at its time_ms limit (50)");
    assert!(started.elapsed() < core::time::Duration::from_secs(5));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_library_that_does_not_load_is_refused_in_its_words() {
    let dir = scratch("refused");
    let with_kinds = |body: &str| format!("{}{body}", preamble());
    put(
        &dir,
        "js_loops",
        "version = \"1.0.0\"\n",
        &with_kinds("while (true) {}\n"),
    );
    put(&dir, "js_broken", "version = \"1.0.0\"\n", "export class {");
    put(
        &dir,
        "js_unversioned",
        "",
        &with_kinds("export const x = 1;\n"),
    );
    put(
        &dir,
        "js_empty",
        "version = \"1.0.0\"\n",
        &with_kinds("export const x = 1;\n"),
    );
    put(
        &dir,
        "js_lacking",
        "version = \"1.0.0\"\n",
        &with_kinds("export class P extends Provider { static kinds = [Greeter]; }\n"),
    );
    put(
        &dir,
        "js_nokinds",
        "version = \"1.0.0\"\n",
        &with_kinds("export class P extends Provider { greet() {} }\n"),
    );
    put(
        &dir,
        "js_unlisted",
        "version = \"1.0.0\"\n",
        &with_kinds("export class P extends Provider { static kinds = [Greeter]; greet() {} }\n"),
    );
    let listed = std::fs::read_to_string(dir.join("js_unlisted.guatiao.toml")).expect("read");
    std::fs::write(
        dir.join("js_unlisted.guatiao.toml"),
        listed.replace(
            "kinds = [\"greeter\", \"counter\"]",
            "kinds = [\"counter\"]",
        ),
    )
    .expect("written");
    put(
        &dir,
        "js_configured",
        "version = \"1.0.0\"\n",
        &with_kinds(
            "export class P extends Provider { static kinds = [Greeter]; static config = {}; greet() {} }\n",
        ),
    );
    put(
        &dir,
        "js_outside",
        "version = \"1.0.0\"\n",
        &with_kinds(
            "import '../elsewhere.js';\nexport class P extends Provider { static kinds = [Greeter]; greet() {} }\n",
        ),
    );
    put(
        &dir,
        "js_package",
        "version = \"1.0.0\"\n",
        &with_kinds(
            "import 'node:fs';\nexport class P extends Provider { static kinds = [Greeter]; greet() {} }\n",
        ),
    );
    put(
        &dir,
        "js_promised",
        "version = \"1.0.0\"\n",
        &with_kinds(
            "await new Promise(() => {});\nexport class P extends Provider { static kinds = [Greeter]; greet() {} }\n",
        ),
    );
    put(
        &dir,
        "js_hash",
        "version = \"1.0.0\"\n",
        "import { Provider } from 'guatiao/provide';\nimport { kindsOf } from 'guatiao/kinds';\nconst { greeter } = kindsOf({ format: 1, kinds: [{ name: 'greeter', floor_hash: 1, object: false, slots: [{ name: 'greet', required: true, fallible: true, args: [], answer: { type: 'map' } }] }] });\nexport class P extends Provider { static kinds = [greeter]; greet() {} }\n",
    );
    std::fs::write(dir.join("elsewhere.js"), "export {};\n").expect("written");
    // A manifest naming a file that is not there.
    std::fs::write(
        dir.join("js_missing.guatiao.toml"),
        "type = \"js\"\nname = \"js_missing\"\nversion = \"1.0.0\"\nmain = \"nothing.js\"\nkinds = [\"greeter\"]\n",
    )
    .expect("written");
    // A library inside its own directory, so that `../elsewhere.js` is out of its paths.
    let inner = dir.join("inner");
    std::fs::create_dir_all(&inner).expect("made");
    for name in ["js_outside"] {
        for ext in ["js", "guatiao.toml"] {
            std::fs::rename(
                dir.join(format!("{name}.{ext}")),
                inner.join(format!("{name}.{ext}")),
            )
            .expect("moved");
        }
    }

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
    let has = |id: &str, words: &str| {
        let why = reason(id);
        assert!(why.contains(words), "{id}: {why}");
    };
    has("js_loops", "operations limit");
    has("js_broken", "js_broken");
    has("js_unversioned", "version");
    has("js_empty", "exports no Provider subclass");
    has(
        "js_lacking",
        "does not implement greet, which the greeter kind requires",
    );
    has("js_nokinds", "lists no kind");
    has("js_unlisted", "which the manifest does not list");
    has("js_configured", "declares `config`");
    has("js_outside", "outside the paths");
    has("js_package", "relative path");
    has("js_promised", "never finishes loading");
    has("js_missing", "is not a file");
    has("js_hash", "is not the one the host gave");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_provider_that_says_it_cannot_run_here_is_left_out() {
    let dir = scratch("available");
    put(
        &dir,
        "js_unavailable",
        "version = \"1.0.0\"\n",
        &format!(
            "{}export class Here extends Provider {{ static kinds = [Greeter]; static displayName = 'Over here'; static meta = {{ summary: 'a note', tags: ['a', 'b'] }}; greet() {{ return {{ greeting: 'here' }}; }} }}\nexport class Named extends Provider {{ static kinds = [Greeter]; static named = 'custom'; greet() {{ return {{ greeting: 'named' }}; }} }}\nexport class WithId extends Provider {{ static kinds = [Greeter]; static id = 'chosen_id'; greet() {{ return {{ greeting: 'id' }}; }} }}\nexport class HelloWorld extends Provider {{ static kinds = [Greeter]; greet() {{ return {{ greeting: 'snake' }}; }} }}\nexport class There extends Provider {{ static kinds = [Greeter]; static available() {{ return 'not on this machine'; }} greet() {{ return {{ greeting: 'there' }}; }} }}\n",
            preamble()
        ),
    );
    let mut registry = serving(&config(&dir, None));
    assert!(registry.refresh().refused.is_empty());
    assert!(
        registry
            .offer::<dyn Greeter>("js_unavailable_there")
            .is_none()
    );
    assert_eq!(
        greeting(
            &greeter(&registry, "js_unavailable_here")
                .greet("x")
                .expect("greeted")
        ),
        "here"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_call_made_from_inside_a_call_reaches_the_runtime_again() {
    // The listener the host lends is called by JavaScript; it calls another
    // provider of the same runtime before it answers.
    let dir = scratch("nested");
    put(
        &dir,
        "js_nested",
        "version = \"1.0.0\"\n",
        &format!(
            "{}export class Outer extends Provider {{\n  static kinds = [Greeter];\n  greet(name) {{ return {{ greeting: `outer ${{name}}` }}; }}\n  greetInto(name, listener) {{ listener.heard(name); }}\n}}\nexport class Inner extends Provider {{\n  static kinds = [Greeter];\n  greet(name) {{ return {{ greeting: `inner ${{name}}` }}; }}\n}}\n",
            preamble()
        ),
    );
    let mut registry = serving(&config(&dir, None));
    assert!(registry.refresh().refused.is_empty());
    let outer = greeter(&registry, "js_nested_outer");
    let inner = greeter(&registry, "js_nested_inner");
    struct Calls(Arc<dyn Greeter>, Arc<Mutex<Vec<String>>>);
    impl Listener for Calls {
        fn heard(&mut self, what: &str) {
            let answer = self.0.greet(what).expect("the inner greeter answers");
            self.1.lock().unwrap().push(greeting(&answer).to_string());
        }
    }
    let said = Arc::new(Mutex::new(Vec::new()));
    let mut listener = Calls(inner, Arc::clone(&said));
    outer.greet_into("a", &mut listener).expect("told");
    assert_eq!(*said.lock().unwrap(), ["inner a"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn what_a_method_throws_or_returns_wrongly_crosses_in_words() {
    let dir = scratch("throws");
    put(
        &dir,
        "js_throws",
        "version = \"1.0.0\"\n",
        &format!(
            "{}import {{ GuatiaoError, NotFound }} from 'guatiao';\nawait Promise.resolve();\nexport class Throws extends Provider {{\n  static kinds = [Greeter];\n  greet(name) {{\n    if (name === 'status') throw new NotFound('no such greeting');\n    if (name === 'number') throw new GuatiaoError(13, 'by number');\n    if (name === 'text') throw 'plain text';\n    if (name === 'object') throw {{ status: 10, message: 'not an error' }};\n    if (name === 'type') null.x;\n    if (name === 'undefined') return undefined;\n    if (name === 'text-answer') return 'hello';\n    if (name === 'promise') return Promise.resolve({{ greeting: 'later' }});\n    if (name === 'cycle') {{ const t = {{}}; t.me = t; return t; }}\n    if (name === 'class') return new (class Point {{}})();\n    return {{ greeting: name }};\n  }}\n  async shout(name) {{ return name; }}\n}}\n",
            preamble()
        ),
    );
    let mut registry = serving(&config(&dir, None));
    let changed = registry.refresh();
    assert!(
        changed.refused.is_empty(),
        "a module may await at its top: {changed:?}"
    );
    let g = greeter(&registry, "js_throws_throws");
    let refused = |name: &str| g.greet(name).unwrap_err();

    let e = refused("status");
    assert_eq!(
        (e.status, e.message()),
        (Status::GUATIAO_ERR_NOT_FOUND, "no such greeting")
    );
    let e = refused("number");
    assert_eq!(
        (e.status, e.message()),
        (Status::GUATIAO_ERR_UNSUPPORTED, "by number")
    );
    let e = refused("text");
    assert_eq!(
        (e.status, e.message()),
        (Status::GUATIAO_ERR_INTERNAL, "thrown: plain text")
    );
    let e = refused("object");
    assert_eq!(
        e.status,
        Status::GUATIAO_ERR_INTERNAL,
        "only a GuatiaoError carries a status"
    );
    let e = refused("type");
    assert_eq!(e.status, Status::GUATIAO_ERR_INTERNAL);
    assert!(e.message().starts_with("TypeError: "), "{}", e.message());
    let e = refused("undefined");
    assert!(
        matches!(
            e.status,
            Status::GUATIAO_ERR_WRONG_KIND | Status::GUATIAO_ERR_BAD_VALUE
        ),
        "{}: {}",
        u32::from(e.status),
        e.message()
    );
    let e = refused("text-answer");
    assert!(
        matches!(
            e.status,
            Status::GUATIAO_ERR_WRONG_KIND | Status::GUATIAO_ERR_BAD_VALUE
        ),
        "{}: {}",
        u32::from(e.status),
        e.message()
    );
    let e = refused("promise");
    assert_eq!(e.status, Status::GUATIAO_ERR_INTERNAL, "{}", e.message());
    assert!(
        e.message().contains("returned a promise"),
        "{}",
        e.message()
    );
    let e = refused("cycle");
    assert!(e.message().contains("contains itself"), "{}", e.message());
    let e = refused("class");
    assert!(
        e.message().contains("a Point is not plain data"),
        "{}",
        e.message()
    );
    // `shout` is written `async` and answers text: a promise is no text.
    let e = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| g.shout("x")));
    assert!(
        e.is_err(),
        "a slot that cannot fail and fails is a broken contract to a Rust caller"
    );
    assert_eq!(greeting(&g.greet("fine").expect("greeted")), "fine");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn text_encoder_and_decoder_do_what_nodes_do() {
    let dir = scratch("text");
    put(
        &dir,
        "js_text",
        "version = \"1.0.0\"\n",
        &format!(
            "{}export class Text extends Provider {{\n  static kinds = [Greeter];\n  greet(name) {{\n    const encode = (s) => JSON.stringify([...new TextEncoder().encode(s)]);\n    if (name === 'encode') return {{ greeting: encode('\\u00e9\\u{{1f600}}a\\0b') }};\n    if (name === 'lone') return {{ greeting: encode('\\ud800x') }};\n    if (name === 'decode') return {{ greeting: new TextDecoder().decode(new Uint8Array([0xff,0x41,0xe2,0x82,0xc3,0xa9,0xf0,0x9f,0x98,0x80,0xed,0xa0,0x80,0xc0,0x80])) }};\n    if (name === 'view') return {{ greeting: new TextDecoder().decode(new Uint8Array([0x78, 0xc3, 0xa9, 0x79]).subarray(1, 3)) }};\n    console.log('logged', name, {{ a: 1 }});\n    return {{ greeting: typeof console.log + typeof console.warn }};\n  }}\n}}\n",
            preamble()
        ),
    );
    let mut registry = serving(&config(&dir, None));
    assert!(registry.refresh().refused.is_empty());
    let g = greeter(&registry, "js_text_text");
    let said = |name: &str| greeting(&g.greet(name).expect("greeted")).to_string();
    // Node 22 prints the same four.
    assert_eq!(said("encode"), "[195,169,240,159,152,128,97,0,98]");
    assert_eq!(said("lone"), "[239,191,189,120]");
    assert_eq!(
        said("decode"),
        "\u{fffd}A\u{fffd}\u{e9}\u{1f600}\u{fffd}\u{fffd}\u{fffd}\u{fffd}\u{fffd}"
    );
    assert_eq!(said("view"), "\u{e9}");
    assert_eq!(said("x"), "functionfunction");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sixty_four_bit_numbers_cross_whole_as_bigint() {
    let dir = scratch("wide");
    put(
        &dir,
        "js_wide",
        "version = \"1.0.0\"\n",
        &format!(
            "{}import {{ Wide }} from '{}';\nexport class Echo extends Provider {{\n  static kinds = [Wide];\n  echoI64(x) {{ if (typeof x !== 'bigint') throw new Error('not a bigint: ' + typeof x); return x; }}\n  echoU64(x) {{ if (typeof x !== 'bigint') throw new Error('not a bigint: ' + typeof x); return x; }}\n}}\n",
            preamble(),
            kinds_module()
        ),
    );
    let listed = std::fs::read_to_string(dir.join("js_wide.guatiao.toml")).expect("read");
    std::fs::write(
        dir.join("js_wide.guatiao.toml"),
        listed.replace("kinds = [\"greeter\", \"counter\"]", "kinds = [\"wide\"]"),
    )
    .expect("written");
    let mut registry = serving(&config(&dir, None));
    let changed = registry.refresh();
    assert!(changed.refused.is_empty(), "{changed:?}");
    let wide = registry
        .offer::<dyn Wide>("js_wide_echo")
        .expect("served")
        .expect("as the trait")
        .shared()
        .expect("an instance");
    for x in [i64::MIN, -1, 0, 1 << 53, (1 << 53) + 1, i64::MAX] {
        assert_eq!(wide.echo_i64(x).expect("echoed"), x);
    }
    for x in [0, 1 << 53, (1 << 53) + 1, u64::MAX] {
        assert_eq!(wide.echo_u64(x).expect("echoed"), x);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

fn find_compiler() -> Option<PathBuf> {
    for name in ["clang", r"C:\Program Files\LLVM\bin\clang.exe", "cc", "gcc"] {
        let path = PathBuf::from(name);
        if path.is_absolute() && !path.exists() {
            continue;
        }
        if std::process::Command::new(&path)
            .arg("--version")
            .output()
            .is_ok()
        {
            return Some(path);
        }
    }
    None
}

#[test]
fn a_c_host_calls_the_javascript_greeter() {
    let Some(file) = source_file() else {
        println!("skipped: this crate's file is not built beside the tests");
        return;
    };
    let Some(cc) = find_compiler() else {
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
        "js_c_host.exe"
    } else {
        "js_c_host"
    });
    let mut compile = std::process::Command::new(&cc);
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
    run.arg(&file).arg(examples()).arg(kinds_file());
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
        stdout.contains("quickjs_greeter_hello greets: hello, ana"),
        "{stdout}"
    );
}
