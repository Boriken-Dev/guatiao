// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The id `#[derive(Provider)]` gives a provider that names none is its
//! package and its type, read through `::core::env!`, so a crate's own
//! `env!` in scope cannot change it.

#![cfg(all(feature = "provider", feature = "load"))]

use greeter_kind::Counter;
use guatiao::Alloc;
use guatiao::library::Registry;
use guatiao::library::kind::{ProviderDecl, ProviderParts};

/// A crate's own `env!`, in scope wherever the derive expands below.
macro_rules! env {
    ($($ignored:tt)*) => {
        "shadowed"
    };
}

#[derive(Default, guatiao::Provider)]
#[provider(Counter)]
pub struct Plain;

impl Counter for Plain {
    fn count(&self) -> i64 {
        1
    }
}

#[test]
fn a_crates_own_env_macro_does_not_change_the_default_id() {
    assert_eq!(env!("CARGO_PKG_NAME"), "shadowed", "the shadow is in scope");
    let mut registry = Registry::new("default-id", "1.0");
    let parts: &'static ProviderParts = Box::leak(Box::new(
        Plain::provider(registry.host(), Alloc::rust()).expect("builds"),
    ));
    let view = parts.info().view().expect("a well-formed descriptor");
    assert_eq!(view.id(), "guatiao_plain");
}
