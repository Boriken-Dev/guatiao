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

extern crate alloc;

use guatiao::exports::merge::{
    GUATIAO_MERGE_DEEP, GUATIAO_MERGE_OPT_MERGELISTS, GUATIAO_MERGE_SUBSTITUTE, MergeOverride,
    guatiao_merge,
};
use guatiao::exports::schema::guatiao_schema_validate;
use guatiao::exports::value::{
    guatiao_list_push, guatiao_map_clear, guatiao_map_copy_from, guatiao_map_set,
    guatiao_string_push, guatiao_value_clone,
};
use guatiao::schema::{FieldBuilder, KindBuilder, SchemaBuilder};
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
        list.push(Value::from(Text::new(v)));
    }
    let mut map = Map::new();
    map.set(key, list);
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
            core::ptr::null(),
            0,
            &mut out,
            core::ptr::null_mut(),
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
                core::ptr::null(),
                0,
                &mut out,
                core::ptr::null_mut(),
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
            list.push(Value::from(Text::new(v)));
        }
        earlier.set(key, list);
    }
    for (key, values) in [("tags", &["canary"][..]), ("fallbacks", &["b"][..])] {
        let mut list = List::new();
        for v in values {
            list.push(Value::from(Text::new(v)));
        }
        later.set(key, list);
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
            core::ptr::null_mut(),
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
            core::ptr::null_mut(),
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
    inner_a.set("a", Value::from(1i64));
    let mut earlier = List::new();
    earlier.push(inner_a);

    let mut inner_b = Map::new();
    inner_b.set("a", Value::from(2i64));
    let mut later = List::new();
    later.push(inner_b);

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
                core::ptr::null(),
                0,
                &mut out,
                core::ptr::null_mut(),
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
    earlier.set("k", List::new());
    let mut later = Map::new();
    later.set("k", Map::new());
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
            core::ptr::null(),
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
    good.set("port", Value::from(5900i64));
    let good = Value::from(good);
    // SAFETY: both are well-formed values.
    let status =
        unsafe { guatiao_schema_validate(&schema, &good, alloc.as_raw(), core::ptr::null_mut()) };
    assert_eq!(status, Status::GUATIAO_OK);

    let mut bad = Map::new();
    bad.set("port", Value::from(Text::new("hunter2")));
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
            core::ptr::null(),
            &map,
            alloc.as_raw(),
            0,
            core::ptr::null(),
            0,
            &mut out,
            core::ptr::null_mut(),
        )
    };
    assert_eq!(status, Status::GUATIAO_ERR_NULL);

    // SAFETY: as above.
    let status = unsafe {
        guatiao_schema_validate(
            core::ptr::null(),
            &map,
            alloc.as_raw(),
            core::ptr::null_mut(),
        )
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
    map.set("k", 1);
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
    list.push("x");
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
    let status = unsafe { guatiao_map_clear(core::ptr::null_mut()) };
    assert_eq!(status, Status::GUATIAO_ERR_NULL);
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
    map.set("a", 1);
    map.set("b", 2);
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
    child.set("x", 1);
    child.set("y", 2);

    let mut map = Map::new_in(alloc);
    map.set("child", child);
    map.set("a", 3);
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
            core::ptr::null(),
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
            core::ptr::null(),
            &b,
            alloc.as_raw(),
            0,
            core::ptr::null(),
            0,
            &mut out,
            core::ptr::null_mut(),
        )
    };
    assert_eq!(status, Status::GUATIAO_ERR_NULL);
    assert!(is_absent(&out), "a refused call still wrote the slot");

    // The schema and value exports, the same way.
    let mut error = Value::from(true);
    // SAFETY: as above.
    let status =
        unsafe { guatiao_schema_validate(core::ptr::null(), &a, alloc.as_raw(), &mut error) };
    assert_eq!(status, Status::GUATIAO_ERR_NULL);
    assert!(is_absent(&error));

    let mut out = Value::from(true);
    // SAFETY: a null allocator is the case under test.
    let status = unsafe { guatiao_value_clone(core::ptr::null(), &a, &mut out) };
    assert_eq!(status, Status::GUATIAO_ERR_ALLOC);
    assert!(is_absent(&out));
}

/// Bytes that are not UTF-8, as a C caller can pass them: Rust cannot
/// build such a `Str`, so this writes the two fields as C lays them out.
fn not_utf8() -> Str<'static> {
    static BYTES: [u8; 2] = [b'h', 0xff];
    #[repr(C)]
    struct AsC {
        ptr: *const u8,
        len: usize,
    }
    // SAFETY: `Str` is `repr(C)` with exactly these two fields, the marker
    // being zero-sized, which is the layout the C header declares.
    unsafe {
        core::mem::transmute::<AsC, Str<'static>>(AsC {
            ptr: BYTES.as_ptr(),
            len: BYTES.len(),
        })
    }
}

/// An out-slot holding something that owns nothing, so a test can tell
/// "written" from "left as it was" without leaking.
fn stale() -> Value {
    Value::from(true)
}

/// Runs `call` with a stale out-slot and checks it answered `status` and
/// left the slot ABSENT.
fn leaves_absent(name: &str, status: Status, call: impl FnOnce(*mut Value) -> Status) {
    let mut out = stale();
    assert_eq!(call(&mut out), status, "{name}");
    assert!(is_absent(&out), "{name} left its out-parameter as it was");
}

/// Every export of this crate with an out-parameter, called so that it
/// fails after its prologue, and checked to have written the slot. The
/// names are what the coverage test compares with the header.
fn out_parameter_cases() -> Vec<(&'static str, fn())> {
    use guatiao::exports::value::*;
    let mut cases: Vec<(&'static str, fn())> = vec![
        ("guatiao_value_clone", || {
            let src = Value::null();
            // SAFETY: a null allocator is the failure under test.
            leaves_absent("clone", Status::GUATIAO_ERR_ALLOC, |out| unsafe {
                guatiao_value_clone(core::ptr::null(), &src, out)
            });
        }),
        // The three that need no allocator fail only on a null `out`,
        // which leaves nothing to write.
        ("guatiao_value_null", || {
            // SAFETY: null is the case under test.
            let status = unsafe { guatiao_value_null(core::ptr::null_mut()) };
            assert_eq!(status, Status::GUATIAO_ERR_NULL);
        }),
        ("guatiao_value_absent", || {
            // SAFETY: as above.
            let status = unsafe { guatiao_value_absent(core::ptr::null_mut()) };
            assert_eq!(status, Status::GUATIAO_ERR_NULL);
        }),
        ("guatiao_value_bool", || {
            // SAFETY: as above.
            let status = unsafe { guatiao_value_bool(true, core::ptr::null_mut()) };
            assert_eq!(status, Status::GUATIAO_ERR_NULL);
        }),
        ("guatiao_value_map", || {
            // SAFETY: a null allocator is the failure under test.
            leaves_absent("map", Status::GUATIAO_ERR_ALLOC, |out| unsafe {
                guatiao_value_map(core::ptr::null(), out)
            });
        }),
        ("guatiao_value_list", || {
            // SAFETY: as above.
            leaves_absent("list", Status::GUATIAO_ERR_ALLOC, |out| unsafe {
                guatiao_value_list(core::ptr::null(), out)
            });
        }),
        ("guatiao_value_string", || {
            let alloc = Alloc::rust();
            // SAFETY: a readable view; its content is the failure.
            leaves_absent("string", Status::GUATIAO_ERR_BAD_VALUE, |out| unsafe {
                guatiao_value_string(alloc.as_raw(), not_utf8(), out)
            });
        }),
        ("guatiao_value_number", || {
            let alloc = Alloc::rust();
            // SAFETY: as above.
            leaves_absent("number", Status::GUATIAO_ERR_BAD_VALUE, |out| unsafe {
                guatiao_value_number(alloc.as_raw(), Str::new("not a number"), out)
            });
        }),
        ("guatiao_value_bytes", || {
            // SAFETY: a null allocator is the failure under test.
            leaves_absent("bytes", Status::GUATIAO_ERR_ALLOC, |out| unsafe {
                guatiao_value_bytes(
                    core::ptr::null(),
                    guatiao::value::types::Bytes::new(b"x"),
                    out,
                )
            });
        }),
        ("guatiao_alloc_default", || {
            // Its one failure is a null `out`.
            // SAFETY: null is the case under test.
            let status = unsafe { guatiao_alloc_default(core::ptr::null_mut()) };
            assert_eq!(status, Status::GUATIAO_ERR_NULL);
        }),
        ("guatiao_wire_encode", || {
            let value = Value::null();
            // SAFETY: a null allocator is the failure under test.
            leaves_absent("wire_encode", Status::GUATIAO_ERR_ALLOC, |out| unsafe {
                guatiao_wire_encode(core::ptr::null(), &value, out)
            });
        }),
        ("guatiao_wire_decode", || {
            let alloc = Alloc::rust();
            // SAFETY: readable bytes that are not a value.
            leaves_absent("wire_decode", Status::GUATIAO_ERR_BAD_VALUE, |out| unsafe {
                guatiao_wire_decode(
                    alloc.as_raw(),
                    guatiao::value::types::Bytes::new(b"\xff"),
                    out,
                )
            });
        }),
        ("guatiao_merge", || {
            let (a, b) = (Value::from(Map::new()), Value::from(Map::new()));
            let mut error = stale();
            // SAFETY: an undeclared mode is the failure under test.
            leaves_absent("merge", Status::GUATIAO_ERR_BAD_VALUE, |out| unsafe {
                guatiao_merge(
                    0,
                    &a,
                    &b,
                    Alloc::rust().as_raw(),
                    0,
                    core::ptr::null(),
                    0,
                    out,
                    &mut error,
                )
            });
            assert!(is_absent(&error), "merge's error slot");
        }),
        ("guatiao_schema_validate", || {
            let config = Value::from(Map::new());
            // SAFETY: a null schema is the failure under test.
            leaves_absent("schema_validate", Status::GUATIAO_ERR_NULL, |out| unsafe {
                guatiao_schema_validate(core::ptr::null(), &config, Alloc::rust().as_raw(), out)
            });
        }),
    ];
    #[cfg(feature = "load")]
    cases.extend(registry_cases());
    cases
}

/// The registry's exports: a null handle fails after the prologue.
#[cfg(feature = "load")]
fn registry_cases() -> Vec<(&'static str, fn())> {
    use guatiao::exports::library::*;
    fn reg() -> *mut HostRegistry {
        core::ptr::null_mut()
    }
    fn a() -> *const guatiao::value::alloc::Allocator {
        Alloc::rust().as_raw()
    }
    const NULL: Status = Status::GUATIAO_ERR_NULL;
    vec![
        ("guatiao_registry_provider_create", || {
            let config = Value::from(Map::new());
            let mut instance = core::ptr::NonNull::<core::ffi::c_void>::dangling().as_ptr();
            let mut err = guatiao::library::ProviderError::new(Status::GUATIAO_ERR_BUSY, "");
            // SAFETY: a null handle is the failure under test.
            let status = unsafe {
                guatiao_registry_provider_create(
                    reg(),
                    Str::new("k"),
                    &config,
                    &mut instance,
                    &mut err,
                )
            };
            assert_eq!(status, NULL);
            assert!(
                instance.is_null(),
                "create left its instance slot as it was"
            );
            assert_eq!(
                err.status,
                Status::GUATIAO_OK,
                "create left its error slot as it was"
            );
            assert!(err.message().is_empty());
        }),
        ("guatiao_registry_load_file", || {
            // SAFETY: a null handle is the failure under test.
            leaves_absent("load_file", NULL, |out| unsafe {
                guatiao_registry_load_file(reg(), Str::new("x"), a(), out)
            });
        }),
        ("guatiao_registry_register_entry", || {
            // SAFETY: as above.
            leaves_absent("register_entry", NULL, |out| unsafe {
                guatiao_registry_register_entry(reg(), Str::new("x"), None, a(), out)
            });
        }),
        ("guatiao_registry_scan_dir", || {
            // SAFETY: as above.
            leaves_absent("scan_dir", NULL, |out| unsafe {
                guatiao_registry_scan_dir(reg(), Str::new("x"), false, a(), out)
            });
        }),
        ("guatiao_registry_scan_dir_rules", || {
            // SAFETY: as above.
            leaves_absent("scan_dir_rules", NULL, |out| unsafe {
                guatiao_registry_scan_dir_rules(reg(), Str::new("x"), false, Str::new(""), a(), out)
            });
        }),
        ("guatiao_registry_scan_path", || {
            // SAFETY: as above.
            leaves_absent("scan_path", NULL, |out| unsafe {
                guatiao_registry_scan_path(reg(), Str::new("x"), false, Str::new(""), a(), out)
            });
        }),
        ("guatiao_registry_libraries", || {
            // SAFETY: as above.
            leaves_absent("libraries", NULL, |out| unsafe {
                guatiao_registry_libraries(reg(), a(), out)
            });
        }),
        ("guatiao_registry_providers", || {
            // SAFETY: as above.
            leaves_absent("providers", NULL, |out| unsafe {
                guatiao_registry_providers(reg(), Str::new(""), a(), out)
            });
        }),
        ("guatiao_registry_provider", || {
            // SAFETY: as above.
            leaves_absent("provider", NULL, |out| unsafe {
                guatiao_registry_provider(reg(), Str::new("k"), a(), out)
            });
        }),
        ("guatiao_registry_available", || {
            // SAFETY: as above.
            leaves_absent("available", NULL, |out| unsafe {
                guatiao_registry_available(reg(), Str::new(""), a(), out)
            });
        }),
        ("guatiao_registry_why_not", || {
            // SAFETY: as above.
            leaves_absent("why_not", NULL, |out| unsafe {
                guatiao_registry_why_not(reg(), Str::new(""), a(), out)
            });
        }),
        ("guatiao_registry_best", || {
            // SAFETY: as above.
            leaves_absent("best", NULL, |out| unsafe {
                guatiao_registry_best(reg(), Str::new(""), a(), out)
            });
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
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("include/guatiao.h"),
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
        if !name.starts_with("guatiao_") || !statement.ends_with(')') {
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
    assert!(declared.len() > 20, "the header parse found {declared:?}");
    assert_eq!(declared, covered);
}

/// A registry export reports what is wrong with each argument by the
/// status that names it, not as a null.
#[cfg(feature = "load")]
#[test]
fn a_registry_argument_is_refused_by_what_is_wrong_with_it() {
    use guatiao::exports::library::{
        guatiao_registry_free, guatiao_registry_load_file, guatiao_registry_new,
    };
    // SAFETY: two readable views and no allocator.
    let reg = unsafe { guatiao_registry_new(Str::new("host"), Str::new("1"), core::ptr::null()) };
    assert!(!reg.is_null());
    let alloc = Alloc::rust();
    // SAFETY: a live handle; the path's content is the failure.
    leaves_absent(
        "non-UTF-8 path",
        Status::GUATIAO_ERR_BAD_VALUE,
        |out| unsafe { guatiao_registry_load_file(reg, not_utf8(), alloc.as_raw(), out) },
    );
    // SAFETY: a live handle; the null allocator is the failure.
    leaves_absent("null allocator", Status::GUATIAO_ERR_ALLOC, |out| unsafe {
        guatiao_registry_load_file(reg, Str::new("x"), core::ptr::null(), out)
    });
    // SAFETY: the handle from `guatiao_registry_new`, freed once.
    unsafe { guatiao_registry_free(reg) };
}

// --- statuses ------------------------------------------------------------

/// The kind is checked before the content: the wrong kind is
/// `GUATIAO_ERR_WRONG_KIND`, and the right kind holding text that is not
/// UTF-8 is `GUATIAO_ERR_BAD_VALUE`.
#[test]
fn the_wrong_kind_and_bad_content_are_told_apart() {
    use guatiao::exports::value::{guatiao_list_discard, guatiao_map_discard};
    static BAD: [u8; 2] = [0xff, 0xfe];
    let bad_string = || {
        // SAFETY: memory shape right (a borrowed, readable buffer); only
        // the content is wrong, which is what a reader must refuse.
        unsafe {
            Value::from_raw_parts(
                u32::from(Tag::GUATIAO_STRING),
                guatiao::value::types::Payload::text(Text::from_raw_parts(
                    BAD.as_ptr().cast_mut(),
                    2,
                    0,
                    core::ptr::null(),
                )),
            )
        }
    };
    let alloc = Alloc::rust();
    let mut list = Value::from(List::new());
    let mut map = Value::from(Map::new());
    // SAFETY: every pointer addresses a well-formed value.
    unsafe {
        assert_eq!(
            guatiao_map_discard(&mut list, Str::new("a")),
            Status::GUATIAO_ERR_WRONG_KIND
        );
        assert_eq!(
            guatiao_map_discard(&mut map, Str::new("a")),
            Status::GUATIAO_ERR_NOT_FOUND
        );
        assert_eq!(
            guatiao_map_discard(&mut map, not_utf8()),
            Status::GUATIAO_ERR_BAD_VALUE
        );
        assert_eq!(
            guatiao_list_discard(&mut map, 0),
            Status::GUATIAO_ERR_WRONG_KIND
        );
        assert_eq!(
            guatiao_list_discard(&mut list, 0),
            Status::GUATIAO_ERR_NOT_FOUND
        );

        let mut number = Value::from(5i64);
        assert_eq!(
            guatiao_string_push(alloc.as_raw(), &mut number, Str::new("x")),
            Status::GUATIAO_ERR_WRONG_KIND
        );
        let mut bad = bad_string();
        assert_eq!(
            guatiao_string_push(alloc.as_raw(), &mut bad, Str::new("x")),
            Status::GUATIAO_ERR_BAD_VALUE
        );
    }

    let bad = bad_string();
    let refused = <&str>::try_from(&bad).unwrap_err();
    assert!(
        matches!(refused, guatiao::MapError::BadValue { .. }),
        "{refused:?}"
    );
    let refused = <&str>::try_from(&Value::from(5i64)).unwrap_err();
    assert!(
        matches!(refused, guatiao::MapError::WrongType { .. }),
        "{refused:?}"
    );
}

/// A store refused for want of memory leaves the caller's value where it
/// was: `GUATIAO_ERR_ALLOC` means nothing was written, and nothing taken.
#[test]
fn a_refused_store_leaves_the_callers_value_alone() {
    use core::ffi::c_void;
    use guatiao::exports::value::{guatiao_value_list, guatiao_value_map};
    unsafe extern "C" fn refuse(_: *mut c_void, _: usize, _: usize) -> *mut c_void {
        core::ptr::null_mut()
    }
    unsafe extern "C" fn release(_: *mut c_void, _: *mut c_void, _: usize, _: usize) {}
    let mut full = guatiao::value::alloc::rust_alloc();
    full.alloc = Some(refuse);
    full.free = Some(release);

    let mut map = Value::null();
    let mut list = Value::null();
    let mut value = Value::from("keep me");
    // SAFETY: every pointer addresses a well-formed value, and the
    // allocator outlives the call and every container recording it.
    unsafe {
        assert!(guatiao_value_map(&full, &mut map).is_ok());
        assert!(guatiao_value_list(&full, &mut list).is_ok());
        assert_eq!(
            guatiao_map_set(&full, &mut map, Str::new("k"), &mut value),
            Status::GUATIAO_ERR_ALLOC
        );
        assert_eq!(
            TryAsRef::<str>::try_as_ref(&value),
            Some("keep me"),
            "map_set took it"
        );
        assert_eq!(
            guatiao_list_push(&full, &mut list, &mut value),
            Status::GUATIAO_ERR_ALLOC
        );
        assert_eq!(
            TryAsRef::<str>::try_as_ref(&value),
            Some("keep me"),
            "list_push took it"
        );
    }
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
        outer.set("next", value);
        value = outer;
    }
    let mut config = Map::new_in(alloc);
    config.set("root", value);
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
        core::mem::transmute(AsC {
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
    map.set("n", guatiao::Number::new("1.10").unwrap());
    map.set("s", "text");
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
