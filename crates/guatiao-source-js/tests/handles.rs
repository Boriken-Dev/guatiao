// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The objects a host gives a library, and the objects one library serves
//! to another through the host: when they are destroyed, who may hand them
//! back, what a refused call leaves of them.
//!
//! Two libraries of one source, so that a host object one of them serves
//! reaches the other as a handle and is destroyed through the first.

extern crate alloc;

use alloc::sync::Arc;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use guatiao::library::kind::{
    Answer, Arg, Caller, DescribedRemote, KindDescription, Serve, ServedTable,
};
use guatiao::library::{Candidate, Host, LocalLibrary, ProviderError, Registry, Source};
use guatiao::{List, Map};
use guatiao_bridge::BridgeSource;
use guatiao_libffi::Libffi;
use guatiao_source_js::JsEngine;

const KINDS: &str = r#"{"format": 1, "kinds": [
  {"name": "sink", "floor_hash": 7001, "object": true, "slots": [
    {"name": "put", "required": true, "fallible": true,
     "args": [{"name": "tree", "type": "map", "pass": "borrowed"}]},
    {"name": "with", "required": true, "fallible": true,
     "args": [{"name": "other", "type": "object", "kind": "sink", "pass": "lent"},
              {"name": "n", "type": "u8"}]},
    {"name": "fill", "required": true, "fallible": false,
     "args": [{"name": "dst", "type": "bytes_mut"}], "answer": {"type": "i64"}},
    {"name": "tick", "required": true, "fallible": false,
     "args": [{"name": "n", "type": "u8"}]},
    {"name": "zero", "required": true, "fallible": false, "args": [],
     "answer": {"type": "f64"}}
  ]},
  {"name": "maker", "floor_hash": 7002, "object": false, "slots": [
    {"name": "make", "required": true, "fallible": true, "args": [],
     "answer": {"type": "object", "kind": "sink"}},
    {"name": "forge", "required": true, "fallible": true, "args": [],
     "answer": {"type": "object", "kind": "sink"}},
    {"name": "disposed", "required": true, "fallible": false, "args": [],
     "answer": {"type": "u64"}}
  ]},
  {"name": "holder", "floor_hash": 7003, "object": false, "slots": [
    {"name": "hold", "required": true, "fallible": true,
     "args": [{"name": "sink", "type": "object", "kind": "sink", "pass": "taken"}]},
    {"name": "drop_it", "required": true, "fallible": true,
     "args": [{"name": "sink", "type": "object", "kind": "sink", "pass": "taken"}]},
    {"name": "use_held", "required": true, "fallible": true, "args": []},
    {"name": "probe", "required": true, "fallible": true,
     "args": [{"name": "sink", "type": "object", "kind": "sink", "pass": "lent"}]},
    {"name": "check_zero", "required": true, "fallible": true, "args": []},
    {"name": "fill_big", "required": true, "fallible": true, "args": []},
    {"name": "leak", "required": true, "fallible": true, "args": []},
    {"name": "nested", "required": true, "fallible": true, "args": []},
    {"name": "peek_flag", "required": true, "fallible": false, "args": [],
     "answer": {"type": "u64"}},
    {"name": "forge_bounds", "required": true, "fallible": true, "args": []},
    {"name": "close_and_spin", "required": true, "fallible": true, "args": []}
  ]}
]}"#;

const MAKER: &str = r#"
import { Provider } from 'guatiao/provide';
import { kindsOf } from 'guatiao/kinds';

const { maker } = kindsOf(KINDS);

let disposed = 0n;

// An object this library serves; disposing it allocates, as a script's
// cleanup may.
class Thing {
  put() {}
  with() {}
  fill() { return 0n; }
  tick() {}
  zero() { return 0; }
  [Symbol.dispose]() {
    disposed += 1n;
    const junk = [];
    for (let i = 0; i < 5000; i++) junk.push({ i, text: 'x'.repeat(50) });
  }
}

export class Makes extends Provider {
  static kinds = [maker];
  make() { return new Thing(); }
  forge() { return { [Symbol.for('guatiao.handle')]: 1 }; }
  disposed() { return disposed; }
}
"#;

const HOLDER: &str = r#"
import { Provider } from 'guatiao/provide';
import { kindsOf } from 'guatiao/kinds';

const { holder } = kindsOf(KINDS);

const expect = (what, got, wanted) => {
  if (got !== wanted) throw new Error(`${what}: ${String(got)}, wanted ${String(wanted)}`);
};

let flag = false;

export class Holds extends Provider {
  static kinds = [holder];
  held = null;

  hold(sink) { this.held = sink; }
  dropIt(sink) { /* dropped, not closed */ }
  useHeld() { this.held.put({ still: 'here' }); }

  probe(sink) {
    let refused = false;
    try {
      sink.with(this.held, 'not a number');
    } catch (e) {
      refused = true;
    }
    expect('the call was refused', refused, true);
    this.held.put({ probe: 'ok' });
  }

  checkZero() { expect('negative zero', Object.is(this.held.zero(), -0), true); }

  leak() {
    // The first reaction never ends; the fifty after it call the host.
    Promise.resolve().then(() => { for (;;) {} });
    for (let i = 0; i < 50; i++) Promise.resolve().then(() => this.held.tick(1));
  }

  nested() {
    flag = false;
    Promise.resolve().then(() => { flag = true; });
    // The host answers by calling this library again, from inside.
    this.held.put({ reenter: true });
    expect('no reaction ran in the middle of the script', flag, false);
  }

  peekFlag() { return flag ? 1n : 0n; }

  forgeBounds() {
    const core = globalThis[Symbol.for('guatiao.core')];
    core.maxBytes = Infinity;
    core.maxItems = Infinity;
    for (const [what, make] of [
      ['a buffer', () => this.held.fill(300 * 1024 * 1024)],
      ['a list', () => this.held.put({ l: new Array(4000000) })],
    ]) {
      let said = '';
      try { make(); } catch (e) { said = e.message; }
      expect(`${what} is refused as too big`, /more than/.test(said), true);
    }
  }

  closeAndSpin() {
    this.held.close();
    for (;;) {}
  }

  fillBig() {
    let refused = false;
    try {
      this.held.fill(100 * 1024 * 1024);
    } catch (e) {
      refused = true;
    }
    expect('a buffer past the memory limit is refused', refused, true);
  }
}
"#;

/// What the host's sinks saw, and which it destroyed.
#[derive(Default)]
struct Sinks {
    log: Mutex<Vec<String>>,
    destroyed: Mutex<Vec<usize>>,
    /// The holder, for a `put` that asks the library something back.
    holder: Mutex<Option<Arc<DescribedRemote>>>,
}

impl Serve for Sinks {
    fn call(
        &self,
        ctx: usize,
        kind: &KindDescription,
        slot: usize,
        mut args: Vec<Arg<'_>>,
    ) -> Result<Answer, ProviderError> {
        match (kind.slots()[slot].name(), args.pop()) {
            ("put", Some(Arg::Map(map))) => {
                if map.get("reenter").is_some() {
                    let holder = self.holder.lock().unwrap().clone().expect("a holder");
                    let Ok(Answer::Uint(flag)) = holder.call_named("peek_flag", vec![]) else {
                        panic!("a flag");
                    };
                    self.log.lock().unwrap().push(format!("flag {flag}"));
                    return Ok(Answer::Nothing);
                }
                let keys: Vec<&str> = map.keys().collect();
                self.log
                    .lock()
                    .unwrap()
                    .push(format!("{ctx} put {}", keys.join(",")));
                Ok(Answer::Nothing)
            }
            ("zero", None) => Ok(Answer::Float(-0.0)),
            ("tick", Some(Arg::Uint(_))) => {
                self.log.lock().unwrap().push("tick".to_string());
                Ok(Answer::Nothing)
            }
            ("fill", Some(Arg::Buffer(_))) => Ok(Answer::Int(0)),
            (other, rest) => panic!("{other}: {rest:?}"),
        }
    }

    fn destroy(&self, ctx: usize) {
        self.destroyed.lock().unwrap().push(ctx);
    }
}

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

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("guatiao-js-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

fn described(name: &str) -> Arc<KindDescription> {
    let value = guatiao_serde::text::json::from_str(
        KINDS,
        guatiao::Alloc::rust(),
        guatiao_serde::Presentation::new(),
    )
    .expect("JSON");
    let all = KindDescription::document(&value).expect("descriptions");
    Arc::new(
        all.into_iter()
            .find(|k| k.name() == name)
            .expect("a kind of that name"),
    )
}

/// A registry serving the maker and the holder from one source.
fn registry(dir: &Path, limits: Option<Map>) -> Registry {
    std::fs::write(dir.join("kinds.json"), KINDS).expect("written");
    for (name, code, kind) in [("maker", MAKER, "maker"), ("holder", HOLDER, "holder")] {
        std::fs::write(dir.join(format!("{name}.js")), code.replace("KINDS", KINDS))
            .expect("written");
        std::fs::write(
            dir.join(format!("{name}.guatiao.toml")),
            format!(
                "type = \"js\"\nname = \"{name}\"\nversion = \"1.0.0\"\nmain = \"{name}.js\"\nkinds = [\"{kind}\"]\n"
            ),
        )
        .expect("written");
    }
    let mut config = Map::new();
    let mut dirs = List::new();
    dirs.push(dir.display().to_string().as_str());
    config.set("dirs", dirs);
    let mut kinds = List::new();
    kinds.push(dir.join("kinds.json").display().to_string().as_str());
    config.set("kinds", kinds);
    if let Some(limits) = limits {
        config.set("limits", limits);
    }
    let source = Arc::new(
        BridgeSource::<JsEngine>::pinned(&config, "js-handles")
            .expect("QuickJS starts")
            .with_mechanism(Arc::new(Libffi)),
    );
    let mut registry = Registry::new("js-handles", "1.0");
    registry.add_source("js", Held(source)).expect("added");
    let changed = registry.refresh();
    assert!(changed.refused.is_empty(), "{changed:?}");
    registry
}

fn remote(registry: &Registry, key: &str, kind: &str) -> DescribedRemote {
    let provider = registry.provider(key).expect("served");
    let caller = Arc::new(Caller::new(described(kind), &Libffi).expect("interfaces"));
    DescribedRemote::new(provider.view(), caller).expect("its table fits")
}

fn sinks() -> (Arc<Sinks>, ServedTable) {
    let sinks = Arc::new(Sinks::default());
    let table =
        ServedTable::new(described("sink"), |_| true, sinks.clone(), &Libffi).expect("built");
    (sinks, table)
}

/// Closes every library, so that what a script held of the host's is
/// destroyed while the host's tables are still there.
fn finish(registry: &mut Registry) {
    let keys: Vec<String> = registry
        .loaded()
        .iter()
        .map(|l| l.key.to_string())
        .collect();
    for key in keys {
        registry.retire(&key).expect("retired");
    }
    assert!(!registry.collect().is_empty(), "closed");
}

fn ok(answer: Result<Answer, ProviderError>) -> Answer {
    answer.unwrap_or_else(|e| panic!("{}", e.message()))
}

#[test]
fn an_object_one_library_serves_and_another_drops_is_destroyed_outside_the_collector() {
    let dir = scratch("handles-finalizer");
    let mut registry = registry(&dir, None);
    let maker = remote(&registry, "maker_makes", "maker");
    let holder = remote(&registry, "holder_holds", "holder");

    // The maker's object reaches the holder as a handle; the holder lets it
    // go unclosed, so QuickJS frees it, and the host destroys the object
    // through the maker, whose cleanup runs script.
    for _ in 0..3 {
        let Answer::Object(made) = ok(maker.call_named("make", vec![])) else {
            panic!("an object");
        };
        ok(holder.call_named("drop_it", vec![Arg::Object(made)]));
    }
    let Answer::Uint(disposed) = ok(maker.call_named("disposed", vec![])) else {
        panic!("a count");
    };
    assert_eq!(disposed, 3, "each object was disposed once, after the call");
    drop((maker, holder));
    finish(&mut registry);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_library_cannot_hand_back_an_object_it_was_not_given() {
    let dir = scratch("handles-forged");
    let (sinks, table) = sinks();
    let mut registry = registry(&dir, None);
    let maker = remote(&registry, "maker_makes", "maker");
    let holder = remote(&registry, "holder_holds", "holder");

    ok(holder.call_named(
        "hold",
        vec![Arg::Object(table.object(1).expect("an object"))],
    ));
    // The maker answers a lookalike of the first handle there is.
    let forged = maker.call_named("forge", vec![]);
    assert!(
        forged.is_err(),
        "the holder's object must not leave through the maker"
    );
    assert!(
        sinks.destroyed.lock().unwrap().is_empty(),
        "nothing destroyed"
    );
    ok(holder.call_named("use_held", vec![]));
    assert_eq!(*sinks.log.lock().unwrap(), ["1 put still"]);
    drop((maker, holder));
    finish(&mut registry);
    assert_eq!(
        *sinks.destroyed.lock().unwrap(),
        [1],
        "closing the library destroys what it held"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_refused_call_leaves_the_objects_it_was_lent_alone() {
    let dir = scratch("handles-refused");
    let (sinks, table) = sinks();
    let mut registry = registry(&dir, None);
    let holder = remote(&registry, "holder_holds", "holder");

    ok(holder.call_named(
        "hold",
        vec![Arg::Object(table.object(1).expect("an object"))],
    ));
    let other = table.object(2).expect("an object");
    ok(holder.call_named("probe", vec![Arg::LentObject(other.lend())]));
    assert!(
        sinks.destroyed.lock().unwrap().is_empty(),
        "an object only lent to a refused call is not destroyed"
    );
    assert_eq!(*sinks.log.lock().unwrap(), ["1 put probe"]);
    drop((holder,));
    finish(&mut registry);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_float_answer_keeps_its_sign() {
    let dir = scratch("handles-zero");
    let (_sinks, table) = sinks();
    let mut registry = registry(&dir, None);
    let holder = remote(&registry, "holder_holds", "holder");
    ok(holder.call_named(
        "hold",
        vec![Arg::Object(table.object(1).expect("an object"))],
    ));
    ok(holder.call_named("check_zero", vec![]));
    drop((holder,));
    finish(&mut registry);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_buffer_a_script_asks_a_host_for_is_bounded_by_the_memory_limit() {
    let dir = scratch("handles-buffer");
    let mut limits = Map::new();
    limits.set("memory", 16u64 * 1024 * 1024);
    let (_sinks, table) = sinks();
    let mut registry = registry(&dir, Some(limits));
    let holder = remote(&registry, "holder_holds", "holder");
    ok(holder.call_named(
        "hold",
        vec![Arg::Object(table.object(1).expect("an object"))],
    ));
    ok(holder.call_named("fill_big", vec![]));
    drop((holder,));
    finish(&mut registry);
    let _ = std::fs::remove_dir_all(&dir);
}

fn with_limits(pairs: &[(&str, u64)]) -> Option<Map> {
    let mut map = Map::new();
    for (k, v) in pairs {
        map.set(k, *v);
    }
    Some(map)
}

#[test]
fn a_stopped_calls_reactions_reach_no_host() {
    let dir = scratch("handles-reactions");
    let (sinks, table) = sinks();
    let mut registry = registry(&dir, with_limits(&[("operations", 200_000)]));
    let holder = remote(&registry, "holder_holds", "holder");
    ok(holder.call_named(
        "hold",
        vec![Arg::Object(table.object(1).expect("an object"))],
    ));
    let stopped = holder.call_named("leak", vec![]).unwrap_err();
    assert_eq!(
        stopped.message(),
        "stopped at its operations limit (200000)"
    );
    assert_eq!(
        *sinks.log.lock().unwrap(),
        Vec::<String>::new(),
        "no reaction of a stopped call called the host"
    );
    drop(holder);
    finish(&mut registry);
}

#[test]
fn a_reaction_does_not_run_inside_a_call_made_in_the_middle_of_its_script() {
    let dir = scratch("handles-nested");
    let (sinks, table) = sinks();
    let mut registry = registry(&dir, None);
    let holder = Arc::new(remote(&registry, "holder_holds", "holder"));
    *sinks.holder.lock().unwrap() = Some(Arc::clone(&holder));
    ok(holder.call_named(
        "hold",
        vec![Arg::Object(table.object(1).expect("an object"))],
    ));
    ok(holder.call_named("nested", vec![]));
    assert_eq!(
        *sinks.log.lock().unwrap(),
        ["flag 0"],
        "the reaction waited for the script it was queued by"
    );
    let Answer::Uint(flag) = ok(holder.call_named("peek_flag", vec![])) else {
        panic!("a flag");
    };
    assert_eq!(flag, 1, "and ran when that call returned");
    *sinks.holder.lock().unwrap() = None;
    drop(holder);
    finish(&mut registry);
}

#[test]
fn a_script_cannot_raise_the_bounds_it_is_held_to() {
    let dir = scratch("handles-bounds");
    let (_sinks, table) = sinks();
    let mut registry = registry(&dir, with_limits(&[("memory", 16 * 1024 * 1024)]));
    let holder = remote(&registry, "holder_holds", "holder");
    ok(holder.call_named(
        "hold",
        vec![Arg::Object(table.object(1).expect("an object"))],
    ));
    ok(holder.call_named("forge_bounds", vec![]));
    drop(holder);
    finish(&mut registry);
}

#[test]
fn closing_an_object_does_not_hide_the_limit_a_call_then_hits() {
    let dir = scratch("handles-steal");
    let mut registry = registry(&dir, with_limits(&[("operations", 200_000)]));
    let maker = remote(&registry, "maker_makes", "maker");
    let holder = remote(&registry, "holder_holds", "holder");
    // An object this very source serves, so that destroying it runs a
    // release of its own while the call's stop is pending.
    let Answer::Object(made) = ok(maker.call_named("make", vec![])) else {
        panic!("an object");
    };
    ok(holder.call_named("hold", vec![Arg::Object(made)]));
    let stopped = holder.call_named("close_and_spin", vec![]).unwrap_err();
    assert_eq!(
        stopped.message(),
        "stopped at its operations limit (200000)"
    );
    drop((maker, holder));
    finish(&mut registry);
}
