// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Every slot shape of the example kinds, across a link: borrowed, taken
//! and lent arguments, every answer, objects both ways, callbacks during a
//! call, refusals; and pipelined calls completing out of order.

extern crate alloc;

mod common;

use alloc::sync::Arc;
use core::sync::atomic::Ordering;
use core::time::Duration;
use std::sync::Mutex;

use common::{CHATS, Ear, eventually, pair};
use greeter_kind::{Counter, Greeter, Listener, Wide};
use guatiao::library::kind::KindDescription;
use guatiao::value::convert::TryAsRef;
use guatiao::{Map, Status};

fn greeting(map: &Map) -> String {
    map.get("greeting")
        .and_then(TryAsRef::<str>::try_as_ref)
        .unwrap_or_default()
        .to_string()
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

    // Borrowed text in, a map out; a refusal with its status and words.
    assert_eq!(greeting(&greeter.greet("ana").unwrap()), "hello, ana");
    let e = greeter.greet("").unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(e.message(), "nobody to greet");

    // Large text in and a large map out, each a frame of its own over a
    // transport that carries whole messages.
    let big = "b".repeat(1 << 20);
    assert_eq!(
        greeting(&greeter.greet(&big).unwrap()),
        format!("hello, {big}")
    );

    // Text answered.
    assert_eq!(greeter.shout("ana"), "HELLO, ANA");

    // A map handed over.
    let mut config = Map::new();
    config.set("name", "bo");
    assert_eq!(greeting(&greeter.greet_from(config).unwrap()), "hello, bo");

    // A map lent: what the callee added is the caller's after the call.
    let mut config = Map::new();
    config.set("name", "cy");
    assert!(greeter.check(&mut config).unwrap());
    assert_eq!(greeting(&config), "hello, cy");

    // An object lent for one call, called back twice during it.
    let mut ear = Ear::new();
    greeter.greet_into("di", &mut ear).unwrap();
    assert_eq!(*ear.heard.lock().unwrap(), ["hello, di", "twice"]);

    // An object handed over, an object answered, a callback through the
    // handed one during a call on the answered one, a buffer written.
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

    // Dropping the answered object releases it on the server, which drops
    // the listener it held, which releases the host's.
    drop(conversation);
    eventually("the conversation is dropped on the server", || {
        CHATS.load(Ordering::SeqCst) == 0
    });
    eventually("the host's listener is dropped", || {
        *dropped.lock().unwrap() == 1
    });
    assert_eq!(source.objects_out(), 0);

    // Scalars.
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
    // Each `heard` calls `greet_into` again, which calls `heard` again:
    // every round nests one call deeper on both sides' threads.
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
    assert!(
        outer.is_err() || !again.refused.is_empty(),
        "a call past the bound was refused"
    );
    // The link carries on.
    assert!(greeter.greet("after").is_ok());
}

/// A remote provider carries its kinds' document, as the peer's own
/// derived provider does: a host binds it with no file of its own.
#[test]
fn a_remote_provider_carries_its_kinds_descriptions() {
    let (server, _source, registry) = pair(None);
    let remote = registry
        .providers("greeter")
        .next()
        .expect("the remote greeter");
    let carried = remote.kind_descriptions().expect("a document");
    let names: Vec<&str> = carried.iter().map(KindDescription::name).collect();
    assert_eq!(
        names,
        ["greeter", "listener", "conversation", "counter", "wide"]
    );

    let own = server
        .registry()
        .providers("greeter")
        .next()
        .expect("the peer's greeter")
        .kind_descriptions()
        .expect("the derived document");
    assert_eq!(carried, own);
}
