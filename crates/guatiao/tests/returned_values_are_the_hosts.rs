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

// --- the caller's allocator, carried in ------------------------------------

/// Blocks an allocator has handed out: all of them, and those not yet
/// taken back. Each test below counts through its own.
struct Counter {
    made: AtomicIsize,
    live: AtomicIsize,
}

impl Counter {
    const fn new() -> Counter {
        Counter {
            made: AtomicIsize::new(0),
            live: AtomicIsize::new(0),
        }
    }
    fn made(&self) -> isize {
        self.made.load(Ordering::SeqCst)
    }
    fn live(&self) -> isize {
        self.live.load(Ordering::SeqCst)
    }
}

unsafe extern "C" fn counter_alloc(ctx: *mut c_void, size: usize, align: usize) -> *mut c_void {
    // SAFETY: every vtable below names a `Counter` that lives for the process.
    let counter = unsafe { &*ctx.cast::<Counter>() };
    counter.made.fetch_add(1, Ordering::SeqCst);
    counter.live.fetch_add(1, Ordering::SeqCst);
    let real = rust_alloc().alloc.expect("the rust allocator has an alloc");
    // SAFETY: forwarding an unchanged request.
    unsafe { real(core::ptr::null_mut(), size, align) }
}

unsafe extern "C" fn counter_free(ctx: *mut c_void, p: *mut c_void, size: usize, align: usize) {
    // SAFETY: as above.
    let counter = unsafe { &*ctx.cast::<Counter>() };
    counter.live.fetch_sub(1, Ordering::SeqCst);
    let real = rust_alloc().free.expect("the rust allocator has a free");
    // SAFETY: the same block with the layout it was allocated with.
    unsafe { real(core::ptr::null_mut(), p, size, align) }
}

const fn counting(counter: &'static Counter) -> Counting {
    Counting(Allocator {
        struct_size: size_of::<Allocator>() as u32,
        ctx: (counter as *const Counter).cast_mut().cast(),
        alloc: Some(counter_alloc),
        free: Some(counter_free),
        release: None,
    })
}

fn alloc_of(vtable: &'static Counting) -> Alloc {
    // SAFETY: a complete constant vtable that lives for the process.
    unsafe { Alloc::from_raw(&vtable.0) }.expect("the vtable is complete")
}

/// What the in-place provider would build through when nobody named an
/// allocator: its own.
static OWN_COUNT: Counter = Counter::new();
static OWN: Counting = counting(&OWN_COUNT);
/// An allocator of the host's that is not its default one: an arena.
static ARENA_COUNT: Counter = Counter::new();
static ARENA: Counting = counting(&ARENA_COUNT);

/// A provider that builds every tree where it is asked to.
struct InPlace;

impl Trees for InPlace {
    fn value(&self) -> Value {
        self.value_in(alloc_of(&OWN))
    }
    fn value_in(&self, alloc: Alloc) -> Value {
        let mut inner = Map::new_in(alloc);
        inner
            .set_in("name", Text::new_in(alloc, "ana").unwrap(), alloc)
            .unwrap();
        Value::from(inner)
    }
    fn map(&self) -> Result<Map, ProviderError> {
        self.map_in(alloc_of(&OWN))
    }
    fn map_in(&self, alloc: Alloc) -> Result<Map, ProviderError> {
        let mut map = Map::new_in(alloc);
        let mut nested = List::new_in(alloc);
        nested.push_in(Text::new_in(alloc, "x")?, alloc)?;
        map.set_in("nested", nested, alloc)?;
        Ok(map)
    }
    fn list(&self) -> List {
        self.list_in(alloc_of(&OWN))
    }
    fn list_in(&self, alloc: Alloc) -> List {
        let mut list = List::new_in(alloc);
        list.push_in(Text::new_in(alloc, "one").unwrap(), alloc)
            .unwrap();
        list
    }
    fn refuses(&self) -> Result<(), ProviderError> {
        Err(ProviderError::from(Status::GUATIAO_ERR_BAD_VALUE))
    }
}

static IN_PLACE: InPlace = InPlace;
static IN_PLACE_TABLE: TreesVtable = TreesVtable::of::<InPlace>();

fn in_place() -> guatiao::library::Remote<dyn Trees> {
    // SAFETY: a table this kind's attribute built for `InPlace`, over the
    // `&InPlace` its slots read, both static.
    unsafe {
        guatiao::library::Remote::<dyn Trees>::from_raw(
            (&IN_PLACE_TABLE as *const TreesVtable).cast(),
            size_of::<TreesVtable>(),
            (&IN_PLACE as *const InPlace).cast_mut().cast(),
        )
    }
    .expect("a valid table")
}

/// A tree built in the allocator the caller carried in crosses as it is:
/// the provider's own allocator makes nothing, and the caller's makes
/// exactly the blocks the provider built, so nothing was copied.
#[test]
fn a_tree_built_where_the_caller_asked_is_not_copied() {
    let trees = in_place();
    let (own, arena) = (OWN_COUNT.made(), ARENA_COUNT.made());

    let map = trees.map_in(alloc_of(&ARENA)).expect("a map");
    // The map's entries, the key `nested`, the list's items and `x`.
    assert_eq!(
        ARENA_COUNT.made() - arena,
        4,
        "the provider's blocks, and no copy"
    );
    assert_eq!(OWN_COUNT.made(), own, "nothing through the provider's own");
    assert_eq!(
        map.alloc().expect("recorded").as_raw(),
        alloc_of(&ARENA).as_raw()
    );
    drop(map);
    assert_eq!(ARENA_COUNT.live(), 0);

    // The plain method asks for this side's own allocator.
    let map = trees.map().expect("a map");
    assert_eq!(OWN_COUNT.made(), own);
    assert!(is_the_hosts(map.alloc()));
    assert!(is_the_hosts(trees.value().alloc()));
    assert!(is_the_hosts(trees.list().alloc()));
}

/// A table that ignores the carrier, as an older library's does: its `map` slot
/// writes a map from its own allocator over the out slot without reading
/// it. The caller copies that one, and frees the original.
#[test]
fn a_library_that_ignores_the_carrier_is_copied() {
    static OLD_COUNT: Counter = Counter::new();
    static OLD: Counting = counting(&OLD_COUNT);

    unsafe extern "C" fn old_map(
        _ctx: *mut c_void,
        out: *mut Map,
        _err: *mut ProviderError,
    ) -> Status {
        let alloc = alloc_of(&OLD);
        let mut map = Map::new_in(alloc);
        map.set_in("k", Text::new_in(alloc, "v").unwrap(), alloc)
            .unwrap();
        // SAFETY: the caller's writable out slot, holding nothing to free.
        unsafe { out.write(map) };
        Status::GUATIAO_OK
    }
    unsafe extern "C" fn old_value(_ctx: *mut c_void, out: *mut Value) -> Status {
        // SAFETY: as above.
        unsafe { out.write(Value::null()) };
        Status::GUATIAO_OK
    }
    unsafe extern "C" fn old_list(_ctx: *mut c_void, out: *mut List) -> Status {
        // SAFETY: as above.
        unsafe { out.write(List::new_in(alloc_of(&OLD))) };
        Status::GUATIAO_OK
    }
    unsafe extern "C" fn old_refuses(_ctx: *mut c_void, _err: *mut ProviderError) -> Status {
        Status::GUATIAO_ERR_BAD_VALUE
    }
    static OLD_TABLE: TreesVtable = TreesVtable {
        header: guatiao::library::KindHeader::new(
            size_of::<TreesVtable>(),
            TreesVtable::FLOOR_HASH,
        ),
        value: Some(old_value),
        map: Some(old_map),
        list: Some(old_list),
        refuses: Some(old_refuses),
    };

    // SAFETY: a table of this kind over a `ctx` none of its slots reads.
    let trees = unsafe {
        guatiao::library::Remote::<dyn Trees>::from_raw(
            (&OLD_TABLE as *const TreesVtable).cast(),
            size_of::<TreesVtable>(),
            core::ptr::null_mut(),
        )
    }
    .expect("a valid table");
    static INTO_COUNT: Counter = Counter::new();
    static INTO: Counting = counting(&INTO_COUNT);
    let map = trees.map_in(alloc_of(&INTO)).expect("a map");
    assert_eq!(
        map.alloc().expect("recorded").as_raw(),
        alloc_of(&INTO).as_raw()
    );
    assert_eq!(
        map.get("k").and_then(TryAsRef::<str>::try_as_ref),
        Some("v")
    );
    assert_eq!(OLD_COUNT.live(), 0, "the original is freed through its own");
    assert!(OLD_COUNT.made() > 0);
}

/// A caller that carries no allocator -- an older host, or a slot it
/// zeroed -- has the shim build through its own side's; one
/// that carries one gets its answer there.
#[test]
fn a_caller_that_carries_nothing_gets_the_providers_allocator() {
    let map_slot = IN_PLACE_TABLE.map.expect("filled");
    let ctx = (&IN_PLACE as *const InPlace).cast_mut().cast();
    static ASKED_COUNT: Counter = Counter::new();
    static ASKED: Counting = counting(&ASKED_COUNT);
    for carried in [None, Some(alloc_of(&ASKED))] {
        let mut out = match carried {
            None => {
                // SAFETY: an empty map over no storage.
                unsafe { Map::from_raw_parts(core::ptr::null_mut(), 0, 0, core::ptr::null()) }
            }
            Some(alloc) => Map::new_in(alloc),
        };
        let mut err = ProviderError::none();
        // SAFETY: the slot's own signature, a static `ctx`, writable slots.
        let status = unsafe { map_slot(ctx, &mut out, &mut err) };
        assert_eq!(status, Status::GUATIAO_OK);
        let want = carried.unwrap_or_else(Alloc::rust);
        assert_eq!(out.alloc().expect("recorded").as_raw(), want.as_raw());
    }

    let value_slot = IN_PLACE_TABLE.value.expect("filled");
    let mut out = Value::absent();
    // SAFETY: as above.
    assert_eq!(unsafe { value_slot(ctx, &mut out) }, Status::GUATIAO_OK);
    assert!(is_the_hosts(out.alloc()), "ABSENT carries nothing");
}
