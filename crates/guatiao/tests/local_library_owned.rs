// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A library the host links can be handed to the registry as owned parts:
//! kept while it is registered or a handle into it lives, freed after.
//! Counted through an allocator the parts' schema and meta are built in.

#![cfg(all(feature = "provider", feature = "load"))]
extern crate alloc;

use core::ffi::c_void;
use core::sync::atomic::{AtomicIsize, Ordering};

use greeter_kind::Counter;
use guatiao::library::Registry;
use guatiao::library::kind::{LibraryParts, ProviderParts};
use guatiao::value::alloc::{Alloc, Allocator, rust_alloc};
use guatiao::{Map, Value};

/// Live blocks, one counter per allocator so parallel tests do not share.
struct Count(AtomicIsize);

unsafe extern "C" fn counted_alloc(ctx: *mut c_void, size: usize, align: usize) -> *mut c_void {
    // SAFETY: `ctx` is the `Count` of a `static` allocator below.
    unsafe { &*ctx.cast::<Count>() }
        .0
        .fetch_add(1, Ordering::SeqCst);
    let real = rust_alloc().alloc.expect("the rust allocator has an alloc");
    // SAFETY: forwarding an unchanged request.
    unsafe { real(core::ptr::null_mut(), size, align) }
}

unsafe extern "C" fn counted_free(ctx: *mut c_void, p: *mut c_void, size: usize, align: usize) {
    // SAFETY: as above.
    unsafe { &*ctx.cast::<Count>() }
        .0
        .fetch_sub(1, Ordering::SeqCst);
    let real = rust_alloc().free.expect("the rust allocator has a free");
    // SAFETY: the same block with the layout it was allocated with.
    unsafe { real(core::ptr::null_mut(), p, size, align) }
}

/// An `Allocator` holds a `*mut c_void`, so it is not `Sync` and cannot
/// be a `static` without saying why.
struct Counting(Allocator);
// SAFETY: never written; `ctx` addresses a `Count`, which is atomic, and
// both functions are thread-safe.
unsafe impl Sync for Counting {}

static HAND_COUNT: Count = Count(AtomicIsize::new(0));
static HAND: Counting = Counting(Allocator {
    struct_size: size_of::<Allocator>() as u32,
    ctx: &HAND_COUNT as *const Count as *mut c_void,
    alloc: Some(counted_alloc),
    free: Some(counted_free),
    release: None,
});

static DERIVED_COUNT: Count = Count(AtomicIsize::new(0));
static DERIVED: Counting = Counting(Allocator {
    struct_size: size_of::<Allocator>() as u32,
    ctx: &DERIVED_COUNT as *const Count as *mut c_void,
    alloc: Some(counted_alloc),
    free: Some(counted_free),
    release: None,
});

fn alloc_of(counting: &'static Counting) -> Alloc {
    // SAFETY: a complete constant vtable that lives for the process.
    unsafe { Alloc::from_raw(&counting.0) }.expect("the vtable is complete")
}

/// Parts built by hand, handed over by value, freed at the retire.
#[test]
fn an_owned_librarys_parts_are_freed_when_it_is_retired() {
    let alloc = alloc_of(&HAND);
    let before = HAND_COUNT.0.load(Ordering::SeqCst);

    let mut schema = Map::new_in(alloc);
    schema.try_set("type", "object").expect("allocates");
    let mut meta = Map::new_in(alloc);
    meta.try_set("vendor", "acme").expect("allocates");
    let provider = ProviderParts::builder("owned_provider")
        .name("Owned")
        .kinds(&["owned"])
        .config(Value::from(schema))
        .meta(meta)
        .build();
    let parts = LibraryParts::new("owned", "1.0.0", vec![provider]);
    let built = HAND_COUNT.0.load(Ordering::SeqCst);
    assert!(
        built > before,
        "the parts hold blocks of the counted allocator"
    );

    let mut registry = Registry::new("owned-tests", "1.0");
    registry
        .register_local("owned", move |_host| Some(parts))
        .expect("it registers")
        .loaded()
        .expect("it accepts this host");
    let provider = registry
        .provider("owned_provider")
        .expect("filed under its id");
    assert!(provider.config_schema().is_some());
    assert_eq!(provider.meta().map(Map::len), Some(1));
    assert_eq!(
        HAND_COUNT.0.load(Ordering::SeqCst),
        built,
        "the registry keeps the parts, and its own copies are on its heap"
    );
    let host = registry.host();
    let answered = host
        .get("owned_provider")
        .expect("the host offers services");
    assert_eq!(
        answered.map(|v| v.id().to_string()).as_deref(),
        Some("owned_provider")
    );

    let retired = registry.retire("owned").expect("it is registered");
    assert_eq!(retired.providers, 1);
    assert_eq!(
        HAND_COUNT.0.load(Ordering::SeqCst),
        before,
        "every block the parts held is freed at the retire"
    );
    assert!(matches!(host.get("owned_provider"), Ok(None)));
}

static PINNED_COUNT: Count = Count(AtomicIsize::new(0));
static PINNED: Counting = Counting(Allocator {
    struct_size: size_of::<Allocator>() as u32,
    ctx: &PINNED_COUNT as *const Count as *mut c_void,
    alloc: Some(counted_alloc),
    free: Some(counted_free),
    release: None,
});

/// A host's own library whose schema and meta are built in `alloc`.
fn parts_in(alloc: Alloc, id: &str, kind: &'static str) -> LibraryParts {
    let mut schema = Map::new_in(alloc);
    schema.try_set("type", "object").expect("allocates");
    let mut meta = Map::new_in(alloc);
    meta.try_set("vendor", "acme").expect("allocates");
    let provider = ProviderParts::builder(&format!("{id}_provider"))
        .kinds(&[kind])
        .config(Value::from(schema))
        .meta(meta)
        .build();
    LibraryParts::new(id, "1.0.0", vec![provider])
}

/// A library that asks its host for a provider holds a pin on it: retiring
/// the provider's library frees nothing while the answer lives.
#[test]
fn a_host_answer_holds_its_library_across_a_retire() {
    let before = PINNED_COUNT.0.load(Ordering::SeqCst);
    let mut registry = Registry::new("owned-tests", "1.0");
    registry
        .register_local("pinned", |_host| {
            Some(parts_in(alloc_of(&PINNED), "pinned", "pinned"))
        })
        .expect("it registers")
        .loaded()
        .expect("it accepts this host");
    let host = registry.host();
    let answer = host
        .get("pinned_provider")
        .expect("the host offers services")
        .expect("the provider is found");

    registry.retire("pinned").expect("it is registered");
    assert!(
        matches!(host.get("pinned_provider"), Ok(None)),
        "retired, it is no longer answered"
    );
    assert!(
        PINNED_COUNT.0.load(Ordering::SeqCst) > before,
        "the answer's pin holds the lease, and the lease the parts"
    );
    assert_eq!(answer.id(), "pinned_provider");
    assert!(answer.config().is_some() && answer.meta().is_some());
    let again = answer.clone();
    drop(answer);
    assert!(
        PINNED_COUNT.0.load(Ordering::SeqCst) > before,
        "a clone pins too"
    );
    drop(again);
    assert_eq!(
        PINNED_COUNT.0.load(Ordering::SeqCst),
        before,
        "the last pin let go of the parts"
    );
}

/// A library asking its host from another thread, while the host retires
/// and registers its provider's library over and over, reads only
/// descriptors that are alive: each answer is pinned before the services
/// let go of the snapshot it came from.
#[test]
fn a_lookup_racing_a_retire_reads_nothing_freed() {
    use alloc::sync::Arc;
    use core::sync::atomic::AtomicBool;

    let rounds = if cfg!(miri) { 8 } else { 2000 };
    let mut registry = Registry::new("owned-tests", "1.0");
    let host = registry.host();
    let stop = Arc::new(AtomicBool::new(false));
    let reader = {
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            let mut seen = 0usize;
            while !stop.load(Ordering::SeqCst) {
                if let Ok(Some(answer)) = host.get("racing_provider") {
                    assert_eq!(answer.id(), "racing_provider");
                    assert!(answer.config().is_some());
                    seen += 1;
                }
                for answer in host.list("racing").unwrap_or_default() {
                    assert_eq!(answer.kinds(), ["racing"]);
                }
                std::thread::yield_now();
            }
            seen
        })
    };
    for _ in 0..rounds {
        registry
            .register_local("racing", |_host| {
                Some(parts_in(Alloc::rust(), "racing", "racing"))
            })
            .expect("it registers")
            .loaded()
            .expect("it accepts this host");
        std::thread::yield_now();
        registry.retire("racing").expect("it is registered");
    }
    stop.store(true, Ordering::SeqCst);
    let seen = reader.join().expect("the reader read nothing freed");
    println!("the reader saw the provider {seen} times");
}

/// Built only from a configuration, whose schema the derive builds in the
/// host's arena.
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

/// Its own one instance, which a `Remote` taken from it keeps calling
/// after the retire: its lease keeps the library's parts and tables.
#[derive(Default, guatiao::Provider)]
#[provider(Counter)]
pub struct Plain;

impl Counter for Plain {
    fn count(&self) -> i64 {
        1
    }
}

guatiao::local_providers!(
    id = "derived_owned",
    version = "1.0.0",
    providers = [Configured, Plain]
);

/// `local_providers!` hands its parts over: they go with the last handle
/// once the library is retired, and a second registration builds afresh.
#[test]
fn local_providers_hands_its_parts_to_the_registry() {
    let before = DERIVED_COUNT.0.load(Ordering::SeqCst);
    let mut registry = Registry::with_alloc("owned-tests", "1.0", Some(alloc_of(&DERIVED)));
    registry
        .register_local("derived", library)
        .expect("it registers")
        .loaded()
        .expect("it accepts this host");
    assert!(
        DERIVED_COUNT.0.load(Ordering::SeqCst) > before,
        "the configuration schema is built in the host's arena"
    );
    let plain = registry
        .offer::<dyn Counter>("guatiao_plain")
        .expect("filed under its id")
        .expect("a valid table")
        .remote()
        .expect("its own instance");

    registry.retire("derived_owned").expect("it is registered");
    assert!(
        DERIVED_COUNT.0.load(Ordering::SeqCst) > before,
        "the handle holds the lease, and the lease the parts"
    );
    assert_eq!(plain.as_dyn().count(), 1, "a handle outlives the retire");
    drop(plain);
    assert_eq!(
        DERIVED_COUNT.0.load(Ordering::SeqCst),
        before,
        "the parts went with the last handle"
    );

    registry
        .register_local("derived", library)
        .expect("it registers again")
        .loaded()
        .expect("built afresh");
    drop(registry);
    assert_eq!(
        DERIVED_COUNT.0.load(Ordering::SeqCst),
        before,
        "dropping the registry frees what it owns"
    );
}
