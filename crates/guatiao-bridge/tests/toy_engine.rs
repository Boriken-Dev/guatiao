// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A toy engine served through every piece of the bridge: its libraries
//! are `.toy` files of `key = value` lines described by manifests; it is
//! not `Send`, so it lives on the engine thread; it has no FFI, so its
//! providers are served through tables libffi builds from `greeter_kind`'s
//! descriptions. A Rust host calls it through the kinds' traits.

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::sync::Arc;
use core::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread::{self, ThreadId};

use greeter_kind::{Conversation, Counter, Greeter, Listener};
use guatiao::library::kind::{Arg, Caller, RawObject};
use guatiao::library::{Candidate, Host, LocalLibrary, ProviderError, Registry, Source};
use guatiao::value::convert::TryAsRef;
use guatiao::{List, Map, Status, Value};
use guatiao_bridge::{
    BridgeError, BridgeSource, Call, CallArg, Config, Engine, Kinds, Limits, Manifest,
    ManifestFormat, Opened, Provided, Reply, Target, ThreadModel,
};
use guatiao_libffi::Libffi;

/// One open library: its words, and the conversations it holds.
struct Lib {
    id: String,
    word: String,
    spin: bool,
    counted: Cell<i64>,
    conversations: RefCell<BTreeMap<u64, (RawObject, Vec<String>)>>,
}

/// The engine: not `Send` (its state is in `Rc`s and `RefCell`s).
struct Toy {
    thread: ThreadId,
    limits: Limits,
    listener: Caller,
    libraries: RefCell<BTreeMap<u64, Rc<Lib>>>,
    next: Cell<u64>,
    closed: Cell<usize>,
}

/// Reads `key = value` lines.
fn lines(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (k, v) = line
            .split_once('=')
            .ok_or_else(|| format!("line {} is not `key = value`", n + 1))?;
        out.insert(k.trim().to_string(), v.trim().to_string());
    }
    Ok(out)
}

fn code_of(manifest: &Manifest) -> Result<BTreeMap<String, String>, BridgeError> {
    let file = Path::new(&manifest.paths()[0]).join(format!("{}.toy", manifest.id()));
    let text = std::fs::read_to_string(&file)
        .map_err(|e| BridgeError::engine(format!("{}: {e}", file.display())))?;
    lines(&text).map_err(|why| BridgeError::engine(format!("{}: {why}", manifest.id())))
}

fn refused(why: &str) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, why)
}

impl Toy {
    fn library(&self, handle: u64) -> Result<Rc<Lib>, ProviderError> {
        self.libraries
            .borrow()
            .get(&handle)
            .cloned()
            .ok_or_else(|| refused("closed"))
    }
}

impl Engine for Toy {
    type Library = u64;
    const THREADS: ThreadModel = ThreadModel::Pinned;
    const MANIFESTS: ManifestFormat = ManifestFormat {
        serves: "toy",
        id_key: "module",
        code_extensions: &["toy"],
        skip_dirs: &[],
    };

    fn start(config: &Config) -> Result<Toy, BridgeError> {
        let mut kinds = Kinds::new();
        for path in config.kinds() {
            kinds.load(path)?;
        }
        let listener = kinds
            .get("listener")
            .cloned()
            .ok_or_else(|| BridgeError::UnknownKind("listener".into()))?;
        Ok(Toy {
            thread: thread::current().id(),
            limits: *config.limits(),
            listener: Caller::new(listener, &Libffi)
                .map_err(|e| BridgeError::Table(e.to_string()))?,
            libraries: RefCell::new(BTreeMap::new()),
            next: Cell::new(0),
            closed: Cell::new(0),
        })
    }

    /// `version = ..` read as a line, without running anything.
    fn version_of(&self, manifest: &Manifest) -> Result<String, BridgeError> {
        code_of(manifest)?
            .remove("version")
            .ok_or_else(|| BridgeError::engine("the code states no version"))
    }

    fn open(&self, manifest: &Manifest, _version: &str) -> Result<Opened<u64>, BridgeError> {
        let code = code_of(manifest)?;
        if code.contains_key("broken") {
            return Err(BridgeError::engine(format!(
                "{} does not run: it says it is broken",
                manifest.id()
            )));
        }
        let handle = self.next.get() + 1;
        self.next.set(handle);
        self.libraries.borrow_mut().insert(
            handle,
            Rc::new(Lib {
                id: manifest.id().to_string(),
                word: code.get("word").cloned().unwrap_or_else(|| "hello".into()),
                spin: code.contains_key("spin"),
                counted: Cell::new(0),
                conversations: RefCell::new(BTreeMap::new()),
            }),
        );
        let id = manifest.id();
        Ok(Opened::Described {
            library: handle,
            providers: vec![
                Provided::new(&format!("{id}_greeter"))
                    .name("A toy greeter")
                    .serves("greeter", ["greet", "check", "start"])
                    .serves("counter", ["count"]),
            ],
        })
    }

    fn call(&self, handle: &u64, call: &mut Call) -> Result<Reply, ProviderError> {
        if thread::current().id() != self.thread {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_INTERNAL,
                "called off the engine's thread",
            ));
        }
        let lib = self.library(*handle)?;
        let text = |i: usize| match call.args.get(i) {
            Some(CallArg::Value(v)) => TryAsRef::<str>::try_as_ref(v).map(str::to_string),
            _ => None,
        };
        match (call.target().clone(), call.method()) {
            (Target::Provider(_), "greet") => {
                if lib.spin {
                    let mut budget = self.limits.budget();
                    loop {
                        budget.spend(1)?;
                    }
                }
                let name = text(0).unwrap_or_default();
                if name.is_empty() {
                    return Err(refused("nobody to greet"));
                }
                let mut answer = Map::new();
                answer.set("greeting", format!("{}, {name}", lib.word).as_str());
                Ok(Reply::Value(Value::from(answer)))
            }
            (Target::Provider(_), "check") => {
                let Some(CallArg::Lent(config)) = call.args.get_mut(0) else {
                    return Err(refused("no configuration lent"));
                };
                let map: &mut Map = guatiao::value::convert::TryAsMut::try_as_mut(config)
                    .ok_or_else(|| refused("not a map"))?;
                if map.get("name").is_none() {
                    map.set("name", "ana");
                }
                Ok(Reply::Value(Value::from(true)))
            }
            (Target::Provider(_), "count") => {
                lib.counted.set(lib.counted.get() + 1);
                Ok(Reply::Value(Value::from(lib.counted.get())))
            }
            (Target::Provider(_), "start") => {
                let Some(CallArg::Object(listener)) = call.args.pop() else {
                    return Err(refused("no listener"));
                };
                let id = self.next.get() + 1;
                self.next.set(id);
                lib.conversations
                    .borrow_mut()
                    .insert(id, (listener, Vec::new()));
                Ok(Reply::Object {
                    kind: "conversation".to_string(),
                    id,
                })
            }
            (Target::Object(id), "say") => {
                let what = text(0).unwrap_or_default();
                let mut talks = lib.conversations.borrow_mut();
                let (listener, said) = talks.get_mut(&id).ok_or_else(|| refused("no such talk"))?;
                listener.call(&self.listener, 0, vec![Arg::Str(&what)])?;
                said.push(what);
                Ok(Reply::Nothing)
            }
            (Target::Object(id), "turns") => {
                let talks = lib.conversations.borrow();
                let (_, said) = talks.get(&id).ok_or_else(|| refused("no such talk"))?;
                Ok(Reply::Value(Value::from(said.len() as i64)))
            }
            (Target::Object(id), "read") => {
                let talks = lib.conversations.borrow();
                let (_, said) = talks.get(&id).ok_or_else(|| refused("no such talk"))?;
                let text = said.join(" ");
                let Some(CallArg::Buffer(dst)) = call.args.get_mut(0) else {
                    return Err(refused("no buffer"));
                };
                let n = text.len().min(dst.len());
                dst[..n].copy_from_slice(&text.as_bytes()[..n]);
                Ok(Reply::Value(Value::from(n as i64)))
            }
            (target, method) => Err(ProviderError::new(
                Status::GUATIAO_ERR_UNSUPPORTED,
                &format!("{} has no `{method}` for {target:?}", lib.id),
            )),
        }
    }

    fn release(&self, handle: &u64, object: u64) {
        if let Ok(lib) = self.library(*handle) {
            // The listener goes with it, destroyed on this thread.
            lib.conversations.borrow_mut().remove(&object);
        }
    }

    fn close(&self, handle: u64) {
        self.libraries.borrow_mut().remove(&handle);
        self.closed.set(self.closed.get() + 1);
    }
}

/// The source the registry holds, shared with the test so it can ask the
/// engine what happened.
struct Held(Arc<BridgeSource<Toy>>);

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

fn scratch(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("guatiao-bridge-toy-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

fn put(dir: &Path, id: &str, manifest_extra: &str, code: &str) {
    std::fs::write(
        dir.join(format!("{id}.guatiao.toml")),
        format!("type = \"toy\"\nmodule = \"{id}\"\nkinds = [\"greeter\", \"counter\"]\n{manifest_extra}"),
    )
    .expect("written");
    std::fs::write(dir.join(format!("{id}.toy")), code).expect("written");
}

fn config(dir: &Path, limits: Option<u64>) -> Map {
    let mut dirs = List::new();
    dirs.push(dir.display().to_string().as_str());
    let mut kinds = List::new();
    kinds.push(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/greeter_kind/include/greeter_kind.kinds.json")
            .display()
            .to_string()
            .as_str(),
    );
    let mut config = Map::new();
    config.set("dirs", dirs);
    config.set("kinds", kinds);
    if let Some(operations) = limits {
        let mut limits = Map::new();
        limits.set("operations", operations);
        config.set("limits", limits);
    }
    config
}

fn serving(dir: &Path, limits: Option<u64>) -> (Registry, Arc<BridgeSource<Toy>>) {
    let source = Arc::new(
        BridgeSource::<Toy>::pinned(&config(dir, limits), "toy-engine")
            .expect("the engine starts")
            .with_mechanism(Arc::new(Libffi)),
    );
    assert_eq!(source.threads(), ThreadModel::Pinned);
    let mut registry = Registry::new("toy-host", "1.0");
    registry
        .add_source("toy", Held(Arc::clone(&source)))
        .expect("added");
    (registry, source)
}

fn greeter(registry: &Registry, key: &str) -> Arc<dyn Greeter> {
    registry
        .offer::<dyn Greeter>(key)
        .expect("served")
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

#[test]
fn a_toy_greeter_is_served_through_tables_built_from_its_description() {
    let dir = scratch("serve");
    put(&dir, "toy_hello", "version = \"1.0.0\"\n", "word = hello\n");
    let (mut registry, _source) = serving(&dir, None);
    let changed = registry.refresh();
    assert!(changed.refused.is_empty(), "{changed:?}");
    assert_eq!(changed.loaded.len(), 1);

    let hello = greeter(&registry, "toy_hello_greeter");
    assert_eq!(
        greeting(&hello.greet("ana").expect("greeted")),
        "hello, ana"
    );
    // From another thread: queued to the engine's, which answers.
    let from_another = {
        let hello = Arc::clone(&hello);
        thread::spawn(move || hello.greet("bo").map(|m| greeting(&m).to_string()))
            .join()
            .expect("joined")
    };
    assert_eq!(from_another.expect("greeted"), "hello, bo");

    let refused = hello.greet("").unwrap_err();
    assert_eq!(refused.status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(refused.message(), "nobody to greet");

    // `shout` is not implemented: the trait's default runs, through `greet`.
    assert_eq!(hello.shout("cy"), "HELLO, CY");

    // A lent map, completed in place by the engine and written back.
    let mut config = Map::new();
    assert!(hello.check(&mut config).expect("checked"));
    assert_eq!(
        config.get("name").and_then(TryAsRef::<str>::try_as_ref),
        Some("ana")
    );

    let counter = registry
        .offers::<dyn Counter>()
        .next()
        .expect("offered")
        .shared()
        .expect("an instance");
    assert_eq!(counter.count(), 1);
    assert_eq!(counter.count(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_edit_serves_new_code_while_the_old_instance_keeps_working() {
    let dir = scratch("edit");
    // No version in the manifest: the engine reads it from the code.
    put(&dir, "toy_edit", "", "version = 1.0.0\nword = hello\n");
    let (mut registry, source) = serving(&dir, None);
    let first = registry.refresh();
    assert_eq!(first.loaded.len(), 1, "{first:?}");
    let old = greeter(&registry, "toy_edit_greeter");
    assert_eq!(registry.loaded()[0].version, "1.0.0");

    std::thread::sleep(core::time::Duration::from_millis(30));
    std::fs::write(dir.join("toy_edit.toy"), "version = 1.0.1\nword = hey\n").expect("edited");
    let second = registry.refresh();
    assert_eq!(second.loaded.len(), 1, "{second:?}");
    assert_eq!(second.retired.len(), 1, "{second:?}");
    let new = greeter(&registry, "toy_edit_greeter");
    assert_eq!(greeting(&new.greet("ana").expect("greeted")), "hey, ana");
    assert_eq!(
        greeting(&old.greet("ana").expect("still greets")),
        "hello, ana",
        "the old build serves the instance taken from it"
    );
    assert_eq!(source.run(|toy| toy.closed.get()).expect("ran"), 0);
    drop(old);
    registry.collect();
    assert_eq!(
        source.run(|toy| toy.closed.get()).expect("ran"),
        1,
        "the old build closed once nothing held it"
    );

    // Gone from disk: retired, then closed.
    drop(new);
    std::fs::remove_file(dir.join("toy_edit.guatiao.toml")).expect("removed");
    let third = registry.refresh();
    assert_eq!(third.retired.len(), 1, "{third:?}");
    registry.collect();
    assert_eq!(source.run(|toy| toy.closed.get()).expect("ran"), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_described_library_is_refused_by_a_source_with_no_mechanism() {
    let dir = scratch("no-mechanism");
    put(&dir, "toy_hello", "version = \"1.0.0\"\n", "word = hello\n");
    let source =
        BridgeSource::<Toy>::pinned(&config(&dir, None), "toy-engine").expect("the engine starts");
    let mut registry = Registry::new("toy-host", "1.0");
    registry.add_source("toy", source).expect("added");
    let changed = registry.refresh();
    assert!(changed.loaded.is_empty(), "{changed:?}");
    let reasons: Vec<&str> = changed.refused.iter().map(|r| r.reason.as_str()).collect();
    assert!(
        reasons.iter().any(|r| r.contains("needs a mechanism")),
        "{reasons:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_library_that_does_not_run_is_refused_in_its_words() {
    let dir = scratch("broken");
    put(
        &dir,
        "toy_broken",
        "version = \"1.0.0\"\n",
        "broken = yes\n",
    );
    put(&dir, "toy_unversioned", "", "word = hi\n");
    std::fs::write(dir.join("toy_bad.guatiao.toml"), "module = \n").expect("written");
    let (mut registry, _source) = serving(&dir, None);
    let changed = registry.refresh();
    assert!(changed.loaded.is_empty(), "{changed:?}");
    let reasons: Vec<&str> = changed.refused.iter().map(|r| r.reason.as_str()).collect();
    assert!(
        reasons
            .iter()
            .any(|r| r.contains("toy_broken does not run")),
        "{reasons:?}"
    );
    assert!(
        reasons.iter().any(|r| r.contains("states no version")),
        "{reasons:?}"
    );
    assert!(
        reasons.iter().any(|r| r.contains("toy_bad.guatiao.toml")),
        "{reasons:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_runaway_call_is_stopped_by_its_limit_and_the_host_carries_on() {
    let dir = scratch("limits");
    put(&dir, "toy_spin", "version = \"1.0.0\"\n", "spin = yes\n");
    put(&dir, "toy_calm", "version = \"1.0.0\"\n", "word = hi\n");
    let (mut registry, _source) = serving(&dir, Some(10_000));
    assert_eq!(registry.refresh().loaded.len(), 2);
    let spin = greeter(&registry, "toy_spin_greeter");
    let stopped = spin.greet("ana").unwrap_err();
    assert_eq!(stopped.status, Status::GUATIAO_ERR_INTERNAL);
    assert_eq!(stopped.message(), "stopped at its operations limit (10000)");
    let calm = greeter(&registry, "toy_calm_greeter");
    assert_eq!(greeting(&calm.greet("bo").expect("greeted")), "hi, bo");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A listener the host implements.
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

#[test]
fn objects_cross_the_engine_both_ways() {
    let dir = scratch("objects");
    put(&dir, "toy_talk", "version = \"1.0.0\"\n", "word = hello\n");
    let (mut registry, _source) = serving(&dir, None);
    assert_eq!(registry.refresh().loaded.len(), 1);
    let hello = greeter(&registry, "toy_talk_greeter");
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mut talk: guatiao::library::Object<dyn Conversation> = hello
        .start(Heard(Arc::clone(&heard)).into_object())
        .expect("a conversation the engine holds");
    talk.say("one").expect("said");
    talk.say("two").expect("said");
    assert_eq!(talk.turns(), 2);
    let mut buf = [0u8; 32];
    let n = talk.read(&mut buf) as usize;
    assert_eq!(&buf[..n], b"one two");
    assert_eq!(*heard.lock().unwrap(), ["one", "two"]);
    drop(talk);
    assert_eq!(
        *heard.lock().unwrap(),
        ["one", "two", "<destroyed>"],
        "the engine let the listener go when the conversation was destroyed"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
