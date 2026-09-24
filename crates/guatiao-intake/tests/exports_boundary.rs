// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The C surface, called from Rust.
//!
//! `tests/c_consumer.rs` compiles a real C program against the committed
//! header, which is the proof the surface is usable — and it skips where
//! there is no C compiler. These never skip: they are the statuses the
//! header promises, asserted on every machine.

use std::ptr;

use guatiao::schema::{ArmBuilder, FieldBuilder, KindBuilder, SchemaBuilder};
use guatiao::value::alloc::{Allocator, rust_alloc};
use guatiao::value::convert::TryAsRef;
use guatiao::value::status::Status;
use guatiao::value::types::{Str, Value};
use guatiao::{Alloc, List, Map, Value as V};
use guatiao_intake::exports::{guatiao_intake_is_visible, guatiao_intake_layout};
use guatiao_intake::exports_flat::{
    guatiao_intake_flat_keys, guatiao_intake_flatten, guatiao_intake_resolve,
    guatiao_intake_unflatten,
};
use guatiao_intake::{Form, Hints};

fn schema() -> Value {
    SchemaBuilder::new()
        .field(FieldBuilder::new("verify", KindBuilder::bool()))
        .field(FieldBuilder::new("ca", KindBuilder::string()))
        .finish()
        .expect("a schema this small builds")
}

fn form() -> Value {
    Form::new()
        .field("ca", Hints::new().visible_when("verify", true))
        .finish()
        .expect("a form this small builds")
}

fn values() -> Value {
    let mut map = Map::new();
    map.set("verify", true).unwrap();
    map.into()
}

fn key(text: &str) -> Str<'_> {
    Str::new(text)
}

/// An allocator that cannot allocate is `GUATIAO_ERR_ALLOC`, not
/// `GUATIAO_ERR_BAD_VALUE`: the caller passed something, and what it
/// passed cannot build the answer. The same status `guatiao-serde` gives,
/// so one C caller's error handling covers both.
#[test]
fn an_allocator_that_cannot_allocate_says_so() {
    let (schema, form) = (schema(), form());
    let mut out = V::null();

    // SAFETY: both trees are live, `out` holds a null — which owns nothing
    // — and the null allocator is what this asserts about.
    let status = unsafe { guatiao_intake_layout(&schema, &form, ptr::null(), &mut out) };
    assert_eq!(status, Status::GUATIAO_ERR_ALLOC);

    let empty = Allocator {
        struct_size: size_of::<Allocator>() as u32,
        ctx: ptr::null_mut(),
        alloc: None,
        free: None,
        release: None,
    };
    // SAFETY: as above, with a present but incomplete allocator.
    let status = unsafe { guatiao_intake_layout(&schema, &form, &empty, &mut out) };
    assert_eq!(status, Status::GUATIAO_ERR_ALLOC);
}

/// A key naming no field is `GUATIAO_ERR_NOT_FOUND`, and the answer slot
/// is left alone: a caller that ignored the status cannot read an answer
/// that was never given.
#[test]
fn a_key_the_schema_does_not_declare_is_not_found() {
    let (schema, form, values) = (schema(), form(), values());
    let mut shown = false;

    // SAFETY: every pointer addresses a live value, and `shown` is one
    // writable bool.
    let status =
        unsafe { guatiao_intake_is_visible(&schema, &form, key("hots"), &values, &mut shown) };
    assert_eq!(status, Status::GUATIAO_ERR_NOT_FOUND);
    assert!(!shown, "the answer slot is untouched");

    // SAFETY: as above, with a key the schema declares.
    let status =
        unsafe { guatiao_intake_is_visible(&schema, &form, key("ca"), &values, &mut shown) };
    assert_eq!(status, Status::GUATIAO_OK);
    assert!(shown, "`verify` holds true, so `ca` shows");
}

/// **One null-check shape at this boundary.** Every pointer a call needs
/// is checked before anything else happens, so the status never depends
/// on how far a body got.
#[test]
fn every_required_pointer_is_refused_the_same_way() {
    let (schema, form, values) = (schema(), form(), values());
    let mut out = V::null();
    let mut shown = false;
    let vt = rust_alloc();

    // SAFETY: each call passes one null where a value is required and live
    // values everywhere else.
    unsafe {
        assert_eq!(
            guatiao_intake_layout(ptr::null(), &form, &vt, &mut out),
            Status::GUATIAO_ERR_NULL
        );
        assert_eq!(
            guatiao_intake_layout(&schema, ptr::null(), &vt, &mut out),
            Status::GUATIAO_ERR_NULL
        );
        assert_eq!(
            guatiao_intake_layout(&schema, &form, &vt, ptr::null_mut()),
            Status::GUATIAO_ERR_NULL
        );
        assert_eq!(
            guatiao_intake_is_visible(ptr::null(), &form, key("ca"), &values, &mut shown),
            Status::GUATIAO_ERR_NULL
        );
        assert_eq!(
            guatiao_intake_is_visible(&schema, &form, key("ca"), ptr::null(), &mut shown),
            Status::GUATIAO_ERR_NULL
        );
        assert_eq!(
            guatiao_intake_is_visible(&schema, &form, key("ca"), &values, ptr::null_mut()),
            Status::GUATIAO_ERR_NULL
        );
    }
}

// --- the flat projection ------------------------------------------------

/// A schema with a variant field, which is the only shape the projection
/// applies to.
fn variant_schema() -> Value {
    let alloc = Alloc::rust();
    SchemaBuilder::new_in(alloc)
        .field(FieldBuilder::new_in(
            alloc,
            "auth",
            KindBuilder::variant_in(
                alloc,
                "auth",
                vec![
                    ArmBuilder::new_in(alloc, "sso", "Single sign-on"),
                    ArmBuilder::new_in(alloc, "userpass", "Username and password")
                        .field(FieldBuilder::new_in(
                            alloc,
                            "username",
                            KindBuilder::string_in(alloc),
                        ))
                        .field(
                            FieldBuilder::new_in(alloc, "password", KindBuilder::string_in(alloc))
                                .sensitive(),
                        ),
                ],
            ),
        ))
        .finish()
        .expect("a schema this small does not exhaust an allocator")
}

/// A flat key resolves through one level of projection, and the field it
/// lands on is the ARM FIELD's, not the parent's.
#[test]
fn a_flat_key_resolves_to_the_option_that_governs_it() {
    let schema = variant_schema();

    // SAFETY: a well-formed schema and a readable view.
    let direct = unsafe { guatiao_intake_resolve(&schema, Str::new("auth")) };
    assert!(!direct.is_null(), "the field itself resolves");

    // SAFETY: as above.
    let projected = unsafe { guatiao_intake_resolve(&schema, Str::new("auth.password")) };
    assert!(
        !projected.is_null(),
        "a projected key resolves to the arm field it names"
    );
    assert_ne!(direct, projected, "and not to the parent field");

    // SAFETY: as above.
    let nothing = unsafe { guatiao_intake_resolve(&schema, Str::new("auth.nonesuch")) };
    assert!(
        nothing.is_null(),
        "a key nobody declared resolves to nothing"
    );
}

/// A value goes out flat and comes back whole.
#[test]
fn a_tagged_value_survives_the_round_trip_through_flat_text() {
    let alloc = Alloc::rust();
    let schema = variant_schema();

    let mut chosen = Map::new();
    chosen.set("auth", "userpass").unwrap();
    chosen.set("username", "ana").unwrap();
    chosen.set("password", "hunter2").unwrap();
    let chosen: Value = chosen.into();

    let mut flat = Value::absent();
    // SAFETY: every pointer addresses what its type says.
    let status = unsafe {
        guatiao_intake_flatten(
            &schema,
            Str::new("auth"),
            &chosen,
            alloc.as_raw(),
            &mut flat,
        )
    };
    assert_eq!(status, Status::GUATIAO_OK);

    assert_eq!(
        TryAsRef::<Map>::try_as_ref(&flat)
            .and_then(|m| m.get("auth"))
            .and_then(TryAsRef::<str>::try_as_ref),
        Some("userpass"),
        "the tag crosses as text"
    );
    assert_eq!(
        TryAsRef::<Map>::try_as_ref(&flat)
            .and_then(|m| m.get("auth.password"))
            .and_then(TryAsRef::<str>::try_as_ref),
        Some("hunter2"),
        "and the arm's fields are projected under it"
    );

    let mut back = Value::absent();
    // SAFETY: as above; `flat` is a map whose values are all strings.
    let status = unsafe {
        guatiao_intake_unflatten(&schema, Str::new("auth"), &flat, alloc.as_raw(), &mut back)
    };
    assert_eq!(status, Status::GUATIAO_OK);
    assert_eq!(
        TryAsRef::<Map>::try_as_ref(&back)
            .and_then(|m| m.get("auth"))
            .and_then(TryAsRef::<str>::try_as_ref),
        Some("userpass")
    );
    assert_eq!(
        TryAsRef::<Map>::try_as_ref(&back)
            .and_then(|m| m.get("username"))
            .and_then(TryAsRef::<str>::try_as_ref),
        Some("ana")
    );
    assert_eq!(
        TryAsRef::<Map>::try_as_ref(&back)
            .and_then(|m| m.get("password"))
            .and_then(TryAsRef::<str>::try_as_ref),
        Some("hunter2")
    );
}

/// A store holding anything but text is refused rather than stringified.
#[test]
fn a_flat_store_that_is_not_all_text_is_refused() {
    let alloc = Alloc::rust();
    let schema = variant_schema();

    let mut flat = Map::new();
    flat.set("auth", "userpass").unwrap();
    // A number, where the projection's contract says text.
    flat.set("username", 5900).unwrap();
    let flat: Value = flat.into();

    let mut back = Value::absent();
    // SAFETY: as above.
    let status = unsafe {
        guatiao_intake_unflatten(&schema, Str::new("auth"), &flat, alloc.as_raw(), &mut back)
    };
    assert_eq!(
        status,
        Status::GUATIAO_ERR_WRONG_KIND,
        "a flat store holds text by definition, so a number in one is a \
         mistake worth hearing about at the boundary rather than two layers in"
    );
}

/// The keys a field projects onto, for a renderer laying out a form.
#[test]
fn the_flat_keys_of_an_option_are_listed() {
    let alloc = Alloc::rust();
    let schema = variant_schema();

    let mut keys = Value::absent();
    // SAFETY: a well-formed schema and writable storage.
    let status =
        unsafe { guatiao_intake_flat_keys(&schema, Str::new("auth"), alloc.as_raw(), &mut keys) };
    assert_eq!(status, Status::GUATIAO_OK);

    let listed: Vec<&str> = TryAsRef::<List>::try_as_ref(&keys)
        .map(|list| &list[..])
        .expect("a list")
        .iter()
        .filter_map(TryAsRef::<str>::try_as_ref)
        .collect();
    assert_eq!(
        listed,
        ["auth", "auth.username", "auth.password"],
        "the field's own key first, then one per arm field, in declaration \
         order — which is the order a form renders them in"
    );
}

/// Null is refused rather than dereferenced, here as everywhere.
#[test]
fn the_flat_exports_refuse_null() {
    let alloc = Alloc::rust();
    let schema = variant_schema();
    let mut out = Value::absent();

    // SAFETY: passing null is the case under test.
    unsafe {
        assert!(guatiao_intake_resolve(std::ptr::null(), Str::new("k")).is_null());
        assert_eq!(
            guatiao_intake_flat_keys(std::ptr::null(), Str::new("auth"), alloc.as_raw(), &mut out),
            Status::GUATIAO_ERR_NULL
        );
        assert_eq!(
            guatiao_intake_flatten(
                std::ptr::null(),
                Str::new("auth"),
                &schema,
                alloc.as_raw(),
                &mut out
            ),
            Status::GUATIAO_ERR_NULL
        );
        assert_eq!(
            guatiao_intake_unflatten(
                std::ptr::null(),
                Str::new("auth"),
                &schema,
                alloc.as_raw(),
                &mut out
            ),
            Status::GUATIAO_ERR_NULL
        );
    }
}

/// **A failed call still wrote the out-parameter.** The slot is written
/// the absent marker before any check, so a caller reading it after a
/// failure reads ABSENT rather than whatever its uninitialised local
/// happened to hold. Moved here with the export it exercises.
#[test]
fn a_refused_flat_call_still_writes_its_out_parameter() {
    let alloc = Alloc::rust();
    let schema = variant_schema();

    let mut out = Value::from(true);
    // SAFETY: every pointer addresses what its type says; the key naming
    // no field is the case under test.
    let status = unsafe {
        guatiao_intake_flat_keys(&schema, Str::new("nonesuch"), alloc.as_raw(), &mut out)
    };
    assert_eq!(status, Status::GUATIAO_ERR_WRONG_KIND);
    assert!(
        out.tag() == Ok(guatiao::Tag::GUATIAO_ABSENT),
        "a refused call still wrote the slot"
    );
}
