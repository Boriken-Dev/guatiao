// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The derives and `#[kind]` pointed at a re-export of the crate with
//! `crate = <path>`, which is how a provider depends on a kind crate alone.
//!
//! This crate names `guatiao` itself, so what is shown here is that the
//! re-rooted expansion compiles and runs; that nothing in it is still
//! rooted at `::guatiao` is asserted on the tokens, in `guatiao-derive`.

#![cfg(all(feature = "provider", feature = "load"))]

/// What a kind crate does: re-export the crate its macros expand against.
mod kinds {
    pub use guatiao;
}

use kinds::guatiao::library::kind::{ProviderDecl, ProviderParts};
use kinds::guatiao::library::{ProviderError, Registry};
use kinds::guatiao::{Alloc, FromValue, Map, Provider, Schema, ToValue, Value};

#[kinds::guatiao::kind(crate = crate::kinds::guatiao)]
pub trait Greeter: Send + Sync {
    fn greet(&self, name: &str) -> Result<String, ProviderError>;
    fn settings(&self) -> Result<Settings, ProviderError>;
}

#[derive(Debug, Clone, Copy, PartialEq, ToValue, FromValue, Schema)]
#[map(crate = crate::kinds::guatiao)]
enum Tone {
    Calm,
    Loud,
}

#[derive(Debug, Clone, PartialEq, ToValue, FromValue, Schema)]
#[map(crate = crate::kinds::guatiao)]
pub struct Settings {
    prefix: String,
    #[map(default = 2)]
    bangs: u8,
    tone: Option<Tone>,
}

#[derive(Provider)]
#[provider(Greeter, config = Settings, crate = crate::kinds::guatiao)]
struct Shouter(Settings);

impl TryFrom<Settings> for Shouter {
    type Error = ProviderError;
    fn try_from(settings: Settings) -> Result<Shouter, ProviderError> {
        Ok(Shouter(settings))
    }
}

impl Greeter for Shouter {
    fn greet(&self, name: &str) -> Result<String, ProviderError> {
        Ok(format!(
            "{} {name}{}",
            self.0.prefix,
            "!".repeat(usize::from(self.0.bangs))
        ))
    }

    fn settings(&self) -> Result<Settings, ProviderError> {
        Ok(self.0.clone())
    }
}

#[test]
fn a_provider_written_against_a_re_export_builds_and_answers() {
    let mut registry = Registry::new("crate-path", "1.0");
    let parts: &'static ProviderParts = Box::leak(Box::new(
        Shouter::provider(registry.host(), Alloc::rust()).expect("builds"),
    ));

    let mut config = Map::new();
    config.set("prefix", "hey").unwrap();
    let shouter = parts
        .info()
        .instantiate::<dyn Greeter>(&Value::from(config))
        .expect("a configuration with the default left out builds");
    assert_eq!(shouter.greet("ana").unwrap(), "hey ana!!");
    assert_eq!(
        shouter.settings().unwrap(),
        Settings {
            prefix: "hey".into(),
            bangs: 2,
            tone: None,
        },
        "a derived type crosses as a value, in both directions"
    );

    let schema = Settings::schema(Alloc::rust()).expect("a schema");
    let written = Settings {
        prefix: "x".into(),
        bangs: 1,
        tone: Some(Tone::Loud),
    }
    .to_value(Alloc::rust())
    .unwrap();
    kinds::guatiao::schema::validate_map(
        kinds::guatiao::schema::SchemaRef::new(&schema).unwrap(),
        &written,
    )
    .expect("what the type writes, its schema accepts");
    assert_eq!(
        Settings::from_value(&written).unwrap().tone,
        Some(Tone::Loud)
    );
}
