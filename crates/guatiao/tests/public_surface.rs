// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a consumer can name, from outside the crate.
//!
//! # Two different failures this catches
//!
//! **A name the API header promises and the crate does not export.** The
//! header is what a consuming agent reads instead of the source, so a
//! name that is only in the header is a name somebody writes and cannot
//! compile.
//!
//! **A refusal and a bound are public; the arms are not.** `value::error`
//! holds the two names a consumer needs, and the arm a tag selects is
//! reached only through [`TryAsRef`]/[`TryAsMut`], which answer `None`
//! rather than handing out a `&mut Text` over a number's digits.

// --- what `value::error` exposes ----------------------------------------

/// The two names that module is public for, at every path that names them.
#[test]
fn the_error_module_exposes_the_depth_bound_and_the_error() {
    use guatiao::value::error::{MAX_DEPTH, ValueError};

    // A value, so the type is named rather than merely imported.
    let e: ValueError = ValueError::WrongKind;
    assert_eq!(e, ValueError::WrongKind);

    // The same constant, at the two shorter paths the rest of the crate
    // uses.
    assert_eq!(guatiao::MAX_DEPTH, MAX_DEPTH);
    assert_eq!(guatiao::value::MAX_DEPTH, MAX_DEPTH);

    // And the crate-root spelling of the error, which is what a consumer
    // writes in its own signatures.
    let root: guatiao::ValueError = e;
    assert_eq!(root, ValueError::WrongKind);
}

// --- what the crate root exposes -----------------------------------------

/// Every name the API header writes at the crate root resolves there.
///
/// `Text::new`, `Buffer::new`, `Entry::key()` and `Tag` were documented at
/// this level while only `Alloc`, `List`, `Map`, `ReadValue`, `Status`,
/// `Value` and `ValueError` were re-exported, so each of those lines was
/// an import that did not compile.
#[test]
fn the_root_exports_the_names_the_header_names() {
    use guatiao::{Alloc, Buffer, Entry, List, Map, Status, Str, Tag, Text, Value, ValueError};

    let text = Text::new("hello");
    assert_eq!(&*text, "hello");
    let buffer = Buffer::new(b"\x00\xff");
    assert_eq!(&buffer[..], b"\x00\xff");

    let mut map = Map::new();
    map.set("k", "v").unwrap();
    let entry: &Entry = &map.entries()[0];
    assert_eq!(entry.key(), "k");
    assert_eq!(entry.value().tag(), Ok(Tag::GUATIAO_STRING));

    let mut list = List::new();
    list.push(1).unwrap();
    assert_eq!(list.len(), 1);

    let view = Str::new("k");
    assert_eq!(view.len(), 1);

    assert_eq!(Value::null().tag(), Ok(Tag::GUATIAO_NULL));
    assert_eq!(
        Status::from(ValueError::WrongKind),
        Status::GUATIAO_ERR_WRONG_KIND
    );
    let _ = Alloc::rust();
}

/// The standard traits a container is expected to have.
#[test]
fn the_owned_containers_default_to_empty() {
    use guatiao::{Buffer, Text};

    assert_eq!(&*Text::default(), "");
    assert_eq!(&Buffer::default()[..], b"");
}

/// The borrowed byte view is constructed the way the borrowed text view
/// is, rather than by writing its fields out.
#[test]
fn the_borrowed_byte_view_has_the_constructors_its_sibling_has() {
    use guatiao::value::types::Bytes;

    static RAW: &[u8] = &[0x00, 0xff];
    let view = Bytes::new(RAW);
    assert_eq!(view.len(), 2);
    assert!(!view.ptr().is_null());

    let empty = Bytes::empty();
    assert_eq!(empty.len(), 0);
    assert!(empty.ptr().is_null());
}

/// An entry hands out its value mutably, so a pair can be changed without
/// being taken apart.
///
/// The key is not offered the same way: a map is looked up by exact bytes
/// and rendered in insertion order, so changing a key in place would move
/// a value to a key nobody searched for.
#[test]
fn an_entry_hands_out_its_value_mutably() {
    use guatiao::value::convert::TryAsRef;
    use guatiao::{Entry, Text, Value};

    let mut entry = Entry::new(Text::new("k"), Value::from(1i64));
    *entry.value_mut() = Value::from(Text::new("two"));
    assert_eq!(TryAsRef::<str>::try_as_ref(entry.value()), Some("two"));
    assert_eq!(entry.key(), "k");
}

// --- the standard traits the owned types carry ----------------------------

/// `Value`, `Map`, `List`, `Text` and `Buffer` are `Clone` (a deep copy
/// through the source's own allocator), `PartialEq` (structural), `Send`
/// and `Sync`: what lets a record type derive `Clone, PartialEq` over them
/// and a registry hold a schema value behind an `Arc<dyn Trait + Send +
/// Sync>`.
#[test]
fn the_owned_types_are_clone_eq_send_and_sync() {
    use guatiao::value::convert::TryAsRef;
    use guatiao::{Buffer, List, Map, Number, Text, Value};

    fn is_send_sync<T: Send + Sync>() {}
    is_send_sync::<Value>();
    is_send_sync::<Map>();
    is_send_sync::<List>();
    is_send_sync::<Text>();
    is_send_sync::<Buffer>();

    let mut inner = List::new();
    inner.push(1).unwrap();
    inner.push("two").unwrap();
    let mut map = Map::new();
    map.set("k", "v").unwrap();
    map.set("l", inner).unwrap();

    let copy = map.clone();
    assert_eq!(copy, map);
    assert_eq!(
        copy.get("k").and_then(TryAsRef::<str>::try_as_ref),
        Some("v")
    );
    assert_eq!(
        copy.get("l")
            .and_then(TryAsRef::<List>::try_as_ref)
            .map(|list| list.len()),
        Some(2)
    );

    let mut other = map.clone();
    other.set("k", "w").unwrap();
    assert_ne!(other, map, "a changed copy is a different map");
    drop(other);
    assert_eq!(copy, map, "and dropping it touched neither");

    let value: Value = map.into();
    let twin = value.clone();
    assert_eq!(value, twin);
    assert_ne!(value, Value::null());
    assert_eq!(
        Value::from(3i64).clone(),
        Value::from(3i64),
        "a scalar has no allocator and still clones"
    );
    assert_eq!(
        Value::from(Number::new("1.10").unwrap()),
        Value::from(Number::new("1.10").unwrap())
    );
    assert_ne!(
        Value::from(Number::new("1.10").unwrap()),
        Value::from(Number::new("1.1").unwrap()),
        "a number is its text"
    );

    assert_eq!(Text::new("a").clone(), Text::new("a"));
    assert_eq!(Buffer::new(b"\x00\xff").clone(), Buffer::new(b"\x00\xff"));

    // Across a thread, and back.
    let sent = std::thread::spawn(move || {
        assert_eq!(
            TryAsRef::<Map>::try_as_ref(&twin)
                .and_then(|m| m.get("k"))
                .and_then(TryAsRef::<str>::try_as_ref),
            Some("v")
        );
        twin
    })
    .join()
    .expect("the thread returned the value");
    assert_eq!(sent, value);
}

/// `into_map` and `into_list` take the container out of a value by
/// value, and hand a value of another kind back untouched.
#[test]
fn a_value_gives_up_its_container_by_value() {
    use guatiao::value::convert::TryAsRef;
    use guatiao::{List, Map, Text, Value};

    let mut map = Map::new();
    map.set("k", "v").unwrap();
    let value: Value = map.into();
    let mut map = Map::try_from(value).expect("a map");
    assert_eq!(
        map.remove("k")
            .as_ref()
            .and_then(TryAsRef::<str>::try_as_ref),
        Some("v")
    );

    let mut list = List::new();
    list.push(1).unwrap();
    let list = List::try_from(Value::from(list)).expect("a list");
    assert_eq!(list.len(), 1);

    let not_a_map = Map::try_from(Value::from(3i64)).expect_err("an int is not a map");
    assert_eq!(not_a_map, Value::from(3i64), "handed back untouched");
    assert!(List::try_from(Value::from(Text::new("x"))).is_err());
}
