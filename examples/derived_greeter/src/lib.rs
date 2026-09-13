// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A library that is nothing but its trait impls. The descriptor, the
//! tables, the entry symbol, the id (`derived_greeter_hello`) and the
//! version all come from the derive and the one macro line at the end.

use std::sync::atomic::{AtomicI64, Ordering};

use greeter_kind::{Counter, Greeter};
use guatiao::library::ProviderError;
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
}

impl Counter for Hello {
    fn count(&self) -> i64 {
        self.asked.fetch_add(1, Ordering::Relaxed) + 1
    }
}

guatiao::providers!(Hello);
