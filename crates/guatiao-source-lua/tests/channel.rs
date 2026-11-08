// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The Lua echo, served by the source and called from a Rust host as the
//! channel kind, keeps every rule that needs no thread of its own (the
//! engine calls a provider only on its own thread).

#![cfg(feature = "source")]

extern crate alloc;

#[path = "../../../examples/channel_echo/tests/rules/mod.rs"]
mod rules;

use alloc::sync::Arc;
use std::path::PathBuf;

use channel_echo_kind::ChannelEcho;
use guatiao::library::{Candidate, Host, LocalLibrary, ProviderError, Registry, Source};
use guatiao::{List, Map};
use guatiao_bridge::BridgeSource;
use guatiao_libffi::Libffi;
use guatiao_source_lua::LuaEngine;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
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

#[test]
fn the_lua_echo_keeps_every_inline_rule() {
    let mut config = Map::new();
    let mut dirs = List::new();
    dirs.push(
        repo()
            .join("examples/channel_echo/lua")
            .display()
            .to_string(),
    );
    config.set("dirs", dirs);
    let mut kinds = List::new();
    kinds.push(
        repo()
            .join("examples/channel_echo/kind/include/channel_echo.kinds.json")
            .display()
            .to_string(),
    );
    config.set("kinds", kinds);
    let source = Arc::new(
        BridgeSource::<LuaEngine>::pinned(&config, "lua-channel-test")
            .expect("Lua starts")
            .with_mechanism(Arc::new(Libffi)),
    );
    let mut registry = Registry::new("lua-channel-host", "1.0");
    registry.add_source("lua", Held(source)).expect("added");
    let changed = registry.refresh();
    assert!(changed.refused.is_empty(), "{changed:?}");
    let echo = registry
        .offer::<dyn ChannelEcho>("channel_echo_lua_echo")
        .expect("the echo is served")
        .expect("as the trait")
        .shared()
        .expect("an instance");
    rules::hold_every_inline_rule("the Lua echo", &|config: &Map, sink| {
        echo.start(config, sink)
    });
}
