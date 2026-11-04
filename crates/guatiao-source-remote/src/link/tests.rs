// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A link over memory to a scripted side: one whose answers panic, one
//! whose pool threads are slow to finish.

use alloc::sync::Arc;
use core::time::Duration;
use std::path::Path;
use std::sync::{Condvar, Mutex};
use std::time::Instant;

use guatiao::Status;
use guatiao::Value;
use guatiao::library::ProviderError;
use guatiao::library::kind::{Arg, KindDescription};

use super::pool::Point;
use super::{Handler, Inner, Link, LinkOptions, Nothing, Proxies, Setup, Table};
use crate::lock;
use crate::protocol::{Target, Trust};
use crate::transport::memory;

/// A side whose catalog and libraries panic when asked for.
struct Panicking;

impl Handler for Panicking {
    fn list(&self, _link: &Arc<Inner>) -> Result<Value, ProviderError> {
        panic!("a catalog that panics");
    }

    fn open(&self, _link: &Arc<Inner>, _library: &str, _stamp: &str) -> Result<(), ProviderError> {
        panic!("a library that panics");
    }

    fn table(&self, _link: &Arc<Inner>, _id: u64) -> Option<Arc<dyn Table>> {
        panic!("a table that panics");
    }
}

/// The example greeter kind, as the peer's tables would carry it.
fn greeter() -> KindDescription {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/greeter_kind/include/greeter_kind.kinds.json");
    let text = std::fs::read_to_string(path).unwrap();
    KindDescription::document_json(&text)
        .unwrap()
        .into_iter()
        .find(|k| k.name() == "greeter")
        .expect("the greeter kind")
}

fn setup(handler: Arc<dyn Handler>) -> Setup {
    Setup {
        handler,
        proxies: Proxies::new(),
        kinds: Arc::default(),
        name: "link-test".to_string(),
    }
}

/// A host's link to a side served by `handler`, running at most
/// `max_calls` calls, and that side's own end.
fn served(handler: Arc<dyn Handler>, max_calls: usize) -> (Link, Link) {
    let (host, device) = memory::pair();
    let options = LinkOptions::new(Trust::Local)
        .max_calls(max_calls)
        .timeout(Some(Duration::from_secs(5)));
    let theirs = options.clone();
    let served = std::thread::spawn(move || Link::server(device, &theirs, setup(handler)));
    let link = Link::client(host, &options, setup(Arc::new(Nothing))).expect("links");
    let server = served.join().unwrap().expect("serves");
    (link, server)
}

/// A host's link to a `Panicking` side running at most `max_calls` calls.
fn panicking(max_calls: usize) -> Link {
    let (link, server) = served(Arc::new(Panicking), max_calls);
    // The serving end lives as long as the link it answers on.
    std::thread::spawn(move || {
        while server.alive() {
            std::thread::sleep(Duration::from_millis(20));
        }
    });
    link
}

#[test]
fn a_call_that_panics_on_the_peer_is_answered_internal_and_its_place_given_back() {
    let link = panicking(2);
    // More than the peer's places, one after another: each panics, and
    // none may leave its place taken or its caller waiting for the timeout.
    for _ in 0..5 {
        let started = Instant::now();
        let e = link.list().expect_err("the catalog panicked");
        assert_eq!(e.status, Status::GUATIAO_ERR_INTERNAL, "{e:?}");
        assert!(e.message().contains("a catalog that panics"), "{e:?}");
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "answered by the timeout"
        );
        let e = link.open("shapes", "1").expect_err("the library panicked");
        assert_eq!(e.status, Status::GUATIAO_ERR_INTERNAL, "{e:?}");
    }
    assert!(link.alive(), "{}", link.why());
}

#[test]
fn a_call_on_a_table_that_panics_is_answered_internal_and_the_next_one_runs() {
    let link = panicking(2);
    let kind = greeter();
    let slot = kind
        .slots()
        .iter()
        .position(|s| s.name() == "greet")
        .expect("a greet slot");
    for _ in 0..5 {
        let started = Instant::now();
        let e = link
            .inner()
            .call(Target::Table(1), &kind, slot, vec![Arg::Str("world")])
            .expect_err("the table panicked");
        assert_eq!(e.status, Status::GUATIAO_ERR_INTERNAL, "{e:?}");
        assert!(e.message().contains("a table that panics"), "{e:?}");
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "answered by the timeout"
        );
    }
    assert!(link.alive(), "{}", link.why());
}

/// Holds a pool thread that has finished a job, before it waits for the
/// next, until the pool has placed or refused a job beyond those already
/// run, or `let_go` is called.
#[derive(Default)]
struct Slow {
    seen: Mutex<Seen>,
    moved: Condvar,
}

#[derive(Default)]
struct Seen {
    /// Jobs the pool has placed or refused.
    handed: u32,
    /// Jobs a pool thread has finished.
    ran: u32,
    /// Hold no more.
    let_go: bool,
}

impl Slow {
    fn at(&self, point: Point) {
        let mut seen = lock(&self.seen);
        match point {
            Point::Handed => {
                seen.handed += 1;
                self.moved.notify_all();
            }
            Point::Ran => {
                seen.ran += 1;
                // The next job may be placed before this thread gets here:
                // then it is queued for this thread, which must not wait.
                let _held = self
                    .moved
                    .wait_timeout_while(seen, Duration::from_secs(10), |s| {
                        s.handed <= s.ran && !s.let_go
                    });
            }
            _ => {}
        }
    }

    fn let_go(&self) {
        lock(&self.seen).let_go = true;
        self.moved.notify_all();
    }
}

#[test]
fn a_peer_waiting_for_each_answer_is_never_refused_busy_by_a_thread_slow_to_finish() {
    // One call at a time, and its thread is still finishing when the next
    // request arrives: the peer kept to `max_calls`, so it is not refused.
    let (link, server) = served(Arc::new(Nothing), 1);
    let slow = Arc::new(Slow::default());
    let at = Arc::clone(&slow);
    server
        .inner()
        .pool
        .set_hook(Arc::new(move |point| at.at(point)));
    let kind = greeter();
    let slot = kind
        .slots()
        .iter()
        .position(|s| s.name() == "greet")
        .expect("a greet slot");
    for _ in 0..3 {
        let e = link.list().expect_err("this side serves nothing");
        assert_eq!(e.status, Status::GUATIAO_ERR_UNSUPPORTED, "{}", e.message());
        let e = link
            .open("shapes", "1")
            .expect_err("this side serves nothing");
        assert_eq!(e.status, Status::GUATIAO_ERR_UNSUPPORTED, "{}", e.message());
        let e = link
            .inner()
            .call(Target::Table(1), &kind, slot, vec![Arg::Str("world")])
            .expect_err("this side has no tables");
        assert_eq!(e.status, Status::GUATIAO_ERR_NOT_FOUND, "{}", e.message());
    }
    slow.let_go();
    assert!(link.alive(), "{}", link.why());
}
