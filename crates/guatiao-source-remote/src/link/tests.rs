// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A link over memory to a scripted side whose answers panic.

use alloc::sync::Arc;
use core::time::Duration;
use std::path::Path;
use std::time::Instant;

use guatiao::Status;
use guatiao::Value;
use guatiao::library::ProviderError;
use guatiao::library::kind::{Arg, KindDescription};

use super::{Handler, Inner, Link, LinkOptions, Nothing, Proxies, Setup, Table};
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

/// A host's link to a `Panicking` side running at most `max_calls` calls.
fn panicking(max_calls: usize) -> Link {
    let (host, device) = memory::pair();
    let options = LinkOptions::new(Trust::Local)
        .max_calls(max_calls)
        .timeout(Some(Duration::from_secs(5)));
    let theirs = options.clone();
    let served =
        std::thread::spawn(move || Link::server(device, &theirs, setup(Arc::new(Panicking))));
    let link = Link::client(host, &options, setup(Arc::new(Nothing))).expect("links");
    let server = served.join().unwrap().expect("serves");
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
