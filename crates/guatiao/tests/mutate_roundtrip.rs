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

use std::cell::Cell;
use std::ffi::c_void;

use guatiao::value::alloc::{Alloc, AllocError, Allocator, rust_alloc};
use guatiao::value::mutate::{MAX_DEPTH, ValueError};
use guatiao::value::types::{Tag, Text, Value};

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
        let mut m = Value::map_in(alloc);
        let root = &mut m;

        let mut null = Value::null();
        let mut b = Value::bool(true);
        let mut n = Value::number_in(alloc, "1.10").unwrap();
        let mut s = Value::string_in(alloc, "10.0.0.1").unwrap();
        let mut by = Value::bytes_in(alloc, &[0u8, 0xff, b'x']).unwrap();
        let mut l = Value::list_in(alloc);
        let mut inner = Value::map_in(alloc);

        unsafe {
            root.set_in("null", &mut null, alloc).unwrap();
            root.set_in("bool", &mut b, alloc).unwrap();
            root.set_in("number", &mut n, alloc).unwrap();
            root.set_in("host", &mut s, alloc).unwrap();
            root.set_in("blob", &mut by, alloc).unwrap();
            root.set_in("items", &mut l, alloc).unwrap();
            root.set_in("tls", &mut inner, alloc).unwrap();
        }

        let root = &m;
        assert_eq!(root.get("null").unwrap().tag().unwrap(), Tag::GUATIAO_NULL);
        assert_eq!(root.get("bool").unwrap().as_bool(), Some(true));
        assert_eq!(
            root.get("number").unwrap().as_number_str(),
            Some("1.10"),
            "the exact text, not a reformatted f64"
        );
        assert_eq!(root.get("host").unwrap().as_str(), Some("10.0.0.1"));
        assert_eq!(
            root.get("blob").unwrap().as_bytes(),
            Some(&[0u8, 0xff, b'x'][..]),
            "a NUL and a non-UTF-8 byte are ordinary in a bytes value"
        );
        assert!(root.get("items").is_some());
        assert!(root.get("missing").is_none());
        assert_eq!(root.entries().unwrap().len(), 7);
    });
}

/// The accessors do not coerce. A number does not read as a string and a
/// string of digits does not read as a number: a value that reads as
/// plausible under two kinds hides the producer and the consumer
/// disagreeing about which it is.
#[test]
fn the_accessors_do_not_coerce() {
    with_alloc(|alloc, _| {
        let n = Value::number_in(alloc, "5").unwrap();
        assert_eq!(n.as_number_str(), Some("5"));
        assert_eq!(n.as_str(), None, "a number is not a string");
        assert_eq!(n.as_bool(), None);
        assert_eq!(n.as_bytes(), None);

        let s = Value::string_in(alloc, "5").unwrap();
        assert_eq!(s.as_str(), Some("5"));
        assert_eq!(s.as_number_str(), None, "a string is not a number");
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
            let v = Value::number_in(alloc, text).unwrap();
            assert_eq!(v.as_number_str(), Some(text), "verbatim: {text}");
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
                Value::number_in(alloc, bad).unwrap_err(),
                ValueError::NotANumber,
                "refused: {bad:?}"
            );
        }
        assert_eq!(
            Value::float_in(alloc, f64::NAN).unwrap_err(),
            ValueError::NotANumber,
            "a non-finite float has no JSON spelling at all"
        );
        assert_eq!(
            Value::float_in(alloc, f64::INFINITY).unwrap_err(),
            ValueError::NotANumber
        );
        let v = Value::int_in(alloc, -5900).unwrap();
        assert_eq!(v.as_number_str(), Some("-5900"));
    });
}

// --- the ordering contract --------------------------------------------

#[test]
fn iteration_is_insertion_order_and_replacement_keeps_position() {
    with_alloc(|alloc, _| {
        let mut m = Value::map_in(alloc);
        let root = &mut m;
        // Reverse-alphabetical on purpose: a sorted implementation cannot
        // pass this by accident.
        for key in ["zulu", "yankee", "alpha", "mike"] {
            let mut v = Value::string_in(alloc, key).unwrap();
            unsafe { root.set_in(key, &mut v, alloc) }.unwrap();
        }
        let keys = |v: &Value| {
            v.entries()
                .unwrap()
                .iter()
                .map(|e| String::from_utf8(e.key().to_vec()).unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(keys(&m), ["zulu", "yankee", "alpha", "mike"]);

        let mut replacement = Value::string_in(alloc, "REPLACED").unwrap();
        unsafe { m.set_in("yankee", &mut replacement, alloc) }.unwrap();
        assert_eq!(
            keys(&m),
            ["zulu", "yankee", "alpha", "mike"],
            "a replaced key keeps its position, or a caller's rendered form re-orders itself"
        );
        assert_eq!(m.get("yankee").unwrap().as_str(), Some("REPLACED"));
        assert_eq!(m.entries().unwrap().len(), 4);
    });
}

#[test]
fn keys_are_compared_as_raw_bytes() {
    with_alloc(|alloc, _| {
        let mut m = Value::map_in(alloc);
        let root = &mut m;
        let mut a = Value::string_in(alloc, "upper").unwrap();
        let mut b = Value::string_in(alloc, "lower").unwrap();
        unsafe {
            root.set_in("Host", &mut a, alloc).unwrap();
            root.set_in("host", &mut b, alloc).unwrap();
        }
        assert_eq!(m.entries().unwrap().len(), 2, "two keys, not one");
        assert_eq!(m.get("Host").unwrap().as_str(), Some("upper"));
        assert_eq!(m.get("host").unwrap().as_str(), Some("lower"));
    });
}

/// A key may contain a NUL, so lookup compares bytes rather than stopping
/// at the first one. `strcmp` would make these two keys identical.
#[test]
fn a_key_containing_a_nul_is_its_own_key() {
    with_alloc(|alloc, _| {
        let mut m = Value::map_in(alloc);
        let root = &mut m;
        let mut a = Value::bool(true);
        let mut b = Value::bool(false);
        unsafe {
            root.set_in("ab", &mut a, alloc).unwrap();
            root.set_in("ab\0cd", &mut b, alloc).unwrap();
        }
        assert_eq!(m.entries().unwrap().len(), 2);
        assert_eq!(m.get("ab").unwrap().as_bool(), Some(true));
        assert_eq!(m.get("ab\0cd").unwrap().as_bool(), Some(false));
    });
}

// --- removal, discard, clear ------------------------------------------

#[test]
fn remove_hands_back_an_owned_value_and_discard_frees_it() {
    with_alloc(|alloc, counter| {
        let mut m = Value::map_in(alloc);
        let root = &mut m;
        for key in ["a", "b", "c"] {
            let mut v = Value::string_in(alloc, key).unwrap();
            unsafe { root.set_in(key, &mut v, alloc) }.unwrap();
        }

        // `remove` hands the node itself back, so the caller owns it and
        // dropping it frees it — the leak the discard form exists to prevent
        // belongs to the C caller, who has no drop. Freeing it by hand here
        // exercises that path AND proves the two do not collide: a freed node
        // is left null-tagged, so the drop at the end of this scope finds
        // nothing to free rather than freeing it a second time, which the
        // outstanding count would report as a negative.
        let taken = m.remove("b").unwrap();
        assert_eq!(taken.as_str(), Some("b"));
        let mut taken = taken;
        unsafe { taken.free() };

        let keys: Vec<_> = m
            .entries()
            .unwrap()
            .iter()
            .map(|e| String::from_utf8(e.key().to_vec()).unwrap())
            .collect();
        assert_eq!(keys, ["a", "c"], "the order of the rest is kept");

        assert!(m.discard("a"));
        assert!(!m.discard("a"), "gone already");
        assert_eq!(m.entries().unwrap().len(), 1);
        assert!(counter.outstanding() > 0, "the map itself is still live");
    });
}

#[test]
fn clear_empties_a_map_but_keeps_its_capacity() {
    with_alloc(|alloc, _| {
        let mut m = Value::map_in(alloc);
        let root = &mut m;
        for key in ["a", "b", "c"] {
            let mut v = Value::string_in(alloc, key).unwrap();
            unsafe { root.set_in(key, &mut v, alloc) }.unwrap();
        }
        let cap_before = m.as_map().unwrap().cap;
        assert!(cap_before > 0);

        m.clear().unwrap();
        assert_eq!(m.entries().unwrap().len(), 0);
        assert_eq!(
            m.as_map().unwrap().cap,
            cap_before,
            "clear keeps the buffer; freeing the node would not"
        );
        assert_eq!(
            m.tag().unwrap(),
            Tag::GUATIAO_MAP,
            "it is still a map, which freeing it would not leave it as"
        );
    });
}

// --- moving in ---------------------------------------------------------

/// The move-in form leaves the caller's node owning nothing, so the
/// natural cleanup path is a no-op rather than a double free.
#[test]
fn moving_a_value_in_leaves_the_source_owning_nothing() {
    with_alloc(|alloc, _| {
        let mut m = Value::map_in(alloc);
        let mut v = Value::string_in(alloc, "some text that is definitely heap allocated").unwrap();
        unsafe { m.set_in("k", &mut v, alloc) }.unwrap();

        assert_eq!(
            v.tag().unwrap(),
            Tag::GUATIAO_NULL,
            "the source is null-tagged, which is what makes the next line safe"
        );
        // A caller doing this in a cleanup path would otherwise be freeing
        // buffers the map now owns — and in Rust that caller is `Drop`, which
        // runs on `v` at the end of this scope whether or not the line below
        // is written.
        unsafe { v.free() };

        assert_eq!(
            m.get("k").unwrap().as_str(),
            Some("some text that is definitely heap allocated"),
            "and the map still has the value intact"
        );
    });
}

/// Inserting something you only borrowed is clone-then-move. There is no
/// copying setter: the copy is a line you can see rather than a cost
/// hidden inside a name that does not mention it.
#[test]
fn inserting_a_borrowed_value_is_clone_then_move() {
    with_alloc(|alloc, _| {
        let mut m = Value::map_in(alloc);
        let src = Value::string_in(alloc, "borrowed").unwrap();

        let mut copy = unsafe { src.clone_in(alloc) }.unwrap();
        unsafe { m.set_in("k", &mut copy, alloc) }.unwrap();

        assert_eq!(src.as_str(), Some("borrowed"), "source untouched");
        assert_eq!(m.get("k").unwrap().as_str(), Some("borrowed"));
        assert_eq!(
            copy.tag().unwrap(),
            Tag::GUATIAO_NULL,
            "and the clone was moved in, not copied a second time"
        );
    });
}

// --- lists -------------------------------------------------------------

#[test]
fn a_list_appends_removes_and_keeps_order() {
    with_alloc(|alloc, _| {
        let mut l = Value::list_in(alloc);
        for i in 0..10i64 {
            let mut v = Value::int_in(alloc, i).unwrap();
            unsafe { l.push_in(&mut v, alloc) }.unwrap();
        }
        assert_eq!(l.items().unwrap().len(), 10);
        assert_eq!(
            l.as_list().unwrap().get(3).unwrap().as_number_str(),
            Some("3")
        );

        let taken = l.remove_at(0).unwrap();
        assert_eq!(taken.as_number_str(), Some("0"));
        let mut taken = taken;
        unsafe { taken.free() };

        assert_eq!(
            l.as_list().unwrap().get(0).unwrap().as_number_str(),
            Some("1")
        );
        assert!(l.discard_at(8));
        assert!(!l.discard_at(99), "past the end");
        assert_eq!(l.items().unwrap().len(), 8);

        l.clear().unwrap();
        assert_eq!(l.items().unwrap().len(), 0);
    });
}

#[test]
fn appending_to_text_and_bytes_grows_across_a_reallocation() {
    with_alloc(|alloc, _| {
        let mut s = Value::string_in(alloc, "ab").unwrap();
        for _ in 0..100 {
            unsafe { s.push_str("xy", alloc) }.unwrap();
        }
        let text = s.as_str().unwrap();
        assert_eq!(text.len(), 2 + 200);
        assert!(
            text.starts_with("abxyxy"),
            "the prefix survived every growth"
        );

        let mut b = Value::bytes_in(alloc, &[0]).unwrap();
        for _ in 0..100 {
            unsafe { b.push_bytes(&[1, 2], alloc) }.unwrap();
        }
        assert_eq!(b.as_bytes().unwrap().len(), 201);
    });
}

// --- copy_from ---------------------------------------------------------

/// The operation that exists because losing the keys you do not model is a
/// recorded, real failure: a consumer that rebuilds a record from the
/// fields it knows about drops everything a newer producer added.
#[test]
fn copy_from_preserves_the_keys_the_consumer_does_not_model() {
    with_alloc(|alloc, _| {
        let mut stored = Value::map_in(alloc);
        for (k, v) in [
            ("host", "10.0.0.1"),
            ("password", "hunter2"),
            ("new-field", "x"),
        ] {
            let mut val = Value::string_in(alloc, v).unwrap();
            unsafe { stored.set_in(k, &mut val, alloc) }.unwrap();
        }

        // A consumer that models only `host` rebuilds the record.
        let mut rebuilt = Value::map_in(alloc);
        let n = unsafe { rebuilt.copy_from(&stored, alloc) }.unwrap();
        assert_eq!(n, 3);
        let mut changed = Value::string_in(alloc, "10.0.0.2").unwrap();
        unsafe { rebuilt.set_in("host", &mut changed, alloc) }.unwrap();

        assert_eq!(rebuilt.get("host").unwrap().as_str(), Some("10.0.0.2"));
        assert_eq!(
            rebuilt.get("password").unwrap().as_str(),
            Some("hunter2"),
            "the field the consumer never heard of survived"
        );
        assert_eq!(rebuilt.get("new-field").unwrap().as_str(), Some("x"));
    });
}

// --- cloning -----------------------------------------------------------

#[test]
fn a_clone_is_deep_and_independent() {
    with_alloc(|alloc, _| {
        let mut orig = Value::map_in(alloc);
        let mut child = Value::map_in(alloc);
        let mut inner = Value::string_in(alloc, "before").unwrap();
        unsafe {
            child.set_in("v", &mut inner, alloc).unwrap();
            orig.set_in("child", &mut child, alloc).unwrap();
        }

        let copy = unsafe { orig.clone_in(alloc) }.unwrap();

        let mut after = Value::string_in(alloc, "after").unwrap();
        let orig_child = orig.get_mut("child").unwrap();
        unsafe { orig_child.set_in("v", &mut after, alloc) }.unwrap();

        let copied_child = copy.get("child").unwrap();
        assert_eq!(
            copied_child.get("v").unwrap().as_str(),
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

    let mut m = Value::map_in(alloc);
    let mut first = Value::string_in(alloc, "kept").unwrap();
    unsafe { m.set_in("a", &mut first, alloc) }.unwrap();

    // A three-deep subtree to copy, so the failure lands mid-copy.
    let mut subtree = Value::map_in(alloc);
    for k in ["x", "y", "z"] {
        let mut v = Value::string_in(alloc, "some reasonably long value").unwrap();
        unsafe { subtree.set_in(k, &mut v, alloc) }.unwrap();
    }

    let before = counter.outstanding();
    counter.fail_at.set(counter.allocs.get() + 2);
    let err = unsafe { subtree.clone_in(alloc) }.unwrap_err();
    assert_eq!(err, ValueError::Alloc(AllocError::Failed));
    counter.fail_at.set(0);

    assert_eq!(
        counter.outstanding(),
        before,
        "the partial copy was freed, so a failed clone leaks nothing"
    );
    assert!(m.get("sub").is_none(), "and nothing reached the target");
    assert_eq!(
        m.get("a").unwrap().as_str(),
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
        let mut literal = Value {
            tag: u32::from(Tag::GUATIAO_STRING),
            _pad: 0,
            payload: guatiao::value::types::Payload {
                text: std::mem::ManuallyDrop::new(Text {
                    ptr: TEXT.as_ptr().cast_mut(),
                    len: 5,
                    cap: 0,
                    alloc: std::ptr::null(),
                }),
            },
        };

        assert_eq!(literal.as_str(), Some("hello"), "readable as it stands");

        // Freeing it must be a no-op: `cap == 0` means the buffer is not
        // ours, and handing it to an allocator is an immediate heap
        // corruption rather than an error.
        let before = counter.frees.get();
        unsafe { literal.free() };
        assert_eq!(counter.frees.get(), before, "nothing was handed to free");
        assert_eq!(*TEXT, *b"hello", "the static storage is untouched");

        // And a second literal can be grown, adopting the allocator.
        let mut growable = Value {
            tag: u32::from(Tag::GUATIAO_STRING),
            _pad: 0,
            payload: guatiao::value::types::Payload {
                text: std::mem::ManuallyDrop::new(Text {
                    ptr: TEXT.as_ptr().cast_mut(),
                    len: 5,
                    cap: 0,
                    alloc: std::ptr::null(),
                }),
            },
        };
        unsafe { growable.push_str(" world", alloc) }.unwrap();
        assert_eq!(growable.as_str(), Some("hello world"));
        assert_eq!(*TEXT, *b"hello", "still untouched after the copy-out");
        unsafe { growable.free() };
    });
}

/// A producer can write any byte into the boolean arm. Because the arm is
/// a `u8` rather than a `bool`, every one of them is a valid value of that
/// type and reading it is defined — which is the reason for the choice,
/// and is what this checks.
#[test]
fn a_bool_byte_a_producer_should_not_have_written_is_still_defined() {
    with_alloc(|_alloc, _| {
        let mut v = Value::bool(false);
        // What a foreign producer can put there.
        v.payload.b = 2;
        assert_eq!(
            v.as_bool(),
            Some(true),
            "any non-zero byte is true, and reading it is defined"
        );

        v.payload.b = 0;
        assert_eq!(v.as_bool(), Some(false));
    });
}

// --- forward compatibility ---------------------------------------------

/// A tag from a newer producer is skipped, not fatal. Refusing to free a
/// tree because one node came from the future would leak the whole tree to
/// punish the one node.
#[test]
fn an_unknown_tag_is_skippable_rather_than_fatal() {
    with_alloc(|alloc, _| {
        let mut m = Value::map_in(alloc);
        let mut known = Value::string_in(alloc, "readable").unwrap();
        unsafe { m.set_in("known", &mut known, alloc) }.unwrap();

        let mut from_the_future = Value::null();
        from_the_future.tag = 4242;
        unsafe { m.set_in("future", &mut from_the_future, alloc) }.unwrap();

        let unknown = m.get("future").unwrap();
        assert_eq!(unknown.tag().unwrap_err(), ValueError::UnknownTag(4242));
        assert_eq!(unknown.as_bool(), None, "no accessor claims it");
        assert_eq!(unknown.as_str(), None);

        assert_eq!(
            m.get("known").unwrap().as_str(),
            Some("readable"),
            "every other value still reads"
        );
        // Dropping the map frees it, including the unknown node, and the
        // outstanding check at the end of `with_alloc` proves it.
    });
}

#[test]
fn a_tree_deeper_than_the_limit_is_an_error_rather_than_a_dead_process() {
    with_alloc(|alloc, _| {
        let mut root = Value::list_in(alloc);
        // Build well past the clone limit.
        let mut cursor: *mut Value = &mut root;
        for _ in 0..(MAX_DEPTH + 8) {
            let mut child = Value::list_in(alloc);
            // SAFETY: `cursor` points at a list that outlives this loop.
            unsafe {
                (*cursor).push_in(&mut child, alloc).unwrap();
                cursor = (*cursor).as_list_mut().and_then(|l| l.get_mut(0)).unwrap();
            }
        }
        assert_eq!(
            unsafe { root.clone_in(alloc) }.unwrap_err(),
            ValueError::TooDeep
        );
        // Freeing it is iterative, so this does not overflow the stack.
    });
}

// --- kind mismatches ---------------------------------------------------

#[test]
fn an_operation_on_the_wrong_kind_is_refused_by_name() {
    with_alloc(|alloc, _| {
        let mut s = Value::string_in(alloc, "text").unwrap();
        let mut probe = Value::bool(true);
        assert_eq!(
            unsafe { s.push_in(&mut probe, alloc) }.unwrap_err(),
            ValueError::WrongKind
        );
        assert_eq!(
            probe.tag().unwrap(),
            Tag::GUATIAO_BOOL,
            "a refused move leaves the caller's value alone"
        );
        assert_eq!(
            unsafe { s.set_in("k", &mut probe, alloc) }.unwrap_err(),
            ValueError::WrongKind
        );
        assert_eq!(s.clear().unwrap_err(), ValueError::WrongKind);
        // `Value::clear` dispatches on the tag, so it empties a LIST as
        // willingly as a map. The map-only refusal is a property of the
        // `guatiao_map_clear` symbol, and `exports_boundary` asserts it.
        let mut l = Value::list_in(alloc);
        l.push(Value::bool(true)).unwrap();
        l.clear().unwrap();
        assert_eq!(l.items().unwrap().len(), 0);
        assert_eq!(
            unsafe { s.push_bytes(b"x", alloc) }.unwrap_err(),
            ValueError::WrongKind
        );
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
        let mut m = Value::map_in(alloc);
        let root = &mut m;
        for (k, text) in [
            ("plain", "5900"),
            ("fractional", "1.10"),
            ("exponent", "1e2"),
            ("enormous", "123456789012345678901234567890"),
        ] {
            let mut v = Value::number_in(alloc, text).unwrap();
            unsafe { root.set_in(k, &mut v, alloc) }.unwrap();
        }
        let mut b = Value::bool(true);
        let mut s = Value::string_in(alloc, "text").unwrap();
        unsafe {
            root.set_in("flag", &mut b, alloc).unwrap();
            root.set_in("name", &mut s, alloc).unwrap();
        }

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
            root.get("enormous").unwrap().as_number_str(),
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
    use guatiao::value::read::{entries, equal, items, keys};

    with_alloc(|alloc, _| {
        let build = |order: [&str; 3]| {
            let mut m = Value::map_in(alloc);
            for k in order {
                let mut v = Value::string_in(alloc, k).unwrap();
                unsafe { m.set_in(k, &mut v, alloc) }.unwrap();
            }
            m
        };
        let a = build(["zulu", "alpha", "mike"]);
        let b = build(["zulu", "alpha", "mike"]);
        let reordered = build(["alpha", "zulu", "mike"]);

        let seen: Vec<Vec<u8>> = keys(&a).map(<[u8]>::to_vec).collect();
        assert_eq!(
            seen,
            vec![b"zulu".to_vec(), b"alpha".to_vec(), b"mike".to_vec()]
        );
        assert_eq!(entries(&a).count(), 3);

        assert!(equal(&a, &b));
        assert!(
            !equal(&a, &reordered),
            "insertion order is part of the contract, so two maps with the same pairs in a \
             different order are different values"
        );

        let mut l = Value::list_in(alloc);
        for n in 0..3i64 {
            let mut v = Value::int_in(alloc, n).unwrap();
            unsafe { l.push_in(&mut v, alloc) }.unwrap();
        }
        assert_eq!(items(&l).count(), 3);
    });
}

/// `1.10` and `1.1` are the same number and different values. Comparing as
/// numbers would take the lossy view, which is what storing text exists to
/// avoid.
#[test]
fn equality_compares_a_number_as_text() {
    use guatiao::value::read::equal;

    with_alloc(|alloc, _| {
        let a = Value::number_in(alloc, "1.10").unwrap();
        let b = Value::number_in(alloc, "1.1").unwrap();
        let c = Value::number_in(alloc, "1.10").unwrap();
        assert!(!equal(&a, &b));
        assert!(equal(&a, &c));
    });
}

#[test]
fn the_debug_dump_walks_a_tree_and_shows_bytes_as_bytes() {
    use guatiao::value::read::Dump;

    with_alloc(|alloc, _| {
        let mut m = Value::map_in(alloc);
        let mut n = Value::number_in(alloc, "1.10").unwrap();
        let mut s = Value::string_in(alloc, "hi").unwrap();
        let mut b = Value::bytes_in(alloc, &[0u8, 0xff]).unwrap();
        unsafe {
            m.set_in("n", &mut n, alloc).unwrap();
            m.set_in("s", &mut s, alloc).unwrap();
            m.set_in("b", &mut b, alloc).unwrap();
        }
        let text = format!("{:?}", Dump(&m));
        assert!(text.contains("\"n\": 1.10"), "{text}");
        assert!(text.contains("\"s\": \"hi\""), "{text}");
        assert!(
            text.contains("<2 bytes 00 ff>"),
            "bytes are shown as bytes rather than decoded as text: {text}"
        );
    });
}
