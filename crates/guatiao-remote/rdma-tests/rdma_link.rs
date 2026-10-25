// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The RDMA transport on a real device (Soft-RoCE will do): every slot
//! shape the other transports carry, the trusted level's key and ranges,
//! frames far larger than a ring, many at once, and a link ended from
//! either side. With no device every test fails, saying so.
//!
//! `GUATIAO_RDMA_DEVICE` names the device (default: the first listed).

extern crate alloc;

#[path = "../tests/common/mod.rs"]
mod common;

use alloc::sync::Arc;
use core::sync::atomic::Ordering;
use core::time::Duration;
use std::io::Read as _;
use std::net::TcpStream;
use std::sync::Mutex;

use common::{CHATS, Ear, eventually};
use greeter_kind::{Counter, Greeter, Listener, Wide};
use guatiao::library::Registry;
use guatiao::value::convert::TryAsRef;
use guatiao::{Map, Status};
use guatiao_remote::transport::rdma::{self, RING, RdmaConnect, RdmaListen, RdmaPort};
use guatiao_remote::transport::tcp::{Accepted, Range};
use guatiao_remote::{LinkOptions, RemoteSource, Server, Trust};

const KEY: &[u8] = b"rdma-test-key";

fn port() -> RdmaPort {
    let devices = rdma::devices().unwrap_or_else(|e| {
        panic!("no RDMA device could be listed: {e}");
    });
    assert!(
        !devices.is_empty(),
        "no RDMA device: for Soft-RoCE, `modprobe rdma_rxe` and \
         `rdma link add rxe0 type rxe netdev <interface>`"
    );
    match std::env::var("GUATIAO_RDMA_DEVICE") {
        Ok(name) => RdmaPort::named(&name),
        Err(_) => RdmaPort::first(),
    }
}

/// A trusted server on loopback, set up over TCP and carried by RDMA.
fn serve(server: Server) -> String {
    let listener = RdmaListen::bind("127.0.0.1:0", &[Range::parse("127.0.0.1").unwrap()])
        .expect("listens")
        .port(port());
    let address = listener.local_addr().unwrap().to_string();
    std::thread::spawn(move || {
        while let Ok(accepted) = listener.accept() {
            if let Accepted::Connection(conn) = accepted {
                let server = server.clone();
                std::thread::spawn(move || {
                    let _ = server.serve(conn);
                });
            }
        }
    });
    address
}

fn trusted() -> LinkOptions {
    LinkOptions::new(Trust::Trusted).key(KEY)
}

fn source(address: &str, options: LinkOptions) -> RemoteSource {
    RemoteSource::new(Box::new(RdmaConnect::new(address).port(port())), options)
}

/// A server of every slot shape, and a host linked to it.
fn pair(timeout: Option<Duration>) -> (Server, RemoteSource, Registry) {
    let server = common::server(trusted());
    let address = serve(server.clone());
    let source = source(&address, trusted().timeout(timeout));
    let registry = common::host(&source);
    (server, source, registry)
}

fn greeting(map: &Map) -> String {
    map.get("greeting")
        .and_then(TryAsRef::<str>::try_as_ref)
        .unwrap_or_default()
        .to_string()
}

#[test]
fn a_device_is_present() {
    port();
}

#[test]
fn every_slot_shape_crosses() {
    let (_server, source, registry) = pair(None);
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .expect("the remote greeter is offered")
        .shared()
        .expect("a default instance");

    assert_eq!(greeting(&greeter.greet("ana").unwrap()), "hello, ana");
    let e = greeter.greet("").unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(e.message(), "nobody to greet");
    assert_eq!(greeter.shout("ana"), "HELLO, ANA");

    let mut config = Map::new();
    config.set("name", "bo");
    assert_eq!(greeting(&greeter.greet_from(config).unwrap()), "hello, bo");

    let mut config = Map::new();
    config.set("name", "cy");
    assert!(greeter.check(&mut config).unwrap());
    assert_eq!(greeting(&config), "hello, cy");

    let mut ear = Ear::new();
    greeter.greet_into("di", &mut ear).unwrap();
    assert_eq!(*ear.heard.lock().unwrap(), ["hello, di", "twice"]);

    let ear = Ear::new();
    let (heard, dropped) = (Arc::clone(&ear.heard), Arc::clone(&ear.dropped));
    let mut conversation = greeter.start(ear.into_object()).unwrap();
    assert_eq!(CHATS.load(Ordering::SeqCst), 1);
    conversation.say("hi").unwrap();
    conversation.say("there").unwrap();
    assert_eq!(*heard.lock().unwrap(), ["hi", "there"]);
    assert_eq!(conversation.turns(), 2);
    let mut buf = [0u8; 16];
    assert_eq!(conversation.read(&mut buf), 7);
    assert_eq!(&buf[..7], b"hithere");
    let e = conversation.say("").unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(source.objects_out(), 1);

    drop(conversation);
    eventually("the conversation is dropped on the server", || {
        CHATS.load(Ordering::SeqCst) == 0
    });
    eventually("the host's listener is dropped", || {
        *dropped.lock().unwrap() == 1
    });
    assert_eq!(source.objects_out(), 0);

    let counter = registry
        .offers::<dyn Counter>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    assert_eq!(counter.count(), 1);
    assert_eq!(counter.count(), 2);
    let wide = registry
        .offers::<dyn Wide>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    assert_eq!(wide.echo_i64(-5).unwrap(), -5);
    assert_eq!(wide.echo_i64(i64::MIN).unwrap(), i64::MIN);
    assert_eq!(wide.echo_u64(u64::MAX >> 30).unwrap(), u64::MAX >> 30);
}

#[test]
fn pipelined_calls_complete_out_of_order() {
    let (_server, _source, registry) = pair(None);
    let wide = registry
        .offers::<dyn Wide>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    let finished = Arc::new(Mutex::new(Vec::new()));
    let threads: Vec<_> = [1300i64, 1010, 1150]
        .into_iter()
        .map(|x| {
            let (wide, finished) = (wide.clone(), Arc::clone(&finished));
            std::thread::spawn(move || {
                assert_eq!(wide.echo_i64(x).unwrap(), x);
                finished.lock().unwrap().push(x);
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    assert_eq!(*finished.lock().unwrap(), [1010, 1150, 1300]);
}

#[test]
fn a_callback_runs_on_the_thread_that_lent() {
    let (_server, _source, registry) = pair(None);
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    struct Here(std::thread::ThreadId, Vec<bool>);
    impl Listener for Here {
        fn heard(&mut self, _: &str) {
            self.1.push(std::thread::current().id() == self.0);
        }
    }
    let mut here = Here(std::thread::current().id(), Vec::new());
    greeter.greet_into("x", &mut here).unwrap();
    assert_eq!(here.1, [true, true]);
}

#[test]
fn a_slow_call_times_out_gone_and_the_link_carries_on() {
    let (_server, _source, registry) = pair(Some(Duration::from_millis(200)));
    let wide = registry
        .offers::<dyn Wide>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    let e = wide.echo_i64(1600).unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_GONE);
    assert!(e.message().contains("no answer"), "{}", e.message());
    assert_eq!(wide.echo_i64(3).unwrap(), 3);
}

#[test]
fn callbacks_nested_without_end_are_refused_and_unwind() {
    let (_server, _source, registry) = pair(None);
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    struct Again {
        greeter: Arc<dyn Greeter>,
        depth: usize,
        refused: Vec<Status>,
    }
    impl Listener for Again {
        fn heard(&mut self, _: &str) {
            if self.depth > 500 || !self.refused.is_empty() {
                return;
            }
            self.depth += 1;
            let greeter = self.greeter.clone();
            if let Err(e) = greeter.greet_into("again", self) {
                self.refused.push(e.status);
            }
        }
    }
    let mut again = Again {
        greeter: greeter.clone(),
        depth: 0,
        refused: Vec::new(),
    };
    let outer = greeter.greet_into("start", &mut again);
    assert!(
        again.depth < 500,
        "the nesting was bounded at {}",
        again.depth
    );
    assert!(outer.is_err() || !again.refused.is_empty());
    assert!(greeter.greet("after").is_ok());
}

#[test]
fn frames_far_larger_than_a_ring_cross_both_ways_many_at_once() {
    let (_server, _source, registry) = pair(Some(Duration::from_secs(120)));
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    // 40 MiB out and back: ten rings each way, wrapping many times.
    let big: String = (0..10 * RING + 12_345)
        .map(|i| char::from(b'a' + (i % 26) as u8))
        .collect();
    let answer = greeting(&greeter.greet(&big).unwrap());
    assert_eq!(answer.len(), big.len() + "hello, ".len());
    assert!(answer.ends_with(&big[big.len() - 1000..]));
    assert_eq!(&answer["hello, ".len()..], big);

    // Eight callers at once, each a frame over a ring and small calls
    // between, every answer its own.
    let threads: Vec<_> = (0..8u8)
        .map(|t| {
            let greeter = greeter.clone();
            std::thread::spawn(move || {
                let mine: String = core::iter::repeat_n(char::from(b'A' + t), RING + 999).collect();
                for round in 0..3 {
                    let answer = greeting(&greeter.greet(&mine).unwrap());
                    assert_eq!(
                        &answer["hello, ".len()..],
                        mine,
                        "caller {t}, round {round}"
                    );
                    let small = format!("{t}-{round}");
                    assert_eq!(
                        greeting(&greeter.greet(&small).unwrap()),
                        format!("hello, {small}")
                    );
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
}

#[test]
fn many_small_calls_cross_in_order() {
    let (_server, _source, registry) = pair(None);
    let wide = registry
        .offers::<dyn Wide>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    // Each answer is its own call's (below 1000 nothing waits).
    for i in 0..20_000i64 {
        assert_eq!(wide.echo_i64(-i).unwrap(), -i);
    }
}

#[test]
fn the_right_key_links_and_others_are_refused() {
    let address = serve(common::server(trusted()));
    let refresh = |options: LinkOptions| {
        let mut registry = Registry::new("host", "1.0");
        registry
            .add_source("remote", source(&address, options))
            .unwrap();
        let refreshed = registry.refresh();
        let refused: Vec<String> = refreshed.refused.iter().map(|r| r.reason.clone()).collect();
        (registry, refused)
    };
    let (registry, refused) = refresh(trusted());
    assert!(refused.is_empty(), "{refused:?}");
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    assert!(greeter.greet("rdma").is_ok());

    let (registry, refused) = refresh(LinkOptions::new(Trust::Trusted).key(b"other"));
    assert_eq!(registry.offers::<dyn Greeter>().count(), 0);
    assert!(
        refused
            .iter()
            .any(|r| r.contains("does not hold this link's key")),
        "{refused:?}"
    );
    let (_, refused) = refresh(LinkOptions::new(Trust::Local));
    assert!(
        refused
            .iter()
            .any(|r| r.contains("serves the trusted level")),
        "{refused:?}"
    );
}

#[test]
fn a_peer_outside_the_ranges_is_turned_away_unread() {
    let listener = RdmaListen::bind("127.0.0.1:0", &[Range::parse("127.0.0.2").unwrap()])
        .unwrap()
        .port(port());
    let mut c = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    assert!(matches!(listener.accept().unwrap(), Accepted::Refused(_)));
    c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut byte = [0u8; 1];
    assert!(matches!(c.read(&mut byte), Ok(0) | Err(_)));
}

#[test]
fn a_link_ended_by_either_side_ends_on_both() {
    let (server, source, registry) = pair(None);
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

    let (server, source, registry) = pair(None);
    assert_eq!(server.links(), 1);
    drop(registry);
    source.disconnect();
    eventually("the server's link ends", || server.links() == 0);
}

#[test]
fn a_refresh_after_the_link_ends_links_again() {
    let (server, source, mut registry) = pair(None);
    let link = source.link().unwrap();
    server.end_links();
    eventually("the host's link ends", || !link.alive());
    let refreshed = registry.refresh();
    assert!(refreshed.refused.is_empty(), "{:?}", refreshed.refused);
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    assert_eq!(greeting(&greeter.greet("again").unwrap()), "hello, again");
}
