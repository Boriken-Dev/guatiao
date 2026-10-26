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
    let name = listen(&server, tag);
    (server, name)
}

fn listen(server: &Server, tag: &str) -> String {
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
    name
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

/// A kind taking bytes last, which a large call sends as its frame's tail.
mod weighing {
    #[guatiao::kind]
    pub trait Scale: Send + Sync {
        /// `seed` plus every byte of `data` times its position from 1.
        fn weigh(&self, seed: u64, data: &[u8]) -> u64;
    }

    pub fn weight(seed: u64, data: &[u8]) -> u64 {
        data.iter().zip(1u64..).fold(seed, |w, (&b, at)| {
            w.wrapping_add(u64::from(b).wrapping_mul(at))
        })
    }

    #[derive(Default, guatiao::Provider)]
    #[provider(Scale)]
    pub struct Weigher;

    impl Scale for Weigher {
        fn weigh(&self, seed: u64, data: &[u8]) -> u64 {
            weight(seed, data)
        }
    }

    guatiao::local_providers!(id = "weighing", version = "1.0.0", providers = [Weigher]);
}

#[test]
fn large_and_small_bytes_from_many_threads_arrive_whole() {
    use weighing::{Scale, weight};
    let mut registry = Registry::new("server", "1.0");
    registry
        .register_local("weighing", weighing::library)
        .expect("registers")
        .loaded()
        .expect("loaded");
    // The kind comes from the provider's own description.
    let server = Server::new(
        registry,
        guatiao_source_remote::Described::new(),
        LinkOptions::new(Trust::Local),
    );
    let name = listen(&server, "bytes");
    let (_source, registry) = host(&name);
    let scale = registry
        .offers::<dyn Scale>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    let threads: Vec<_> = (0..8u64)
        .map(|t| {
            let s = scale.clone();
            std::thread::spawn(move || {
                for i in 0..20u64 {
                    // Large ones, small ones, and either side of the size
                    // from which a last argument leaves as the frame's tail.
                    let len = match (t + i) % 4 {
                        0 => (1 << 20) + t * 1000 + i,
                        1 if i < 4 => (64 << 10) - 1 + i % 2,
                        _ => t + i,
                    };
                    // No period a chunk of the stream could repeat unseen.
                    let mut x = 0x9E37_79B9_7F4A_7C15 ^ (t << 32 | i);
                    let data: Vec<u8> = (0..len)
                        .map(|_| {
                            x ^= x << 13;
                            x ^= x >> 7;
                            x ^= x << 17;
                            x as u8
                        })
                        .collect();
                    assert_eq!(s.weigh(t * 100 + i, &data), weight(t * 100 + i, &data));
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
}

#[test]
fn a_client_that_says_nothing_is_dropped_at_the_handshake_bound() {
    use guatiao_source_remote::transport::Connect;
    let server =
        common::server(LinkOptions::new(Trust::Local).handshake_timeout(Duration::from_secs(1)));
    let name = endpoint("silent");
    let listener = LocalListen::bind(&name).expect("listens");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let conn = listener.accept().expect("accepts");
        let started = std::time::Instant::now();
        let failed = server.serve(conn).is_err();
        let _ = tx.send((failed, started.elapsed()));
    });
    // Connected, and never says HELLO.
    let _silent = LocalConnect::new(&name).connect().expect("connects");
    let (failed, took) = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the handshake ended at its bound");
    assert!(failed, "a silent client never links");
    assert!(took < Duration::from_secs(5), "{took:?}");
}
