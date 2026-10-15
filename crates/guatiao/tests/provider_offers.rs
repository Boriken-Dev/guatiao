// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! An offer is only what can be called: no default instance where there
//! is none, no object kind served by a provider, a configuration checked
//! before the provider is asked, and a panic in an instance's `Drop`
//! stopped at the boundary.

#![cfg(all(feature = "provider", feature = "load"))]

use core::ffi::c_void;

use greeter_kind::{Conversation, ConversationVtable, Counter};
use guatiao::library::kind::{LibraryParts, ProviderParts};
use guatiao::library::{
    Host, InstantiateError, KindMismatch, KindTable, LocalLibrary, ProviderError, Registry,
};
use guatiao::schema::ValidationError;
use guatiao::{Map, Value};

/// Built only from a configuration: no default instance.
#[derive(guatiao::Schema, guatiao::FromValue, guatiao::Provider)]
#[provider(Counter, config)]
pub struct Configured {
    start: i64,
}

impl Counter for Configured {
    fn count(&self) -> i64 {
        self.start
    }
}

/// Its own one instance.
#[derive(Default, guatiao::Provider)]
#[provider(Counter)]
pub struct Plain;

impl Counter for Plain {
    fn count(&self) -> i64 {
        1
    }
}

/// An instance whose `Drop` panics when it was built with `n == 1`.
#[derive(guatiao::Schema, guatiao::FromValue, guatiao::Provider)]
#[provider(Counter, config)]
pub struct Boom {
    n: i64,
}

impl Counter for Boom {
    fn count(&self) -> i64 {
        self.n
    }
}

impl Drop for Boom {
    fn drop(&mut self) {
        if self.n == 1 {
            panic!("a provider bug in Drop");
        }
    }
}

guatiao::local_providers!(
    id = "offers",
    version = "1.0.0",
    providers = [Configured, Plain, Boom]
);

fn registry() -> Registry {
    let mut registry = Registry::new("offers-tests", "1.0");
    registry
        .register_local("offers", library)
        .expect("the linked library registers")
        .loaded()
        .expect("it accepts this host");
    registry
}

fn config(key: &str, value: i64) -> Value {
    let mut map = Map::new();
    map.set(key, value).unwrap();
    map.into()
}

#[test]
fn a_provider_built_only_from_a_configuration_offers_no_default_instance() {
    let registry = registry();
    let configured = registry
        .offer::<dyn Counter>("guatiao_configured")
        .expect("filed under its id")
        .expect("a valid table");
    assert!(configured.builds_instances());
    assert!(
        configured.default_instance().is_none(),
        "its ctx is null: nothing to call until it is instantiated"
    );
    assert!(
        configured.remote().is_none(),
        "nor a proxy, whose infallible methods could only panic"
    );
    assert!(matches!(
        registry
            .provider("guatiao_configured")
            .expect("filed under its id")
            .as_kind::<dyn Counter>(),
        Ok(None)
    ));
    let built = configured
        .instantiate(&config("start", 5))
        .expect("a fitting configuration builds");
    assert_eq!(built.count(), 5);
    assert!(configured.boxed().is_none());

    let plain = registry
        .offer::<dyn Counter>("guatiao_plain")
        .expect("filed under its id")
        .expect("a valid table");
    assert_eq!(plain.default_instance().map(|c| c.count()), Some(1));
    assert_eq!(plain.remote().map(|r| r.as_dyn().count()), Some(1));
    let plain_kind = registry
        .provider("guatiao_plain")
        .expect("filed under its id")
        .as_kind::<dyn Counter>()
        .expect("a valid table")
        .expect("its own instance");
    assert_eq!(plain_kind.as_dyn().count(), 1);
    assert_eq!(plain.boxed().map(|c| c.count()), Some(1));
}

#[test]
fn a_configuration_is_checked_against_the_schema_before_the_provider_is_asked() {
    let registry = registry();
    let configured = registry
        .offer::<dyn Counter>("guatiao_configured")
        .unwrap()
        .unwrap();
    match configured.instantiate(&config("strat", 5)) {
        Err(InstantiateError::Invalid(ValidationError::UnknownOption { key, known })) => {
            assert_eq!(key, "strat");
            assert_eq!(known, ["start"], "the known keys, to suggest from");
        }
        other => panic!("a misspelled setting is refused by name: {other:?}"),
    }
    let mut wrong = Map::new();
    wrong.set("start", "five").unwrap();
    assert!(matches!(
        configured.instantiate(&Value::from(wrong)),
        Err(InstantiateError::Invalid(ValidationError::BadValue { .. }))
    ));
    let e = configured
        .instantiate(&Value::from(Map::new()))
        .unwrap_err();
    assert_eq!(e.status(), guatiao::Status::GUATIAO_ERR_BAD_VALUE, "{e}");
}

#[test]
fn a_panicking_instance_drop_stops_at_the_destroy_slot() {
    let registry = registry();
    let boom = registry
        .offer::<dyn Counter>("guatiao_boom")
        .unwrap()
        .unwrap();
    let instance = boom.instantiate(&config("n", 1)).expect("builds");
    assert_eq!(instance.count(), 1);
    // The derive's `destroy` slot is `extern "C"`: a panic out of it would
    // abort the whole test binary here.
    drop(instance);
    let calm = boom.instantiate(&config("n", 2)).expect("builds");
    assert_eq!(
        calm.count(),
        2,
        "the provider still answers after the panic"
    );
}

/// A conversation, to put behind a provider descriptor written by hand.
struct Talk {
    turns: i64,
}

impl Conversation for Talk {
    fn say(&mut self, _what: &str) -> Result<(), ProviderError> {
        self.turns += 1;
        Ok(())
    }
    fn read(&mut self, _dst: &mut [u8]) -> i64 {
        0
    }
    fn turns(&self) -> i64 {
        self.turns
    }
}

/// A hand-built library whose one provider claims an object kind, with a
/// real table for it: what a C library could declare.
fn object_library(_host: Host) -> Option<LocalLibrary> {
    let table: &'static ConversationVtable = Box::leak(Box::new(ConversationVtable::of::<Talk>()));
    let ctx: *mut c_void = Box::into_raw(Box::new(Talk { turns: 41 })).cast();
    // SAFETY: the table is `Conversation`'s, built for `Talk`, and `ctx` is
    // a leaked `Talk`; both live for the process. That the registry still
    // refuses to serve it is the property under test.
    let provider = unsafe {
        ProviderParts::builder("talker")
            .name("Talker")
            .kinds(&["conversation"])
            .table(KindTable::new(
                "conversation",
                (table as *const ConversationVtable).cast(),
                size_of::<ConversationVtable>(),
            ))
            .ctx(ctx)
            .build()
    };
    let parts: &'static LibraryParts = Box::leak(Box::new(LibraryParts::new(
        "objects",
        "1.0.0",
        vec![provider],
    )));
    Some(LocalLibrary::from(parts))
}

#[test]
fn an_object_kind_is_never_served_by_a_provider() {
    let mut registry = Registry::new("offers-tests", "1.0");
    registry
        .register_local("objects", object_library)
        .unwrap()
        .loaded()
        .expect("it registers");
    assert_eq!(registry.offers::<dyn Conversation>().count(), 0);
    let mismatches: Vec<_> = registry
        .mismatches::<dyn Conversation>()
        .map(|(p, why)| (p.id().to_string(), why))
        .collect();
    assert_eq!(
        mismatches,
        [("talker".to_string(), KindMismatch::ObjectKind)]
    );
    assert!(matches!(
        registry
            .provider("talker")
            .unwrap()
            .as_kind::<dyn Conversation>(),
        Err(KindMismatch::ObjectKind)
    ));
}

/// The long form's `unload = <fn>`: asked after the live counts, and
/// bound outside the macro's `unsafe`, so a plain function path is what it
/// takes.
mod unloading {
    use core::sync::atomic::{AtomicUsize, Ordering};

    use greeter_kind::Counter;

    static ASKED: AtomicUsize = AtomicUsize::new(0);

    /// The library's own say, counted.
    unsafe extern "C" fn refuse() -> guatiao::Status {
        ASKED.fetch_add(1, Ordering::SeqCst);
        guatiao::Status::GUATIAO_ERR_BUSY
    }

    #[derive(Default, guatiao::Provider)]
    #[provider(Counter)]
    pub struct Idle;

    impl Counter for Idle {
        fn count(&self) -> i64 {
            0
        }
    }

    guatiao::providers!(
        id = "unloading",
        version = "1.0.0",
        providers = [Idle],
        unload = refuse
    );

    #[test]
    fn the_long_forms_unload_is_asked_after_the_counts() {
        assert_eq!(__guatiao_unload(), guatiao::Status::GUATIAO_ERR_BUSY);
        assert_eq!(
            ASKED.load(Ordering::SeqCst),
            1,
            "nothing alive, so it was asked"
        );
    }
}
