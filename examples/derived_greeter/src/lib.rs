// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A library that is nothing but its trait impls. The descriptor, the
//! tables, the entry symbol, the id (`derived_greeter_hello`) and the
//! version all come from the derive and the one macro line at the end.

use std::sync::atomic::{AtomicI64, Ordering};

use greeter_kind::{Conversation, Counter, Greeter, Listener};
use guatiao::library::{Object, ProviderError};
use guatiao::{Map, Provider, Status};

/// One provider serving two kinds.
#[derive(Default, Provider)]
#[provider(Greeter, Counter)]
pub struct Hello {
    asked: AtomicI64,
}

impl Greeter for Hello {
    fn greet(&self, name: &str) -> Result<Map, ProviderError> {
        if name == "panic" {
            panic!("a provider bug, on request");
        }
        if name.is_empty() {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_BAD_VALUE,
                "nobody to greet",
            ));
        }
        let mut map = Map::new();
        map.set("greeting", format!("hello, {name}"))?;
        Ok(map)
    }

    /// A conversation: an object this library builds and hands back,
    /// holding the listener the host handed in.
    fn start(
        &self,
        listener: Object<dyn Listener>,
    ) -> Result<Object<dyn Conversation>, ProviderError> {
        Ok(Chat {
            listener,
            transcript: String::new(),
            turns: 0,
        }
        .into_object())
    }
}

/// One conversation. Plain Rust; `into_object()` makes it a handle.
struct Chat {
    listener: Object<dyn Listener>,
    transcript: String,
    turns: i64,
}

impl Conversation for Chat {
    fn say(&mut self, what: &str) -> Result<(), ProviderError> {
        if what.is_empty() {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_BAD_VALUE,
                "nothing to say",
            ));
        }
        if !self.transcript.is_empty() {
            self.transcript.push('\n');
        }
        self.transcript.push_str(what);
        self.turns += 1;
        // Across the boundary and back: the listener is the host's.
        self.listener.heard(what);
        Ok(())
    }

    fn read(&mut self, dst: &mut [u8]) -> i64 {
        let n = self.transcript.len().min(dst.len());
        dst[..n].copy_from_slice(&self.transcript.as_bytes()[..n]);
        n as i64
    }

    fn turns(&self) -> i64 {
        self.turns
    }
}

impl Counter for Hello {
    fn count(&self) -> i64 {
        self.asked.fetch_add(1, Ordering::Relaxed) + 1
    }
}

/// A provider built from a configuration: no default instance, one
/// instance per configuration, each its own `self`. It is its own
/// configuration: a host reads this type's schema, fills it in, and the
/// value it hands to `instantiate` decodes straight into a `Shouter`.
#[derive(guatiao::Schema, guatiao::FromValue, Provider)]
#[provider(Greeter, config)]
pub struct Shouter {
    /// What every shout starts with.
    prefix: String,
}

impl Greeter for Shouter {
    fn greet(&self, name: &str) -> Result<Map, ProviderError> {
        let mut map = Map::new();
        map.set(
            "greeting",
            format!("{} {}", self.prefix, name.to_uppercase()),
        )?;
        Ok(map)
    }
}

guatiao::providers!(Hello, Shouter);
