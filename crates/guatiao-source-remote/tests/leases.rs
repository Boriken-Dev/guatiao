// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! An object's hold on the far side ends when its handle is dropped, when
//! the call that would have answered it is cancelled, and when the link is
//! lost; a peer gone mid-call answers `GONE` and the host carries on.

extern crate alloc;

mod common;

use alloc::sync::Arc;
use core::sync::atomic::Ordering;
use core::time::Duration;
use std::sync::Mutex;
use std::time::Instant;

use common::{CHATS, Ear, eventually, pair};
use greeter_kind::{Greeter, Listener, Wide};
use guatiao::Status;

/// The tests count one server's conversations: one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

#[test]
fn a_cancelled_call_drops_the_object_it_answers_late() {
    let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (_server, source, registry) = pair(Some(Duration::from_millis(150)));
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    let wide = registry
        .offers::<dyn Wide>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    wide.echo_u64((1 << 40) + 400).unwrap();

    let ear = Ear::new();
    let dropped = Arc::clone(&ear.dropped);
    let e = greeter.start(ear.into_object()).unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_GONE);
    // The server builds the conversation after the host stopped waiting:
    // nobody holds it, so it goes, and with it the listener it was handed.
    eventually("the late conversation is dropped", || {
        CHATS.load(Ordering::SeqCst) == 0
    });
    eventually("the listener handed for it is dropped", || {
        *dropped.lock().unwrap() == 1
    });
    assert_eq!(source.objects_out(), 0);
    wide.echo_u64(1 << 40).unwrap();
}

#[test]
fn losing_the_link_ends_every_hold_on_both_sides() {
    let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (server, source, registry) = pair(None);
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    let ear = Ear::new();
    let dropped = Arc::clone(&ear.dropped);
    let mut conversation = greeter.start(ear.into_object()).unwrap();
    conversation.say("before").unwrap();
    assert_eq!(CHATS.load(Ordering::SeqCst), 1);

    server.end_links();
    eventually("the server drops what it held for the link", || {
        CHATS.load(Ordering::SeqCst) == 0
    });
    eventually("the host drops what it held for the link", || {
        *dropped.lock().unwrap() == 1
    });
    let e = conversation.say("after").unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_GONE);
    drop(conversation);
    assert_eq!(source.objects_out(), 0);
}

#[test]
fn a_peer_gone_mid_call_answers_gone_and_a_refresh_reconnects() {
    let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (server, _source, mut registry) = pair(None);
    let wide = registry
        .offers::<dyn Wide>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    let started = Instant::now();
    let call = std::thread::spawn(move || wide.echo_i64(3000));
    std::thread::sleep(Duration::from_millis(100));
    server.end_links();
    let e = call.join().unwrap().unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_GONE);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "answered at once"
    );

    // The host is fine; its providers answer GONE until it refreshes.
    let stale = registry
        .offers::<dyn Wide>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    assert_eq!(
        stale.echo_i64(1).unwrap_err().status,
        Status::GUATIAO_ERR_GONE
    );
    let refreshed = registry.refresh();
    assert_eq!(refreshed.loaded.len(), 1, "{refreshed:?}");
    assert_eq!(refreshed.retired.len(), 1, "{refreshed:?}");
    drop(stale);
    let wide = registry
        .offers::<dyn Wide>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    assert_eq!(wide.echo_i64(7).unwrap(), 7);
    let _ = registry.collect();
}

#[test]
fn an_object_lent_for_a_call_answers_gone_after_it() {
    let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // A listener the host lends is the server's to call only during the
    // call: the server has no way to keep it, so this checks the host's
    // side, that nothing of the lend stays registered.
    let (_server, source, registry) = pair(None);
    let greeter = registry
        .offers::<dyn Greeter>()
        .next()
        .unwrap()
        .shared()
        .unwrap();
    struct Quiet;
    impl Listener for Quiet {
        fn heard(&mut self, _: &str) {}
    }
    for _ in 0..50 {
        greeter.greet_into("x", &mut Quiet).unwrap();
    }
    assert_eq!(source.objects_out(), 0);
}
