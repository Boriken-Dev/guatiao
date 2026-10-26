// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The shared-memory transport: calls and callbacks cross, a frame larger
//! than a ring streams through it, and a link ended from either side ends
//! on both.

#![cfg(feature = "shm")]

mod common;

use core::time::Duration;

use common::{Ear, eventually};
use greeter_kind::Greeter;
use guatiao::Status;
use guatiao::library::Registry;
use guatiao::value::convert::TryAsRef;
use guatiao_source_remote::transport::shm::{RING, ShmConnect, ShmListen};
use guatiao_source_remote::{LinkOptions, RemoteSource, Server, Trust};

fn endpoint(tag: &str) -> String {
    let unique = format!("guatiao-remote-shm-{tag}-{}", std::process::id());
    if cfg!(windows) {
        unique
    } else {
        std::env::temp_dir()
            .join(format!("{unique}.sock"))
            .display()
            .to_string()
    }
}

fn serve(tag: &str) -> (Server, String) {
    let server = common::server(LinkOptions::new(Trust::Local));
    let name = endpoint(tag);
    let listener = ShmListen::bind(&name).expect("listens");
    let serving = server.clone();
    std::thread::spawn(move || {
        while let Ok(conn) = listener.accept() {
            let s = serving.clone();
            std::thread::spawn(move || {
                let _ = s.serve(conn);
            });
        }
    });
    (server, name)
}

fn host(name: &str) -> (RemoteSource, Registry) {
    let source = RemoteSource::new(
        Box::new(ShmConnect::new(name)),
        LinkOptions::new(Trust::Local).timeout(Some(Duration::from_secs(20))),
    );
    let registry = common::host(&source);
    (source, registry)
}

#[test]
fn calls_callbacks_and_a_frame_larger_than_a_ring_cross() {
    let (_server, name) = serve("calls");
    let (_source, registry) = host(&name);
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    for i in 0..1000 {
        let name = format!("n{i}");
        let answer = greeter.greet(&name).unwrap();
        assert_eq!(
            answer.get("greeting").and_then(TryAsRef::<str>::try_as_ref),
            Some(format!("hello, {name}").as_str())
        );
    }
    let mut ear = Ear::new();
    greeter.greet_into("ring", &mut ear).unwrap();
    assert_eq!(*ear.heard.lock().unwrap(), ["hello, ring", "twice"]);

    let big = "x".repeat(RING as usize + RING as usize / 2);
    let answer = greeter.greet(&big).unwrap();
    let greeting = answer
        .get("greeting")
        .and_then(TryAsRef::<str>::try_as_ref)
        .unwrap();
    assert_eq!(greeting.len(), big.len() + "hello, ".len());
}

#[test]
fn a_link_ended_by_either_side_ends_on_both() {
    let (server, name) = serve("ends");
    let (source, registry) = host(&name);
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    assert!(greeter.greet("x").is_ok());
    let link = source.link().unwrap();
    server.end_links();
    eventually("the host's link ends", || !link.alive());
    assert_eq!(
        greeter.greet("y").unwrap_err().status,
        Status::GUATIAO_ERR_GONE
    );

    let (server, name) = serve("host-ends");
    let (source, registry) = host(&name);
    assert_eq!(server.links(), 1);
    drop(registry);
    source.disconnect();
    eventually("the server's link ends", || server.links() == 0);
}

#[test]
fn a_client_that_never_sets_up_does_not_hold_up_the_next() {
    use guatiao_source_remote::transport::Connect;
    use guatiao_source_remote::transport::local::LocalConnect;
    let (_server, name) = serve("silent");
    // Connected, and never sends its rings' path.
    let silent = LocalConnect::new(&name).connect().expect("connects");
    let (linked_tx, linked_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let (_source, registry) = host(&name);
        let greeter = registry
            .offers::<dyn Greeter>()
            .next()
            .unwrap()
            .shared()
            .unwrap();
        let answer = greeter.greet("next").unwrap();
        let greeting = answer.get("greeting").and_then(TryAsRef::<str>::try_as_ref);
        let _ = linked_tx.send(greeting.map(str::to_string));
    });
    assert_eq!(
        linked_rx.recv_timeout(Duration::from_secs(5)),
        Ok(Some("hello, next".to_string())),
        "the next client waited for the silent one"
    );
    drop(silent);
}

/// Serves the one connection a fresh listener accepts, under a 1 s
/// handshake, with `client` holding what it connected; answers how long
/// the handshake took to fail.
fn silent_handshake<T>(tag: &str, client: impl FnOnce(&str) -> T) -> Duration {
    let server =
        common::server(LinkOptions::new(Trust::Local).handshake_timeout(Duration::from_secs(1)));
    let name = endpoint(tag);
    let listener = ShmListen::bind(&name).expect("listens");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let conn = listener.accept().expect("accepts");
        let started = std::time::Instant::now();
        let failed = server.serve(conn).is_err();
        let _ = tx.send((failed, started.elapsed()));
    });
    let _held = client(&name);
    let (failed, took) = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the handshake ended at its bound");
    assert!(failed, "a silent client never links");
    took
}

#[test]
fn a_client_that_maps_the_rings_and_says_nothing_is_dropped_at_the_handshake_bound() {
    use guatiao_source_remote::transport::Connect;
    // The rings mapped, and no HELLO over them.
    let took = silent_handshake("mapped", |name| {
        ShmConnect::new(name).connect().expect("connects")
    });
    assert!(took < Duration::from_secs(5), "{took:?}");
}

#[test]
fn a_client_that_never_sends_its_path_is_dropped_at_the_handshake_bound() {
    use guatiao_source_remote::transport::Connect;
    use guatiao_source_remote::transport::local::LocalConnect;
    let took = silent_handshake("pathless", |name| {
        LocalConnect::new(name).connect().expect("connects")
    });
    assert!(took < Duration::from_secs(5), "{took:?}");
}
