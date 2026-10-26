// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The local transport (a Unix socket, a named pipe): calls and callbacks
//! cross, and a link ended from either side ends on both.

#![cfg(feature = "local")]

mod common;

use core::time::Duration;

use common::{Ear, eventually};
use greeter_kind::Greeter;
use guatiao::Status;
use guatiao::library::Registry;
use guatiao_source_remote::transport::local::{LocalConnect, LocalListen};
use guatiao_source_remote::{LinkOptions, RemoteSource, Server, Trust};

fn endpoint(tag: &str) -> String {
    let unique = format!("guatiao-remote-{tag}-{}", std::process::id());
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
    let listener = LocalListen::bind(&name).expect("listens");
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
        Box::new(LocalConnect::new(name)),
        LinkOptions::new(Trust::Local).timeout(Some(Duration::from_secs(10))),
    );
    let registry = common::host(&source);
    (source, registry)
}

#[test]
fn calls_and_callbacks_cross_a_local_link() {
    let (_server, name) = serve("calls");
    let (_source, registry) = host(&name);
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    assert!(greeter.greet("local").is_ok());
    let mut ear = Ear::new();
    greeter.greet_into("pipe", &mut ear).unwrap();
    assert_eq!(*ear.heard.lock().unwrap(), ["hello, pipe", "twice"]);
}

#[test]
fn a_link_ended_by_the_server_ends_on_the_host() {
    let (server, name) = serve("server-ends");
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
}

#[test]
fn a_link_ended_by_the_host_ends_on_the_server() {
    let (server, name) = serve("host-ends");
    let (source, registry) = host(&name);
    assert_eq!(server.links(), 1);
    drop(registry);
    source.disconnect();
    eventually("the server's link ends", || server.links() == 0);
}

#[cfg(unix)]
#[test]
fn a_socket_is_its_owners_alone_and_nothing_else_is_replaced() {
    use std::os::unix::fs::PermissionsExt;
    let name = endpoint("mode");
    let _listener = LocalListen::bind(&name).unwrap();
    let mode = std::fs::metadata(&name).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    let file = endpoint("file");
    std::fs::write(&file, b"not a socket").unwrap();
    assert!(LocalListen::bind(&file).is_err());
    std::fs::remove_file(&file).unwrap();
}

fn greeting(map: &guatiao::Map) -> usize {
    use guatiao::value::convert::TryAsRef;
    map.get("greeting")
        .and_then(TryAsRef::<str>::try_as_ref)
        .map_or(0, str::len)
}

#[test]
fn large_and_small_calls_from_many_threads_arrive_whole() {
    let (_server, name) = serve("large");
    let (_source, registry) = host(&name);
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    // Megabyte frames written while other threads' small frames queue
    // behind them, and the other way round.
    let threads: Vec<_> = (0..8)
        .map(|t| {
            let g = greeter.clone();
            std::thread::spawn(move || {
                for i in 0..20 {
                    let name = if (t + i) % 4 == 0 {
                        "y".repeat((1 << 20) + t * 1000 + i)
                    } else {
                        format!("t{t}i{i}")
                    };
                    let answer = g.greet(&name).unwrap();
                    assert_eq!(greeting(&answer), name.len() + "hello, ".len());
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    // A large lent map, written back after the answer.
    let mut config = guatiao::Map::new();
    config.set("name", "z".repeat(1 << 20).as_str());
    assert!(greeter.check(&mut config).unwrap());
    assert_eq!(greeting(&config), (1 << 20) + "hello, ".len());
}
