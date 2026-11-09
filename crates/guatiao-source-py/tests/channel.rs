// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The Python echo, served by the bridge and called from a Rust host as
//! the channel kind, keeps every rule of the channel: it pushes from a
//! `threading.Thread` of its own, which the GIL lets call the host's sink.
//!
//! The bridge is this crate's cdylib, which `cargo test` does not build:
//! `cargo build` first.

extern crate alloc;

#[path = "../../../examples/channel_echo/tests/rules/mod.rs"]
mod rules;

use std::path::PathBuf;

use channel_echo_kind::ChannelEcho;
use guatiao::library::Registry;
use guatiao::{List, Map};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn bridge() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let debug = exe.parent()?.parent()?;
    let path = debug.join(format!(
        "{}guatiao_source_py{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    path.is_file().then_some(path)
}

fn text_list(items: &[PathBuf]) -> List {
    let mut list = List::new();
    for item in items {
        list.push(item.to_string_lossy().as_ref());
    }
    list
}

/// On Windows, the Python installation the bridge's `python3.dll` is
/// mapped from: the first directory on `PATH` holding it beside a `Lib`.
fn python_home() -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .find(|dir| dir.join("python3.dll").is_file() && dir.join("Lib").is_dir())
}

#[test]
fn the_python_echo_keeps_every_rule() {
    let Some(bridge) = bridge() else {
        assert!(
            std::env::var_os("GUATIAO_REQUIRE_ALL").is_none(),
            "the bridge is not built beside this test, and GUATIAO_REQUIRE_ALL is set"
        );
        println!("skipped: the bridge is not built beside this test; `cargo build` first");
        return;
    };
    let mut config = Map::new();
    config.set(
        "dirs",
        text_list(&[repo().join("examples/channel_echo/python/channel_echo_py")]),
    );
    config.set(
        "paths",
        text_list(&[repo().join("bindings/python/guatiao/src")]),
    );
    if cfg!(windows) {
        let home =
            python_home().expect("a Python installation on PATH, as the bridge built against");
        config.set("import_dirs", text_list(&[home]));
    }
    let mut registry = Registry::new("python-channel-host", "1.0");
    registry
        .add_source_file(&bridge, &config)
        .unwrap_or_else(|e| panic!("the bridge was refused: {e}"));
    let changed = registry.refresh();
    assert!(changed.refused.is_empty(), "{changed:?}");
    let echo = registry
        .offer::<dyn ChannelEcho>("channel_echo_py_echo")
        .expect("the echo is served")
        .expect("as the trait")
        .shared()
        .expect("an instance");
    rules::hold_every_rule("the Python echo", &|config: &Map, sink| {
        echo.start(config, sink)
    });
}

#[test]
fn a_python_start_that_raises_gives_up_the_sink_it_took() {
    let Some(bridge) = bridge() else {
        assert!(
            std::env::var_os("GUATIAO_REQUIRE_ALL").is_none(),
            "the bridge is not built beside this test, and GUATIAO_REQUIRE_ALL is set"
        );
        println!("skipped: the bridge is not built beside this test; `cargo build` first");
        return;
    };
    let mut config = Map::new();
    config.set(
        "dirs",
        text_list(&[repo().join("crates/guatiao-source-py/tests/python/channel_refuser")]),
    );
    config.set(
        "paths",
        text_list(&[repo().join("bindings/python/guatiao/src")]),
    );
    if cfg!(windows) {
        let home =
            python_home().expect("a Python installation on PATH, as the bridge built against");
        config.set("import_dirs", text_list(&[home]));
    }
    let mut registry = Registry::new("python-refuser-host", "1.0");
    registry
        .add_source_file(&bridge, &config)
        .unwrap_or_else(|e| panic!("the bridge was refused: {e}"));
    let changed = registry.refresh();
    assert!(changed.refused.is_empty(), "{changed:?}");
    let refuser = registry
        .offer::<dyn ChannelEcho>("channel_refuser_refuser")
        .expect("the refuser is served")
        .expect("as the trait")
        .shared()
        .expect("an instance");
    let (probe, sink) = rules::Probe::new(rules::PATIENCE);
    assert!(refuser.start(&Map::new(), sink).is_err(), "start raised");
    assert!(
        probe.destroyed(),
        "the sink was destroyed before start returned"
    );
}
