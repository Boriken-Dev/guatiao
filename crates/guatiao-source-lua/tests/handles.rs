// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A script's objects through a call that fails: refused before it runs
//! at a later argument, or, in Luau, failing to write a lent tree back
//! after it ran. The object an earlier argument took out of its handle
//! goes back in, and the script calls it again.

#![cfg(feature = "source")]

extern crate alloc;

use alloc::sync::Arc;
use std::path::Path;
use std::sync::Mutex;

use guatiao::library::{Candidate, Host, LocalLibrary, Object, ProviderError, Registry, Source};
use guatiao::{List, Map};
use guatiao_bridge::BridgeSource;
use guatiao_libffi::Libffi;
use guatiao_source_lua::LuaEngine;

/// What the host holds: the script calls its slots.
#[guatiao::kind(object)]
pub trait Echo: Send {
    /// Answers the text back.
    fn say(&mut self, what: &str) -> Result<String, ProviderError>;
    /// How many times it was asked.
    fn tick(&mut self) -> i64;
    /// Ticks `other`, lent for the call, `times` times.
    fn pair(&mut self, other: &mut dyn Echo, times: u8) -> Result<i64, ProviderError>;
    /// Takes `other` over, and answers `times`.
    fn adopt(&mut self, other: Object<dyn Echo>, times: u8) -> Result<i64, ProviderError>;
    /// Marks `config`, lent for the call, and ticks `other`.
    fn tweak(&mut self, config: &mut Map, other: &mut dyn Echo) -> Result<(), ProviderError>;
}

/// What the script serves.
#[guatiao::kind]
pub trait Relay: Send + Sync {
    /// Passes `other` to slots of `target` in calls that fail, then calls
    /// `other`: how each failed and what `other` said.
    fn refused(
        &self,
        target: &mut dyn Echo,
        other: Object<dyn Echo>,
    ) -> Result<String, ProviderError>;
    /// In Luau, lends `target` a frozen table and then `other`, so the call
    /// runs and writing the table back fails; then calls `other`.
    fn unwritten(
        &self,
        target: &mut dyn Echo,
        other: Object<dyn Echo>,
    ) -> Result<String, ProviderError>;
}

struct Parrot(Arc<Mutex<Vec<String>>>, i64);

impl Echo for Parrot {
    fn say(&mut self, what: &str) -> Result<String, ProviderError> {
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
    fn tweak(&mut self, config: &mut Map, other: &mut dyn Echo) -> Result<(), ProviderError> {
        config.set("tweaked", true);
        other.tick();
        Ok(())
    }
}

impl Drop for Parrot {
    fn drop(&mut self) {
        self.0.lock().unwrap().push("<destroyed>".to_string());
    }
}

struct Held(Arc<BridgeSource<LuaEngine>>);

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

const RELAY: &str = r##"
local M = { _VERSION = "1.0.0" }

local Relay = { kinds = { "relay" } }

-- "refused" when calling `f` fails in the words expected.
local function attempt(words, f, ...)
  local ok, err = pcall(f, ...)
  if ok then
    return "ran"
  end
  return tostring(err):find(words, 1, true) and "refused" or tostring(err)
end

function Relay:refused(target, other)
  local said = {}
  -- Each refused at its second argument, after `other` was read.
  said[#said + 1] = attempt("not a string", target.pair, target, other, "twice")
  said[#said + 1] = attempt("not a string", target.adopt, target, other, "twice")
  local echoed, why = other:say("still here")
  said[#said + 1] = echoed or ("gone: " .. tostring(why))
  said[#said + 1] = "#" .. tostring(other:tick())
  return table.concat(said, " ")
end

function Relay:unwritten(target, other)
  if not table.freeze then
    return "no frozen tables in this Lua"
  end
  local frozen = table.freeze({ name = "ana" })
  local said = { attempt("readonly", target.tweak, target, frozen, other) }
  local echoed, why = other:say("still here")
  said[#said + 1] = echoed or ("gone: " .. tostring(why))
  said[#said + 1] = "#" .. tostring(other:tick())
  return table.concat(said, " ")
end

M.providers = { relay = Relay }
return M
"##;

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
    std::fs::write(dir.join("lua_relay.lua"), RELAY).expect("written");
    std::fs::write(
        dir.join("lua_relay.guatiao.toml"),
        "type = \"lua\"\nmodule = \"lua_relay\"\nkinds = [\"relay\"]\n",
    )
    .expect("written");
    let mut config = Map::new();
    config.set("dirs", texts(&[dir]));
    config.set("kinds", texts(&[&kinds]));
    let source = Arc::new(
        BridgeSource::<LuaEngine>::pinned(&config, "lua-handles")
            .expect("Lua starts")
            .with_mechanism(Arc::new(Libffi)),
    );
    let mut registry = Registry::new("lua-handles-host", "1.0");
    registry.add_source("lua", Held(source)).expect("added");
    let changed = registry.refresh();
    assert!(changed.refused.is_empty(), "{changed:?}");
    registry
}

/// Hands the script a fresh `other` through `call`, and answers what the
/// script said and what `other` saw by the time the call returned.
fn through(
    name: &str,
    call: impl FnOnce(&dyn Relay, &mut dyn Echo, Object<dyn Echo>) -> Result<String, ProviderError>,
) -> (String, Vec<String>) {
    let dir =
        std::env::temp_dir().join(format!("guatiao-lua-handles-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    let registry = registry(&dir);
    let relay: Arc<dyn Relay> = registry
        .offer::<dyn Relay>("lua_relay_relay")
        .expect("served")
        .expect("as the trait")
        .shared()
        .expect("an instance");
    let mut target = Parrot(Arc::new(Mutex::new(Vec::new())), 0);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let other = Parrot(Arc::clone(&seen), 0).into_object();
    let said = call(&*relay, &mut target, other).unwrap_or_else(|e| panic!("{}", e.message()));
    let seen = seen.lock().unwrap().clone();
    let _ = std::fs::remove_dir_all(&dir);
    (said, seen)
}

#[test]
fn a_call_refused_before_it_runs_leaves_the_scripts_objects_alive() {
    let (said, seen) = through("refused", |relay, target, other| {
        relay.refused(target, other)
    });
    assert_eq!(said, "refused refused echo still here #1");
    assert_eq!(seen, ["still here"], "the object is the script's still");
}

#[cfg(feature = "luau")]
#[test]
fn a_lent_tree_that_cannot_be_written_back_leaves_the_scripts_objects_alive() {
    let (said, seen) = through("unwritten", |relay, target, other| {
        relay.unwritten(target, other)
    });
    // The tweak ran, and ticked `other` once.
    assert_eq!(said, "refused echo still here #2");
    assert_eq!(seen, ["still here"], "the object is the script's still");
}
