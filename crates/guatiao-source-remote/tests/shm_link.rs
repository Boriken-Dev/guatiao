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
