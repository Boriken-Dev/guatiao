// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The C surface, called from Rust.
//!
//! `tests/c_consumer.rs` compiles a real C program against the committed
//! header, which is the proof the surface is usable — and it skips where
//! there is no C compiler. These never skip: they are the statuses the
//! header promises, asserted on every machine.

extern crate alloc;

use core::ptr;

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
    map.set("verify", true);
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

/// A condition written with `in` answers through the same C call, with no
/// new symbol: a renderer in C, Python or Dart asks the judge, so a richer
/// condition costs it nothing.
#[test]
fn a_condition_with_several_values_crosses_unchanged() {
    let schema = SchemaBuilder::new()
        .field(FieldBuilder::new("mode", KindBuilder::string()))
        .field(FieldBuilder::new("ca", KindBuilder::string()))
        .finish()
        .expect("it builds");
    let form = Form::new()
        .field("ca", Hints::new().visible_when_in("mode", ["tls", "mtls"]))
        .finish()
        .expect("it builds");
    for (mode, expected) in [("tls", true), ("mtls", true), ("plain", false)] {
        let mut values = Map::new();
        values.set("mode", mode);
        let values: Value = values.into();
        let mut shown = !expected;
        // SAFETY: every pointer addresses a live value, and `shown` is one
        // writable bool.
        let status =
            unsafe { guatiao_intake_is_visible(&schema, &form, key("ca"), &values, &mut shown) };
        assert_eq!(status, Status::GUATIAO_OK);
        assert_eq!(shown, expected, "mode {mode}");
    }
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
    chosen.set("auth", "userpass");
    chosen.set("username", "ana");
    chosen.set("password", "hunter2");
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
    flat.set("auth", "userpass");
    // A number, where the projection's contract says text.
    flat.set("username", 5900);
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
        assert!(guatiao_intake_resolve(core::ptr::null(), Str::new("k")).is_null());
        assert_eq!(
            guatiao_intake_flat_keys(
                core::ptr::null(),
                Str::new("auth"),
                alloc.as_raw(),
                &mut out
            ),
            Status::GUATIAO_ERR_NULL
        );
        assert_eq!(
            guatiao_intake_flatten(
                core::ptr::null(),
                Str::new("auth"),
                &schema,
                alloc.as_raw(),
                &mut out
            ),
            Status::GUATIAO_ERR_NULL
        );
        assert_eq!(
            guatiao_intake_unflatten(
                core::ptr::null(),
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

// --- what a failed call leaves behind --------------------------------------

/// Whether a node is the absent marker.
fn is_absent(value: &Value) -> bool {
    value.tag() == Ok(guatiao::Tag::GUATIAO_ABSENT)
}

/// Runs `call` with a stale out-slot and checks it answered `status` and
/// left the slot ABSENT.
fn leaves_absent(name: &str, status: Status, call: impl FnOnce(*mut Value) -> Status) {
    let mut out = V::from(true);
    assert_eq!(call(&mut out), status, "{name}");
    assert!(is_absent(&out), "{name} left its out-parameter as it was");
}

/// Every export of this crate with an out-parameter, called so that it
/// fails, and checked to have written the slot. The names are what the
/// coverage test compares with the header.
fn out_parameter_cases() -> Vec<(&'static str, fn())> {
    use guatiao_intake::exports::{
        guatiao_intake_check, guatiao_intake_for_schema, guatiao_intake_form_for,
    };
    vec![
        ("guatiao_intake_check", || {
            let form = form();
            // SAFETY: a null schema is the failure under test.
            leaves_absent("check", Status::GUATIAO_ERR_NULL, |out| unsafe {
                guatiao_intake_check(ptr::null(), &form, ptr::null(), out)
            });
        }),
        ("guatiao_intake_layout", || {
            let (schema, form) = (schema(), form());
            // SAFETY: a null allocator is the failure under test.
            leaves_absent("layout", Status::GUATIAO_ERR_ALLOC, |out| unsafe {
                guatiao_intake_layout(&schema, &form, ptr::null(), out)
            });
        }),
        ("guatiao_intake_for_schema", || {
            let schema = schema();
            // SAFETY: a null allocator is the failure under test.
            leaves_absent("for_schema", Status::GUATIAO_ERR_ALLOC, |out| unsafe {
                guatiao_intake_for_schema(&schema, ptr::null(), out)
            });
        }),
        ("guatiao_intake_form_for", || {
            let (schema, form) = (schema(), form());
            // SAFETY: a null allocator is the failure under test.
            leaves_absent("form_for", Status::GUATIAO_ERR_ALLOC, |out| unsafe {
                guatiao_intake_form_for(&form, &schema, key("ca"), ptr::null(), out)
            });
        }),
        ("guatiao_intake_flat_keys", || {
            let (schema, alloc) = (variant_schema(), Alloc::rust());
            // SAFETY: a key naming no field is the failure under test.
            leaves_absent("flat_keys", Status::GUATIAO_ERR_WRONG_KIND, |out| unsafe {
                guatiao_intake_flat_keys(&schema, key("nonesuch"), alloc.as_raw(), out)
            });
        }),
        ("guatiao_intake_flatten", || {
            let (schema, alloc) = (variant_schema(), Alloc::rust());
            let value = V::null();
            // SAFETY: a key naming no field is the failure under test.
            leaves_absent("flatten", Status::GUATIAO_ERR_WRONG_KIND, |out| unsafe {
                guatiao_intake_flatten(&schema, key("nonesuch"), &value, alloc.as_raw(), out)
            });
        }),
        ("guatiao_intake_unflatten", || {
            let (schema, alloc) = (variant_schema(), Alloc::rust());
            let flat = V::from(Map::new());
            // SAFETY: a key naming no field is the failure under test.
            leaves_absent("unflatten", Status::GUATIAO_ERR_WRONG_KIND, |out| unsafe {
                guatiao_intake_unflatten(&schema, key("nonesuch"), &flat, alloc.as_raw(), out)
            });
        }),
        // Its out-parameter is a `bool`, and the header says a key naming no
        // field leaves it unwritten.
        ("guatiao_intake_is_visible", || {
            let (schema, form, values) = (schema(), form(), values());
            let mut shown = true;
            // SAFETY: a key naming no field is the failure under test.
            let status = unsafe {
                guatiao_intake_is_visible(&schema, &form, key("nonesuch"), &values, &mut shown)
            };
            assert_eq!(status, Status::GUATIAO_ERR_NOT_FOUND);
            assert!(shown, "is_visible wrote its out-parameter on a refusal");
        }),
    ]
}

#[test]
fn every_export_with_an_out_parameter_writes_it_before_failing() {
    for (_, case) in out_parameter_cases() {
        case();
    }
}

/// The cases above against the header: an export that gains an
/// out-parameter without a case here fails this test.
#[test]
#[cfg_attr(miri, ignore = "reads the committed header from disk")]
fn the_out_parameter_cases_name_every_export_that_has_one() {
    let header = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("include/guatiao_intake.h"),
    )
    .unwrap();
    // Comments first: a doc comment holds parentheses and semicolons.
    let mut code = String::new();
    let mut rest = header.as_str();
    while let Some(start) = rest.find("/*") {
        code.push_str(&rest[..start]);
        let end = rest[start..].find("*/").expect("a closed comment");
        rest = &rest[start + end + 2..];
    }
    code.push_str(rest);
    let code: String = code
        .lines()
        .map(|line| line.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let flat = code.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut declared = alloc::collections::BTreeSet::new();
    for statement in flat.split(';') {
        let Some(open) = statement.find('(') else {
            continue;
        };
        let Some(name) = statement[..open].split(' ').next_back() else {
            continue;
        };
        if !name.starts_with("guatiao_intake_") || !statement.ends_with(')') {
            continue;
        }
        let params = &statement[open + 1..statement.len() - 1];
        let has_out = params.split(',').any(|p| {
            p.trim()
                .rsplit(['*', ' '])
                .next()
                .is_some_and(|param| param.starts_with("out"))
        });
        if has_out {
            declared.insert(name.to_string());
        }
    }
    let covered: alloc::collections::BTreeSet<String> = out_parameter_cases()
        .into_iter()
        .map(|(name, _)| name.to_string())
        .collect();
    assert!(declared.len() >= 8, "the header parse found {declared:?}");
    assert_eq!(declared, covered);
}
