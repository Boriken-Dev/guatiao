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

use guatiao::schema::{FieldBuilder, KindBuilder, SchemaBuilder};
use guatiao::value::alloc::{Allocator, rust_alloc};
use guatiao::value::status::Status;
use guatiao::value::types::{Str, Value};
use guatiao::{Map, Value as V};
use guatiao_intake::exports::{guatiao_intake_is_visible, guatiao_intake_layout};
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
