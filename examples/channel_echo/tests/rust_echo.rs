// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The echo library, loaded from its file and called as the trait, keeps
//! every channel rule.

#[path = "rules/mod.rs"]
mod rules;

use std::path::PathBuf;

use channel_echo_kind::ChannelEcho;
use guatiao::Map;
use guatiao::library::Registry;

/// The library built beside this test binary.
fn library() -> PathBuf {
    let exe = std::env::current_exe().expect("a test binary knows its own path");
    let deps = exe.parent().expect("in a directory");
    let name = format!(
        "{}channel_echo{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    [deps.to_path_buf(), deps.join("..")]
        .into_iter()
        .map(|dir| dir.join(&name))
        .find(|path| path.is_file())
        .expect("cargo builds the package's cdylib before its tests")
}

#[test]
fn the_rust_echo_keeps_every_rule() {
    let mut registry = Registry::new("channel-host", "1.0");
    registry.load_file(&library()).expect("the echo loads");
    let echo = registry
        .offer::<dyn ChannelEcho>("channel_echo_echo")
        .expect("offered")
        .expect("as the trait")
        .shared()
        .expect("its one instance");
    rules::hold_every_rule("the Rust echo", &|config: &Map, sink| {
        echo.start(config, sink)
    });
}

#[test]
fn the_rust_echo_lets_the_sink_go_once_closed() {
    let mut registry = Registry::new("channel-host", "1.0");
    registry.load_file(&library()).expect("the echo loads");
    let echo = registry
        .offer::<dyn ChannelEcho>("channel_echo_echo")
        .expect("offered")
        .expect("as the trait")
        .shared()
        .expect("its one instance");
    let (probe, sink) = rules::Probe::new();
    let channel = echo.start(&Map::new(), sink).expect("started");
    channel.close(0);
    assert_eq!(probe.closed(), Ok(0));
    probe.settle().expect("nothing after closed");
    assert!(probe.destroyed(), "the sink was destroyed after closed");
}

#[test]
fn a_configuration_that_is_not_a_count_is_refused() {
    let mut registry = Registry::new("channel-host", "1.0");
    registry.load_file(&library()).expect("the echo loads");
    let echo = registry
        .offer::<dyn ChannelEcho>("channel_echo_echo")
        .expect("offered")
        .expect("as the trait")
        .shared()
        .expect("its one instance");
    let (probe, sink) = rules::Probe::new();
    let mut config = Map::new();
    config.set("close_after", "soon");
    let Err(refused) = echo.start(&config, sink) else {
        panic!("a count that is not one was accepted");
    };
    assert_eq!(refused.message(), "`close_after` is not a count");
    assert!(
        probe.destroyed(),
        "a refused start still destroys the sink it was handed"
    );
}
