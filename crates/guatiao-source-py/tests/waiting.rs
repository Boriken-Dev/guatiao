// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A Python call that waits on its host (a listener that takes a second to
//! answer) does not hold the interpreter: the GIL is let go while the host
//! runs, so another call into a library answers meanwhile.
//!
//! The bridge is this crate's cdylib, which `cargo test` does not build:
//! `cargo build` first, or this states a skip.

extern crate alloc;

use core::time::Duration;
use std::path::PathBuf;
use std::time::Instant;

use greeter_kind::{Conversation, Greeter, Listener};
use guatiao::library::{Object, Registry};
use guatiao::value::convert::TryAsRef;
use guatiao::{List, Map};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn bridge() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let name = format!(
        "{}guatiao_source_py{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let path = exe.parent()?.parent()?.join(name);
    path.is_file().then_some(path)
}

fn text_list(items: &[PathBuf]) -> List {
    let mut list = List::new();
    for item in items {
        list.push(item.to_string_lossy().as_ref());
    }
    list
}

/// A listener whose `heard` takes a second.
struct Slow;

impl Listener for Slow {
    fn heard(&mut self, _what: &str) {
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[test]
fn a_call_waiting_on_its_host_lets_another_call_answer() {
    let Some(bridge) = bridge() else {
        println!("skipped: the bridge is not built beside this test; `cargo build` first");
        return;
    };
    let mut config = Map::new();
    config.set(
        "dirs",
        text_list(&[repo().join("examples/python_provider")]),
    );
    config.set(
        "paths",
        text_list(&[
            repo().join("bindings/python/guatiao/src"),
            repo().join("examples/greeter_kind/python"),
        ]),
    );
    if cfg!(windows) {
        let home = std::env::split_paths(&std::env::var_os("PATH").expect("PATH"))
            .find(|dir| dir.join("python3.dll").is_file() && dir.join("Lib").is_dir())
            .expect("a Python installation on PATH, as the bridge built against");
        config.set("import_dirs", text_list(&[home]));
    }
    let mut registry = Registry::new("waiting-host", "1.0");
    registry
        .add_source_file(&bridge, &config)
        .unwrap_or_else(|e| panic!("the bridge was refused: {e}"));
    let changed = registry.refresh();
    assert!(changed.refused.is_empty(), "{changed:?}");
    let hello = registry
        .offer::<dyn Greeter>("py_greeter_hello")
        .expect("served")
        .expect("as the trait")
        .shared()
        .expect("an instance");
    let mut talk: Object<dyn Conversation> = hello.start(Slow.into_object()).expect("started");
    let waiting = std::thread::spawn(move || {
        let started = Instant::now();
        talk.say("one").expect("said");
        started.elapsed()
    });
    std::thread::sleep(Duration::from_millis(300));
    let started = Instant::now();
    let answer = hello.greet("ana").expect("greeted");
    let took = started.elapsed();
    assert_eq!(
        answer
            .get("greeting")
            .and_then(TryAsRef::<str>::try_as_ref)
            .unwrap_or(""),
        "hello, ana"
    );
    assert!(took < Duration::from_millis(500), "answered after {took:?}");
    assert!(waiting.join().expect("joined") >= Duration::from_millis(900));
}
