// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `#[derive(Provider)]` and `guatiao::providers!`, in one process: the
//! descriptor a derived type builds, and the library a line names.

#![cfg(all(feature = "provider", feature = "load"))]

use guatiao::library::kind::{ProviderDecl, ProviderParts};
use guatiao::library::{Host, ProviderError, Registry};
use guatiao::{Alloc, FromValue, Map, Provider, Schema};

#[guatiao::kind]
pub trait Greeter: Send + Sync {
    fn greet(&self, name: &str) -> Result<String, ProviderError>;
}

#[guatiao::kind(name = "tally")]
pub trait Counter: Send + Sync {
    fn count(&self) -> i64;
}

/// Everything defaulted.
#[derive(Default, Provider)]
#[provider(Greeter, Counter)]
struct Hello;

impl Greeter for Hello {
    fn greet(&self, name: &str) -> Result<String, ProviderError> {
        Ok(format!("hello, {name}"))
    }
}

impl Counter for Hello {
    fn count(&self) -> i64 {
        1
    }
}

/// Everything overridden. The one instance is built by whichever test's
/// registry reaches it first, so every registry in this file is the
/// host `ready` looks for.
#[derive(Provider)]
#[provider(
    kinds(Greeter),
    id = "acme_loud",
    name = "Loud",
    version = "2.0.0",
    new_with_host = Loud::build,
    available = Loud::ready
)]
struct Loud {
    host: Host,
}

impl Loud {
    fn build(host: Host) -> Loud {
        Loud { host }
    }
    fn ready(&self) -> Result<(), &'static str> {
        if self.host.id() == "no-loud" {
            Err("this host does not want it loud")
        } else {
            Ok(())
        }
    }
}

impl Greeter for Loud {
    fn greet(&self, name: &str) -> Result<String, ProviderError> {
        Ok(format!("HELLO, {}!", name.to_uppercase()))
    }
}

/// What a shouter is configured with.
#[derive(Schema, FromValue)]
struct ShoutConfig {
    /// What every shout starts with.
    prefix: String,
    /// How many exclamation marks.
    bangs: u8,
}

/// Built from a configuration; no default instance.
#[derive(Provider)]
#[provider(Greeter, config = ShoutConfig)]
struct Shouter {
    prefix: String,
    bangs: usize,
}

static SHOUTERS_ALIVE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

impl TryFrom<ShoutConfig> for Shouter {
    type Error = ProviderError;
    fn try_from(config: ShoutConfig) -> Result<Shouter, ProviderError> {
        if config.bangs == 0 {
            return Err(ProviderError::new(
                guatiao::Status::GUATIAO_ERR_BAD_VALUE,
                "a shout needs at least one bang",
            ));
        }
        SHOUTERS_ALIVE.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(Shouter {
            prefix: config.prefix,
            bangs: usize::from(config.bangs),
        })
    }
}

impl Drop for Shouter {
    fn drop(&mut self) {
        SHOUTERS_ALIVE.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Greeter for Shouter {
    fn greet(&self, name: &str) -> Result<String, ProviderError> {
        Ok(format!(
            "{} {}{}",
            self.prefix,
            name.to_uppercase(),
            "!".repeat(self.bangs)
        ))
    }
}

guatiao::providers!(Hello, Loud, Shouter);

/// An instance is built from a configuration, and its address is the
/// context every call on it takes.
#[test]
fn a_provider_builds_instances_from_a_configuration() {
    use guatiao::Value;
    let mut registry = Registry::new("no-loud", "1.0");
    let parts: &'static ProviderParts = Box::leak(Box::new(
        Shouter::provider(registry.host(), Alloc::rust()).expect("builds"),
    ));
    let view = parts.info().view().unwrap();
    assert!(
        view.ctx.is_null(),
        "no default instance: built only from a configuration"
    );
    assert!(view.create.is_some() && view.destroy.is_some());
    assert!(
        view.config.is_some(),
        "the schema is the configuration's, so a host can ask before building"
    );

    let mut config = Map::new();
    config.set("prefix", "hey").unwrap();
    config.set("bangs", 2).unwrap();
    let config = Value::from(config);
    let before = SHOUTERS_ALIVE.load(std::sync::atomic::Ordering::SeqCst);
    let one = parts
        .info()
        .instantiate::<dyn Greeter>(&config)
        .expect("a fitting configuration builds");
    assert_eq!(one.greet("ana").unwrap(), "hey ANA!!");
    let mut other = Map::new();
    other.set("prefix", "yo").unwrap();
    other.set("bangs", 1).unwrap();
    let two = parts
        .info()
        .instantiate::<dyn Greeter>(&Value::from(other))
        .unwrap();
    assert_eq!(two.greet("bo").unwrap(), "yo BO!");
    assert_eq!(
        SHOUTERS_ALIVE.load(std::sync::atomic::Ordering::SeqCst),
        before + 2,
        "two instances, two contexts"
    );
    drop(one);
    drop(two);
    assert_eq!(
        SHOUTERS_ALIVE.load(std::sync::atomic::Ordering::SeqCst),
        before,
        "dropping an instance runs the provider's destroy"
    );

    // A configuration the type refuses, and one the schema refuses.
    let mut bad = Map::new();
    bad.set("prefix", "x").unwrap();
    bad.set("bangs", 0).unwrap();
    let e = parts
        .info()
        .instantiate::<dyn Greeter>(&Value::from(bad))
        .unwrap_err();
    assert_eq!(e.message(), "a shout needs at least one bang");
    let e = parts
        .info()
        .instantiate::<dyn Greeter>(&Value::from(Map::new()))
        .unwrap_err();
    assert_eq!(e.status, guatiao::Status::GUATIAO_ERR_BAD_VALUE, "{e}");

    // A provider that is its one instance builds none.
    let hello: &'static ProviderParts = Box::leak(Box::new(
        Hello::provider(registry.host(), Alloc::rust()).unwrap(),
    ));
    let e = hello
        .info()
        .instantiate::<dyn Greeter>(&config)
        .unwrap_err();
    assert_eq!(e.status, guatiao::Status::GUATIAO_ERR_NULL);
}

#[test]
fn a_derived_provider_describes_itself_with_the_defaults() {
    let mut registry = Registry::new("no-loud", "1.0");
    let parts: &'static ProviderParts = Box::leak(Box::new(
        Hello::provider(registry.host(), Alloc::rust()).expect("builds"),
    ));
    let view = parts.info().view().expect("a readable descriptor");
    assert_eq!(
        view.id, "guatiao_hello",
        "{{package}}_{{type}}, with `-` as `_`"
    );
    assert_eq!(view.display_name, "Hello");
    assert_eq!(view.version, None, "empty means the library's");
    assert_eq!(view.kinds, ["greeter", "tally"]);
    assert!(view.config.is_none());
    assert_eq!(view.available(), Ok(()));
    let sizes: Vec<(&str, usize)> = view
        .tables
        .iter()
        .map(|(k, _, s)| (k.as_str(), *s))
        .collect();
    assert_eq!(
        sizes,
        [
            ("greeter", size_of::<GreeterVtable>()),
            ("tally", size_of::<CounterVtable>())
        ]
    );
    assert!(
        view.vtable.is_null(),
        "a derived provider has per-kind tables only"
    );

    // And the tables call through.
    let greeter = parts
        .info()
        .as_kind::<dyn Greeter>()
        .expect("a valid table");
    assert_eq!(greeter.greet("ana").unwrap(), "hello, ana");
    let counter = parts
        .info()
        .as_kind::<dyn Counter>()
        .expect("a valid table");
    assert_eq!(counter.count(), 1);
}

#[test]
fn a_derived_provider_takes_every_override() {
    let mut registry = Registry::new("no-loud", "1.0");
    let parts: &'static ProviderParts = Box::leak(Box::new(
        Loud::provider(registry.host(), Alloc::rust()).expect("builds"),
    ));
    let view = parts.info().view().unwrap();
    assert_eq!(view.id, "acme_loud");
    assert_eq!(view.display_name, "Loud");
    assert_eq!(view.version.as_deref(), Some("2.0.0"));
    assert_eq!(view.kinds, ["greeter"]);
    assert!(
        view.config.is_none(),
        "no configuration: its one instance is built at load"
    );
    assert_eq!(
        view.available(),
        Err("this host does not want it loud"),
        "`available` saw the host `new_with_host` was given"
    );
    let greeter = parts.info().as_kind::<dyn Greeter>().unwrap();
    assert_eq!(greeter.greet("bo").unwrap(), "HELLO, BO!");
}

#[test]
fn a_library_line_names_the_whole_library() {
    let mut registry = Registry::new("no-loud", "1.0");
    let info = __guatiao_describe(registry.host()).expect("the library describes itself");
    assert_eq!(
        unsafe_str(info.id),
        env!("CARGO_PKG_NAME"),
        "the id is this crate's own name"
    );
    assert_eq!(unsafe_str(info.version), env!("CARGO_PKG_VERSION"));
    assert_eq!(info.providers.len, 3);
    assert_eq!(
        info.providers.stride,
        size_of::<guatiao::library::ProviderInfo>()
    );
    assert_eq!(info.abi_version, guatiao::library::ABI_VERSION);
    // Built once: the same descriptor comes back.
    let again = __guatiao_describe(registry.host()).unwrap();
    assert!(std::ptr::eq(info, again));
}

/// A `Str` a descriptor names, checked as a host reading it would.
fn unsafe_str(s: guatiao::Str<'static>) -> &'static str {
    std::str::from_utf8(s.into()).unwrap()
}
