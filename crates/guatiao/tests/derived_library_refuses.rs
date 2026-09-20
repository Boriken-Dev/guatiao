// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A derived library refuses to be unloaded while anything it handed out
//! is alive, and agrees once it is not. Its own binary: it unmaps
//! `derived_greeter`, which no other test may be holding.

#![cfg(all(feature = "provider", feature = "load"))]

use std::path::PathBuf;

use greeter_kind::{Greeter, Listener};
use guatiao::library::{Registry, UnloadError};
use guatiao::{Map, Status, Value};

fn library_path(name: &str) -> PathBuf {
    let exe = std::env::current_exe().expect("a test binary knows its own path");
    let deps = exe.parent().expect("the test binary is in a directory");
    let file = format!(
        "{}{name}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    [
        deps.to_path_buf(),
        deps.parent().map(PathBuf::from).unwrap_or_default(),
    ]
    .into_iter()
    .map(|dir| dir.join(&file))
    .find(|p| p.exists())
    .expect("cargo built the dev-dependency cdylib")
}

struct Deaf;

impl Listener for Deaf {
    fn heard(&mut self, _what: &str) {}
}

fn busy(why: &UnloadError) -> bool {
    matches!(why, UnloadError::Refused { status, .. } if *status == Status::GUATIAO_ERR_BUSY)
}

#[test]
fn it_refuses_while_an_instance_or_an_object_is_alive() {
    let mut registry = Registry::new("refusal-tests", "1.0");
    registry
        .load_file(&library_path("derived_greeter"))
        .expect("loads")
        .loaded()
        .expect("is a library for this host");

    // An instance built from a configuration.
    let mut config = Map::new();
    config.set("prefix", "hey").unwrap();
    let instance = registry
        .offer::<dyn Greeter>("derived_greeter_shouter")
        .unwrap()
        .unwrap()
        .instantiate(&Value::from(config))
        .expect("a fitting configuration builds");
    // SAFETY: deliberately false; the library's own count is what refuses.
    let why = unsafe { registry.unload("derived_greeter") }.expect_err("an instance is alive");
    assert!(busy(&why), "{why:?}");
    drop(instance);

    // An object one of its kinds handed back.
    let chat = registry
        .offer::<dyn Greeter>("derived_greeter_hello")
        .unwrap()
        .unwrap()
        .start(Deaf.into_object())
        .expect("the derived greeter holds conversations");
    // SAFETY: as above.
    let why = unsafe { registry.unload("derived_greeter") }.expect_err("an object is alive");
    assert!(busy(&why), "{why:?}");
    assert_eq!(registry.loaded().len(), 1, "a refusal changes nothing");
    drop(chat);

    // SAFETY: nothing taken from the library is held any more.
    unsafe { registry.unload("derived_greeter") }.expect("nothing is alive now");
    assert!(registry.loaded().is_empty());
}
