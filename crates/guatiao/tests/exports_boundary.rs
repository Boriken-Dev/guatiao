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
use guatiao::exports::value::{
    guatiao_list_push, guatiao_map_clear, guatiao_map_copy_from, guatiao_map_set,
    guatiao_string_push, guatiao_value_clone,
};
use guatiao::schema::{ArmBuilder, FieldBuilder, FormFieldBuilder, KindBuilder, SchemaBuilder};
use guatiao::value::convert::TryAsRef;
use guatiao::value::read::{int_or, str_or};
use guatiao::value::status::Status;
use guatiao::value::types::{Str, Tag, Text, Value};
use guatiao::{Alloc, List, Map};

/// A map of one key holding a list of strings, which is the shape every
/// merge case below needs.
fn list_map(key: &str, values: &[&str]) -> Value {
    let mut list = List::new();
    for v in values {
        list.push(Value::from(Text::new(v))).unwrap();
    }
    let mut map = Map::new();
    map.set(key, list).unwrap();
    map.into()
}

/// Whether a node is the absent marker, which is what every export
/// writes through an out-parameter before it can fail.
fn is_absent(value: &Value) -> bool {
    value.tag() == Ok(Tag::GUATIAO_ABSENT)
}

fn strings_at(value: &Value, key: &str) -> Vec<String> {
    let Some(list) = TryAsRef::<Map>::try_as_ref(value)
        .and_then(|m| m.get(key))
        .and_then(TryAsRef::<List>::try_as_ref)
    else {
        return Vec::new();
    };
    list.iter()
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
    let a = Value::from(Map::new());
    let b = Value::from(Map::new());
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
    let mut earlier = Map::new();
    let mut later = Map::new();
    for (key, values) in [("tags", &["prod"][..]), ("fallbacks", &["a"][..])] {
        let mut list = List::new();
        for v in values {
            list.push(Value::from(Text::new(v))).unwrap();
        }
        earlier.set(key, list).unwrap();
    }
    for (key, values) in [("tags", &["canary"][..]), ("fallbacks", &["b"][..])] {
        let mut list = List::new();
        for v in values {
            list.push(Value::from(Text::new(v))).unwrap();
        }
        later.set(key, list).unwrap();
    }
    let (earlier, later) = (Value::from(earlier), Value::from(later));

    let overrides = [MergeOverride {
        path: Str::new("tags"),
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
        path: Str::new("tags"),
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
    let mut inner_a = Map::new();
    inner_a.set("a", Value::from(1i64)).unwrap();
    let mut earlier = List::new();
    earlier.push(inner_a).unwrap();

    let mut inner_b = Map::new();
    inner_b.set("a", Value::from(2i64)).unwrap();
    let mut later = List::new();
    later.push(inner_b).unwrap();

    let (earlier, later) = (Value::from(earlier), Value::from(later));

    for (fields, expected) in [(0u32, 2usize), (GUATIAO_MERGE_OPT_MERGELISTS, 1usize)] {
        let mut out = Value::absent();
        // SAFETY: as above.
        let status = unsafe {
            guatiao_merge(
                GUATIAO_MERGE_DEEP,
                &earlier,
                &later,
                alloc.as_raw(),
                fields,
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
            TryAsRef::<List>::try_as_ref(&merged)
                .map(|list| &list[..])
                .unwrap_or(&[])
                .len(),
            expected,
            "mergelists {fields} folds the two records into {expected}"
        );
    }
}

/// A shape disagreement comes back as a status plus a value carrying the
/// detail a status cannot hold.
#[test]
fn a_failure_reports_its_path_through_the_error_value() {
    let alloc = Alloc::rust();
    let mut earlier = Map::new();
    earlier.set("k", List::new()).unwrap();
    let mut later = Map::new();
    later.set("k", Map::new()).unwrap();
    let (earlier, later) = (Value::from(earlier), Value::from(later));

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
        TryAsRef::<Map>::try_as_ref(&detail)
            .unwrap()
            .required("path")
            .unwrap()
            .try_into(),
        Ok("k"),
        "the status cannot say WHICH path disagreed; the value can"
    );
    assert!(
        !str_or(
            TryAsRef::<Map>::try_as_ref(&detail).and_then(|m| m.get("message")),
            ""
        )
        .is_empty(),
        "and it carries a sentence a person can read"
    );
}

/// Validation crosses the same way, and its error names the field
/// without ever quoting the value that was refused.
#[test]
fn validation_crosses_and_never_quotes_the_refused_value() {
    let alloc = Alloc::rust();
    let schema = SchemaBuilder::new_in(alloc)
        .field(
            FieldBuilder::new_in(alloc, "port", KindBuilder::int_range_in(alloc, 1, 65535))
                .required(),
        )
        .finish()
        .unwrap();

    let mut good = Map::new();
    good.set("port", Value::from(5900i64)).unwrap();
    let good = Value::from(good);
    // SAFETY: both are well-formed values.
    let status =
        unsafe { guatiao_schema_validate(&schema, &good, alloc.as_raw(), std::ptr::null_mut()) };
    assert_eq!(status, Status::GUATIAO_OK);

    let mut bad = Map::new();
    bad.set("port", Value::from(Text::new("hunter2"))).unwrap();
    let bad = Value::from(bad);
    let mut error = Value::absent();
    // SAFETY: as above, and `error` is writable.
    let status = unsafe { guatiao_schema_validate(&schema, &bad, alloc.as_raw(), &mut error) };
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);

    // An owned map, freed when this binding ends.
    let detail = error;
    assert_eq!(
        TryAsRef::<Map>::try_as_ref(&detail)
            .unwrap()
            .required("key")
            .unwrap()
            .try_into(),
        Ok("port")
    );
    let message = str_or(
        TryAsRef::<Map>::try_as_ref(&detail).and_then(|m| m.get("message")),
        "",
    );
    assert!(
        !message.contains("hunter2"),
        "a field may be sensitive, so the error says what would have been \
         accepted and never what was given: {message}"
    );
}

/// A null where a pointer was required is a status, not a crash.
#[test]
fn a_null_is_refused_rather_than_dereferenced() {
    let alloc = Alloc::rust();
    let map = Value::from(Map::new());
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
    assert_eq!(
        TryAsRef::<Map>::try_as_ref(&map)
            .map(Map::entries)
            .unwrap()
            .len(),
        0,
        "the map is emptied"
    );

    let mut list = List::new();
    list.push("x").unwrap();
    let mut list: Value = list.into();
    // SAFETY: a well-formed list — the wrong kind, which is the case
    // under test.
    let status = unsafe { guatiao_map_clear(&mut list) };
    assert_eq!(status, Status::GUATIAO_ERR_WRONG_KIND);
    assert_eq!(
        TryAsRef::<List>::try_as_ref(&list)
            .map(|list| &list[..])
            .unwrap()
            .len(),
        1,
        "a refused call leaves the value untouched"
    );

    // SAFETY: null is the other case this symbol has to survive.
    let status = unsafe { guatiao_map_clear(std::ptr::null_mut()) };
    assert_eq!(status, Status::GUATIAO_ERR_NULL);
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
    let direct = unsafe { guatiao_schema_resolve(&schema, Str::new("auth")) };
    assert!(!direct.is_null(), "the field itself resolves");

    // SAFETY: as above.
    let projected = unsafe { guatiao_schema_resolve(&schema, Str::new("auth.password")) };
    assert!(
        !projected.is_null(),
        "a projected key resolves to the arm field it names"
    );
    assert_ne!(direct, projected, "and not to the parent field");

    // SAFETY: as above.
    let nothing = unsafe { guatiao_schema_resolve(&schema, Str::new("auth.nonesuch")) };
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
        guatiao_schema_flatten(
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
        guatiao_schema_unflatten(&schema, Str::new("auth"), &flat, alloc.as_raw(), &mut back)
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
        guatiao_schema_unflatten(&schema, Str::new("auth"), &flat, alloc.as_raw(), &mut back)
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
        unsafe { guatiao_schema_flat_keys(&schema, Str::new("auth"), alloc.as_raw(), &mut keys) };
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
        assert!(guatiao_schema_resolve(std::ptr::null(), Str::new("k")).is_null());
        assert_eq!(
            guatiao_schema_flat_keys(std::ptr::null(), Str::new("auth"), alloc.as_raw(), &mut out),
            Status::GUATIAO_ERR_NULL
        );
        assert_eq!(
            guatiao_schema_flatten(
                std::ptr::null(),
                Str::new("auth"),
                &schema,
                alloc.as_raw(),
                &mut out
            ),
            Status::GUATIAO_ERR_NULL
        );
        assert_eq!(
            guatiao_schema_unflatten(
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

// --- overlapping arguments ----------------------------------------------

/// The same node as both sides of a copy is refused rather than acted on.
///
/// **Acting on it was a use-after-free.** The copy walked `src`'s entry
/// array while `dst` grew, and the first append freed the array the walk
/// was reading; the map came back with a key renamed to `""`. Copying a
/// map onto itself is also a caller mistake in its own right: every key
/// would replace itself with a copy of itself.
#[test]
fn a_map_copied_onto_itself_is_refused() {
    let alloc = Alloc::rust();
    let mut map = Map::new_in(alloc);
    map.set("a", 1).unwrap();
    map.set("b", 2).unwrap();
    let mut map: Value = map.into();

    // SAFETY: one well-formed map, passed as both arguments, which is the
    // case under test.
    let status = unsafe { guatiao_map_copy_from(alloc.as_raw(), &mut map, &map) };
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);

    let keys: Vec<String> = TryAsRef::<Map>::try_as_ref(&map)
        .map(Map::entries)
        .unwrap()
        .iter()
        .map(|e| e.key().to_string())
        .collect();
    assert_eq!(keys, ["a", "b"], "a refused call changed nothing");
}

/// A source stored INSIDE the target still copies correctly.
///
/// The shape that reproduced the use-after-free: `src` is a node the
/// growing `dst` owns, so a snapshot of its entries dangles the moment
/// `dst` reallocates. The source is copied whole before `dst` is touched,
/// which is what makes this a plain copy rather than a race.
#[test]
fn a_source_inside_the_target_copies_whole() {
    let alloc = Alloc::rust();
    let mut child = Map::new_in(alloc);
    child.set("x", 1).unwrap();
    child.set("y", 2).unwrap();

    let mut map = Map::new_in(alloc);
    map.set("child", child).unwrap();
    map.set("a", 3).unwrap();
    let mut map: Value = map.into();

    let child_ptr: *const Value = TryAsRef::<Map>::try_as_ref(&map)
        .and_then(|m| m.get("child"))
        .unwrap();
    // SAFETY: both address well-formed maps; `src` is a node `dst` owns,
    // which is the case under test.
    let status = unsafe { guatiao_map_copy_from(alloc.as_raw(), &mut map, child_ptr) };
    assert_eq!(status, Status::GUATIAO_OK);

    let keys: Vec<String> = TryAsRef::<Map>::try_as_ref(&map)
        .map(Map::entries)
        .unwrap()
        .iter()
        .map(|e| e.key().to_string())
        .collect();
    assert_eq!(
        keys,
        ["child", "a", "x", "y"],
        "the child's keys are appended and nothing is renamed"
    );
    assert_eq!(
        int_or(
            TryAsRef::<Map>::try_as_ref(&map).and_then(|m| m.get("x")),
            0
        ),
        1
    );
    assert_eq!(
        int_or(
            TryAsRef::<Map>::try_as_ref(&map).and_then(|m| m.get("y")),
            0
        ),
        2
    );
}

/// Storing a node into itself is refused: the move would read the 40
/// bytes it is writing.
#[test]
fn a_node_stored_into_itself_is_refused() {
    let alloc = Alloc::rust();
    let mut map: Value = Map::new_in(alloc).into();
    // SAFETY: one well-formed node as both arguments, the case under test.
    let status = unsafe { guatiao_map_set(alloc.as_raw(), &mut map, Str::new("k"), &mut map) };
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(
        TryAsRef::<Map>::try_as_ref(&map)
            .map(Map::entries)
            .unwrap()
            .len(),
        0,
        "nothing was stored"
    );

    let mut list: Value = List::new_in(alloc).into();
    // SAFETY: as above.
    let status = unsafe { guatiao_list_push(alloc.as_raw(), &mut list, &mut list) };
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(
        TryAsRef::<List>::try_as_ref(&list)
            .map(|list| &list[..])
            .unwrap()
            .len(),
        0,
        "nothing was appended"
    );
}

/// Appending a value's own text to itself is a copy, not a
/// use-after-free.
///
/// The source is a view of the node's own buffer, which `reserve` frees
/// the instant it grows. Only a C caller can produce this — the Rust API
/// cannot hand out a `&str` and a `&mut Value` over one node at once —
/// which is exactly why the boundary is where it has to be handled.
#[test]
fn a_string_appended_to_itself_copies_its_own_bytes() {
    let alloc = Alloc::rust();
    let mut node = Text::new_in(alloc, "abc").map(Value::from).unwrap();

    let text = TryAsRef::<str>::try_as_ref(&node).unwrap();
    let (ptr, len) = (text.as_ptr(), text.len());
    // SAFETY: the node's own bytes, readable for the call below; a C
    // caller holding a view while writing the node is the case under test.
    let own = unsafe { Str::from_raw_parts(ptr, len) };
    // SAFETY: a well-formed string, and a view of its own bytes, readable
    // for the call: the aliasing case under test.
    let status = unsafe { guatiao_string_push(alloc.as_raw(), &mut node, own) };
    assert_eq!(status, Status::GUATIAO_OK);
    assert_eq!(TryAsRef::<str>::try_as_ref(&node), Some("abcabc"));
}

// --- what a failed call leaves behind ------------------------------------

/// Every out-parameter is written ABSENT on entry, so a failure leaves
/// the caller reading what the call produced rather than its own
/// uninitialised local.
#[test]
fn a_failed_call_leaves_its_out_parameters_absent() {
    let alloc = Alloc::rust();
    let a = Value::from(Map::new());
    let b = Value::from(Map::new());

    // A mode nobody declared: the failure happens after the prologue.
    let mut out = Value::from(true);
    let mut error = Value::from(true);
    // SAFETY: every pointer addresses what its type says, and neither
    // out-parameter holds anything owned.
    let status = unsafe {
        guatiao_merge(
            0,
            &a,
            &b,
            alloc.as_raw(),
            0,
            std::ptr::null(),
            0,
            &mut out,
            &mut error,
        )
    };
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
    assert!(is_absent(&out), "the result slot is absent, not stale");
    assert!(is_absent(&error), "so is the detail slot");

    // A null pointer: the failure happens before anything is read.
    let mut out = Value::from(true);
    // SAFETY: passing null is the case under test.
    let status = unsafe {
        guatiao_merge(
            GUATIAO_MERGE_DEEP,
            std::ptr::null(),
            &b,
            alloc.as_raw(),
            0,
            std::ptr::null(),
            0,
            &mut out,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(status, Status::GUATIAO_ERR_NULL);
    assert!(is_absent(&out), "a refused call still wrote the slot");

    // The schema and value exports, the same way.
    let mut error = Value::from(true);
    // SAFETY: as above.
    let status =
        unsafe { guatiao_schema_validate(std::ptr::null(), &a, alloc.as_raw(), &mut error) };
    assert_eq!(status, Status::GUATIAO_ERR_NULL);
    assert!(is_absent(&error));

    let mut out = Value::from(true);
    // SAFETY: a null allocator is the case under test.
    let status = unsafe { guatiao_value_clone(std::ptr::null(), &a, &mut out) };
    assert_eq!(status, Status::GUATIAO_ERR_ALLOC);
    assert!(is_absent(&out));

    let mut out = Value::from(true);
    // SAFETY: as above.
    let status =
        unsafe { guatiao_schema_flat_keys(&a, Str::new("nonesuch"), alloc.as_raw(), &mut out) };
    assert_eq!(status, Status::GUATIAO_ERR_WRONG_KIND);
    assert!(is_absent(&out));
}

// --- depth ---------------------------------------------------------------

/// A schema and a value nested past the bound are refused, not followed.
///
/// **Verified as a stack overflow before the bound existed**: the walk
/// recursed once per level over two trees a caller supplied, and a stack
/// overflow on Windows is not catchable — it takes the host down rather
/// than returning a status.
#[test]
fn a_tree_nested_past_the_bound_is_refused_rather_than_followed() {
    let alloc = Alloc::rust();
    const DEPTH: usize = 300;

    // Innermost first, then wrapped outward: an object whose only field
    // is another object.
    let mut kind = KindBuilder::map_in(alloc, Vec::new());
    for _ in 0..DEPTH {
        kind = KindBuilder::map_in(alloc, vec![FieldBuilder::new_in(alloc, "next", kind)]);
    }
    let schema = SchemaBuilder::new_in(alloc)
        .field(FieldBuilder::new_in(alloc, "root", kind))
        .finish()
        .expect("a schema this size does not exhaust an allocator");

    let mut value = Map::new_in(alloc);
    for _ in 0..DEPTH {
        let mut outer = Map::new_in(alloc);
        outer.set("next", value).unwrap();
        value = outer;
    }
    let mut config = Map::new_in(alloc);
    config.set("root", value).unwrap();
    let config = Value::from(config);

    let mut error = Value::absent();
    // SAFETY: both address well-formed values, and `out_error` is
    // writable storage holding nothing owned.
    let status = unsafe { guatiao_schema_validate(&schema, &config, alloc.as_raw(), &mut error) };
    assert_eq!(
        status,
        Status::GUATIAO_ERR_BAD_VALUE,
        "too deep is a refusal, and the process is still here to say so"
    );
    assert!(!is_absent(&error), "the detail says which key and what for");
}

/// Text C passes that is not UTF-8 is refused where it arrives: the
/// export builds the view again through the constructor that checks.
///
/// Rust cannot build such a `Str`, so this writes the two fields as a C
/// caller lays them out.
#[test]
fn text_a_c_caller_passes_that_is_not_utf8_is_refused_where_it_arrives() {
    #[repr(C)]
    struct AsC {
        ptr: *const u8,
        len: usize,
    }
    let bytes = [b'h', 0xff];
    // SAFETY: `Str` is `repr(C)` with exactly these two fields, the marker
    // being zero-sized, which is the layout the C header declares.
    let text: Str<'_> = unsafe {
        std::mem::transmute(AsC {
            ptr: bytes.as_ptr(),
            len: bytes.len(),
        })
    };
    let alloc = Alloc::rust();
    let mut out = Value::null();
    // SAFETY: a readable view and a writable out-slot.
    let status =
        unsafe { guatiao::exports::value::guatiao_value_string(alloc.as_raw(), text, &mut out) };
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
}

/// The wire encoding through the C surface: a round trip, and bytes that
/// are not a value refused as `GUATIAO_ERR_BAD_VALUE` with `out` absent.
#[test]
fn a_value_crosses_the_wire_surface_and_back() {
    use guatiao::exports::value::{guatiao_wire_decode, guatiao_wire_encode};
    use guatiao::value::types::Bytes;

    let alloc = Alloc::rust();
    let mut map = Map::new();
    map.set("n", guatiao::Number::new("1.10").unwrap()).unwrap();
    map.set("s", "text").unwrap();
    let value: Value = map.into();

    let mut encoded = Value::null();
    // SAFETY: valid pointers; `encoded` holds nothing owned.
    let status = unsafe { guatiao_wire_encode(alloc.as_raw(), &value, &mut encoded) };
    assert_eq!(status, Status::GUATIAO_OK);
    let bytes = TryAsRef::<[u8]>::try_as_ref(&encoded)
        .expect("BYTES")
        .to_vec();

    let mut decoded = Value::null();
    // SAFETY: the view borrows `bytes` for the call.
    let status = unsafe { guatiao_wire_decode(alloc.as_raw(), Bytes::new(&bytes), &mut decoded) };
    assert_eq!(status, Status::GUATIAO_OK);
    assert_eq!(decoded, value, "the same tree");

    let mut refused = Value::null();
    // SAFETY: as above.
    let status = unsafe { guatiao_wire_decode(alloc.as_raw(), Bytes::new(&[9]), &mut refused) };
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(
        refused.tag(),
        Ok(Tag::GUATIAO_ABSENT),
        "a failed call leaves ABSENT"
    );
}
