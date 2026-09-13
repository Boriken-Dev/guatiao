// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The `extern "C"` surface, called the way a foreign caller calls it.
//!
//! These go through the exported functions rather than the Rust API, with
//! raw pointers and integers, because that is the half a Rust test
//! otherwise never touches: the mode arriving as a number nobody checked,
//! the override array, the null that a C caller passes by accident, and
//! the failure coming back as a status plus a value rather than as a
//! `Result`.
//!
//! Linking these from an actual C program is a separate thing, and needs
//! an artifact that exports them; this is the half that proves the
//! functions behave.

use guatiao::exports::merge::{
    GUATIAO_MERGE_DEEP, GUATIAO_MERGE_OPT_MERGELISTS, GUATIAO_MERGE_SUBSTITUTE, MergeOverride,
    guatiao_merge,
};
use guatiao::exports::schema::{
    guatiao_schema_flat_keys, guatiao_schema_flatten, guatiao_schema_resolve,
    guatiao_schema_unflatten, guatiao_schema_validate,
};
use guatiao::exports::value::guatiao_map_clear;
use guatiao::schema::{ArmBuilder, KindBuilder, OptionBuilder, SchemaBuilder};
use guatiao::value::read::{items, str_or};
use guatiao::value::status::Status;
use guatiao::value::types::{Str, Value};
use guatiao::{Alloc, List, Map, ReadValue};

/// A map of one key holding a list of strings, which is the shape every
/// merge case below needs.
fn list_map(key: &str, values: &[&str]) -> Value {
    let mut list = Value::list();
    for v in values {
        list.push(Value::string(v)).unwrap();
    }
    let mut map = Value::map();
    map.set(key, list).unwrap();
    map
}

fn strings_at(value: &Value, key: &str) -> Vec<String> {
    let Some(list) = value.get(key) else {
        return Vec::new();
    };
    items(list)
        .map(|v| str_or(Some(v), "").to_string())
        .collect()
}

/// A merged tree comes back through an out-parameter, owned by the caller,
/// and freeing it is the caller's job.
#[test]
fn a_merge_crosses_and_the_caller_owns_what_comes_back() {
    let alloc = Alloc::rust();
    let earlier = list_map("tags", &["prod", "eu"]);
    let later = list_map("tags", &["canary"]);
    let mut out = Value::absent();

    // SAFETY: every pointer addresses what its type says, and `out` is
    // writable storage holding nothing the caller owns.
    let status = unsafe {
        guatiao_merge(
            GUATIAO_MERGE_DEEP,
            &earlier,
            &later,
            alloc.as_raw(),
            0,
            std::ptr::null(),
            0,
            &mut out,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(status, Status::GUATIAO_OK);

    // The call wrote an owned tree through `out`, and it frees when
    // this binding ends: that is what the caller owning it means.
    let merged = out;
    assert_eq!(
        strings_at(&merged, "tags"),
        ["prod", "eu", "canary"],
        "Deep unions the two lists"
    );
}

/// A mode nobody declared is refused rather than transmuted.
///
/// **The reason this function takes a `uint32_t` and not an enum.**
/// Handing an integer outside the declared set to a Rust enum is
/// undefined behaviour at the moment it happens, and the far side of a
/// boundary is exactly where such an integer comes from.
#[test]
fn a_mode_nobody_declared_is_refused() {
    let alloc = Alloc::rust();
    let a = Value::map();
    let b = Value::map();
    let mut out = Value::absent();

    for bad in [0u32, 4, 99, u32::MAX] {
        // SAFETY: as above.
        let status = unsafe {
            guatiao_merge(
                bad,
                &a,
                &b,
                alloc.as_raw(),
                0,
                std::ptr::null(),
                0,
                &mut out,
                std::ptr::null_mut(),
            )
        };
        assert_eq!(
            status,
            Status::GUATIAO_ERR_BAD_VALUE,
            "mode {bad} is not one of the declared three"
        );
    }
}

/// A per-path override arrives as an array of `{path, mode}` rather than
/// as a container the caller has to build first.
#[test]
fn an_override_array_changes_the_mode_for_that_path_only() {
    let alloc = Alloc::rust();
    let mut earlier = Value::map();
    let mut later = Value::map();
    for (key, values) in [("tags", &["prod"][..]), ("fallbacks", &["a"][..])] {
        let mut list = Value::list();
        for v in values {
            list.push(Value::string(v)).unwrap();
        }
        earlier.set(key, list).unwrap();
    }
    for (key, values) in [("tags", &["canary"][..]), ("fallbacks", &["b"][..])] {
        let mut list = Value::list();
        for v in values {
            list.push(Value::string(v)).unwrap();
        }
        later.set(key, list).unwrap();
    }

    let overrides = [MergeOverride {
        path: Str::borrowed("tags"),
        mode: GUATIAO_MERGE_DEEP,
    }];
    let mut out = Value::absent();
    // SAFETY: the array holds one entry and `overrides_len` says so.
    let status = unsafe {
        guatiao_merge(
            GUATIAO_MERGE_SUBSTITUTE,
            &earlier,
            &later,
            alloc.as_raw(),
            0,
            overrides.as_ptr(),
            overrides.len(),
            &mut out,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(status, Status::GUATIAO_OK);
    // An owned tree, freed when this binding ends.
    let merged = out;
    assert_eq!(
        strings_at(&merged, "tags"),
        ["prod", "canary"],
        "the declared Deep mode unions this key"
    );
    assert_eq!(
        strings_at(&merged, "fallbacks"),
        ["b"],
        "every other key keeps the call-site mode"
    );

    // The same, with a mode nobody declared in the array.
    let bad = [MergeOverride {
        path: Str::borrowed("tags"),
        mode: 77,
    }];
    let mut out = Value::absent();
    // SAFETY: as above.
    let status = unsafe {
        guatiao_merge(
            GUATIAO_MERGE_SUBSTITUTE,
            &earlier,
            &later,
            alloc.as_raw(),
            0,
            bad.as_ptr(),
            bad.len(),
            &mut out,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
}

/// The sub-options are a bitmask, for the same reason nothing here passes
/// a `bool` by value: only two of its 256 bit patterns are valid, and a
/// boundary is where the other 254 arrive.
#[test]
fn the_mergelists_bit_reaches_the_merge() {
    let alloc = Alloc::rust();
    let mut inner_a = Value::map();
    inner_a.set("a", Value::int(1)).unwrap();
    let mut earlier = Value::list();
    earlier.push(inner_a).unwrap();

    let mut inner_b = Value::map();
    inner_b.set("a", Value::int(2)).unwrap();
    let mut later = Value::list();
    later.push(inner_b).unwrap();

    for (options, expected) in [(0u32, 2usize), (GUATIAO_MERGE_OPT_MERGELISTS, 1usize)] {
        let mut out = Value::absent();
        // SAFETY: as above.
        let status = unsafe {
            guatiao_merge(
                GUATIAO_MERGE_DEEP,
                &earlier,
                &later,
                alloc.as_raw(),
                options,
                std::ptr::null(),
                0,
                &mut out,
                std::ptr::null_mut(),
            )
        };
        assert_eq!(status, Status::GUATIAO_OK);
        // An owned tree, freed when this binding ends.
        let merged = out;
        assert_eq!(
            items(&merged).count(),
            expected,
            "mergelists {options} folds the two records into {expected}"
        );
    }
}

/// A shape disagreement comes back as a status plus a value carrying the
/// detail a status cannot hold.
#[test]
fn a_failure_reports_its_path_through_the_error_value() {
    let alloc = Alloc::rust();
    let mut earlier = Value::map();
    earlier.set("k", Value::list()).unwrap();
    let mut later = Value::map();
    later.set("k", Value::map()).unwrap();

    let mut out = Value::absent();
    let mut error = Value::absent();
    // SAFETY: both out-parameters are writable and hold nothing owned.
    let status = unsafe {
        guatiao_merge(
            GUATIAO_MERGE_DEEP,
            &earlier,
            &later,
            alloc.as_raw(),
            0,
            std::ptr::null(),
            0,
            &mut out,
            &mut error,
        )
    };
    assert_eq!(status, Status::GUATIAO_ERR_WRONG_KIND);

    // An owned map, freed when this binding ends.
    let detail = error;
    assert_eq!(
        detail.get("path").ok_or_missing().unwrap().try_into(),
        Ok("k"),
        "the status cannot say WHICH path disagreed; the value can"
    );
    assert!(
        !str_or(detail.get("message"), "").is_empty(),
        "and it carries a sentence a person can read"
    );
}

/// Validation crosses the same way, and its error names the option
/// without ever quoting the value that was refused.
#[test]
fn validation_crosses_and_never_quotes_the_refused_value() {
    let alloc = Alloc::rust();
    let schema = SchemaBuilder::new(alloc)
        .option(
            OptionBuilder::new(alloc, "port", KindBuilder::int_range(alloc, 1, 65535)).required(),
        )
        .finish()
        .unwrap();

    let mut good = Value::map();
    good.set("port", Value::int(5900)).unwrap();
    // SAFETY: both are well-formed values.
    let status =
        unsafe { guatiao_schema_validate(&schema, &good, alloc.as_raw(), std::ptr::null_mut()) };
    assert_eq!(status, Status::GUATIAO_OK);

    let mut bad = Value::map();
    bad.set("port", Value::string("hunter2")).unwrap();
    let mut error = Value::absent();
    // SAFETY: as above, and `error` is writable.
    let status = unsafe { guatiao_schema_validate(&schema, &bad, alloc.as_raw(), &mut error) };
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);

    // An owned map, freed when this binding ends.
    let detail = error;
    assert_eq!(
        detail.get("key").ok_or_missing().unwrap().try_into(),
        Ok("port")
    );
    let message = str_or(detail.get("message"), "");
    assert!(
        !message.contains("hunter2"),
        "an option may be sensitive, so the error says what would have been \
         accepted and never what was given: {message}"
    );
}

/// A null where a pointer was required is a status, not a crash.
#[test]
fn a_null_is_refused_rather_than_dereferenced() {
    let alloc = Alloc::rust();
    let map = Value::map();
    let mut out = Value::absent();

    // SAFETY: passing null is the case under test; every other pointer is
    // valid.
    let status = unsafe {
        guatiao_merge(
            GUATIAO_MERGE_DEEP,
            std::ptr::null(),
            &map,
            alloc.as_raw(),
            0,
            std::ptr::null(),
            0,
            &mut out,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(status, Status::GUATIAO_ERR_NULL);

    // SAFETY: as above.
    let status = unsafe {
        guatiao_schema_validate(std::ptr::null(), &map, alloc.as_raw(), std::ptr::null_mut())
    };
    assert_eq!(status, Status::GUATIAO_ERR_NULL);
}

/// `guatiao_map_clear` is the MAP one, and a list handed to it is refused.
///
/// The Rust `Value::clear` dispatches on the tag and empties either kind,
/// which is right for Rust and wrong for a symbol named after one of
/// them: a caller reaching for `guatiao_map_clear` is saying it believes
/// the node is a map, and the boundary is where that belief gets checked
/// rather than acted on.
#[test]
fn the_map_clear_symbol_refuses_a_list() {
    let mut map = Map::new();
    map.set("k", 1).unwrap();
    let mut map: Value = map.into();
    // SAFETY: a well-formed map, and the pointer is valid for the call.
    let status = unsafe { guatiao_map_clear(&mut map) };
    assert_eq!(status, Status::GUATIAO_OK);
    assert_eq!(map.entries().unwrap().len(), 0, "the map is emptied");

    let mut list = List::new();
    list.push("x").unwrap();
    let mut list: Value = list.into();
    // SAFETY: a well-formed list — the wrong kind, which is the case
    // under test.
    let status = unsafe { guatiao_map_clear(&mut list) };
    assert_eq!(status, Status::GUATIAO_ERR_WRONG_KIND);
    assert_eq!(
        list.items().unwrap().len(),
        1,
        "a refused call leaves the value untouched"
    );

    // SAFETY: null is the other case this symbol has to survive.
    let status = unsafe { guatiao_map_clear(std::ptr::null_mut()) };
    assert_eq!(status, Status::GUATIAO_ERR_NULL);
}

// --- the flat projection ------------------------------------------------

/// A schema with a variant option, which is the only shape the projection
/// applies to.
fn variant_schema() -> Value {
    let alloc = Alloc::rust();
    SchemaBuilder::new(alloc)
        .option(OptionBuilder::new(
            alloc,
            "auth",
            KindBuilder::variant(
                alloc,
                "auth",
                vec![
                    ArmBuilder::new(alloc, "sso", "Single sign-on"),
                    ArmBuilder::new(alloc, "userpass", "Username and password")
                        .field(OptionBuilder::new(
                            alloc,
                            "username",
                            KindBuilder::string(alloc),
                        ))
                        .field(
                            OptionBuilder::new(alloc, "password", KindBuilder::string(alloc))
                                .sensitive(),
                        ),
                ],
            ),
        ))
        .finish()
        .expect("a schema this small does not exhaust an allocator")
}

/// A flat key resolves through one level of projection, and the option it
/// lands on is the ARM FIELD's, not the parent's.
#[test]
fn a_flat_key_resolves_to_the_option_that_governs_it() {
    let schema = variant_schema();

    // SAFETY: a well-formed schema and a readable view.
    let direct = unsafe { guatiao_schema_resolve(&schema, Str::borrowed("auth")) };
    assert!(!direct.is_null(), "the option itself resolves");

    // SAFETY: as above.
    let projected = unsafe { guatiao_schema_resolve(&schema, Str::borrowed("auth.password")) };
    assert!(
        !projected.is_null(),
        "a projected key resolves to the arm field it names"
    );
    assert_ne!(direct, projected, "and not to the parent option");

    // SAFETY: as above.
    let nothing = unsafe { guatiao_schema_resolve(&schema, Str::borrowed("auth.nonesuch")) };
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

    // SAFETY: a well-formed schema.
    let option = unsafe { guatiao_schema_resolve(&schema, Str::borrowed("auth")) };
    assert!(!option.is_null());

    let mut chosen = Map::new();
    chosen.set("auth", "userpass").unwrap();
    chosen.set("username", "ana").unwrap();
    chosen.set("password", "hunter2").unwrap();
    let chosen: Value = chosen.into();

    let mut flat = Value::absent();
    // SAFETY: every pointer addresses what its type says.
    let status = unsafe { guatiao_schema_flatten(option, &chosen, alloc.as_raw(), &mut flat) };
    assert_eq!(status, Status::GUATIAO_OK);

    assert_eq!(
        flat.get("auth").and_then(Value::as_str),
        Some("userpass"),
        "the tag crosses as text"
    );
    assert_eq!(
        flat.get("auth.password").and_then(Value::as_str),
        Some("hunter2"),
        "and the arm's fields are projected under it"
    );

    let mut back = Value::absent();
    // SAFETY: as above; `flat` is a map whose values are all strings.
    let status = unsafe { guatiao_schema_unflatten(option, &flat, alloc.as_raw(), &mut back) };
    assert_eq!(status, Status::GUATIAO_OK);
    assert_eq!(back.get("auth").and_then(Value::as_str), Some("userpass"));
    assert_eq!(back.get("username").and_then(Value::as_str), Some("ana"));
    assert_eq!(
        back.get("password").and_then(Value::as_str),
        Some("hunter2")
    );
}

/// A store holding anything but text is refused rather than stringified.
#[test]
fn a_flat_store_that_is_not_all_text_is_refused() {
    let alloc = Alloc::rust();
    let schema = variant_schema();
    // SAFETY: a well-formed schema.
    let option = unsafe { guatiao_schema_resolve(&schema, Str::borrowed("auth")) };

    let mut flat = Map::new();
    flat.set("auth", "userpass").unwrap();
    // A number, where the projection's contract says text.
    flat.set("username", 5900).unwrap();
    let flat: Value = flat.into();

    let mut back = Value::absent();
    // SAFETY: as above.
    let status = unsafe { guatiao_schema_unflatten(option, &flat, alloc.as_raw(), &mut back) };
    assert_eq!(
        status,
        Status::GUATIAO_ERR_WRONG_KIND,
        "a flat store holds text by definition, so a number in one is a \
         mistake worth hearing about at the boundary rather than two layers in"
    );
}

/// The keys an option projects onto, for a renderer laying out a form.
#[test]
fn the_flat_keys_of_an_option_are_listed() {
    let alloc = Alloc::rust();
    let schema = variant_schema();
    // SAFETY: a well-formed schema.
    let option = unsafe { guatiao_schema_resolve(&schema, Str::borrowed("auth")) };

    let mut keys = Value::absent();
    // SAFETY: a well-formed option and writable storage.
    let status = unsafe { guatiao_schema_flat_keys(option, alloc.as_raw(), &mut keys) };
    assert_eq!(status, Status::GUATIAO_OK);

    let listed: Vec<&str> = keys
        .items()
        .expect("a list")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(
        listed,
        ["auth", "auth.username", "auth.password"],
        "the option's own key first, then one per arm field, in declaration \
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
        assert!(guatiao_schema_resolve(std::ptr::null(), Str::borrowed("k")).is_null());
        assert_eq!(
            guatiao_schema_flat_keys(std::ptr::null(), alloc.as_raw(), &mut out),
            Status::GUATIAO_ERR_NULL
        );
        assert_eq!(
            guatiao_schema_flatten(std::ptr::null(), &schema, alloc.as_raw(), &mut out),
            Status::GUATIAO_ERR_NULL
        );
        assert_eq!(
            guatiao_schema_unflatten(std::ptr::null(), &schema, alloc.as_raw(), &mut out),
            Status::GUATIAO_ERR_NULL
        );
    }
}
