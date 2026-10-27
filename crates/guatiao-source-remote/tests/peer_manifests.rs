// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Peers described by manifests: two identical devices in one registry
//! under their names, a manifest that describes no peer reported in its
//! own words, a peer that cannot be reached reported once and the others
//! served, one that goes away keeping its libraries until it is back, a
//! manifest edited or removed followed by the next refresh, peers slow to
//! refuse listed at once, a replaced peer serving on while its successor
//! cannot be reached, and an empty token file refused.

#![cfg(feature = "tcp")]

mod common;

extern crate alloc;

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::Instant;

use greeter_kind::Greeter;
use guatiao::library::{Refreshed, Registry};
use guatiao::{List, Map, Status};
use guatiao_source_remote::transport::tcp::{Accepted, TcpConnect, TcpListen};
use guatiao_source_remote::{LinkOptions, RemoteSource, Trust};

const KEY: &str = "a key every device here holds";

/// A trusted device on loopback holding `key`; answers its port. While
/// `down` is set it closes every connection unanswered.
fn device_with(key: &str, down: Arc<AtomicBool>) -> u16 {
    let server = common::server(LinkOptions::new(Trust::Trusted).key(key.as_bytes()));
    let listener = TcpListen::bind("127.0.0.1:0", common::permit(&["127.0.0.1"])).unwrap();
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

/// A port where every connection is held unanswered for `wait`, then
/// closed: a peer slow to refuse.
fn slow_to_refuse(wait: Duration) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for conn in listener.incoming().map_while(Result::ok) {
            std::thread::spawn(move || {
                std::thread::sleep(wait);
                drop(conn);
            });
        }
    });
    port
}

/// A device that waits `wait` before it answers a connection; answers its
/// port and a channel told of every connection as it arrives.
fn slow_device(wait: Duration) -> (u16, std::sync::mpsc::Receiver<()>) {
    let server = common::server(LinkOptions::new(Trust::Trusted).key(KEY.as_bytes()));
    let listener = TcpListen::bind("127.0.0.1:0", common::permit(&["127.0.0.1"])).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (arrived, arrivals) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(accepted) = listener.accept() {
            if let Accepted::Connection(conn) = accepted {
                let _ = arrived.send(());
                let server = server.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(wait);
                    let _ = server.serve(conn);
                });
            }
        }
    });
    (port, arrivals)
}

/// A device serving no library, its server kept to end its links; while
/// `down` is set it closes every connection unanswered.
fn empty_device(down: Arc<AtomicBool>) -> (u16, guatiao_source_remote::Server) {
    let server = guatiao_source_remote::Server::new(
        Registry::new("server", "1.0"),
        common::described(),
        LinkOptions::new(Trust::Trusted).key(KEY.as_bytes()),
    );
    let listener = TcpListen::bind("127.0.0.1:0", common::permit(&["127.0.0.1"])).unwrap();
    let port = listener.local_addr().unwrap().port();
    let serving = server.clone();
    std::thread::spawn(move || {
        while let Ok(accepted) = listener.accept() {
            if let Accepted::Connection(conn) = accepted {
                if down.load(Ordering::Relaxed) {
                    continue;
                }
                let server = serving.clone();
                std::thread::spawn(move || {
                    let _ = server.serve(conn);
                });
            }
        }
    });
    (port, server)
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
        "type = \"remote\"\nname = \"{name}\"\nconnect = \"tcp://127.0.0.1:{port}\"\nsecurity = \"none\"\n{extra}"
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
        "inline_token.guatiao.toml",
        &peer("inline_token", device(), "token = \"right here\"\n"),
    );
    write(
        &dir,
        "secure.guatiao.toml",
        "type = \"remote\"\nname = \"far\"\nconnect = \"tcp://192.0.2.1:7000\"\n",
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
    let token = why(&refreshed, "inline_token.guatiao.toml");
    assert!(
        token.contains("`token` is a secret") && token.contains("`token_file`"),
        "{token}"
    );
    // The secure level runs over `tls://` alone, in a build that has it.
    let secure = why(&refreshed, "secure.guatiao.toml");
    if cfg!(feature = "tls") {
        assert!(secure.contains("names this side's identity"), "{secure}");
    } else {
        assert!(
            secure.contains("mutual-tls is not in this build"),
            "{secure}"
        );
    }
    assert!(
        why(&refreshed, "typo.guatiao.toml").contains("`keyfile` is not a key of a peer manifest")
    );
    assert!(
        why(&refreshed, "twice.guatiao.toml")
            .contains("a peer named `good` is already described by")
    );
    assert!(why(&refreshed, "slash.guatiao.toml").contains("`name` holds a `/`"));
    assert!(why(&refreshed, "nameless.guatiao.toml").contains("`name` is missing"));
    assert_eq!(refreshed.refused.len(), 7, "{refreshed:?}");
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
fn an_empty_token_is_refused_for_one_peer_and_for_every_peer_described() {
    let mut one = Map::new();
    one.set("connect", "tcp://127.0.0.1:7000");
    one.set("security", "none");
    one.set("key", KEY);
    one.set("token", "");
    let mut every = Map::new();
    let mut dirs = List::new();
    dirs.push("peers");
    every.set("dirs", dirs);
    every.set("key", KEY);
    every.set("token", " \n");
    for config in [one, every] {
        let refused = RemoteSource::from_config(&config).unwrap_err();
        assert_eq!(refused.status, Status::GUATIAO_ERR_BAD_VALUE);
        assert!(
            refused.message().contains("`token` is empty"),
            "{}",
            refused.message()
        );
    }
}

#[test]
fn an_empty_token_file_is_refused_and_so_is_a_token_beside_one() {
    let dir = scratch("token_file");
    write(&dir, "empty.token", "\u{feff}\r\n");
    write(
        &dir,
        "lab.guatiao.toml",
        &peer("lab", device(), "token_file = \"empty.token\"\n"),
    );
    let mut registry = Registry::new("host", "1.0");
    registry.add_source("peers", source(&dir)).unwrap();
    let refreshed = registry.refresh();
    assert!(refreshed.loaded.is_empty(), "{refreshed:?}");
    let lab = why(&refreshed, "lab.guatiao.toml");
    assert!(
        lab.contains("`token_file`") && lab.contains("the token is empty"),
        "{lab}"
    );

    let empty = dir.join("empty.token").display().to_string();
    let mut one = Map::new();
    one.set("connect", "tcp://127.0.0.1:7000");
    one.set("security", "none");
    one.set("key", KEY);
    one.set("token_file", empty.as_str());
    let mut every = Map::new();
    let mut dirs = List::new();
    dirs.push(dir.display().to_string().as_str());
    every.set("dirs", dirs);
    every.set("key", KEY);
    every.set("token_file", empty.as_str());
    for config in [one.clone(), every] {
        let refused = RemoteSource::from_config(&config).unwrap_err();
        assert_eq!(refused.status, Status::GUATIAO_ERR_BAD_VALUE);
        assert!(
            refused.message().contains("the token is empty"),
            "{}",
            refused.message()
        );
    }

    write(&dir, "lab.token", "t0k");
    one.set(
        "token_file",
        dir.join("lab.token").display().to_string().as_str(),
    );
    one.set("token", "t0k");
    let refused = RemoteSource::from_config(&one).unwrap_err();
    assert_eq!(refused.status, Status::GUATIAO_ERR_BAD_VALUE);
    assert!(
        refused
            .message()
            .contains("`token` or `token_file`, not both"),
        "{}",
        refused.message()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn one_peer_and_manifests_of_peers_are_one_or_the_other() {
    let mut config = Map::new();
    config.set("connect", "tcp://127.0.0.1:7000");
    config.set("security", "none");
    let mut dirs = List::new();
    dirs.push("peers");
    config.set("dirs", dirs);
    let refused = RemoteSource::from_config(&config).unwrap_err();
    assert_eq!(refused.status, Status::GUATIAO_ERR_BAD_VALUE);
    assert!(refused.message().contains("one or the other"));

    // A network link with no security named runs mutual TLS, which names
    // this side's identity.
    let mut secure = Map::new();
    secure.set("connect", "tcp://127.0.0.1:7000");
    let refused = RemoteSource::from_config(&secure).unwrap_err();
    if cfg!(feature = "tls") {
        assert_eq!(refused.status, Status::GUATIAO_ERR_BAD_VALUE);
        assert!(
            refused.message().contains("state_dir"),
            "{}",
            refused.message()
        );
    } else {
        assert_eq!(refused.status, Status::GUATIAO_ERR_UNSUPPORTED);
        assert!(
            refused.message().contains("mutual-tls"),
            "{}",
            refused.message()
        );
    }
}

#[test]
fn a_key_no_manifest_has_is_refused_by_name() {
    let dir = scratch("unknown_key");
    write(
        &dir,
        "odd.guatiao.toml",
        "type = \"remote\"\nname = \"odd\"\nconnect = \"tcp://127.0.0.1:1\"\ntrust = \"trusted\"\n",
    );
    let mut dirs = List::new();
    dirs.push(dir.display().to_string().as_str());
    let mut config = Map::new();
    config.set("dirs", dirs);
    let mut registry = Registry::new("host", "1.0");
    registry
        .add_source("peers", RemoteSource::from_config(&config).unwrap())
        .unwrap();
    let refreshed = registry.refresh();
    let words = format!("{:?}", refreshed.refused);
    assert!(
        words.contains("`trust` is not a key of a peer manifest"),
        "{words}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn each_scheme_runs_the_securities_it_can_and_names_the_fix_otherwise() {
    let refused = |connect: &str, security: Option<&str>| {
        let mut config = Map::new();
        config.set("connect", connect);
        config.set("key", "k");
        if let Some(s) = security {
            config.set("security", s);
        }
        RemoteSource::from_config(&config)
            .err()
            .map(|e| (e.status, e.message().to_string()))
    };
    // None runs on these with a key; local transports run it by default.
    assert_eq!(refused("tcp://127.0.0.1:1", Some("none")), None);
    if cfg!(feature = "ws") {
        assert_eq!(refused("ws://127.0.0.1:1/", Some("none")), None);
        let (status, why) = refused("ws://127.0.0.1:1/", None).unwrap();
        assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
        assert!(
            why.contains("carries no TLS") && why.contains("wss://"),
            "{why}"
        );
        let (_, why) = refused("wss://127.0.0.1:1/", Some("none")).unwrap();
        assert!(why.contains("runs TLS"), "{why}");
    }
    if cfg!(feature = "local") {
        assert_eq!(refused("local://x", None), None);
        let (_, why) = refused("local://x", Some("mutual-tls")).unwrap();
        assert!(why.contains("carries no TLS"), "{why}");
    }
    if cfg!(feature = "tls") {
        let (_, why) = refused("tls://127.0.0.1:1", Some("none")).unwrap();
        assert!(why.contains("runs TLS"), "{why}");
        // Server TLS needs no identity here, but the server's pins.
        let (status, why) = refused("tcp://127.0.0.1:1", Some("server-tls")).unwrap();
        assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
        assert!(why.contains("whom it trusts"), "{why}");
    }
    if cfg!(feature = "rdma") {
        let (_, why) = refused("rdma://127.0.0.1:1", None).unwrap();
        assert!(why.contains("carries no TLS"), "{why}");
    }
    let (_, why) = refused("tcp://127.0.0.1:1", Some("secure")).unwrap();
    assert!(
        why.contains("`none`, `server-tls` or `mutual-tls`"),
        "{why}"
    );
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

/// Six peers that each take a second to refuse, beside one that answers:
/// listed one after another they would hold the refresh six seconds.
#[test]
fn peers_slow_to_refuse_are_listed_at_once() {
    let dir = scratch("slow");
    write(&dir, "near.guatiao.toml", &peer("near", device(), ""));
    for i in 0..6 {
        let port = slow_to_refuse(Duration::from_secs(1));
        write(
            &dir,
            &format!("slow{i}.guatiao.toml"),
            &peer(&format!("slow{i}"), port, ""),
        );
    }
    let mut registry = Registry::new("host", "1.0");
    registry.add_source("peers", source(&dir)).unwrap();
    let started = Instant::now();
    let refreshed = registry.refresh();
    let took = started.elapsed();
    assert_eq!(keys(&refreshed.loaded), ["near/shapes"], "{refreshed:?}");
    assert_eq!(refreshed.refused.len(), 6, "{refreshed:?}");
    assert!(took < Duration::from_secs(4), "the refresh took {took:?}");
    assert_eq!(greet(&registry, "near"), "hello, near");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `disconnect` from another thread does not wait for a connection under
/// way, and that connection is not kept.
#[test]
fn disconnecting_never_waits_for_a_connection_under_way() {
    let (port, arrivals) = slow_device(Duration::from_millis(1500));
    let source = RemoteSource::new(
        Box::new(TcpConnect::new(&format!("127.0.0.1:{port}"))),
        LinkOptions::new(Trust::Trusted).key(KEY.as_bytes()),
    );
    let connecting = {
        let source = source.clone();
        std::thread::spawn(move || source.link())
    };
    arrivals
        .recv_timeout(Duration::from_secs(10))
        .expect("the connection arrives");
    let started = Instant::now();
    source.disconnect();
    let took = started.elapsed();
    assert!(
        took < Duration::from_millis(500),
        "disconnect waited {took:?}"
    );
    let linked = connecting.join().unwrap();
    assert_eq!(
        linked.map(|_| ()).unwrap_err().status,
        Status::GUATIAO_ERR_GONE
    );
    // The next connection is kept.
    assert!(source.link().unwrap().alive());
}

/// An edited manifest naming a peer that cannot be reached leaves the
/// peer it replaced listed and answering.
#[test]
fn a_replaced_peer_serves_on_while_its_successor_cannot_be_reached() {
    let dir = scratch("successor");
    write(&dir, "lab.guatiao.toml", &peer("lab", device(), ""));
    let mut registry = Registry::new("host", "1.0");
    registry.add_source("peers", source(&dir)).unwrap();
    let refreshed = registry.refresh();
    assert_eq!(keys(&refreshed.loaded), ["lab/shapes"], "{refreshed:?}");

    write(&dir, "lab.guatiao.toml", &peer("lab", nobody(), ""));
    let refreshed = registry.refresh();
    assert!(
        refreshed.loaded.is_empty() && refreshed.retired.is_empty(),
        "{refreshed:?}"
    );
    assert!(why(&refreshed, "lab.guatiao.toml").contains("could not be reached"));
    assert_eq!(greet(&registry, "lab"), "hello, lab");
    assert!(registry.refresh().is_empty(), "reported once");

    write(
        &dir,
        "lab.guatiao.toml",
        &peer("lab", device(), "timeout_ms = 5000\n"),
    );
    let refreshed = registry.refresh();
    assert_eq!(keys(&refreshed.loaded), ["lab/shapes"], "{refreshed:?}");
    assert_eq!(keys(&refreshed.retired), ["lab/shapes"]);
    assert_eq!(greet(&registry, "lab"), "hello, lab");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A successor that listed once and then cannot be listed keeps what it
/// listed: the peer it replaced is not listed again, even while the
/// registry still holds that peer's library.
#[test]
fn a_successor_that_listed_once_never_brings_back_the_peer_it_replaced() {
    let dir = scratch("listed-once");
    write(&dir, "lab.guatiao.toml", &peer("lab", device(), ""));
    let mut registry = Registry::new("host", "1.0");
    registry.add_source("peers", source(&dir)).unwrap();
    assert_eq!(keys(&registry.refresh().loaded), ["lab/shapes"]);
    let held = registry
        .offers::<dyn Greeter>()
        .next()
        .unwrap()
        .shared()
        .unwrap();

    let down = Arc::new(AtomicBool::new(false));
    let (port, empty) = empty_device(Arc::clone(&down));
    write(&dir, "lab.guatiao.toml", &peer("lab", port, ""));
    let refreshed = registry.refresh();
    assert_eq!(keys(&refreshed.retired), ["lab/shapes"], "{refreshed:?}");
    assert!(refreshed.loaded.is_empty());

    down.store(true, Ordering::Relaxed);
    empty.end_links();
    let refreshed = registry.refresh();
    assert!(refreshed.loaded.is_empty(), "{refreshed:?}");
    assert_eq!(refreshed.refused.len(), 1, "{refreshed:?}");
    assert!(why(&refreshed, "lab.guatiao.toml").contains("the peer `lab`"));
    drop(held);
    let _ = std::fs::remove_dir_all(&dir);
}
