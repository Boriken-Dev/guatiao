// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `#[derive(Provider)]` and `guatiao::providers!`, in one process: the
//! descriptor a derived type builds, and the library a line names.

#![cfg(all(feature = "provider", feature = "load"))]

use guatiao::library::kind::{ProviderDecl, ProviderParts};
use guatiao::library::{Host, ProviderError, Registry};
use guatiao::{Alloc, Provider, Schema};

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

/// What the configured provider takes.
#[derive(Schema)]
#[allow(dead_code)]
struct Tuning {
    /// How loud.
    volume: u8,
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
    config = Tuning,
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

guatiao::providers!(Hello, Loud);

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
    let sizes: Vec<(&str, usize)> = view.tables.iter().map(|&(k, _, s)| (k, s)).collect();
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
    assert_eq!(view.version, Some("2.0.0"));
    assert_eq!(view.kinds, ["greeter"]);
    let schema = view.config.expect("declares a configuration");
    assert!(
        schema
            .get("properties")
            .and_then(|p| p.get("volume"))
            .is_some(),
        "the schema is Tuning's"
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
    assert_eq!(info.providers.len, 2);
    assert_eq!(
        info.providers.stride,
        size_of::<guatiao::library::ProviderInfo>()
    );
    assert_eq!(info.abi_version, guatiao::library::ABI_VERSION);
    // Built once: the same descriptor comes back.
    let again = __guatiao_describe(registry.host()).unwrap();
    assert!(std::ptr::eq(info, again));
}

/// A `Str` a descriptor names, read as the test's own convenience.
fn unsafe_str(s: guatiao::Str) -> &'static str {
    if s.len == 0 {
        return "";
    }
    // SAFETY: the descriptor's text is a `'static` literal or a box the
    // library parts keep for the process.
    std::str::from_utf8(unsafe { std::slice::from_raw_parts(s.ptr, s.len) }).unwrap()
}
