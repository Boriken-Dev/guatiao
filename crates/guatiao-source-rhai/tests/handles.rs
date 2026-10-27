// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A script calling the slots of objects a host hands it: lent and handed
//! over, with text and number answers, a slot that fails raising a
//! refusal the script can catch or let through, and a call refused before
//! it runs leaving the script's objects as they were.

extern crate alloc;

use alloc::sync::Arc;
use std::path::Path;
use std::sync::Mutex;

use guatiao::library::{Candidate, Host, LocalLibrary, Object, ProviderError, Registry, Source};
use guatiao::{List, Map, Status};
use guatiao_bridge::BridgeSource;
use guatiao_libffi::Libffi;
use guatiao_source_rhai::RhaiEngine;

/// What the host holds: the script calls its slots.
#[guatiao::kind(object)]
pub trait Echo: Send {
    /// Answers the text back, or refuses an empty one.
    fn say(&mut self, what: &str) -> Result<String, ProviderError>;
    /// How many times it was asked.
    fn tick(&mut self) -> i64;
    /// Ticks `other`, lent for the call, `times` times.
    fn pair(&mut self, other: &mut dyn Echo, times: u8) -> Result<i64, ProviderError>;
    /// Takes `other` over, and answers `times`.
    fn adopt(&mut self, other: Object<dyn Echo>, times: u8) -> Result<i64, ProviderError>;
}

/// What the script serves: it is handed an `Echo` to call.
#[guatiao::kind]
pub trait Relay: Send + Sync {
    /// Calls the lent echo, and answers what it said.
    fn relay(&self, target: &mut dyn Echo, what: &str) -> Result<String, ProviderError>;
    /// Calls the echo it is handed over, which it then drops.
    fn relay_taken(&self, target: Object<dyn Echo>) -> Result<String, ProviderError>;
    /// Passes `other` to slots of `target` in calls refused before they
    /// run, then calls `other`: what was refused and what `other` said.
    fn refused(
        &self,
        target: &mut dyn Echo,
        other: Object<dyn Echo>,
    ) -> Result<String, ProviderError>;
}

struct Parrot(Arc<Mutex<Vec<String>>>, i64);

impl Echo for Parrot {
    fn say(&mut self, what: &str) -> Result<String, ProviderError> {
        if what.is_empty() {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_NOT_FOUND,
                "nothing to echo",
            ));
        }
        self.0.lock().unwrap().push(what.to_string());
        Ok(format!("echo {what}"))
    }
    fn tick(&mut self) -> i64 {
        self.1 += 1;
        self.1
    }
    fn pair(&mut self, other: &mut dyn Echo, times: u8) -> Result<i64, ProviderError> {
        let mut last = 0;
        for _ in 0..times {
            last = other.tick();
        }
        Ok(last)
    }
    fn adopt(&mut self, other: Object<dyn Echo>, times: u8) -> Result<i64, ProviderError> {
        drop(other);
        Ok(i64::from(times))
    }
}

impl Drop for Parrot {
    fn drop(&mut self) {
        self.0.lock().unwrap().push("<destroyed>".to_string());
    }
}

struct Held(Arc<BridgeSource<RhaiEngine>>);

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

const RELAY: &str = r#"
const VERSION = "1.0.0";
let relay = #{
    kinds: ["relay"],
    relay: |target, what| {
        if what == "uncaught" {
            return target.say("");
        }
        try {
            return target.say(what) + " #" + target.tick();
        } catch (e) {
            return `refused ${e.status}: ${e.message}`;
        }
    },
    relay_taken: |target| {
        let first = target.say("one");
        // The handle is handed on to nobody: it goes with the call.
        first + " #" + target.tick()
    },
    refused: |target, other| {
        let said = [];
        // Each refused at its second argument, after `other` was read.
        try { target.pair(other, "twice"); said.push("ran"); } catch (e) { said.push(e.status); }
        try { target.adopt(other, "twice"); said.push("ran"); } catch (e) { said.push(e.status); }
        `${said[0]} ${said[1]} ${other.say("still here")} #${other.tick()}`
    },
};
#{ providers: #{ relay: relay } }
"#;

fn texts(items: &[&Path]) -> List {
    let mut list = List::new();
    for item in items {
        list.push(item.display().to_string().as_str());
    }
    list
}

fn registry(dir: &Path) -> Registry {
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
    std::fs::write(dir.join("rhai_relay.rhai"), RELAY).expect("written");
    std::fs::write(
        dir.join("rhai_relay.guatiao.toml"),
        "type = \"rhai\"\nmodule = \"rhai_relay\"\nkinds = [\"relay\"]\n",
    )
    .expect("written");
    let mut config = Map::new();
    config.set("dirs", texts(&[dir]));
    config.set("kinds", texts(&[&kinds]));
    let source = Arc::new(
        BridgeSource::<RhaiEngine>::pinned(&config, "rhai-handles")
            .expect("Rhai starts")
            .with_mechanism(Arc::new(Libffi)),
    );
    let mut registry = Registry::new("rhai-handles-host", "1.0");
    registry.add_source("rhai", Held(source)).expect("added");
    let changed = registry.refresh();
    assert!(changed.refused.is_empty(), "{changed:?}");
    registry
}

#[test]
fn a_script_calls_the_slots_of_the_objects_it_is_handed() {
    let dir = std::env::temp_dir().join(format!("guatiao-rhai-handles-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    let registry = registry(&dir);
    let relay: Arc<dyn Relay> = registry
        .offer::<dyn Relay>("rhai_relay_relay")
        .expect("served")
        .expect("as the trait")
        .shared()
        .expect("an instance");

    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut lent = Parrot(Arc::clone(&seen), 0);
    assert_eq!(relay.relay(&mut lent, "hi").expect("relayed"), "echo hi #1");
    assert_eq!(
        relay
            .relay(&mut lent, "")
            .unwrap_or_else(|e| panic!("{}", e.message())),
        "refused not_found: nothing to echo"
    );
    // A refusal the script lets through arrives with its status and words.
    let through = relay.relay(&mut lent, "uncaught").unwrap_err();
    assert_eq!(through.status, Status::GUATIAO_ERR_NOT_FOUND);
    assert_eq!(through.message(), "nothing to echo");
    assert_eq!(
        *seen.lock().unwrap(),
        ["hi"],
        "a lent object stays the caller's"
    );
    drop(lent);
    assert_eq!(*seen.lock().unwrap(), ["hi", "<destroyed>"]);

    let seen = Arc::new(Mutex::new(Vec::new()));
    let taken = Parrot(Arc::clone(&seen), 10).into_object();
    assert_eq!(relay.relay_taken(taken).expect("relayed"), "echo one #11");
    assert_eq!(
        *seen.lock().unwrap(),
        ["one", "<destroyed>"],
        "an object handed to a script is destroyed when the script lets go"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_call_refused_before_it_runs_leaves_the_scripts_objects_alive() {
    let dir = std::env::temp_dir().join(format!(
        "guatiao-rhai-handles-refused-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    let registry = registry(&dir);
    let relay: Arc<dyn Relay> = registry
        .offer::<dyn Relay>("rhai_relay_relay")
        .expect("served")
        .expect("as the trait")
        .shared()
        .expect("an instance");

    let mut target = Parrot(Arc::new(Mutex::new(Vec::new())), 0);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let other = Parrot(Arc::clone(&seen), 0).into_object();
    let said = relay
        .refused(&mut target, other)
        .unwrap_or_else(|e| panic!("{}", e.message()));
    assert_eq!(said, "bad_value bad_value echo still here #1");
    assert_eq!(
        *seen.lock().unwrap(),
        ["still here", "<destroyed>"],
        "the refused calls left the object to the script, which let it go after"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
