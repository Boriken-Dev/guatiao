// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a call through a kind returns lives in the host's allocator: a
//! tree the provider built through its own is copied at the proxy and
//! the original freed, so the host holds nothing of the provider's once
//! the call returns. Counted through the provider's allocator.

#![cfg(all(feature = "provider", feature = "load"))]

use core::ffi::c_void;
use core::sync::atomic::{AtomicIsize, Ordering};

use guatiao::library::kind::{LibraryParts, ProviderParts};
use guatiao::library::{KindTable, ProviderError, Registry};
use guatiao::value::alloc::{Alloc, Allocator, rust_alloc};
use guatiao::value::convert::TryAsRef;
use guatiao::{List, Map, Status, Text, Value};

/// Blocks the provider's allocator has handed out and not taken back.
static LIVE: AtomicIsize = AtomicIsize::new(0);

unsafe extern "C" fn counted_alloc(_ctx: *mut c_void, size: usize, align: usize) -> *mut c_void {
    LIVE.fetch_add(1, Ordering::SeqCst);
    let real = rust_alloc().alloc.expect("the rust allocator has an alloc");
    // SAFETY: forwarding an unchanged request.
    unsafe { real(core::ptr::null_mut(), size, align) }
}

unsafe extern "C" fn counted_free(_ctx: *mut c_void, p: *mut c_void, size: usize, align: usize) {
    LIVE.fetch_sub(1, Ordering::SeqCst);
    let real = rust_alloc().free.expect("the rust allocator has a free");
    // SAFETY: the same block with the layout it was allocated with.
    unsafe { real(core::ptr::null_mut(), p, size, align) }
}

/// An `Allocator` holds a `*mut c_void`, so it is not `Sync` and cannot
/// be a `static` without saying why.
struct Counting(Allocator);
// SAFETY: never written; its `ctx` is null and both functions are
// thread-safe.
unsafe impl Sync for Counting {}

static PROVIDERS: Counting = Counting(Allocator {
    struct_size: size_of::<Allocator>() as u32,
    ctx: core::ptr::null_mut(),
    alloc: Some(counted_alloc),
    free: Some(counted_free),
    release: None,
});

fn providers() -> Alloc {
    // SAFETY: a complete constant vtable that lives for the process.
    unsafe { Alloc::from_raw(&PROVIDERS.0) }.expect("the vtable is complete")
}

/// Every tree a call can return, each built through the provider's own
/// allocator.
#[guatiao::kind]
pub trait Trees: Send + Sync {
    fn value(&self) -> Value;
    fn map(&self) -> Result<Map, ProviderError>;
    fn list(&self) -> List;
    fn refuses(&self) -> Result<(), ProviderError>;
}

struct Built;

impl Trees for Built {
    fn value(&self) -> Value {
        let mut inner = Map::new_in(providers());
        inner
            .set_in(
                "name",
                Text::new_in(providers(), "ana").unwrap(),
                providers(),
            )
            .unwrap();
        Value::from(inner)
    }

    fn map(&self) -> Result<Map, ProviderError> {
        let mut map = Map::new_in(providers());
        let mut nested = List::new_in(providers());
        nested
            .push_in(Text::new_in(providers(), "x").unwrap(), providers())
            .unwrap();
        map.set_in("nested", nested, providers()).unwrap();
        Ok(map)
    }

    fn list(&self) -> List {
        let mut list = List::new_in(providers());
        list.push_in(Text::new_in(providers(), "one").unwrap(), providers())
            .unwrap();
        list
    }

    fn refuses(&self) -> Result<(), ProviderError> {
        Err(ProviderError {
            status: Status::GUATIAO_ERR_BAD_VALUE,
            message: Text::new_in(providers(), "not today").unwrap(),
        })
    }
}

static BUILT: Built = Built;
static TABLE: TreesVtable = TreesVtable::of::<Built>();

fn library(_host: guatiao::library::Host) -> Option<LibraryParts> {
    let provider = ProviderParts::builder("trees_built")
        .kinds(&["trees"])
        .table(KindTable::new(
            "trees",
            &TABLE as *const TreesVtable as *const c_void,
            size_of::<TreesVtable>(),
        ))
        .ctx(&BUILT as *const Built as *mut c_void)
        .build();
    Some(LibraryParts::new("trees", "1.0.0", vec![provider]))
}

/// The host's own allocator: what every answer records.
fn is_the_hosts(found: Result<Alloc, guatiao::ValueError>) -> bool {
    found.is_ok_and(|a| a.as_raw() == Alloc::rust().as_raw())
}

#[test]
fn every_tree_a_call_returns_lives_in_the_hosts_allocator() {
    let mut registry = Registry::new("trees-tests", "1.0");
    registry
        .register_local("trees", library)
        .expect("it registers")
        .loaded()
        .expect("it accepts this host");
    let offer = registry
        .offer::<dyn Trees>("trees_built")
        .expect("filed under its id")
        .expect("a valid table");
    let trees = offer.default_instance().expect("its own instance");
    let before = LIVE.load(Ordering::SeqCst);

    let value = trees.value();
    assert_eq!(
        LIVE.load(Ordering::SeqCst),
        before,
        "none of the provider's blocks is alive once the call returns"
    );
    assert!(is_the_hosts(value.alloc()));
    let map: &Map = TryAsRef::try_as_ref(&value).expect("a map");
    assert!(is_the_hosts(map.get("name").expect("its member").alloc()));

    let map = trees.map().expect("a map");
    assert_eq!(LIVE.load(Ordering::SeqCst), before);
    assert!(is_the_hosts(map.alloc()));
    let nested: &List = map
        .get("nested")
        .and_then(TryAsRef::try_as_ref)
        .expect("the nested list");
    assert!(is_the_hosts(nested.alloc()));
    assert_eq!(
        nested
            .first()
            .and_then(TryAsRef::<str>::try_as_ref)
            .expect("its one member"),
        "x"
    );

    let list = trees.list();
    assert_eq!(LIVE.load(Ordering::SeqCst), before);
    assert!(is_the_hosts(list.alloc()));

    let refused = trees.refuses().expect_err("it refuses");
    assert_eq!(LIVE.load(Ordering::SeqCst), before);
    assert_eq!(refused.status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(refused.message(), "not today");
    assert!(is_the_hosts(refused.message.alloc()));
}
