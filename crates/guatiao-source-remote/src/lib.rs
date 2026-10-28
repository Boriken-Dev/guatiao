// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! guatiao providers called across a process, a machine or a network.
//!
//! A **server** ([`Server`]) serves a registry's providers; a **host** adds
//! a [`RemoteSource`] (or, in any language, this crate's library file with
//! `add_source_file`) and, after `refresh()`, sees the peer's providers as
//! ordinary ones, called through tables built at run time from the kinds'
//! descriptions. Either side may call the other while a call is open, so
//! an object lent or handed to a remote provider is called back over the
//! same link.
//!
//! One framed protocol ([`protocol`]) runs over every transport
//! ([`transport`]). `docs/remote.md` states it for every language.

#![deny(unsafe_code)]

extern crate alloc;

pub mod client;
pub mod config;
#[allow(unsafe_code)]
mod exports;
pub mod link;
pub mod protocol;
pub mod server;
pub mod transport;

/// The access control lists network listeners admit peers by
/// (`coqui-net-acl`).
#[cfg(feature = "tcp")]
pub use coqui_net_acl as acl;

/// `m`'s guard, taken even when a thread panicked holding it: what a
/// mutex here guards stays consistent between statements, and a link must
/// still be ended after a panic.
pub(crate) fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub use client::RemoteSource;
pub use link::{Link, LinkError, LinkOptions};
pub use protocol::Trust;
pub use server::Server;

/// The kinds a [`Server`] serves, read from their descriptions
/// (`guatiao-bridge`).
pub use guatiao_bridge::Kinds;
