// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Owning a tree without copying one that is already this binary's:
//! `is_local`, `into_local`, `into_alloc` and `FromValue for Value`.
//!
//! Two counts: this thread's heap allocations, which is where a local
//! tree lives, and a counting arena standing for an allocator that is not
//! this binary's (a host's, a library's, a caller's).

extern crate alloc;

use alloc::collections::BTreeMap;
use core::cell::Cell;
use core::ffi::c_void;

use guatiao::value::alloc::{Alloc, AllocError, Allocator, rust_alloc};
use guatiao::{Buffer, FromValue, List, Map, Text, TryAsRef, Value, ValueError};

// --- this thread's heap ----------------------------------------------------

/// Counts this thread's heap allocations; not installed under Miri, where
/// `System` reads a block header outside the `Box` the harness frees.
#[cfg_attr(miri, allow(dead_code))]
struct PerThread;

std::thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

// SAFETY: every call is forwarded to the system allocator unchanged.
unsafe impl core::alloc::GlobalAlloc for PerThread {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|n| n.set(n.get() + 1));
        // SAFETY: forwarded.
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        // SAFETY: forwarded.
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
}

#[cfg(not(miri))]
#[global_allocator]
static HEAP: PerThread = PerThread;

/// `f`'s answer and how many heap allocations it made on this thread;
/// `None` under Miri, where nothing counts them.
fn allocations<R>(f: impl FnOnce() -> R) -> (R, Option<usize>) {
    let before = ALLOCATIONS.with(Cell::get);
    let r = f();
    let made = ALLOCATIONS.with(Cell::get) - before;
    (r, (!cfg!(miri)).then_some(made))
}

// --- an allocator that is not this binary's --------------------------------

#[derive(Default)]
struct Arena {
    allocs: Cell<usize>,
    outstanding: Cell<isize>,
    /// Refuse every allocation from the Nth on, counting from 1; 0 never.
    fail_at: Cell<usize>,
}

unsafe extern "C" fn arena_alloc(ctx: *mut c_void, size: usize, align: usize) -> *mut c_void {
    // SAFETY: `ctx` is the `&Arena` the vtable was built over, which
    // outlives every call made through it.
    let a = unsafe { &*(ctx as *const Arena) };
    a.allocs.set(a.allocs.get() + 1);
    if a.fail_at.get() != 0 && a.allocs.get() >= a.fail_at.get() {
        return core::ptr::null_mut();
    }
    a.outstanding.set(a.outstanding.get() + 1);
    let real = rust_alloc().alloc.expect("the rust allocator has an alloc");
    // SAFETY: forwarding an unchanged request.
    unsafe { real(core::ptr::null_mut(), size, align) }
}

unsafe extern "C" fn arena_free(ctx: *mut c_void, p: *mut c_void, size: usize, align: usize) {
    // SAFETY: as above.
    let a = unsafe { &*(ctx as *const Arena) };
    a.outstanding.set(a.outstanding.get() - 1);
    let real = rust_alloc().free.expect("the rust allocator has a free");
    // SAFETY: forwarding the same block with the same layout.
    unsafe { real(core::ptr::null_mut(), p, size, align) }
}

/// Runs `body` with an arena and checks it holds nothing afterwards.
fn with_arena(body: impl FnOnce(Alloc, &Arena)) {
    let arena = Arena::default();
    let vtable = Allocator {
        struct_size: size_of::<Allocator>() as u32,
        ctx: (&arena as *const Arena).cast_mut().cast(),
        alloc: Some(arena_alloc),
        free: Some(arena_free),
        release: None,
    };
    // SAFETY: a complete vtable that outlives the closure.
    let alloc = unsafe { Alloc::from_raw(&vtable) }.expect("a complete vtable");
    body(alloc, &arena);
    assert_eq!(arena.outstanding.get(), 0, "the arena holds nothing after");
}

/// `{"name": "x", "tags": ["a", b"z"], "inner": {"n": 1}}`, every
/// container and key built through `alloc`.
fn tree_in(alloc: Alloc) -> Map {
    let mut tags = List::new_in(alloc);
    tags.push_in(Text::new_in(alloc, "a").unwrap(), alloc)
        .unwrap();
    tags.push_in(Buffer::new_in(alloc, b"z").unwrap(), alloc)
        .unwrap();
    let mut inner = Map::new_in(alloc);
    inner
        .set_in("n", guatiao::Number::new_in(alloc, "1").unwrap(), alloc)
        .unwrap();
    let mut map = Map::new_in(alloc);
    map.set_in("name", Text::new_in(alloc, "x").unwrap(), alloc)
        .unwrap();
    map.set_in("tags", tags, alloc).unwrap();
    map.set_in("inner", inner, alloc).unwrap();
    map
}

fn first_entry(map: &Map) -> *const guatiao::Entry {
    map.entries().as_ptr()
}

// --- already local: handed back -------------------------------------------

#[test]
fn a_local_tree_is_handed_back_without_allocating() {
    let map = tree_in(Alloc::rust());
    assert!(map.is_local());
    let at = first_entry(&map);
    let (map, made) = allocations(|| map.into_local());
    assert_eq!(first_entry(&map), at, "the same storage, not a copy");
    assert!(made.is_none_or(|n| n == 0), "{made:?} allocations");

    let value = Value::from(map);
    assert!(value.is_local());
    let (value, made) = allocations(|| value.into_local());
    assert!(made.is_none_or(|n| n == 0), "{made:?} allocations");
    let map: &Map = value.try_as_ref().unwrap();
    assert_eq!(first_entry(map), at);

    let (again, made) = allocations(|| value.into_alloc(Alloc::rust()));
    assert!(made.is_none_or(|n| n == 0), "{made:?} allocations");
    let map: &Map = again.as_ref().unwrap().try_as_ref().unwrap();
    assert_eq!(first_entry(map), at);

    let text = Text::new("kept");
    let at = text.as_ptr();
    let (text, made) = allocations(|| text.into_local());
    assert_eq!((text.as_ptr(), made.unwrap_or(0)), (at, 0));
    let buffer = Buffer::new(b"kept");
    let at = buffer.as_ptr();
    let (buffer, made) = allocations(|| buffer.into_local());
    assert_eq!((buffer.as_ptr(), made.unwrap_or(0)), (at, 0));
    let list: List = ["a", "b"].into_iter().collect();
    let at = list.as_ptr();
    let (list, made) = allocations(|| list.into_local());
    assert_eq!((list.as_ptr(), made.unwrap_or(0)), (at, 0));
}

#[test]
fn a_value_that_owns_nothing_is_local() {
    for v in [Value::null(), Value::absent(), Value::from(true)] {
        assert!(v.is_local());
    }
    assert!(Value::from(5900i64).is_local(), "a number built here");
    assert!(Map::new().is_local() && List::new().is_local());
}

// --- elsewhere: copied, the original freed through its own ------------------

#[test]
fn a_tree_in_another_allocator_is_copied_and_the_original_freed() {
    with_arena(|arena, counter| {
        let map = tree_in(arena);
        let expected = Value::from(map.clone());
        assert!(!map.is_local());
        assert!(counter.outstanding.get() > 0);

        let local = map.into_local();
        assert_eq!(counter.outstanding.get(), 0, "the original went back");
        assert!(local.is_local());
        assert_eq!(Value::from(local), expected);

        let value = Value::from(tree_in(arena));
        let local = value.into_local();
        assert_eq!(counter.outstanding.get(), 0);
        assert!(local.is_local());
        assert_eq!(local, expected);

        // And the other way: a local tree into the arena.
        let blocks = counter.allocs.get();
        let homed = local.into_alloc(arena).unwrap();
        assert!(counter.allocs.get() > blocks, "copied into the arena");
        assert!(!homed.is_local());
        assert_eq!(homed.alloc().unwrap().as_raw(), arena.as_raw());
        let blocks = counter.allocs.get();
        let homed = homed.into_alloc(arena).unwrap();
        assert_eq!(counter.allocs.get(), blocks, "already there: kept");
        assert_eq!(homed, expected);
    });
}

#[test]
fn a_mixed_tree_is_copied_whole() {
    with_arena(|arena, counter| {
        let mut map = Map::new();
        map.set("local", "yes");
        map.set_in("there", Text::new_in(arena, "no").unwrap(), arena)
            .unwrap();
        assert!(!map.is_local(), "one string elsewhere is enough");
        let local = map.into_local();
        assert!(local.is_local());
        assert_eq!(counter.outstanding.get(), 0);

        // A key alone in the arena counts too.
        let mut map = Map::new_in(arena);
        map.set_in("k", true, arena).unwrap();
        let mut outer = Map::new();
        outer.set("inner", map);
        assert!(!outer.is_local());
        assert!(outer.into_local().is_local());
    });
}

/// Past the depth judged on the thread's stack, the answer is the same.
#[test]
fn a_deep_tree_is_judged_whole() {
    with_arena(|arena, _| {
        let nest = |leaf: Value| {
            (0..40).fold(leaf, |inner, _| {
                let mut list = List::new();
                list.push(inner);
                Value::from(list)
            })
        };
        assert!(nest(Value::from("here")).is_local());
        let deep = nest(Value::from(Text::new_in(arena, "there").unwrap()));
        assert!(!deep.is_local(), "the one foreign leaf is found");
        assert!(deep.into_local().is_local());
    });
}

#[test]
fn a_literal_holding_no_allocator_is_copied() {
    static BYTES: [u8; 3] = *b"lit";
    // SAFETY: three UTF-8 bytes that live for the program, not owned.
    let lit = unsafe { Text::from_raw_parts(BYTES.as_ptr().cast_mut(), 3, 0, core::ptr::null()) };
    assert!(!lit.is_local());
    let local = lit.into_local();
    assert!(local.is_local());
    assert_eq!(&*local, "lit");
    assert_ne!(local.as_ptr(), BYTES.as_ptr());
}

// --- refusals ---------------------------------------------------------------

#[test]
fn a_refusing_allocator_is_reported_and_the_original_freed() {
    with_arena(|arena, counter| {
        counter.fail_at.set(counter.allocs.get() + 2);
        let err = tree_in(Alloc::rust()).into_alloc(arena).unwrap_err();
        assert_eq!(err, ValueError::Alloc(AllocError::Failed));
        assert_eq!(counter.outstanding.get(), 0, "the partial copy went back");
    });
}

#[test]
fn content_this_build_refuses_is_an_error_or_a_panic() {
    with_arena(|arena, _| {
        // A STRING whose bytes are not UTF-8, as only a foreign producer
        // can write one.
        let bad = || {
            let (ptr, len, cap, a) = Buffer::new_in(arena, b"caf\xe9").unwrap().into_raw_parts();
            // SAFETY: a STRING tag over a text arm whose storage is
            // consistent; its bytes are what is under test.
            unsafe {
                Value::from_raw_parts(
                    u32::from(guatiao::Tag::GUATIAO_STRING),
                    guatiao::Payload::text(Text::from_raw_parts(ptr, len, cap, a)),
                )
            }
        };
        assert!(!bad().is_local());
        let err = bad().into_alloc(Alloc::rust()).unwrap_err();
        assert_eq!(err, ValueError::NotUtf8);
        assert!(Value::from_value(&bad()).is_err());
        let node = bad();
        let caught = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| node.into_local()));
        assert!(caught.is_err(), "into_local panics, as clone does");
    });
}

// --- FromValue for Value -----------------------------------------------------

#[test]
fn reading_a_value_copies_it_into_this_binarys_allocator() {
    with_arena(|arena, _| {
        let tree = Value::from(tree_in(arena));
        let copy = Value::from_value(&tree).unwrap();
        assert!(copy.is_local());
        assert_eq!(copy, tree);

        assert_eq!(Option::<Value>::from_value(&Value::null()).unwrap(), None);
        let read = Option::<Value>::from_value(&tree).unwrap().unwrap();
        assert!(read.is_local());

        let by_key = BTreeMap::<String, Value>::from_value(&tree).unwrap();
        assert_eq!(by_key.len(), 3);
        assert!(by_key.values().all(Value::is_local));
        let list: &Map = tree.try_as_ref().unwrap();
        let tags = Vec::<Value>::from_value(list.get("tags").unwrap()).unwrap();
        assert_eq!(tags.len(), 2);
    });
}
