// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Two kinds, declared as traits. This is the whole crate: the host and
//! every library that offers or consumes a greeter compile against it.
//! The tables, the shims and the proxies are `#[guatiao::kind]`'s.

use guatiao::Map;
use guatiao::library::ProviderError;

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
                    .and_then(|v| v.as_str().map(str::to_uppercase))
            })
            .unwrap_or_default()
    }
}

/// Counts how often it was asked.
#[guatiao::kind]
pub trait Counter: Send + Sync {
    /// The count so far, after this call.
    fn count(&self) -> i64;
}
