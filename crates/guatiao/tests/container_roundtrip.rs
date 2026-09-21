// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Building, changing and freeing a C value tree, through the public API a
//! consumer actually has.
//!
//! Every test here runs against a **counting allocator**, and most of them
//! end by asserting that nothing is outstanding. That is the half a
//! functional test cannot see: a tree can read back perfectly while
//! leaking a block per growth, and the leak only shows up in a consumer's
//! long-running process.
//!
//! The counter lives in the allocator's own `ctx`, which is what that
//! field is for, and is also the shape a real instrumented allocator takes.
//!
//! **Nothing here frees a tree by hand unless the point is to.** A `Value`
//! owns what it holds and frees it in `Drop`, so a test's trees are gone by
//! the time `with_alloc` reads the counter — which is what makes that
//! reading an assertion about the library rather than about the test's own
//! bookkeeping. The explicit `free` calls below are each testing that
//! method, not cleaning up after the test.
//!
//! Every value is built with an `_in` constructor, or with a raw method
//! that is handed the allocator. That is not style: the short forms build on
//! Rust's own heap, which this allocator never sees, and the
//! outstanding-block assertions would then hold for a tree they had not
//! watched.

use guatiao::value::convert::TryAsMut;
use guatiao::value::convert::TryAsRef;
use std::cell::Cell;
use std::ffi::c_void;

use guatiao::value::alloc::{Alloc, AllocError, Allocator, rust_alloc};
use guatiao::value::error::{MAX_DEPTH, ValueError};
use guatiao::value::types::{Buffer, Entry, List, Map, Number, Payload, Tag, Text, Value};

// --- a counting allocator ---------------------------------------------

#[derive(Default)]
struct Counter {
    allocs: Cell<usize>,
    frees: Cell<usize>,
    outstanding: Cell<isize>,
    /// Fail the Nth allocation, counting from 1. 0 never fails.
    fail_at: Cell<usize>,
}

impl Counter {
    fn outstanding(&self) -> isize {
        self.outstanding.get()
    }
}

unsafe extern "C" fn c_alloc(ctx: *mut c_void, size: usize, align: usize) -> *mut c_void {
    // SAFETY: `ctx` is the `&Counter` installed below, which outlives every
    // call made through this vtable.
    let c = unsafe { &*(ctx as *const Counter) };
    c.allocs.set(c.allocs.get() + 1);
    if c.fail_at.get() != 0 && c.allocs.get() >= c.fail_at.get() {
        return std::ptr::null_mut();
    }
    c.outstanding.set(c.outstanding.get() + 1);
    let real = rust_alloc().alloc.expect("the rust allocator has an alloc");
    // SAFETY: forwarding an unchanged request.
    unsafe { real(std::ptr::null_mut(), size, align) }
}

unsafe extern "C" fn c_free(ctx: *mut c_void, p: *mut c_void, size: usize, align: usize) {
    // SAFETY: as above.
    let c = unsafe { &*(ctx as *const Counter) };
    c.frees.set(c.frees.get() + 1);
    c.outstanding.set(c.outstanding.get() - 1);
    let real = rust_alloc().free.expect("the rust allocator has a free");
    // SAFETY: forwarding the same block with the same layout.
    unsafe { real(std::ptr::null_mut(), p, size, align) }
}

fn vtable(c: &Counter) -> Allocator {
    Allocator {
        struct_size: size_of::<Allocator>() as u32,
        ctx: (c as *const Counter).cast_mut().cast(),
        alloc: Some(c_alloc),
        free: Some(c_free),
        release: None,
    }
}

/// Runs `body` with a counting allocator and asserts nothing leaked.
fn with_alloc(body: impl FnOnce(Alloc, &Counter)) {
    let counter = Counter::default();
    let vt = vtable(&counter);
    // SAFETY: `vt` is fully initialised and outlives the closure.
    let alloc = unsafe { Alloc::from_raw(&vt) }.expect("a complete vtable");
    body(alloc, &counter);
    assert_eq!(
        counter.outstanding(),
        0,
        "every block allocated during this test must have been freed"
    );
}

// --- building and reading ---------------------------------------------

#[test]
fn every_kind_survives_a_round_trip() {
    with_alloc(|alloc, _| {
        let mut m = Map::new_in(alloc);
        let root = &mut m;

        let null = Value::null();
        let b = Value::from(true);
        let n = Value::from(Number::new_in(alloc, "1.10").unwrap());
        let s = Text::new_in(alloc, "10.0.0.1").map(Value::from).unwrap();
        let by = Buffer::new_in(alloc, &[0u8, 0xff, b'x'])
            .map(Value::from)
            .unwrap();
        let l = List::new_in(alloc);
        let inner = Map::new_in(alloc);

        root.set_in("null", null, alloc).unwrap();
        root.set_in("bool", b, alloc).unwrap();
        root.set_in("number", n, alloc).unwrap();
        root.set_in("host", s, alloc).unwrap();
        root.set_in("blob", by, alloc).unwrap();
        root.set_in("items", l, alloc).unwrap();
        root.set_in("tls", inner, alloc).unwrap();

        let root = &m;
        assert_eq!(root.get("null").unwrap().tag().unwrap(), Tag::GUATIAO_NULL);
        assert_eq!(bool::try_from(root.get("bool").unwrap()).ok(), Some(true));
        assert_eq!(
            TryAsRef::<Number>::try_as_ref(root.get("number").unwrap()).map(AsRef::<str>::as_ref),
            Some("1.10"),
            "the exact text, not a reformatted f64"
        );
        assert_eq!(
            TryAsRef::<str>::try_as_ref(root.get("host").unwrap()),
            Some("10.0.0.1")
        );
        assert_eq!(
            TryAsRef::<[u8]>::try_as_ref(root.get("blob").unwrap()),
            Some(&[0u8, 0xff, b'x'][..]),
            "a NUL and a non-UTF-8 byte are ordinary in a bytes value"
        );
        assert!(root.get("items").is_some());
        assert!(root.get("missing").is_none());
        assert_eq!(root.entries().len(), 7);
    });
}

/// The accessors do not coerce. A number does not read as a string and a
/// string of digits does not read as a number: a value that reads as
/// plausible under two kinds hides the producer and the consumer
/// disagreeing about which it is.
#[test]
fn the_accessors_do_not_coerce() {
    with_alloc(|alloc, _| {
        let n = Value::from(Number::new_in(alloc, "5").unwrap());
        assert_eq!(
            TryAsRef::<Number>::try_as_ref(&n).map(AsRef::<str>::as_ref),
            Some("5")
        );
        assert_eq!(
            TryAsRef::<str>::try_as_ref(&n),
            None,
            "a number is not a string"
        );
        assert_eq!(bool::try_from(&n).ok(), None);
        assert_eq!(TryAsRef::<[u8]>::try_as_ref(&n), None);

        let s = Text::new_in(alloc, "5").map(Value::from).unwrap();
        assert_eq!(TryAsRef::<str>::try_as_ref(&s), Some("5"));
        assert_eq!(
            TryAsRef::<Number>::try_as_ref(&s).map(AsRef::<str>::as_ref),
            None,
            "a string is not a number"
        );
    });
}

#[test]
fn a_number_keeps_its_exact_text() {
    with_alloc(|alloc, _| {
        for text in [
            "0",
            "-0",
            "1.10",
            "1e400",
            "2.5E-3",
            "123456789012345678901234567890123456789012345678901234567890",
        ] {
            let v = Value::from(Number::new_in(alloc, text).unwrap());
            assert_eq!(
                TryAsRef::<Number>::try_as_ref(&v).map(AsRef::<str>::as_ref),
                Some(text),
                "verbatim: {text}"
            );
        }
    });
}

#[test]
fn text_outside_the_json_number_grammar_is_refused() {
    with_alloc(|alloc, _| {
        for bad in [
            "+1", ".5", "5.", "01", "0x1F", "Infinity", "NaN", "1_000", "",
        ] {
            assert_eq!(
                Number::new_in(alloc, bad).unwrap_err(),
                ValueError::NotANumber,
                "refused: {bad:?}"
            );
        }
        assert_eq!(
            Number::float_in(alloc, f64::NAN)
                .map(Value::from)
                .unwrap_err(),
            ValueError::NotANumber,
            "a non-finite float has no JSON spelling at all"
        );
        assert_eq!(
            Number::float_in(alloc, f64::INFINITY)
                .map(Value::from)
                .unwrap_err(),
            ValueError::NotANumber
        );
        let v = Number::new_in(alloc, &(-5900i64).to_string())
            .map(Value::from)
            .unwrap();
        assert_eq!(
            TryAsRef::<Number>::try_as_ref(&v).map(AsRef::<str>::as_ref),
            Some("-5900")
        );
    });
}

// --- the ordering contract --------------------------------------------

#[test]
fn iteration_is_insertion_order_and_replacement_keeps_position() {
    with_alloc(|alloc, _| {
        let mut m = Map::new_in(alloc);
        let root = &mut m;
        // Reverse-alphabetical on purpose: a sorted implementation cannot
        // pass this by accident.
        for key in ["zulu", "yankee", "alpha", "mike"] {
            let v = Text::new_in(alloc, key).map(Value::from).unwrap();
            root.set_in(key, v, alloc).unwrap();
        }
        let keys = |m: &Map| m.keys().map(str::to_string).collect::<Vec<_>>();
        assert_eq!(keys(&m), ["zulu", "yankee", "alpha", "mike"]);

        let replacement = Text::new_in(alloc, "REPLACED").map(Value::from).unwrap();
        m.set_in("yankee", replacement, alloc).unwrap();
        assert_eq!(
            keys(&m),
            ["zulu", "yankee", "alpha", "mike"],
            "a replaced key keeps its position, or a caller's rendered form re-orders itself"
        );
        assert_eq!(
            TryAsRef::<str>::try_as_ref(m.get("yankee").unwrap()),
            Some("REPLACED")
        );
        assert_eq!(m.entries().len(), 4);
    });
}

#[test]
fn keys_are_compared_as_raw_bytes() {
    with_alloc(|alloc, _| {
        let mut m = Map::new_in(alloc);
        let root = &mut m;
        let a = Text::new_in(alloc, "upper").map(Value::from).unwrap();
        let b = Text::new_in(alloc, "lower").map(Value::from).unwrap();
        root.set_in("Host", a, alloc).unwrap();
        root.set_in("host", b, alloc).unwrap();
        assert_eq!(m.entries().len(), 2, "two keys, not one");
        assert_eq!(
            TryAsRef::<str>::try_as_ref(m.get("Host").unwrap()),
            Some("upper")
        );
        assert_eq!(
            TryAsRef::<str>::try_as_ref(m.get("host").unwrap()),
            Some("lower")
        );
    });
}

/// A key may contain a NUL, so lookup compares bytes rather than stopping
/// at the first one. `strcmp` would make these two keys identical.
#[test]
fn a_key_containing_a_nul_is_its_own_key() {
    with_alloc(|alloc, _| {
        let mut m = Map::new_in(alloc);
        let root = &mut m;
        let a = Value::from(true);
        let b = Value::from(false);
        root.set_in("ab", a, alloc).unwrap();
        root.set_in("ab\0cd", b, alloc).unwrap();
        assert_eq!(m.entries().len(), 2);
        assert_eq!(bool::try_from(m.get("ab").unwrap()).ok(), Some(true));
        assert_eq!(bool::try_from(m.get("ab\0cd").unwrap()).ok(), Some(false));
    });
}

// --- removal, discard, clear ------------------------------------------

#[test]
fn remove_hands_back_an_owned_value_and_discard_frees_it() {
    with_alloc(|alloc, counter| {
        let mut m = Map::new_in(alloc);
        let root = &mut m;
        for key in ["a", "b", "c"] {
            let v = Text::new_in(alloc, key).map(Value::from).unwrap();
            root.set_in(key, v, alloc).unwrap();
        }

        // `remove` hands the node itself back, so the caller owns it and
        // dropping it frees it — the leak the discard form exists to prevent
        // belongs to the C caller, who has no drop. Freeing it by hand here
        // exercises that path AND proves the two do not collide: a freed node
        // is left null-tagged, so the drop at the end of this scope finds
        // nothing to free rather than freeing it a second time, which the
        // outstanding count would report as a negative.
        let taken = m.remove("b").unwrap();
        assert_eq!(TryAsRef::<str>::try_as_ref(&taken), Some("b"));
        let mut taken = taken;
        unsafe { taken.free() };

        let keys: Vec<_> = m.entries().iter().map(|e| e.key().to_string()).collect();
        assert_eq!(keys, ["a", "c"], "the order of the rest is kept");

        assert!(m.discard("a"));
        assert!(!m.discard("a"), "gone already");
        assert_eq!(m.entries().len(), 1);
        assert!(counter.outstanding() > 0, "the map itself is still live");
    });
}

#[test]
fn clear_empties_a_map_but_keeps_its_capacity() {
    with_alloc(|alloc, _| {
        let mut m = Map::new_in(alloc);
        let root = &mut m;
        for key in ["a", "b", "c"] {
            let v = Text::new_in(alloc, key).map(Value::from).unwrap();
            root.set_in(key, v, alloc).unwrap();
        }
        let cap_before = m.capacity();
        assert!(cap_before > 0);

        m.clear();
        assert_eq!(m.entries().len(), 0);
        assert_eq!(
            m.capacity(),
            cap_before,
            "clear keeps the buffer; freeing the node would not"
        );
        assert_eq!(
            Value::from(m).tag().unwrap(),
            Tag::GUATIAO_MAP,
            "it is still a map, which freeing it would not leave it as"
        );
    });
}

// --- moving in ---------------------------------------------------------

/// The move-in form leaves the caller's node owning nothing, so the
/// natural cleanup path is a no-op rather than a double free.
/// Inserting CONSUMES the value, so there is no second owner to free the
/// buffers the map now holds. The borrow checker enforces that, where the
/// C surface has to null-tag the caller's node to get the same property.
#[test]
fn inserting_consumes_the_value_it_stores() {
    with_alloc(|alloc, _| {
        let mut m = Map::new_in(alloc);
        let v = Text::new_in(alloc, "some text that is definitely heap allocated")
            .map(Value::from)
            .unwrap();
        m.set_in("k", v, alloc).unwrap();

        assert_eq!(
            TryAsRef::<str>::try_as_ref(m.get("k").unwrap()),
            Some("some text that is definitely heap allocated"),
            "the map has the value intact, and nothing else can free it"
        );
    });
}

/// Inserting something you only borrowed is clone-then-move. There is no
/// copying setter: the copy is a line you can see rather than a cost
/// hidden inside a name that does not mention it.
#[test]
fn inserting_a_borrowed_value_is_clone_then_move() {
    with_alloc(|alloc, _| {
        let mut m = Map::new_in(alloc);
        let src = Text::new_in(alloc, "borrowed").map(Value::from).unwrap();

        let copy = src.clone_in(alloc).unwrap();
        m.set_in("k", copy, alloc).unwrap();

        assert_eq!(
            TryAsRef::<str>::try_as_ref(&src),
            Some("borrowed"),
            "source untouched"
        );
        assert_eq!(
            TryAsRef::<str>::try_as_ref(m.get("k").unwrap()),
            Some("borrowed"),
            "and the clone was moved in, not copied a second time"
        );
    });
}

// --- lists -------------------------------------------------------------

#[test]
fn a_list_appends_removes_and_keeps_order() {
    with_alloc(|alloc, _| {
        let mut l = List::new_in(alloc);
        for i in 0..10i64 {
            let v = Number::new_in(alloc, &i.to_string())
                .map(Value::from)
                .unwrap();
            l.push_in(v, alloc).unwrap();
        }
        assert_eq!(l.len(), 10);
        assert_eq!(
            TryAsRef::<Number>::try_as_ref(l.get(3).unwrap()).map(AsRef::<str>::as_ref),
            Some("3")
        );

        let taken = l.remove(0).unwrap();
        assert_eq!(
            TryAsRef::<Number>::try_as_ref(&taken).map(AsRef::<str>::as_ref),
            Some("0")
        );
        let mut taken = taken;
        unsafe { taken.free() };

        assert_eq!(
            TryAsRef::<Number>::try_as_ref(l.first().unwrap()).map(AsRef::<str>::as_ref),
            Some("1")
        );
        assert!(l.discard(8));
        assert!(!l.discard(99), "past the end");
        assert_eq!(l.len(), 8);

        l.clear();
        assert_eq!(l.len(), 0);
    });
}

#[test]
fn appending_to_text_and_bytes_grows_across_a_reallocation() {
    with_alloc(|alloc, _| {
        let mut s = Text::new_in(alloc, "ab").unwrap();
        for _ in 0..100 {
            s.push_str_in("xy", alloc).unwrap();
        }
        let s = Value::from(s);
        let text = TryAsRef::<str>::try_as_ref(&s).unwrap();
        assert_eq!(text.len(), 2 + 200);
        assert!(
            text.starts_with("abxyxy"),
            "the prefix survived every growth"
        );

        let mut b = Buffer::new_in(alloc, &[0]).unwrap();
        for _ in 0..100 {
            b.push_in(&[1, 2], alloc).unwrap();
        }
        assert_eq!(b.len(), 201);
    });
}

// --- copy_from ---------------------------------------------------------

/// The operation that exists because losing the keys you do not model is a
/// recorded, real failure: a consumer that rebuilds a record from the
/// fields it knows about drops everything a newer producer added.
#[test]
fn copy_from_preserves_the_keys_the_consumer_does_not_model() {
    with_alloc(|alloc, _| {
        let mut stored = Map::new_in(alloc);
        for (k, v) in [
            ("host", "10.0.0.1"),
            ("password", "hunter2"),
            ("new-field", "x"),
        ] {
            let val = Text::new_in(alloc, v).map(Value::from).unwrap();
            stored.set_in(k, val, alloc).unwrap();
        }

        // A consumer that models only `host` rebuilds the record.
        let mut rebuilt = Map::new_in(alloc);
        let n = rebuilt.copy_from_in(&stored, alloc).unwrap();
        assert_eq!(n, 3);
        let changed = Text::new_in(alloc, "10.0.0.2").map(Value::from).unwrap();
        rebuilt.set_in("host", changed, alloc).unwrap();

        assert_eq!(
            TryAsRef::<str>::try_as_ref(rebuilt.get("host").unwrap()),
            Some("10.0.0.2")
        );
        assert_eq!(
            TryAsRef::<str>::try_as_ref(rebuilt.get("password").unwrap()),
            Some("hunter2"),
            "the field the consumer never heard of survived"
        );
        assert_eq!(
            TryAsRef::<str>::try_as_ref(rebuilt.get("new-field").unwrap()),
            Some("x")
        );
    });
}

/// **A node reached through `get_mut` is grown, trimmed and cleared with
/// no `unsafe`**, and frees exactly what it allocated.
///
/// Sound because every container records the allocator that made it: a
/// write into a nested node allocates for that node's own tree, so nobody
/// has to vouch for which allocator that is. The one door that still takes
/// `unsafe` is `set_in`/`push_in`, for a node that records none.
#[test]
fn a_nested_node_is_mutated_without_unsafe() {
    with_alloc(|alloc, counter| {
        let mut root = Map::new_in(alloc);
        root.set("tls", Map::new_in(alloc)).unwrap();
        root.set("tags", List::new_in(alloc)).unwrap();

        let tls = root.get_mut("tls").expect("tls was set");
        TryAsMut::<Map>::try_as_mut(tls)
            .ok_or(ValueError::WrongKind)
            .and_then(|m| {
                m.set(
                    "ca",
                    Text::new_in(alloc, "/etc/ca.pem").map(Value::from).unwrap(),
                )
            })
            .unwrap();
        TryAsMut::<Map>::try_as_mut(tls)
            .ok_or(ValueError::WrongKind)
            .and_then(|m| m.set("verify", Value::from(true)))
            .unwrap();
        TryAsMut::<Map>::try_as_mut(tls)
            .ok_or(ValueError::WrongKind)
            .and_then(|m| {
                m.push_into(
                    "ciphers",
                    Text::new_in(alloc, "TLS_AES_256_GCM_SHA384")
                        .map(Value::from)
                        .unwrap(),
                )
            })
            .unwrap();
        assert!(
            TryAsMut::<Map>::try_as_mut(tls)
                .and_then(|m| m.remove("verify"))
                .is_some()
        );

        let tags = root.get_mut("tags").expect("tags was set");
        for tag in ["a", "b", "c"] {
            TryAsMut::<List>::try_as_mut(tags)
                .ok_or(ValueError::WrongKind)
                .and_then(|l| l.push(Text::new_in(alloc, tag).map(Value::from).unwrap()))
                .unwrap();
        }
        assert!(TryAsMut::<List>::try_as_mut(tags).is_some_and(|l| l.discard(1)));

        let keys: Vec<&str> = root
            .get("tls")
            .and_then(TryAsRef::<Map>::try_as_ref)
            .map(Map::entries)
            .unwrap()
            .iter()
            .map(Entry::key)
            .collect();
        assert_eq!(keys, ["ca", "ciphers"]);
        let tags: Vec<&str> = root
            .get("tags")
            .and_then(TryAsRef::<List>::try_as_ref)
            .map(|list| &list[..])
            .unwrap()
            .iter()
            .filter_map(TryAsRef::<str>::try_as_ref)
            .collect();
        assert_eq!(tags, ["a", "c"]);

        assert!(
            counter.outstanding() > 0,
            "the nested writes went through the tree's own allocator, which is counting"
        );
        TryAsMut::<Map>::try_as_mut(root.get_mut("tls").unwrap())
            .unwrap()
            .clear();
        assert_eq!(
            root.get("tls")
                .and_then(TryAsRef::<Map>::try_as_ref)
                .map(Map::entries)
                .map(<[_]>::len),
            Some(0)
        );
    });
}

// --- cloning -----------------------------------------------------------

#[test]
fn a_clone_is_deep_and_independent() {
    with_alloc(|alloc, _| {
        let mut orig = Map::new_in(alloc);
        let mut child = Map::new_in(alloc);
        let inner = Text::new_in(alloc, "before").map(Value::from).unwrap();
        child.set_in("v", inner, alloc).unwrap();
        orig.set_in("child", child, alloc).unwrap();

        let copy = orig.clone_in(alloc).unwrap();

        // A nested node is mutated safely: the child map records the
        // allocator that made it, so `set` needs nobody to vouch for one.
        let after = Text::new_in(alloc, "after").map(Value::from).unwrap();
        let orig_child = orig.get_mut("child").unwrap();
        TryAsMut::<Map>::try_as_mut(orig_child)
            .ok_or(ValueError::WrongKind)
            .and_then(|m| m.set("v", after))
            .unwrap();

        let copied_child = copy.get("child").unwrap();
        assert_eq!(
            TryAsRef::<str>::try_as_ref(
                TryAsRef::<Map>::try_as_ref(copied_child)
                    .and_then(|m| m.get("v"))
                    .unwrap()
            ),
            Some("before"),
            "the copy did not follow the original's change"
        );
    });
}

// --- failure leaves nothing behind -------------------------------------

/// The allocator belongs to the caller and can fail. A copy that failed
/// part-way through a subtree must free what it had already built, and
/// must leave the target untouched.
#[test]
fn an_allocation_failure_leaves_the_target_unchanged_and_leaks_nothing() {
    let counter = Counter::default();
    let vt = vtable(&counter);
    // SAFETY: `vt` is fully initialised and outlives its use.
    let alloc = unsafe { Alloc::from_raw(&vt) }.expect("a complete vtable");

    let mut m = Map::new_in(alloc);
    let first = Text::new_in(alloc, "kept").map(Value::from).unwrap();
    m.set_in("a", first, alloc).unwrap();

    // A three-deep subtree to copy, so the failure lands mid-copy.
    let mut subtree = Map::new_in(alloc);
    for k in ["x", "y", "z"] {
        let v = Text::new_in(alloc, "some reasonably long value")
            .map(Value::from)
            .unwrap();
        subtree.set_in(k, v, alloc).unwrap();
    }

    let before = counter.outstanding();
    counter.fail_at.set(counter.allocs.get() + 2);
    let err = subtree.clone_in(alloc).unwrap_err();
    assert_eq!(err, ValueError::Alloc(AllocError::Failed));
    counter.fail_at.set(0);

    assert_eq!(
        counter.outstanding(),
        before,
        "the partial copy was freed, so a failed clone leaks nothing"
    );
    assert!(m.get("sub").is_none(), "and nothing reached the target");
    assert_eq!(
        TryAsRef::<str>::try_as_ref(m.get("a").unwrap()),
        Some("kept"),
        "what was already there is untouched"
    );

    drop(m);
    drop(subtree);
    assert_eq!(counter.outstanding(), 0);
}

// --- literals ----------------------------------------------------------

/// A tree written as a C brace initialiser has `cap == 0` everywhere. It
/// must be readable, growable, and freeable without its static storage
/// ever reaching an allocator.
#[test]
fn a_literal_tree_is_readable_growable_and_safe_to_free() {
    static TEXT: &[u8; 5] = b"hello";

    with_alloc(|alloc, counter| {
        let mut literal = // SAFETY: a borrowed literal: `cap == 0`, five readable UTF-8 bytes,
        // never written, and the tag selects the text arm.
        unsafe {
            Value::from_raw_parts(u32::from(Tag::GUATIAO_STRING),
                Payload::text(Text::from_raw_parts(
                    TEXT.as_ptr().cast_mut(),
                    5,
                    0,
                    std::ptr::null(),
                )),
            )
        };

        assert_eq!(
            TryAsRef::<str>::try_as_ref(&literal),
            Some("hello"),
            "readable as it stands"
        );

        // Freeing it must be a no-op: `cap == 0` means the buffer is not
        // ours, and handing it to an allocator is an immediate heap
        // corruption rather than an error.
        let before = counter.frees.get();
        unsafe { literal.free() };
        assert_eq!(counter.frees.get(), before, "nothing was handed to free");
        assert_eq!(*TEXT, *b"hello", "the static storage is untouched");

        // And a second literal can be grown, adopting the allocator.
        let mut growable = // SAFETY: a borrowed literal: `cap == 0`, five readable UTF-8 bytes,
        // never written, and the tag selects the text arm.
        unsafe {
            Value::from_raw_parts(u32::from(Tag::GUATIAO_STRING),
                Payload::text(Text::from_raw_parts(
                    TEXT.as_ptr().cast_mut(),
                    5,
                    0,
                    std::ptr::null(),
                )),
            )
        };
        TryAsMut::<Text>::try_as_mut(&mut growable)
            .unwrap()
            .push_str_in(" world", alloc)
            .unwrap();
        assert_eq!(TryAsRef::<str>::try_as_ref(&growable), Some("hello world"));
        assert_eq!(*TEXT, *b"hello", "still untouched after the copy-out");
        unsafe { growable.free() };
    });
}

/// The boolean arm is a `bool`, and reads back as one.
#[test]
fn a_bool_byte_a_producer_should_not_have_written_is_still_defined() {
    with_alloc(|_alloc, _| {
        // What a foreign producer can put there, through the raw door.
        // SAFETY: a boolean node owns nothing.
        let v = unsafe { Value::from_raw_parts(u32::from(Tag::GUATIAO_BOOL), Payload::bool(true)) };
        assert_eq!(
            bool::try_from(&v).ok(),
            Some(true),
            "true reads back as true"
        );

        // SAFETY: as above.
        let v =
            unsafe { Value::from_raw_parts(u32::from(Tag::GUATIAO_BOOL), Payload::bool(false)) };
        assert_eq!(bool::try_from(&v).ok(), Some(false));
    });
}

// --- forward compatibility ---------------------------------------------

/// A tag from a newer producer is skipped, not fatal. Refusing to free a
/// tree because one node came from the future would leak the whole tree to
/// punish the one node.
#[test]
fn an_unknown_tag_is_skippable_rather_than_fatal() {
    with_alloc(|alloc, _| {
        let mut m = Map::new_in(alloc);
        let known = Text::new_in(alloc, "readable").map(Value::from).unwrap();
        m.set_in("known", known, alloc).unwrap();

        // SAFETY: a tag this build does not know owns nothing it can see,
        // over a payload every arm of which is initialised.
        let from_the_future = unsafe { Value::from_raw_parts(4242, Payload::bool(false)) };
        m.set_in("future", from_the_future, alloc).unwrap();

        let unknown = m.get("future").unwrap();
        assert_eq!(unknown.tag().unwrap_err(), ValueError::UnknownTag(4242));
        assert_eq!(bool::try_from(unknown).ok(), None, "no accessor claims it");
        assert_eq!(TryAsRef::<str>::try_as_ref(unknown), None);

        assert_eq!(
            TryAsRef::<str>::try_as_ref(m.get("known").unwrap()),
            Some("readable"),
            "every other value still reads"
        );
        // Dropping the map frees it, including the unknown node, and the
        // outstanding check at the end of `with_alloc` proves it.
    });
}

#[test]
fn a_number_a_foreign_producer_malformed_is_no_number_yet_is_freed_and_equals_itself() {
    // What a C producer can write: text under the NUMBER tag that is not a
    // JSON number. Under the counting allocator, so dropping it is proven
    // to free it even though no `Number` is ever handed out for it.
    with_alloc(|alloc, _| {
        let digits = Text::new_in(alloc, "1,5").unwrap();
        // SAFETY: the text arm is the number arm's layout, and this is the
        // shape a foreign producer may write; reading it is the point.
        let bad =
            unsafe { Value::from_raw_parts(u32::from(Tag::GUATIAO_NUMBER), Payload::text(digits)) };

        assert_eq!(bad.tag().unwrap(), Tag::GUATIAO_NUMBER);
        assert!(
            TryAsRef::<Number>::try_as_ref(&bad).is_none(),
            "no Number for digits that are not one"
        );
        assert!(i64::try_from(&bad).is_err());
        assert!(bad == bad, "equal to itself, by its bytes");
        let bad = Number::try_from(bad).expect_err("handed back, not converted");
        drop(bad);
    });
}

/// Bytes a foreign producer wrote as a text, UTF-8 or not, owned by
/// `alloc`: what C can put in a `guatiao_text` without asking anyone.
fn foreign_text(alloc: Alloc, bytes: &[u8]) -> Text {
    let (ptr, len, cap, a) = Buffer::new_in(alloc, bytes).unwrap().into_raw_parts();
    // SAFETY: the storage is consistent; whether its bytes are UTF-8 is
    // what the tests below are about.
    unsafe { Text::from_raw_parts(ptr, len, cap, a) }
}

/// `map` with its first key's bytes replaced, as a foreign producer
/// writing the entries itself could.
fn with_foreign_first_key(map: Map, alloc: Alloc, key: &[u8]) -> Map {
    let (ptr, len, cap, a) = map.into_raw_parts();
    assert!(len > 0);
    // SAFETY: an entry is `repr(C)` with its key first, and the old key is
    // freed before the new one is written over it.
    unsafe {
        let slot = ptr.cast::<Text>();
        std::ptr::drop_in_place(slot);
        std::ptr::write(slot, foreign_text(alloc, key));
        Map::from_raw_parts(ptr, len, cap, a)
    }
}

#[test]
fn a_string_a_foreign_producer_wrote_that_is_not_utf8_is_no_text_yet_is_freed_and_equals_itself() {
    with_alloc(|alloc, _| {
        let bad = unsafe {
            // SAFETY: a STRING tag over a text arm, which is the shape; the
            // bytes are what is under test.
            Value::from_raw_parts(
                u32::from(Tag::GUATIAO_STRING),
                Payload::text(foreign_text(alloc, b"caf\xe9")),
            )
        };
        assert!(
            TryAsRef::<Text>::try_as_ref(&bad).is_none(),
            "no Text for bytes that are not UTF-8"
        );
        assert!(TryAsRef::<str>::try_as_ref(&bad).is_none());
        assert!(bad == bad, "equal to itself, by its bytes");
        assert_eq!(bad.clone_in(alloc).unwrap_err(), ValueError::NotUtf8);
        let mut bad = Text::try_from(bad).expect_err("handed back, not converted");
        assert!(TryAsMut::<Text>::try_as_mut(&mut bad).is_none());
        drop(bad);

        let good = unsafe {
            // SAFETY: as above, with UTF-8 bytes.
            Value::from_raw_parts(
                u32::from(Tag::GUATIAO_STRING),
                Payload::text(foreign_text(alloc, "café".as_bytes())),
            )
        };
        assert_eq!(
            TryAsRef::<str>::try_as_ref(&good),
            Some("café"),
            "checked, then a str"
        );
    });
}

#[test]
fn a_map_a_foreign_producer_wrote_with_a_key_that_is_not_utf8_is_no_map_yet_is_freed_and_equals_itself()
 {
    with_alloc(|alloc, _| {
        let mut map = Map::new_in(alloc);
        map.set("host", "h").unwrap();
        map.set("port", 1).unwrap();
        let bad: Value = with_foreign_first_key(map, alloc, b"\xffhost").into();

        assert!(
            TryAsRef::<Map>::try_as_ref(&bad).is_none(),
            "no Map while a key is not text"
        );
        let (_, payload) = bad.into_raw_parts();
        // SAFETY: the map arm is live under the MAP tag it was built with.
        let arm = unsafe { &*std::ptr::addr_of!(payload).cast::<Map>() };
        // SAFETY: a live map.
        assert_eq!(
            unsafe { Map::from_ptr(arm) }.map(|_| ()),
            Err(ValueError::NotUtf8)
        );
        // SAFETY: rebuilt from its own parts, to be freed below.
        let bad = unsafe { Value::from_raw_parts(u32::from(Tag::GUATIAO_MAP), payload) };
        assert!(<&Map>::try_from(&bad).is_err());
        assert!(bad == bad, "equal to itself, keys by their bytes");
        assert_eq!(bad.clone_in(alloc).unwrap_err(), ValueError::NotUtf8);

        let mut outer = List::new_in(alloc);
        outer.push(bad).unwrap();
        let outer: Value = outer.into();
        assert_eq!(
            outer.clone_in(alloc).unwrap_err(),
            ValueError::NotUtf8,
            "found at any depth"
        );
        // Dropping frees the bad key and everything else: the counting
        // allocator's check at the end of `with_alloc` proves it.
    });
}

#[test]
fn a_number_is_a_str() {
    let number = Number::new("1.10").unwrap();
    assert_eq!(&*number, "1.10");
    assert_eq!(number.len(), 4);
    assert!(number.contains('.'));
    assert_eq!(number.parse::<f64>().unwrap(), 1.1);
    let v: Value = number.into();
    assert_eq!(
        TryAsRef::<Number>::try_as_ref(&v).map(|n| &**n),
        Some("1.10")
    );
}

#[test]
fn an_iterator_left_half_way_frees_what_it_did_not_hand_out() {
    // Under the counting allocator: what is taken is freed by its taker,
    // what is left is freed with the iterator, and nothing is outstanding.
    with_alloc(|alloc, _| {
        let mut list = List::new_in(alloc);
        let mut map = Map::new_in(alloc);
        for i in 0..6 {
            list.push_in(Text::new_in(alloc, &i.to_string()).unwrap(), alloc)
                .unwrap();
            map.set_in(&i.to_string(), Text::new_in(alloc, "v").unwrap(), alloc)
                .unwrap();
        }
        let mut items = list.into_iter();
        drop(items.next());
        drop(items.next());
        drop(items);
        let mut pairs = map.into_iter();
        drop(pairs.next());
        drop(pairs);
    });
}

#[test]
fn a_tree_of_any_depth_copies_and_equals_itself() {
    // Far past `MAX_DEPTH`, built in safe code. Copying and comparing are
    // loops over a heap stack, so neither is bounded by depth and neither
    // can overflow the thread's stack; dropping is too.
    with_alloc(|alloc, _| {
        let depth = MAX_DEPTH as usize * 500;
        let mut tree: Value = Value::from(1);
        for _ in 0..depth {
            let mut list = List::new_in(alloc);
            list.push_in(tree, alloc).unwrap();
            tree = list.into();
        }
        assert!(tree == tree, "equal to itself at any depth");
        let copy = tree.clone_in(alloc).expect("a deep tree copies");
        assert!(copy == tree, "and equal to its copy");

        // A difference at the bottom is found, not given up on.
        let mut other = copy.clone_in(alloc).unwrap();
        let mut cursor = &mut other;
        for _ in 0..depth {
            cursor = &mut TryAsMut::<List>::try_as_mut(cursor).unwrap()[0];
        }
        *cursor = Value::from(2);
        assert!(other != tree, "a leaf changed {depth} levels down");
    });
}

// --- kind mismatches ---------------------------------------------------

#[test]
fn an_operation_on_the_wrong_kind_is_refused_by_name() {
    with_alloc(|alloc, _| {
        // A string node holds none of the container arms, so every
        // container reader refuses it -- which is what keeps a `&mut Map`
        // off a value that is not one.
        let mut s = Text::new_in(alloc, "text").map(Value::from).unwrap();
        assert!(TryAsMut::<List>::try_as_mut(&mut s).is_none());
        assert!(TryAsMut::<Map>::try_as_mut(&mut s).is_none());
        assert!(TryAsMut::<Buffer>::try_as_mut(&mut s).is_none());

        let mut l = List::new_in(alloc);
        l.push(Value::from(true)).unwrap();
        l.clear();
        assert_eq!(l.len(), 0);
    });
}

// --- reading comfortably from Rust ------------------------------------

/// The defaulting getters mirror the header's `static inline` helpers one
/// for one. Two implementations of the same rule that must agree
/// eventually will not, so the C consumer checks the same cases and this
/// checks the Rust side.
#[test]
fn the_defaulting_getters_never_truncate_and_never_coerce() {
    use guatiao::value::read::{bool_or, float_or, int_or, str_or};

    with_alloc(|alloc, _| {
        let mut m = Map::new_in(alloc);
        let root = &mut m;
        for (k, text) in [
            ("plain", "5900"),
            ("fractional", "1.10"),
            ("exponent", "1e2"),
            ("enormous", "123456789012345678901234567890"),
        ] {
            let v = Value::from(Number::new_in(alloc, text).unwrap());
            root.set_in(k, v, alloc).unwrap();
        }
        let b = Value::from(true);
        let s = Text::new_in(alloc, "text").map(Value::from).unwrap();
        root.set_in("flag", b, alloc).unwrap();
        root.set_in("name", s, alloc).unwrap();

        let root = &m;
        assert_eq!(int_or(root.get("plain"), -1), 5900);
        assert_eq!(
            int_or(root.get("fractional"), -1),
            -1,
            "a fractional value is not an integer"
        );
        assert_eq!(
            int_or(root.get("exponent"), -1),
            -1,
            "1e2 is integral in value but not in spelling, and evaluating the exponent is \
             the arithmetic this container exists to avoid"
        );
        assert_eq!(
            int_or(root.get("enormous"), -1),
            -1,
            "out of range answers the default rather than saturating, which is what strtoll \
             would have done"
        );
        assert_eq!(
            TryAsRef::<Number>::try_as_ref(root.get("enormous").unwrap()).map(AsRef::<str>::as_ref),
            Some("123456789012345678901234567890"),
            "and the exact text is still there for a caller that wants it"
        );

        assert!(float_or(root.get("fractional"), 0.0) > 1.09);
        assert!(bool_or(root.get("flag"), false));
        assert!(
            !bool_or(root.get("name"), false),
            "a string does not coerce to a bool"
        );
        assert_eq!(int_or(root.get("name"), -1), -1, "nor to an integer");
        assert_eq!(
            str_or(root.get("plain"), "fallback"),
            "fallback",
            "and a number does not coerce to a string"
        );
        assert_eq!(
            int_or(root.get("missing"), 7),
            7,
            "absent takes the default"
        );
    });
}

#[test]
fn iteration_and_equality_respect_insertion_order() {
    with_alloc(|alloc, _| {
        let build = |order: [&str; 3]| {
            let mut m = Map::new_in(alloc);
            for k in order {
                let v = Text::new_in(alloc, k).map(Value::from).unwrap();
                m.set_in(k, v, alloc).unwrap();
            }
            m
        };
        let a = build(["zulu", "alpha", "mike"]);
        let b = build(["zulu", "alpha", "mike"]);
        let reordered = build(["alpha", "zulu", "mike"]);

        let seen: Vec<&str> = a.keys().collect();
        assert_eq!(seen, ["zulu", "alpha", "mike"]);
        assert_eq!(a.entries().len(), 3);

        assert!(a == b);
        assert!(
            a != reordered,
            "insertion order is part of the contract, so two maps with the same pairs in a \
             different order are different values"
        );

        let mut l = List::new_in(alloc);
        for n in 0..3i64 {
            let v = Number::new_in(alloc, &n.to_string())
                .map(Value::from)
                .unwrap();
            l.push_in(v, alloc).unwrap();
        }
        assert_eq!(l.len(), 3);
    });
}

/// `1.10` and `1.1` are the same number and different values. Comparing as
/// numbers would take the lossy view, which is what storing text exists to
/// avoid.
#[test]
fn equality_compares_a_number_as_text() {
    with_alloc(|alloc, _| {
        let a = Value::from(Number::new_in(alloc, "1.10").unwrap());
        let b = Value::from(Number::new_in(alloc, "1.1").unwrap());
        let c = Value::from(Number::new_in(alloc, "1.10").unwrap());
        assert!(a != b);
        assert!(a == c);
    });
}

#[test]
fn the_debug_dump_walks_a_tree_and_shows_bytes_as_bytes() {
    use guatiao::value::read::Dump;

    with_alloc(|alloc, _| {
        let mut m = Map::new_in(alloc);
        let n = Value::from(Number::new_in(alloc, "1.10").unwrap());
        let s = Text::new_in(alloc, "hi").map(Value::from).unwrap();
        let b = Buffer::new_in(alloc, &[0u8, 0xff])
            .map(Value::from)
            .unwrap();
        m.set_in("n", n, alloc).unwrap();
        m.set_in("s", s, alloc).unwrap();
        m.set_in("b", b, alloc).unwrap();
        let text = format!("{:?}", Dump(&Value::from(m)));
        assert!(text.contains("\"n\": 1.10"), "{text}");
        assert!(text.contains("\"s\": \"hi\""), "{text}");
        assert!(
            text.contains("<2 bytes 00 ff>"),
            "bytes are shown as bytes rather than decoded as text: {text}"
        );
    });
}

// --- literals that are mutated, not only read ---------------------------

/// Borrowed text, as a C brace initialiser writes it: `cap == 0` and no
/// allocator, so the buffer is never freed.
fn text_lit(bytes: &'static [u8]) -> Text {
    // SAFETY: a borrowed literal: `cap == 0`, `len` readable bytes that
    // live for the program, never written through this text.
    unsafe { Text::from_raw_parts(bytes.as_ptr().cast_mut(), bytes.len(), 0, std::ptr::null()) }
}

/// A STRING node over borrowed bytes, whatever those bytes are.
///
/// Whatever they are is the point for one of the tests below: the
/// constructors refuse text that is not UTF-8, so the only way to hold a
/// string a reader cannot decode is to declare one, which a C producer
/// can do by accident.
fn string_lit(bytes: &'static [u8]) -> Value {
    // SAFETY: the tag selects the text arm the payload was built with.
    // The bytes need not be UTF-8: that is what the raw door is for.
    unsafe {
        Value::from_raw_parts(
            u32::from(Tag::GUATIAO_STRING),
            Payload::text(text_lit(bytes)),
        )
    }
}

/// A LIST and a MAP declared by hand are mutated **in place** and then
/// freed, with nothing left outstanding.
///
/// Only growth copies out of a `cap == 0` buffer. Removing, clearing and
/// replacing shift elements and free values through the array the caller
/// declared, which is why these literals are locals rather than `static
/// const` — and why the header says so to a C caller.
#[test]
fn a_literal_list_and_map_are_mutated_in_place_and_free_to_nothing() {
    static A: &[u8] = b"a";
    static B: &[u8] = b"b";

    with_alloc(|alloc, counter| {
        // --- a list of two borrowed strings.
        //
        // `ManuallyDrop` because the array stands in for one a C caller
        // declared, and C has no destructors: a removal shifts the tail
        // down and leaves the last slot holding a duplicate of what moved,
        // which Rust would otherwise free a second time. Everything the
        // node really owns is released through `free` below, and the
        // counter is what proves it.
        let mut items = std::mem::ManuallyDrop::new([string_lit(A), string_lit(B)]);
        // SAFETY: two well-formed nodes in a writable local array, borrowed
        // (`cap == 0`), and the tag selects the list arm.
        let mut list = unsafe {
            Value::from_raw_parts(
                u32::from(Tag::GUATIAO_LIST),
                Payload::list(List::from_raw_parts(
                    items.as_mut_ptr(),
                    2,
                    0,
                    std::ptr::null(),
                )),
            )
        };

        let before = counter.frees.get();
        assert!(
            TryAsMut::<List>::try_as_mut(&mut list).is_some_and(|l| l.discard(0)),
            "the first element is removed"
        );
        assert_eq!(
            TryAsRef::<List>::try_as_ref(&list)
                .map(|list| &list[..])
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            TryAsRef::<str>::try_as_ref(
                &TryAsRef::<List>::try_as_ref(&list)
                    .map(|list| &list[..])
                    .unwrap()[0]
            ),
            Some("b"),
            "the tail shifted down"
        );
        TryAsMut::<List>::try_as_mut(&mut list).unwrap().clear();
        assert_eq!(
            TryAsRef::<List>::try_as_ref(&list)
                .map(|list| &list[..])
                .unwrap()
                .len(),
            0
        );
        assert_eq!(
            counter.frees.get(),
            before,
            "nothing the caller declared reached the allocator"
        );

        // Growth is the one path that copies out, and what it allocates
        // is freed here.
        let item = Text::new_in(alloc, "c").map(Value::from).unwrap();
        // SAFETY: a well-formed list and a well-formed value, moved in.
        TryAsMut::<List>::try_as_mut(&mut list)
            .unwrap()
            .push_in(item, alloc)
            .unwrap();
        assert_eq!(
            TryAsRef::<str>::try_as_ref(
                &TryAsRef::<List>::try_as_ref(&list)
                    .map(|list| &list[..])
                    .unwrap()[0]
            ),
            Some("c")
        );
        // SAFETY: the node owns what it grew into and nothing else refers
        // to it.
        unsafe { list.free() };

        // --- a map of two borrowed entries, `ManuallyDrop` for the
        // reason above.
        let mut entries = std::mem::ManuallyDrop::new([
            Entry::new(text_lit(A), string_lit(A)),
            Entry::new(text_lit(B), string_lit(B)),
        ]);
        // SAFETY: as for the list, over entries.
        let mut map = unsafe {
            Value::from_raw_parts(
                u32::from(Tag::GUATIAO_MAP),
                Payload::map(Map::from_raw_parts(
                    entries.as_mut_ptr(),
                    2,
                    0,
                    std::ptr::null(),
                )),
            )
        };

        let before = counter.frees.get();
        // Replacing an existing key writes into the caller's own array
        // and frees what was there -- which owns nothing, being a literal.
        let replacement = Text::new_in(alloc, "B!").map(Value::from).unwrap();
        // SAFETY: a well-formed map and a well-formed value, moved in.
        TryAsMut::<Map>::try_as_mut(&mut map)
            .unwrap()
            .set_in("b", replacement, alloc)
            .unwrap();
        assert_eq!(
            TryAsRef::<Map>::try_as_ref(&map)
                .and_then(|m| m.get("b"))
                .and_then(TryAsRef::<str>::try_as_ref),
            Some("B!")
        );
        assert!(
            TryAsMut::<Map>::try_as_mut(&mut map).is_some_and(|m| m.discard("a")),
            "and a key is removed in place"
        );
        assert_eq!(
            TryAsRef::<Map>::try_as_ref(&map)
                .map(Map::entries)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            counter.frees.get(),
            before,
            "the entry array itself never reached the allocator"
        );

        // SAFETY: the map owns the replacement it was given; its own
        // array has `cap == 0` and is left alone.
        unsafe { map.free() };
    });
}

/// Two strings a reader cannot decode are equal only if their BYTES are.
///
/// Comparing the decoded text answered `None == None`, which made every
/// undecodable string equal to every other one -- and to a number whose
/// digits were equally undecodable.
#[test]
fn two_different_unreadable_strings_are_not_equal() {
    static X: &[u8] = &[0xff, 0x01];
    static Y: &[u8] = &[0xff, 0x02];

    let x = string_lit(X);
    let y = string_lit(Y);
    assert_eq!(
        TryAsRef::<str>::try_as_ref(&x),
        None,
        "neither decodes, which is the trap"
    );
    assert_eq!(TryAsRef::<str>::try_as_ref(&y), None);
    assert!(x != y, "different bytes are different values");
    assert!(x == string_lit(X), "the same bytes still compare equal");

    // A NUMBER stores its digits in the same container and had the same
    // hole.
    // SAFETY: the text arm is the live one under a NUMBER tag too; the
    // digits need not parse for `equal` to compare them.
    let a = unsafe {
        Value::from_raw_parts(
            u32::from(Tag::GUATIAO_NUMBER),
            string_lit(X).into_raw_parts().1,
        )
    };
    let b = unsafe {
        Value::from_raw_parts(
            u32::from(Tag::GUATIAO_NUMBER),
            string_lit(Y).into_raw_parts().1,
        )
    };
    assert!(a != b);
}
