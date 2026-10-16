// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A derived library is not unloaded while a handle into it is held, nor
//! while anything it handed out is alive, and is once neither is. Its own
//! binary: it unmaps `derived_greeter`, which no other test may be
//! holding.

#![cfg(all(feature = "provider", feature = "load"))]

use std::path::PathBuf;

use greeter_kind::{Conversation, Greeter, Listener};
use guatiao::library::{Object, Registry, UnloadError};
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

fn refused_busy(why: &UnloadError) -> bool {
    matches!(why, UnloadError::Refused { status, .. } if *status == Status::GUATIAO_ERR_BUSY)
}

#[test]
fn it_refuses_while_a_handle_an_instance_or_an_object_is_alive() {
    let mut registry = Registry::new("refusal-tests", "1.0");
    registry
        .load_file(&library_path("derived_greeter"))
        .expect("loads")
        .loaded()
        .expect("is a library for this host");

    // A handle holds the library's lease: the registry counts it and
    // refuses before the library is asked.
    let remote = registry
        .offer::<dyn Greeter>("derived_greeter_hello")
        .unwrap()
        .unwrap()
        .remote()
        .expect("the default greeter is its own instance");
    let why = registry
        .unload("derived_greeter")
        .expect_err("a Remote is held");
    assert!(
        matches!(why, UnloadError::Busy { ref key, leases: 1 } if key == "derived_greeter"),
        "{why:?}"
    );
    assert_eq!(registry.loaded().len(), 1, "a refusal changes nothing");
    assert!(
        registry.provider("derived_greeter_hello").is_some(),
        "and its providers are answered again"
    );

    // An instance built from a configuration holds it too.
    let mut config = Map::new();
    config.set("prefix", "hey");
    let instance = registry
        .offer::<dyn Greeter>("derived_greeter_shouter")
        .unwrap()
        .unwrap()
        .instantiate(&Value::from(config))
        .expect("a fitting configuration builds");
    let why = registry
        .unload("derived_greeter")
        .expect_err("a Remote and an instance are held");
    assert!(
        matches!(why, UnloadError::Busy { leases: 2, .. }),
        "{why:?}"
    );
    drop(instance);

    // An object handed across raw holds no lease; the library's own count
    // of what it made is what refuses then.
    let chat = remote
        .start(Deaf.into_object())
        .expect("the derived greeter holds conversations");
    drop(remote);
    let raw = chat.into_raw();
    let why = registry
        .unload("derived_greeter")
        .expect_err("an object is alive");
    assert!(refused_busy(&why), "{why:?}");
    assert_eq!(registry.loaded().len(), 1, "a refusal changes nothing");
    // SAFETY: the conversation the library handed back, given up just
    // above and taken back once.
    drop(unsafe { Object::<dyn Conversation>::from_raw(raw) }.expect("it validates"));

    registry
        .unload("derived_greeter")
        .expect("nothing is alive now");
    assert!(registry.loaded().is_empty());
    assert!(
        matches!(
            registry.unload("derived_greeter"),
            Err(UnloadError::NotFound { .. })
        ),
        "it left with the unload"
    );
}
