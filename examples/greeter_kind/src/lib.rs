// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Two provider kinds and two object kinds, declared as traits. This is
//! the whole crate: the host and every library that offers or consumes a
//! greeter compile against it. The tables, the shims and the proxies are
//! `#[guatiao::kind]`'s.
//!
//! A **provider** kind (`Greeter`, `Counter`) is shared and offered by a
//! registry. An **object** kind (`Conversation`, `Listener`) is a handle
//! one caller owns: a provider hands a `Conversation` back, the host
//! hands a `Listener` in, and each is destroyed by whoever holds it last.

use guatiao::Map;
use guatiao::library::{Object, ProviderError};
use guatiao::value::convert::TryAsRef;

/// Greets by name.
#[guatiao::kind]
pub trait Greeter: Send + Sync {
    /// `{"greeting": "..."}`, or why not.
    fn greet(&self, name: &str) -> Result<Map, ProviderError>;
    /// Appended after `greet`: a library built before it existed makes
    /// the proxy run this body instead.
    fn shout(&self, name: &str) -> String {
        self.greet(name)
            .ok()
            .and_then(|m| {
                m.get("greeting")
                    .and_then(|v| TryAsRef::<str>::try_as_ref(v).map(str::to_uppercase))
            })
            .unwrap_or_default()
    }
    /// Appended: a conversation the caller then drives, told to the
    /// listener the caller handed in. A greeter built before this slot
    /// existed, or one that holds no conversations, answers the default.
    fn start(
        &self,
        listener: Object<dyn Listener>,
    ) -> Result<Object<dyn Conversation>, ProviderError> {
        drop(listener);
        Err(ProviderError::new(
            guatiao::Status::GUATIAO_ERR_NULL,
            "this greeter holds no conversations",
        ))
    }
}

/// What a conversation tells whoever is listening. Implemented by the
/// HOST and handed to the library, which owns it from then on.
#[guatiao::kind(object)]
pub trait Listener: Send {
    /// Something was said.
    fn heard(&mut self, what: &str);
}

/// One conversation a greeter started: state the caller drives.
#[guatiao::kind(object)]
pub trait Conversation: Send {
    /// Say something; the listener hears it.
    fn say(&mut self, what: &str) -> Result<(), ProviderError>;
    /// Copies the transcript so far into `dst` and answers how many bytes
    /// it wrote — an out-buffer, the caller's own memory.
    fn read(&mut self, dst: &mut [u8]) -> i64;
    /// How many things were said.
    fn turns(&self) -> i64;
}

/// Counts how often it was asked.
#[guatiao::kind]
pub trait Counter: Send + Sync {
    /// The count so far, after this call.
    fn count(&self) -> i64;
}
