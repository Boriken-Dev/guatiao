// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A handle held across an `.await` leaves the future `Send`, so a host
//! can drive a kind from a spawned task.
//!
//! Inside a future the lifetime of a `dyn` type is erased, so the handle
//! must be `Send` for `dyn Trait + 'a` at every `'a`, not at `'static`
//! alone. Compiling is the assertion; nothing here is polled.

#![cfg(feature = "provider")]

use guatiao::library::{Instance, Object, Offer, ProviderError, Remote};

#[guatiao::kind(object)]
pub trait Session: Send {
    fn poll(&mut self) -> Result<(), ProviderError>;
}

#[guatiao::kind]
pub trait Tuner: Send + Sync {
    fn tune(&self) -> Result<(), ProviderError>;
}

async fn object() -> Object<dyn Session> {
    unreachable!("never polled")
}

async fn remote() -> Remote<dyn Tuner> {
    unreachable!("never polled")
}

async fn instance() -> Instance<dyn Tuner> {
    unreachable!("never polled")
}

async fn offer() -> Offer<dyn Tuner> {
    unreachable!("never polled")
}

async fn tick() {}

async fn with_object() {
    let mut session = object().await;
    tick().await;
    let _ = session.poll();
}

async fn with_remote() {
    let tuner = remote().await;
    tick().await;
    let _ = tuner.tune();
}

async fn with_instance() {
    let tuner = instance().await;
    tick().await;
    let _ = tuner.tune();
}

async fn with_offer() {
    let tuner = offer().await;
    tick().await;
    let _ = tuner.tune();
}

fn assert_send<T: Send>(_: T) {}

#[test]
fn a_future_holding_a_handle_across_an_await_is_send() {
    assert_send(with_object());
    assert_send(with_remote());
    assert_send(with_instance());
    assert_send(with_offer());
}
