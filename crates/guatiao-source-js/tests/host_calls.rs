// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! JavaScript calling a host: an object the host lends or hands over is a
//! JavaScript object whose methods are its kind's slots, taking numbers,
//! text, bytes, trees and a buffer, and answering as the kind says.
//!
//! No kind of the repository has an object kind taking trees and buffers
//! from a provider, so these kinds are made here, as a description.

extern crate alloc;

use alloc::sync::Arc;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use guatiao::library::kind::{
    Answer, Arg, Caller, DescribedRemote, KindDescription, Serve, ServedTable,
};
use guatiao::library::{Candidate, Host, LocalLibrary, ProviderError, Registry, Source};
use guatiao::{List, Map, Status};
use guatiao_bridge::BridgeSource;
use guatiao_libffi::Libffi;
use guatiao_source_js::JsEngine;

const KINDS: &str = r#"{"format": 1, "kinds": [
  {"name": "sink", "floor_hash": 4242, "object": true, "slots": [
    {"name": "put", "required": true, "fallible": true,
     "args": [{"name": "tree", "type": "map", "pass": "borrowed"}]},
    {"name": "fill", "required": true, "fallible": false,
     "args": [{"name": "dst", "type": "bytes_mut"}], "answer": {"type": "i64"}},
    {"name": "give", "required": true, "fallible": true,
     "args": [{"name": "items", "type": "list", "pass": "taken"}]},
    {"name": "tweak", "required": true, "fallible": true,
     "args": [{"name": "tree", "type": "map", "pass": "lent"}]},
    {"name": "bits", "required": true, "fallible": true,
     "args": [{"name": "flag", "type": "bool"}, {"name": "n", "type": "u8"},
              {"name": "big", "type": "u64"}, {"name": "x", "type": "f64"},
              {"name": "raw", "type": "bytes"}],
     "answer": {"type": "text"}},
    {"name": "next_up", "required": true, "fallible": false, "args": [],
     "answer": {"type": "u64"}}
  ]},
  {"name": "feeder", "floor_hash": 2424, "object": false, "slots": [
    {"name": "feed", "required": true, "fallible": true,
     "args": [{"name": "sink", "type": "object", "kind": "sink", "pass": "lent"}]},
    {"name": "hand", "required": true, "fallible": true,
     "args": [{"name": "sink", "type": "object", "kind": "sink", "pass": "taken"}]}
  ]}
]}"#;

const FEEDER: &str = r#"
import { Gone, NotFound } from 'guatiao';
import { Provider } from 'guatiao/provide';
import { kindsOf } from 'guatiao/kinds';

const { feeder } = kindsOf(KINDS);

const expect = (what, got, wanted) => {
  if (got !== wanted) throw new Error(`${what}: ${String(got)}, wanted ${String(wanted)}`);
};

export class Feeds extends Provider {
  static kinds = [feeder];

  feed(sink) {
    sink.put({ a: 1, b: [1, 2], c: { d: null }, e: new Uint8Array([9]) });
    try {
      sink.put({ refuse: true });
      throw new Error('a refusal is thrown');
    } catch (e) {
      expect('the status', e instanceof NotFound, true);
      expect('the words', e.message, 'no such thing');
    }
    const dst = new Uint8Array(6);
    expect('bytes written', sink.fill(dst), 4n);
    expect('what was written', dst.join(','), '1,2,3,4,0,0');
    sink.give([1, 'x', { y: 2 }]);
    const lent = { k: 'v' };
    sink.tweak(lent);
    expect('the lent map', JSON.stringify(lent), '{"k":"v","added":"yes"}');
    expect('the answer', sink.bits(true, 7, 2n ** 63n, 1.5, new Uint8Array([1, 2])), 'bits-ok');
    expect('a wide answer', sink.nextUp(), 18446744073709551615n);
    try {
      sink.bits('yes', 7, 1n, 1.5, new Uint8Array(0));
      throw new Error('a wrong argument is refused');
    } catch (e) {
      expect('the argument is named', /argument `flag` is a boolean/.test(e.message), true);
    }
    try {
      sink.bits(true, 256, 1n, 1.5, new Uint8Array(0));
      throw new Error('a number past its width is refused');
    } catch (e) {
      expect('the width', /does not fit|out of range|range/i.test(e.message), true);
    }
  }

  hand(sink) {
    sink.put({ mode: 'handed' });
    sink.close();
    try {
      sink.put({});
      throw new Error('a closed object is gone');
    } catch (e) {
      expect('closed', e instanceof Gone, true);
    }
  }
}
"#;

/// What the host's sink saw, and how many it destroyed.
#[derive(Default)]
struct Sink {
    log: Mutex<Vec<String>>,
    destroyed: Mutex<usize>,
}

impl Serve for Sink {
    fn call(
        &self,
        _ctx: usize,
        kind: &KindDescription,
        slot: usize,
        mut args: Vec<Arg<'_>>,
    ) -> Result<Answer, ProviderError> {
        let name = kind.slots()[slot].name();
        match (name, args.pop()) {
            ("put", Some(Arg::Map(map))) => {
                if map.get("refuse").is_some() {
                    return Err(ProviderError::new(
                        Status::GUATIAO_ERR_NOT_FOUND,
                        "no such thing",
                    ));
                }
                let keys: Vec<&str> = map.keys().collect();
                self.log
                    .lock()
                    .unwrap()
                    .push(format!("put {}", keys.join(",")));
                Ok(Answer::Nothing)
            }
            ("fill", Some(Arg::Buffer(dst))) => {
                dst[..4].copy_from_slice(&[1, 2, 3, 4]);
                Ok(Answer::Int(4))
            }
            ("give", Some(Arg::TakenList(list))) => {
                let shape: Vec<String> = list
                    .iter()
                    .map(|v| format!("{:?}", v.tag().expect("a tag")))
                    .collect();
                self.log
                    .lock()
                    .unwrap()
                    .push(format!("give {}", shape.join(",")));
                Ok(Answer::Nothing)
            }
            ("tweak", Some(Arg::LentMap(map))) => {
                map.set("added", "yes");
                Ok(Answer::Nothing)
            }
            ("next_up", None) => Ok(Answer::Uint(u64::MAX)),
            ("bits", Some(Arg::Bytes(raw))) => {
                // The arguments come off in reverse.
                let Some(Arg::Float(x)) = args.pop() else {
                    panic!("a float")
                };
                let Some(Arg::Uint(big)) = args.pop() else {
                    panic!("a u64")
                };
                let Some(Arg::Uint(n)) = args.pop() else {
                    panic!("a u8")
                };
                let Some(Arg::Bool(flag)) = args.pop() else {
                    panic!("a bool")
                };
                self.log
                    .lock()
                    .unwrap()
                    .push(format!("bits {flag} {n} {big} {x} {raw:?}"));
                Ok(Answer::Text("bits-ok".into()))
            }
            (other, rest) => panic!("{other}: {rest:?}"),
        }
    }

    fn destroy(&self, _ctx: usize) {
        *self.destroyed.lock().unwrap() += 1;
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

fn registry(dir: &Path) -> Registry {
    std::fs::write(dir.join("kinds.json"), KINDS).expect("written");
    std::fs::write(dir.join("feeder.js"), FEEDER.replace("KINDS", KINDS)).expect("written");
    std::fs::write(
        dir.join("feeder.guatiao.toml"),
        "type = \"js\"\nname = \"feeder\"\nversion = \"1.0.0\"\nmain = \"feeder.js\"\nkinds = [\"feeder\"]\n",
    )
    .expect("written");
    let mut config = Map::new();
    let mut dirs = List::new();
    dirs.push(dir.display().to_string().as_str());
    config.set("dirs", dirs);
    let mut kinds = List::new();
    kinds.push(dir.join("kinds.json").display().to_string().as_str());
    config.set("kinds", kinds);
    let source = Arc::new(
        BridgeSource::<JsEngine>::pinned(&config, "js-host-calls")
            .expect("QuickJS starts")
            .with_mechanism(Arc::new(Libffi)),
    );
    let mut registry = Registry::new("js-host-calls", "1.0");
    registry.add_source("js", Held(source)).expect("added");
    let changed = registry.refresh();
    assert!(changed.refused.is_empty(), "{changed:?}");
    registry
}

#[test]
fn javascript_calls_the_objects_a_host_lends_and_hands_over() {
    let dir = scratch("host-calls");
    let registry = registry(&dir);
    let provider = registry.provider("feeder_feeds").expect("served");
    let caller = Arc::new(Caller::new(described("feeder"), &Libffi).expect("interfaces"));
    let feeds = DescribedRemote::new(provider.view(), caller).expect("its table fits");

    let sink = Arc::new(Sink::default());
    let table =
        ServedTable::new(described("sink"), |_| true, sink.clone(), &Libffi).expect("built");
    let lent = table.object(1).expect("an object kind");
    feeds
        .call_named("feed", vec![Arg::LentObject(lent.lend())])
        .unwrap_or_else(|e| panic!("{}", e.message()));
    assert_eq!(
        *sink.log.lock().unwrap(),
        [
            "put a,b,c,e",
            "give GUATIAO_NUMBER,GUATIAO_STRING,GUATIAO_MAP",
            "bits true 7 9223372036854775808 1.5 [1, 2]",
        ]
        .map(String::from)
    );
    assert_eq!(
        *sink.destroyed.lock().unwrap(),
        0,
        "a lend destroys nothing"
    );
    drop(lent);
    assert_eq!(*sink.destroyed.lock().unwrap(), 1, "its owner destroys it");

    let given = table.object(2).expect("an object kind");
    feeds
        .call_named("hand", vec![Arg::Object(given)])
        .unwrap_or_else(|e| panic!("{}", e.message()));
    assert_eq!(
        *sink.destroyed.lock().unwrap(),
        2,
        "the library closed what it was handed"
    );
    assert_eq!(
        sink.log.lock().unwrap().last().map(String::as_str),
        Some("put mode"),
        "the call before the close got through"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
