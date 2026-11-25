// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The channel rules through the foreign loader: the Rust echo, built for
//! the system C library, loaded by a registry the host never tells about
//! the loader (on a static host; a dynamic one is told), keeps every rule
//! from a thread of its own calling the host's sink.
//!
//! Run as a static host: `cargo build -p channel_echo` for the host's
//! architecture, then `GUATIAO_LIBRARY_DIR=<that target>/debug cargo test
//! -p guatiao --features load,provider --target <arch>-unknown-linux-musl
//! --test foreign_channel`. Without the variable the tests say they
//! skipped, and fail under `GUATIAO_REQUIRE_ALL=1`.

#![cfg(all(guatiao_foreign, feature = "provider"))]

#[path = "../../../examples/channel_echo/tests/rules/mod.rs"]
mod rules;

use std::path::PathBuf;

use channel_echo_kind::ChannelEcho;
use guatiao::library::foreign::Settings;
use guatiao::library::{Dlopen, Registry, UnloadError};
use guatiao::{Map, Status};

fn echo() -> Option<PathBuf> {
    match std::env::var_os("GUATIAO_LIBRARY_DIR") {
        Some(dir) => Some(PathBuf::from(dir).join("libchannel_echo.so")),
        None if std::env::var_os("GUATIAO_REQUIRE_ALL").is_some() => {
            panic!("GUATIAO_LIBRARY_DIR names no directory of libraries built for glibc")
        }
        None => {
            eprintln!(
                "skipped: GUATIAO_LIBRARY_DIR names no directory of libraries built for glibc"
            );
            None
        }
    }
}

/// A registry as a host makes one; a dynamic host is told to load through
/// the foreign loader, which a static one does by itself.
fn registry() -> Registry {
    let mut registry = Registry::new("channel-host", "1.0");
    if Dlopen::Auto.resolved() != Dlopen::Foreign {
        registry.set_dlopen(Dlopen::Foreign);
        let mut settings = Settings::default();
        // A glibc host's own C library already handles signal 33.
        settings.allow_reserved_signal_handlers = true;
        registry.set_foreign_settings(settings);
    }
    registry
}

#[test]
fn the_echo_keeps_every_rule_through_the_foreign_loader() {
    let Some(path) = echo() else { return };
    let mut registry = registry();
    registry.load_file(&path).expect("the echo loads");
    let echo = registry
        .offer::<dyn ChannelEcho>("channel_echo_echo")
        .expect("offered")
        .expect("as the trait")
        .shared()
        .expect("its one instance");
    rules::hold_every_rule(
        "the Rust echo through the foreign loader",
        &|config: &Map, sink| echo.start(config, sink),
    );
}

#[test]
fn the_echo_refuses_to_unload_while_its_thread_is_inside_the_sink() {
    let Some(path) = echo() else { return };
    let mut registry = registry();
    registry.load_file(&path).expect("the echo loads");
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
        drop(channel);
    }
    match registry.unload(&key) {
        Err(UnloadError::Refused { status, .. }) if status == Status::GUATIAO_ERR_BUSY => {}
        other => panic!("a thread still inside the sink, and unload answered {other:?}"),
    }
    probe.release();
    probe.closed().expect("closed once released");
}
