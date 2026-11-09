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
use guatiao::Status;
use guatiao::library::{Registry, UnloadError};

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
    let (probe, sink) = rules::Probe::new(rules::PATIENCE);
    let channel = echo.start(&Map::new(), sink).expect("started");
    channel.close(0, "");
    assert_eq!(probe.closed_with(), Ok((0, String::new())));
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
    let (probe, sink) = rules::Probe::new(rules::PATIENCE);
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

#[test]
fn the_library_refuses_to_unload_while_its_thread_is_inside_the_sink() {
    let mut registry = Registry::new("channel-host", "1.0");
    registry.load_file(&library()).expect("the echo loads");
    let key = registry.loaded()[0].key.to_string();
    let (probe, sink) = rules::Probe::new(rules::PATIENCE);
    {
        let echo = registry
            .offer::<dyn ChannelEcho>("channel_echo_echo")
            .expect("offered")
            .expect("as the trait")
            .shared()
            .expect("its one instance");
        probe.hold();
        let channel = echo.start(&Map::new(), sink).expect("started");
        channel.send(b"x").expect("taken");
        std::thread::sleep(rules::GRACE);
        // The channel and every handle on the library are gone; the
        // thread is still inside the sink.
        drop(channel);
    }
    match registry.unload(&key) {
        Err(UnloadError::Refused { status, .. }) if status == Status::GUATIAO_ERR_BUSY => {}
        other => panic!("a thread still inside the sink, and unload answered {other:?}"),
    }
    probe.release();
    probe.closed().expect("closed once released");
    // The thread ends just after `closed` returns; the library is one
    // mapping for every test in this process, so the other tests' threads
    // count too.
    let mut last = None;
    let deadline = std::time::Instant::now() + rules::PATIENCE;
    while std::time::Instant::now() < deadline {
        match registry.unload(&key) {
            Ok(_) => return,
            Err(UnloadError::Refused { status, .. }) if status == Status::GUATIAO_ERR_BUSY => {
                std::thread::sleep(core::time::Duration::from_millis(20));
            }
            Err(e) => last = Some(e),
        }
        if last.is_some() {
            break;
        }
    }
    panic!("the library did not unload once its thread was done: {last:?}");
}
