// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Peers described by manifests: two identical devices in one registry
//! under their names, a manifest that describes no peer reported in its
//! own words, a peer that cannot be reached reported once and the others
//! served, one that goes away keeping its libraries until it is back, and
//! a manifest edited or removed followed by the next refresh.

#![cfg(feature = "tcp")]

mod common;

extern crate alloc;

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};
use std::net::TcpListener;
use std::path::{Path, PathBuf};

use greeter_kind::Greeter;
use guatiao::library::{Refreshed, Registry};
use guatiao::{List, Map, Status};
use guatiao_source_remote::transport::tcp::{Accepted, Range, TcpListen};
use guatiao_source_remote::{LinkOptions, RemoteSource, Trust};

const KEY: &str = "a key every device here holds";

/// A trusted device on loopback holding `key`; answers its port. While
/// `down` is set it closes every connection unanswered.
fn device_with(key: &str, down: Arc<AtomicBool>) -> u16 {
    let server = common::server(LinkOptions::new(Trust::Trusted).key(key.as_bytes()));
    let listener = TcpListen::bind("127.0.0.1:0", &[Range::parse("127.0.0.1").unwrap()]).unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        while let Ok(accepted) = listener.accept() {
            if let Accepted::Connection(conn) = accepted {
                if down.load(Ordering::Relaxed) {
                    continue;
                }
                let server = server.clone();
                std::thread::spawn(move || {
                    let _ = server.serve(conn);
                });
            }
        }
    });
    port
}

fn device() -> u16 {
    device_with(KEY, Arc::default())
}

/// A port nothing listens on.
fn nobody() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gt-peers-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, file: &str, text: &str) {
    std::fs::write(dir.join(file), text).unwrap();
}

fn peer(name: &str, port: u16, extra: &str) -> String {
    format!(
        "type = \"remote\"\nname = \"{name}\"\nconnect = \"tcp://127.0.0.1:{port}\"\ntrust = \"trusted\"\n{extra}"
    )
}

/// A source over `dir`, the key every peer's unless its manifest names a
/// file.
fn source(dir: &Path) -> RemoteSource {
    let mut config = Map::new();
    let mut dirs = List::new();
    dirs.push(dir.display().to_string().as_str());
    config.set("dirs", dirs);
    config.set("key", KEY);
    RemoteSource::from_config(&config).unwrap()
}

fn keys(served: &[guatiao::library::Served]) -> Vec<&str> {
    served.iter().map(|s| s.key.as_str()).collect()
}

fn why(refreshed: &Refreshed, file: &str) -> String {
    refreshed
        .refused
        .iter()
        .find(|r| r.id.as_deref().is_some_and(|id| id.ends_with(file)))
        .map(|r| r.reason.clone())
        .unwrap_or_else(|| panic!("nothing about {file} in {refreshed:?}"))
}

fn greet(registry: &Registry, peer: &str) -> String {
    let provider = registry
        .all()
        .iter()
        .find(|p| p.library() == format!("{peer}/shapes"))
        .unwrap_or_else(|| panic!("no provider of {peer}"));
    let greeter = provider
        .offer::<dyn Greeter>()
        .unwrap()
        .shared()
        .expect("callable");
    let answer = greeter.greet(peer).unwrap();
    guatiao::value::convert::TryAsRef::<str>::try_as_ref(answer.get("greeting").unwrap())
        .unwrap()
        .to_string()
}

#[test]
fn identical_devices_are_one_registry_s_under_their_names() {
    let dir = scratch("two");
    write(&dir, "a.guatiao.toml", &peer("lab-a", device(), ""));
    // b holds a key of its own, which its manifest names over the
    // source's.
    write(
        &dir,
        "b.guatiao.toml",
        &peer(
            "lab-b",
            device_with("b's own key", Arc::default()),
            "key_file = \"b.key\"\n",
        ),
    );
    write(&dir, "b.key", "b's own key");
    write(
        &dir,
        "greeters.guatiao.toml",
        "type = \"python\"\nmodule = \"greeters\"\n",
    );

    let mut registry = Registry::new("host", "1.0");
    registry.add_source("peers", source(&dir)).unwrap();
    let refreshed = registry.refresh();
    assert!(refreshed.refused.is_empty(), "{refreshed:?}");
    assert_eq!(keys(&refreshed.loaded), ["lab-a/shapes", "lab-b/shapes"]);
    assert!(
        registry
            .all()
            .iter()
            .all(|p| p.id().starts_with(&format!("{}/", &p.library()[..5]))),
        "every provider under its peer's name"
    );
    assert_eq!(greet(&registry, "lab-a"), "hello, lab-a");
    assert_eq!(greet(&registry, "lab-b"), "hello, lab-b");
    assert!(registry.refresh().is_empty(), "nothing changed");

    std::fs::remove_file(dir.join("b.guatiao.toml")).unwrap();
    let refreshed = registry.refresh();
    assert_eq!(keys(&refreshed.retired), ["lab-b/shapes"], "{refreshed:?}");
    assert!(refreshed.loaded.is_empty());

    // An edited manifest is a new peer: its library is opened anew over a
    // new link, and the old one retired.
    write(
        &dir,
        "a.guatiao.toml",
        &peer("lab-a", device(), "timeout_ms = 5000\n"),
    );
    let refreshed = registry.refresh();
    assert_eq!(keys(&refreshed.loaded), ["lab-a/shapes"], "{refreshed:?}");
    assert_eq!(keys(&refreshed.retired), ["lab-a/shapes"]);
    assert_eq!(greet(&registry, "lab-a"), "hello, lab-a");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_manifest_that_describes_no_peer_is_reported_in_its_own_words() {
    let dir = scratch("words");
    write(&dir, "good.guatiao.toml", &peer("good", device(), ""));
    write(
        &dir,
        "inline.guatiao.toml",
        &peer("inline", device(), "key = \"right here\"\n"),
    );
    write(
        &dir,
        "secure.guatiao.toml",
        "type = \"remote\"\nname = \"far\"\nconnect = \"tcp://192.0.2.1:7000\"\ntrust = \"secure\"\n",
    );
    write(
        &dir,
        "typo.guatiao.toml",
        &peer("typo", device(), "keyfile = \"link.key\"\n"),
    );
    write(&dir, "twice.guatiao.toml", &peer("good", device(), ""));
    write(&dir, "slash.guatiao.toml", &peer("lab/a", device(), ""));
    write(
        &dir,
        "nameless.guatiao.toml",
        "type = \"remote\"\nconnect = \"tcp://127.0.0.1:1\"\n",
    );

    let mut registry = Registry::new("host", "1.0");
    registry.add_source("peers", source(&dir)).unwrap();
    let refreshed = registry.refresh();
    assert_eq!(keys(&refreshed.loaded), ["good/shapes"], "{refreshed:?}");
    assert!(why(&refreshed, "inline.guatiao.toml").contains("`key` is a secret"));
    assert!(
        why(&refreshed, "secure.guatiao.toml").contains("the secure level is not in this build")
    );
    assert!(
        why(&refreshed, "typo.guatiao.toml").contains("`keyfile` is not a key of a peer manifest")
    );
    assert!(
        why(&refreshed, "twice.guatiao.toml")
            .contains("a peer named `good` is already described by")
    );
    assert!(why(&refreshed, "slash.guatiao.toml").contains("`name` holds a `/`"));
    assert!(why(&refreshed, "nameless.guatiao.toml").contains("`name` is missing"));
    assert_eq!(refreshed.refused.len(), 6, "{refreshed:?}");
    assert!(registry.refresh().is_empty(), "each reported once");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_peer_that_cannot_be_reached_is_reported_once_and_the_others_served() {
    let dir = scratch("gone");
    write(&dir, "near.guatiao.toml", &peer("near", device(), ""));
    write(&dir, "far.guatiao.toml", &peer("far", nobody(), ""));

    let mut registry = Registry::new("host", "1.0");
    registry.add_source("peers", source(&dir)).unwrap();
    let refreshed = registry.refresh();
    assert_eq!(keys(&refreshed.loaded), ["near/shapes"], "{refreshed:?}");
    let far = why(&refreshed, "far.guatiao.toml");
    assert!(far.contains("the peer `far`"), "{far}");
    assert!(far.contains("could not be reached"), "{far}");
    assert!(
        registry.refresh().is_empty(),
        "reported once while it stays away"
    );
    assert_eq!(greet(&registry, "near"), "hello, near");

    // It comes up where its manifest now says: a new peer.
    write(&dir, "far.guatiao.toml", &peer("far", device(), ""));
    let refreshed = registry.refresh();
    assert_eq!(keys(&refreshed.loaded), ["far/shapes"], "{refreshed:?}");
    assert!(refreshed.refused.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn one_peer_and_manifests_of_peers_are_one_or_the_other() {
    let mut config = Map::new();
    config.set("connect", "tcp://127.0.0.1:7000");
    config.set("trust", "trusted");
    let mut dirs = List::new();
    dirs.push("peers");
    config.set("dirs", dirs);
    let refused = RemoteSource::from_config(&config).unwrap_err();
    assert_eq!(refused.status, Status::GUATIAO_ERR_BAD_VALUE);
    assert!(refused.message().contains("one or the other"));

    let mut secure = Map::new();
    secure.set("connect", "tcp://127.0.0.1:7000");
    secure.set("trust", "secure");
    let refused = RemoteSource::from_config(&secure).unwrap_err();
    assert_eq!(refused.status, Status::GUATIAO_ERR_UNSUPPORTED);
}

/// A peer that goes away keeps its libraries listed as they were (calls
/// through them answer GONE) and is reported; back, it is listed anew.
#[test]
fn a_peer_that_goes_away_keeps_its_libraries_until_it_is_back() {
    let dir = scratch("flaky");
    let down = Arc::new(AtomicBool::new(false));
    write(
        &dir,
        "flaky.guatiao.toml",
        &peer("flaky", device_with(KEY, Arc::clone(&down)), ""),
    );
    let source = source(&dir);
    let mut registry = Registry::new("host", "1.0");
    registry.add_source("peers", source.clone()).unwrap();
    let refreshed = registry.refresh();
    assert_eq!(keys(&refreshed.loaded), ["flaky/shapes"], "{refreshed:?}");
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .unwrap()
        .shared()
        .unwrap();

    down.store(true, Ordering::Relaxed);
    source.disconnect();
    let refreshed = registry.refresh();
    assert!(
        refreshed.loaded.is_empty() && refreshed.retired.is_empty(),
        "{refreshed:?}"
    );
    assert!(why(&refreshed, "flaky.guatiao.toml").contains("the peer `flaky`"));
    assert_eq!(
        greeter.greet("anyone").unwrap_err().status,
        Status::GUATIAO_ERR_GONE
    );
    assert!(
        registry.refresh().is_empty(),
        "reported once while it stays away"
    );

    down.store(false, Ordering::Relaxed);
    let refreshed = registry.refresh();
    assert_eq!(keys(&refreshed.loaded), ["flaky/shapes"], "{refreshed:?}");
    assert_eq!(keys(&refreshed.retired), ["flaky/shapes"]);
    assert!(refreshed.refused.is_empty());
    assert_eq!(greet(&registry, "flaky"), "hello, flaky");
    let _ = std::fs::remove_dir_all(&dir);
}
