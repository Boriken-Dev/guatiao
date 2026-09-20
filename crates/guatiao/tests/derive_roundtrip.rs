// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A Rust struct crosses as a map and comes back unchanged.
//!
//! These go through the derive exactly as a consumer writes it: no `use`
//! of anything the generated code names, because the generated code names
//! everything absolutely and this is where that is proven to hold outside
//! the macro crate's own tests.

#![cfg(feature = "derive")]

use guatiao::value::convert::{TryAsMut, TryAsRef};
use std::cell::Cell;
use std::ffi::c_void;

use guatiao::value::alloc::{Alloc, Allocator, rust_alloc};
use guatiao::value::read::str_or;
use guatiao::{Bytes, FromValue, List, Map, MapError, Number, Text, ToValue, Value, ValueError};

// A counting allocator, so every test also proves the tree frees.
#[derive(Default)]
struct Counter {
    outstanding: Cell<isize>,
}

unsafe extern "C" fn c_alloc(ctx: *mut c_void, size: usize, align: usize) -> *mut c_void {
    // SAFETY: `ctx` is the `&Counter` installed below, outliving the call.
    let c = unsafe { &*(ctx as *const Counter) };
    c.outstanding.set(c.outstanding.get() + 1);
    let real = rust_alloc().alloc.expect("the rust allocator has an alloc");
    // SAFETY: forwarding an unchanged request.
    unsafe { real(std::ptr::null_mut(), size, align) }
}

unsafe extern "C" fn c_free(ctx: *mut c_void, p: *mut c_void, size: usize, align: usize) {
    // SAFETY: as above.
    let c = unsafe { &*(ctx as *const Counter) };
    c.outstanding.set(c.outstanding.get() - 1);
    let real = rust_alloc().free.expect("the rust allocator has a free");
    // SAFETY: forwarding the same block with the same layout.
    unsafe { real(std::ptr::null_mut(), p, size, align) }
}

fn with_alloc(body: impl FnOnce(Alloc)) {
    let counter = Counter::default();
    let vt = Allocator {
        struct_size: size_of::<Allocator>() as u32,
        ctx: (&counter as *const Counter).cast_mut().cast(),
        alloc: Some(c_alloc),
        free: Some(c_free),
        release: None,
    };
    // SAFETY: `vt` is fully initialised and outlives the closure.
    let alloc = unsafe { Alloc::from_raw(&vt) }.expect("a complete vtable");
    body(alloc);
    assert_eq!(
        counter.outstanding.get(),
        0,
        "a derived map is an ordinary value, so it frees like one"
    );
}

#[derive(Debug, Clone, PartialEq, guatiao::ToValue, guatiao::FromValue)]
struct Tls {
    verify: bool,
    ca: Option<String>,
}

#[derive(Debug, Clone, PartialEq, guatiao::ToValue, guatiao::FromValue)]
struct Connection {
    host: String,
    port: i64,
    #[map(rename = "tls-verify")]
    verify: bool,
    motd: Option<String>,
    tags: Vec<String>,
    ticket: Bytes,
    tls: Tls,
    #[map(skip)]
    cache: Vec<String>,
}

fn sample() -> Connection {
    Connection {
        host: "10.0.0.1".to_string(),
        port: 5900,
        verify: true,
        motd: Some("hello".to_string()),
        tags: vec!["prod".to_string(), "eu".to_string()],
        ticket: Bytes(vec![0, 1, 0, 2]),
        tls: Tls {
            verify: false,
            ca: None,
        },
        cache: vec!["not stored".to_string()],
    }
}

#[test]
fn a_struct_round_trips_through_a_map() {
    with_alloc(|alloc| {
        let original = sample();
        let value = original
            .to_value(alloc)
            .expect("the allocator is not refusing");

        let back = Connection::from_value(&value).expect("what was just written reads");
        assert_eq!(
            back,
            Connection {
                // A skipped field is never stored, so it reads back as
                // its default rather than as what was in hand.
                cache: Vec::new(),
                ..original
            }
        );
    });
}

/// Keys are declaration order, renames land, and a skipped field leaves no
/// trace at all.
#[test]
fn the_map_holds_exactly_the_keys_the_declaration_asks_for() {
    with_alloc(|alloc| {
        let value = sample().to_value(alloc).unwrap();
        let keys: Vec<String> = TryAsRef::<Map>::try_as_ref(&value)
            .map(Map::entries)
            .unwrap_or(&[])
            .iter()
            .map(|e| (e.key(), e.value()))
            .map(|(k, _)| String::from_utf8_lossy(k).into_owned())
            .collect();
        assert_eq!(
            keys,
            [
                "host",
                "port",
                "tls-verify",
                "motd",
                "tags",
                "ticket",
                "tls"
            ],
            "declaration order, the rename applied, and `cache` nowhere"
        );
    });
}

/// The `Option` contract: `None` omits the key, and both an absent key and
/// a stored null read back as `None`.
#[test]
fn none_omits_the_key_and_both_spellings_read_back_as_none() {
    with_alloc(|alloc| {
        let mut quiet = sample();
        quiet.motd = None;
        let value = quiet.to_value(alloc).unwrap();
        assert!(
            TryAsRef::<Map>::try_as_ref(&value)
                .and_then(|m| m.get("motd"))
                .is_none(),
            "None stores nothing at all, not a null"
        );
        assert_eq!(Connection::from_value(&value).unwrap().motd, None);

        // The other spelling: a producer that writes an explicit null.
        let mut loud = sample().to_value(alloc).unwrap();
        TryAsMut::<Map>::try_as_mut(&mut loud)
            .ok_or(ValueError::WrongKind)
            .and_then(|m| m.set("motd", Value::null()))
            .unwrap();
        assert_eq!(Connection::from_value(&loud).unwrap().motd, None);
    });
}

/// A nested struct reports the whole path, which is the entire reason the
/// error carries one.
#[test]
fn an_error_inside_a_nested_struct_names_the_dotted_path() {
    with_alloc(|alloc| {
        let mut value = sample().to_value(alloc).unwrap();
        let tls = TryAsMut::<Map>::try_as_mut(&mut value)
            .and_then(|m| m.get_mut("tls"))
            .expect("tls was written");
        assert!(TryAsMut::<Map>::try_as_mut(tls).is_some_and(|m| m.discard("verify")));

        let e = Connection::from_value(&value).unwrap_err();
        assert_eq!(
            e,
            MapError::MissingKey {
                key: "tls.verify".into()
            }
        );
        assert!(e.to_string().contains("tls.verify"), "{e}");
    });
}

/// An element of a list names its index, so a bad entry in a long list is
/// findable.
#[test]
fn an_error_inside_a_list_names_the_index() {
    with_alloc(|alloc| {
        let mut value = sample().to_value(alloc).unwrap();
        let tags = TryAsMut::<Map>::try_as_mut(&mut value)
            .and_then(|m| m.get_mut("tags"))
            .expect("tags was written");
        // A number where a string belongs, at a known position.
        assert!(TryAsMut::<List>::try_as_mut(tags).is_some_and(|l| l.discard(1)));
        TryAsMut::<List>::try_as_mut(tags)
            .ok_or(ValueError::WrongKind)
            .and_then(|l| {
                l.push(
                    Number::new_in(alloc, &7.to_string())
                        .map(Value::from)
                        .unwrap(),
                )
            })
            .unwrap();

        let e = Connection::from_value(&value).unwrap_err();
        assert_eq!(e.key(), "tags[1]");
        assert!(matches!(e, MapError::WrongType { .. }), "{e:?}");
    });
}

/// Handing a struct's reader something that is not a map says so, rather
/// reporting every field missing and sending the reader after the wrong
/// bug.
#[test]
fn a_value_that_is_not_a_map_is_rejected_as_that() {
    with_alloc(|alloc| {
        let text = Text::new_in(alloc, "not a map").map(Value::from).unwrap();
        let e = Connection::from_value(&text).unwrap_err();
        assert!(matches!(e, MapError::WrongType { .. }), "{e:?}");
        assert_eq!(e.key(), "", "the top-level value has no key to name");
        assert!(
            e.to_string().contains("the value is a string"),
            "a nameless key reads as 'the value', never as '': {e}"
        );
    });
}

/// A number that does not fit is the right kind and the wrong value, which
/// is a different answer from a type mismatch and has a different fix.
#[test]
fn a_number_that_does_not_fit_is_bad_value_not_wrong_type() {
    with_alloc(|alloc| {
        let mut value = sample().to_value(alloc).unwrap();
        TryAsMut::<Map>::try_as_mut(&mut value)
            .ok_or(ValueError::WrongKind)
            .and_then(|m| {
                m.set(
                    "port",
                    Value::from(Number::new_in(alloc, "9223372036854775808").unwrap()),
                )
            })
            .unwrap();
        let e = Connection::from_value(&value).unwrap_err();
        assert!(matches!(e, MapError::BadValue { .. }), "{e:?}");
        assert_eq!(e.key(), "port");

        // ... and a fractional spelling is refused rather than truncated.
        TryAsMut::<Map>::try_as_mut(&mut value)
            .ok_or(ValueError::WrongKind)
            .and_then(|m| {
                m.set(
                    "port",
                    Value::from(Number::new_in(alloc, "5900.5").unwrap()),
                )
            })
            .unwrap();
        let e = Connection::from_value(&value).unwrap_err();
        assert!(matches!(e, MapError::BadValue { .. }), "{e:?}");
    });
}

/// Bytes are their own kind and a `Vec<T>` is a list, including for
/// `Vec<u8>`. The two must not be confusable, or a ticket becomes a list
/// of small numbers on the far side of a boundary.
#[test]
fn bytes_and_a_sequence_are_different_kinds() {
    with_alloc(|alloc| {
        #[derive(Debug, PartialEq, guatiao::ToValue, guatiao::FromValue)]
        struct Both {
            blob: Bytes,
            numbers: Vec<u8>,
        }

        let original = Both {
            blob: Bytes(vec![1, 2, 3]),
            numbers: vec![1, 2, 3],
        };
        let value = original.to_value(alloc).unwrap();

        assert_eq!(
            guatiao::value::read::bytes_or(
                TryAsRef::<Map>::try_as_ref(&value).and_then(|m| m.get("blob")),
                &[]
            ),
            &[1u8, 2, 3]
        );
        assert_eq!(
            TryAsRef::<Map>::try_as_ref(&value)
                .and_then(|m| m.get("numbers"))
                .and_then(TryAsRef::<List>::try_as_ref)
                .unwrap()
                .len(),
            3,
            "a Vec<u8> is a list like any other Vec<T>"
        );
        assert_eq!(Both::from_value(&value).unwrap(), original);
    });
}

/// A key may contain any byte, a NUL included, because a key is pointer
/// and length rather than a C string. The derive must not invent a
/// restriction the value model does not have.
#[test]
fn a_key_with_a_nul_in_it_works() {
    with_alloc(|alloc| {
        #[derive(Debug, PartialEq, guatiao::ToValue, guatiao::FromValue)]
        struct Odd {
            #[map(rename = "a\0b")]
            field: i64,
        }

        let value = Odd { field: 7 }.to_value(alloc).unwrap();
        assert_eq!(
            str_or(
                TryAsRef::<Map>::try_as_ref(&value).and_then(|m| m.get("a\0b")),
                ""
            ),
            "",
            "the key holds the NUL; the value is a number, so this reads as the fallback"
        );
        assert_eq!(Odd::from_value(&value).unwrap(), Odd { field: 7 });
    });
}

/// **`#[schema(...)]` compiles on a type that derives only `ToValue`.**
///
/// An attribute is registered by the derive that sees it, not by the one
/// that gives it meaning: a type carrying a presentation hint and no
/// `Schema` derive would otherwise fail with "cannot find attribute", at
/// the declaration, for a reason nothing in it explains. So `ToValue` and
/// `FromValue` both register `schema` and both ignore it.
#[test]
fn a_schema_attribute_is_accepted_by_the_other_two_derives() {
    with_alloc(|alloc| {
        #[derive(Debug, PartialEq, guatiao::ToValue, guatiao::FromValue)]
        struct Described {
            #[schema(label = "x", sensitive)]
            secret: i64,
        }

        let value = Described { secret: 1 }.to_value(alloc).unwrap();
        assert_eq!(
            Described::from_value(&value).unwrap(),
            Described { secret: 1 }
        );
    });
}
