// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What each merge mode does, and what provenance says afterwards.
//!
//! Beside the implementation rather than in `tests/`: these reach the
//! private helpers the modes are built from, and a behaviour test that
//! can only see the public surface cannot pin which of them was wrong.

use super::*;
use crate::value::convert::TryAsRef;

use crate::value::alloc::Alloc;
use crate::value::read::{int_or, str_or};
use crate::value::types::{Buffer, List, Map, Text};

/// The allocator a merge builds its result through. The values handed to
/// it need none — they are already on Rust's heap.
fn alloc() -> Alloc {
    Alloc::rust()
}

/// Builds a map from `(key, value)` pairs.
fn m(pairs: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    let mut built = Map::new();
    for (key, value) in pairs {
        built.set(key, value).expect("a test map is small");
    }
    built.into()
}

/// Builds a list from values.
fn l(values: impl IntoIterator<Item = Value>) -> Value {
    let mut built = List::new();
    for value in values {
        built.push(value).expect("a test list is small");
    }
    built.into()
}

/// A string value, for brevity in the cases below.
fn s(text: &str) -> Value {
    Value::from(Text::new(text))
}

/// A number value from an integer, for brevity in the cases below.
/// Numbers are stored as text, so `n(1)` is the value spelled `"1"`;
/// merging never looks inside one.
fn n(value: i64) -> Value {
    Value::from(value)
}

/// The stored nothing.
fn null() -> Value {
    Value::null()
}

/// A boolean value.
fn b(value: bool) -> Value {
    Value::from(value)
}

/// The absent sentinel, which the C form can store in a container and the
/// model this replaced could not.
fn absent() -> Value {
    Value::absent()
}

/// A merge with the default options and no overrides.
fn merge(mode: MergeMode, earlier: &Value, later: &Value) -> Result<Value, MergeError> {
    mode.merge(earlier, later, alloc())
}

/// A merge with the sub-options and overrides spelled out.
fn merge_with(
    mode: MergeMode,
    earlier: &Value,
    later: &Value,
    options: MergeOptions,
    overrides: Option<&MergeOverrides>,
) -> Result<Value, MergeError> {
    mode.merge_with(earlier, later, alloc(), options, overrides)
}

/// Structural equality: order-significant for a map, byte-exact for a
/// Structural equality, which is what a merge's result is judged on.
fn same(left: &Value, right: &Value) -> bool {
    left == right
}

/// The integer under `key`, or zero.
fn int_at(value: &Value, key: &str) -> i64 {
    int_or(
        TryAsRef::<Map>::try_as_ref(value).and_then(|m| m.get(key)),
        0,
    )
}

/// The text under `key`, or the empty string.
fn str_at<'v>(value: &'v Value, key: &str) -> &'v str {
    str_or(
        TryAsRef::<Map>::try_as_ref(value).and_then(|m| m.get(key)),
        "",
    )
}

/// How many entries a map has.
fn len_of(value: &Value) -> usize {
    TryAsRef::<Map>::try_as_ref(value)
        .map(Map::entries)
        .unwrap_or(&[])
        .len()
}

/// The elements of a list, for comparison.
fn elements(value: &Value) -> Vec<&Value> {
    TryAsRef::<List>::try_as_ref(value)
        .map(|list| &list[..])
        .unwrap_or(&[])
        .iter()
        .collect()
}

/// Asserts a merge produced exactly `expected`, printing both when it did
/// not.
macro_rules! assert_merges_to {
    ($actual:expr, $expected:expr $(,)?) => {{
        let actual = $actual;
        let expected = $expected;
        match &actual {
            Ok(got) => assert!(
                same(got, &expected),
                "merged {:?}\n but expected {:?}",
                &got,
                &expected
            ),
            Err(e) => panic!("expected a merge, got {e}"),
        }
    }};
}

/// A case-for-case port of the reference suite, which is the closest
/// thing to a written specification of the three strategies: the
/// positional list rule, the `mergelists` overlap condition and the
/// map-absorbs-list-of-maps behaviour are pinned only here. Each test
/// names its reference so the two can be diffed by hand.
mod port_of_the_reference_suite {
    use super::*;

    // ------------------------------------------------------------------
    // Simple  (reference: TestSimpleMerge)
    // ------------------------------------------------------------------

    /// Reference: `test_scalar_replaces`.
    #[test]
    fn simple_scalar_replaces() {
        assert_merges_to!(merge(MergeMode::Simple, &n(1), &n(2)), n(2));
    }

    /// Reference: `test_dict_update_shallow`.
    #[test]
    fn simple_dict_update_shallow() {
        let earlier = m([("x", n(1)), ("y", n(2))]);
        let later = m([("y", n(99)), ("z", n(3))]);
        let merged = merge(MergeMode::Simple, &earlier, &later).unwrap();
        assert_eq!(int_at(&merged, "x"), 1);
        assert_eq!(int_at(&merged, "y"), 99);
        assert_eq!(int_at(&merged, "z"), 3);
        assert_eq!(len_of(&merged), 3);
    }

    /// Reference: `test_dict_replaces_scalar`.
    #[test]
    fn simple_dict_replaces_scalar() {
        let later = m([("k", s("v"))]);
        let merged = merge(MergeMode::Simple, &s("old"), &later).unwrap();
        assert_eq!(str_at(&merged, "k"), "v");
    }

    /// Reference: `test_list_element_replace` — `[1,2,3]` updated with
    /// `[10,20]` is `[10,20,3]`. The element that SURVIVES is the point:
    /// `Simple` cannot shorten a list.
    #[test]
    fn simple_list_element_replace() {
        let earlier = l([n(1), n(2), n(3)]);
        let later = l([n(10), n(20)]);
        assert_merges_to!(
            merge(MergeMode::Simple, &earlier, &later),
            l([n(10), n(20), n(3)])
        );
    }

    /// Reference: `test_list_extends_when_b_longer`.
    #[test]
    fn simple_list_extends_when_later_is_longer() {
        let earlier = l([n(1)]);
        let later = l([n(10), n(20), n(30)]);
        assert_merges_to!(
            merge(MergeMode::Simple, &earlier, &later),
            l([n(10), n(20), n(30)])
        );
    }

    /// Reference: `test_list_replaces_scalar`.
    #[test]
    fn simple_list_replaces_scalar() {
        let later = l([n(1), n(2)]);
        assert_merges_to!(merge(MergeMode::Simple, &s("old"), &later), l([n(1), n(2)]));
    }

    // ------------------------------------------------------------------
    // Substitute  (reference: TestSubstituteMerge)
    // ------------------------------------------------------------------

    /// Reference: `test_scalar_replaces`.
    #[test]
    fn substitute_scalar_replaces() {
        assert_merges_to!(merge(MergeMode::Substitute, &s("a"), &s("b")), s("b"));
    }

    /// Reference: `test_list_replaces` — wholesale, unlike `Simple`'s
    /// positional rule.
    #[test]
    fn substitute_list_replaces() {
        let earlier = l([n(1), n(2)]);
        let later = l([n(3), n(4)]);
        assert_merges_to!(
            merge(MergeMode::Substitute, &earlier, &later),
            l([n(3), n(4)])
        );
    }

    /// Reference: `test_list_replaces_scalar`.
    #[test]
    fn substitute_list_replaces_scalar() {
        let later = l([n(1), n(2)]);
        assert_merges_to!(
            merge(MergeMode::Substitute, &s("x"), &later),
            l([n(1), n(2)])
        );
    }

    /// Reference: `test_dict_recursive`.
    #[test]
    fn substitute_dict_recursive() {
        let earlier = m([("a", n(1)), ("b", m([("x", n(10)), ("y", n(20))]))]);
        let later = m([("b", m([("y", n(99)), ("z", n(30))])), ("c", n(3))]);
        let merged = merge(MergeMode::Substitute, &earlier, &later).unwrap();

        assert_eq!(int_at(&merged, "a"), 1);
        assert_eq!(int_at(&merged, "c"), 3);
        // A child of a merged tree is a borrowed `Value` just as the root
        // is, so the same `int_at` reads both.
        let b = TryAsRef::<Map>::try_as_ref(&merged)
            .and_then(|m| m.get("b"))
            .expect("the merged map keeps `b`");
        assert_eq!(
            int_at(b, "x"),
            10,
            "a key only the earlier layer had must stand"
        );
        assert_eq!(int_at(b, "y"), 99);
        assert_eq!(int_at(b, "z"), 30);
    }

    /// Reference: `test_dict_from_list_of_dicts` — a map on the left
    /// absorbs a list of maps on the right, folded in sequentially. The
    /// non-obvious one.
    #[test]
    fn substitute_map_absorbs_a_list_of_maps() {
        let earlier = m([("a", n(1))]);
        let later = l([m([("b", n(2))]), m([("c", n(3))])]);
        let merged = merge(MergeMode::Substitute, &earlier, &later).unwrap();
        assert_eq!(int_at(&merged, "a"), 1);
        assert_eq!(int_at(&merged, "b"), 2);
        assert_eq!(int_at(&merged, "c"), 3);
    }

    /// Reference: `test_dict_from_list_with_non_dict_raises` — a
    /// `TypeError` there, a `MergeError` here.
    #[test]
    fn substitute_map_absorbing_a_non_map_element_is_an_error() {
        let earlier = m([("a", n(1))]);
        let later = l([n(42)]);
        let result = merge(MergeMode::Substitute, &earlier, &later);
        assert!(
            result.is_err(),
            "a scalar folded into a map has no reading: {result:?}"
        );
    }

    // ------------------------------------------------------------------
    // Deep  (reference: TestDeepMerge)
    // ------------------------------------------------------------------

    /// Reference: `test_scalar_replaces`.
    #[test]
    fn deep_scalar_replaces() {
        assert_merges_to!(merge(MergeMode::Deep, &n(1), &n(2)), n(2));
    }

    /// Reference: `test_none_a_takes_b` — the earlier side being nothing
    /// means the later value simply stands. This is where `Deep` and
    /// `Substitute` deliberately differ; see
    /// `the_a_is_none_handling_differs_between_deep_and_substitute`.
    #[test]
    fn deep_null_on_the_earlier_side_takes_the_later_value() {
        assert_merges_to!(merge(MergeMode::Deep, &null(), &n(42)), n(42));
    }

    /// Reference: `test_dict_deep`.
    #[test]
    fn deep_dict_recursive() {
        let earlier = m([("a", n(1)), ("b", m([("x", n(10)), ("y", n(20))]))]);
        let later = m([("b", m([("y", n(99)), ("z", n(30))])), ("c", n(3))]);
        let merged = merge(MergeMode::Deep, &earlier, &later).unwrap();
        // A nested map is a borrowed `Value` like any other, so `int_at`
        // reaches it directly.
        let b = TryAsRef::<Map>::try_as_ref(&merged)
            .and_then(|m| m.get("b"))
            .expect("the merged map keeps `b`");
        assert_eq!(
            (int_at(b, "x"), int_at(b, "y"), int_at(b, "z")),
            (10, 99, 30)
        );
        assert_eq!((int_at(&merged, "a"), int_at(&merged, "c")), (1, 3));
    }

    /// Reference: `test_list_extends_unique_scalars` — `3` is already
    /// present, so only `4` and `5` are appended.
    #[test]
    fn deep_list_extends_with_unique_scalars() {
        let earlier = l([n(1), n(2), n(3)]);
        let later = l([n(3), n(4), n(5)]);
        assert_merges_to!(
            merge(MergeMode::Deep, &earlier, &later),
            l([n(1), n(2), n(3), n(4), n(5)])
        );
    }

    /// Reference: `test_list_does_not_duplicate`.
    #[test]
    fn deep_list_does_not_duplicate() {
        let earlier = l([n(1), n(2)]);
        let later = l([n(1), n(2)]);
        assert_merges_to!(merge(MergeMode::Deep, &earlier, &later), l([n(1), n(2)]));
    }

    /// Reference: `test_dict_from_list` — `Deep` absorbs a list of maps
    /// into a map too.
    #[test]
    fn deep_map_absorbs_a_list_of_maps() {
        let earlier = m([("a", n(1))]);
        let later = l([m([("b", n(2))])]);
        let merged = merge(MergeMode::Deep, &earlier, &later).unwrap();
        assert_eq!((int_at(&merged, "a"), int_at(&merged, "b")), (1, 2));
    }

    /// Reference: `test_mergelists_false_no_positional_dict_merge` —
    /// without `mergelists`, maps inside lists are appended, never merged.
    #[test]
    fn deep_mergelists_off_appends_maps_in_lists() {
        let earlier = l([m([("k", n(1))])]);
        let later = l([m([("k", n(2))])]);
        let merged = merge_with(
            MergeMode::Deep,
            &earlier,
            &later,
            MergeOptions::new().with_mergelists(false),
            None,
        )
        .unwrap();
        let got = elements(&merged);
        assert_eq!(got.len(), 2, "two maps, appended rather than merged");
        assert_eq!(
            int_or(
                TryAsRef::<Map>::try_as_ref(got[0]).and_then(|m| m.get("k")),
                0
            ),
            1
        );
        assert_eq!(
            int_or(
                TryAsRef::<Map>::try_as_ref(got[1]).and_then(|m| m.get("k")),
                0
            ),
            2
        );
    }

    /// Reference: `test_mergelists_true_merges_matching_dicts` — the maps
    /// share the key `k`, so they merge positionally into one.
    #[test]
    fn deep_mergelists_on_merges_positionally_matching_maps() {
        let earlier = l([m([("k", n(1)), ("v", s("a"))])]);
        let later = l([m([("k", n(1)), ("v", s("b"))])]);
        let merged = merge_with(
            MergeMode::Deep,
            &earlier,
            &later,
            MergeOptions::new().with_mergelists(true),
            None,
        )
        .unwrap();
        let got = elements(&merged);
        assert_eq!(got.len(), 1, "sharing a key means one record, not two");
        assert_eq!(
            str_or(
                TryAsRef::<Map>::try_as_ref(got[0]).and_then(|m| m.get("v")),
                ""
            ),
            "b"
        );
    }
}

// ----------------------------------------------------------------------
// Beyond the reference: what this port has that the original did not.
// ----------------------------------------------------------------------

/// The overlap condition on `mergelists`, which the reference's own
/// suite only tests in the positive.
///
/// Two positionally-matching maps with **no** shared key are two
/// adjacent records, not one described twice, so the later is
/// appended rather than folded in. Fusing them would be worse than a
/// duplicate, because the duplicate is visible.
#[test]
fn mergelists_on_appends_a_positional_map_with_no_overlapping_key() {
    let earlier = l([m([("name", s("alpha"))])]);
    let later = l([m([("other", s("beta"))])]);
    let merged = merge_with(
        MergeMode::Deep,
        &earlier,
        &later,
        MergeOptions::new().with_mergelists(true),
        None,
    )
    .unwrap();
    let got = elements(&merged);
    assert_eq!(
        got.len(),
        2,
        "no shared key means no positional merge: {got:?}"
    );
    assert_eq!(
        str_or(
            TryAsRef::<Map>::try_as_ref(got[0]).and_then(|m| m.get("name")),
            ""
        ),
        "alpha"
    );
    assert_eq!(
        str_or(
            TryAsRef::<Map>::try_as_ref(got[1]).and_then(|m| m.get("other")),
            ""
        ),
        "beta"
    );
}

/// `mergelists` on and off over the SAME input, so the sub-option's whole
/// effect is one diff rather than two unrelated assertions.
#[test]
fn mergelists_on_and_off_over_the_same_input() {
    // One binding each rather than the reference's two closures: a merge
    // borrows its inputs now, so the same pair feeds both calls.
    let earlier = l([m([("id", n(1)), ("v", s("old"))])]);
    let later = l([m([("id", n(1)), ("v", s("new"))])]);

    let off = merge_with(
        MergeMode::Deep,
        &earlier,
        &later,
        MergeOptions::new().with_mergelists(false),
        None,
    )
    .unwrap();
    let on = merge_with(
        MergeMode::Deep,
        &earlier,
        &later,
        MergeOptions::new().with_mergelists(true),
        None,
    )
    .unwrap();

    assert_eq!(elements(&off).len(), 2, "off: appended");
    assert_eq!(elements(&on).len(), 1, "on: merged by position");
    assert_eq!(
        str_or(
            TryAsRef::<Map>::try_as_ref(elements(&on)[0]).and_then(|m| m.get("v")),
            ""
        ),
        "new"
    );
}

/// The map axis and the list axis are **independent**, and conflating
/// them is what made the original design question wrong. Asserted
/// separately per mode, over one shared input, so the table in the module
/// doc is checked column by column.
#[test]
fn each_modes_map_and_list_axes_are_independent() {
    let map_earlier = m([("outer", m([("kept", n(1)), ("changed", n(2))]))]);
    let map_later = m([("outer", m([("changed", n(99))]))]);

    // Map axis: Simple is shallow (the whole inner map is replaced, so
    // `kept` is GONE); Substitute and Deep recurse (so `kept` stands).
    let simple = merge(MergeMode::Simple, &map_earlier, &map_later).unwrap();
    let outer = TryAsRef::<Map>::try_as_ref(&simple)
        .and_then(|m| m.get("outer"))
        .expect("the outer key survives");
    // The reference asserted `ValueType::Absent` here; this model spells
    // the same thing as the key simply not being in the map, since a
    // merge never writes the stored-absent sentinel.
    assert!(
        TryAsRef::<Map>::try_as_ref(outer)
            .and_then(|m| m.get("kept"))
            .is_none(),
        "Simple is shallow"
    );
    assert_eq!(
        int_or(
            TryAsRef::<Map>::try_as_ref(outer).and_then(|m| m.get("changed")),
            0
        ),
        99
    );

    for mode in [MergeMode::Substitute, MergeMode::Deep] {
        let merged = merge(mode, &map_earlier, &map_later).unwrap();
        let outer = TryAsRef::<Map>::try_as_ref(&merged)
            .and_then(|m| m.get("outer"))
            .expect("the outer key survives");
        assert_eq!(
            int_or(
                TryAsRef::<Map>::try_as_ref(outer).and_then(|m| m.get("kept")),
                0
            ),
            1,
            "{mode:?} recurses, so `kept` survives"
        );
        assert_eq!(
            int_or(
                TryAsRef::<Map>::try_as_ref(outer).and_then(|m| m.get("changed")),
                0
            ),
            99
        );
    }

    // List axis: three different answers to the same input.
    let list_earlier = l([n(1), n(2), n(3)]);
    let list_later = l([n(3), n(4)]);

    let simple = merge(MergeMode::Simple, &list_earlier, &list_later).unwrap();
    assert!(
        same(&simple, &l([n(3), n(4), n(3)])),
        "Simple overwrites positionally and keeps the excess: {:?}",
        simple
    );
    let substituted = merge(MergeMode::Substitute, &list_earlier, &list_later).unwrap();
    assert!(
        same(&substituted, &l([n(3), n(4)])),
        "Substitute replaces wholesale: {:?}",
        substituted
    );
    let deep = merge(MergeMode::Deep, &list_earlier, &list_later).unwrap();
    assert!(
        same(&deep, &l([n(1), n(2), n(3), n(4)])),
        "Deep unions: {:?}",
        deep
    );
}

/// A case `Simple` gets wrong, and a case `Substitute` gets wrong — the
/// pair that shows neither is a strict improvement on the other.
#[test]
fn simple_and_substitute_each_get_a_case_the_other_gets_right() {
    // `Substitute` is right and `Simple` is wrong: overriding one inner
    // key must not delete its siblings.
    let earlier = m([("tls", m([("ca", s("/etc/ca.pem")), ("verify", b(true))]))]);
    let later = m([("tls", m([("verify", b(false))]))]);

    let substituted = merge(MergeMode::Substitute, &earlier, &later).unwrap();
    let tls = TryAsRef::<Map>::try_as_ref(&substituted)
        .and_then(|m| m.get("tls"))
        .expect("the tls key survives");
    assert_eq!(
        str_or(
            TryAsRef::<Map>::try_as_ref(tls).and_then(|m| m.get("ca")),
            ""
        ),
        "/etc/ca.pem",
        "the sibling survives"
    );
    assert!(!crate::value::read::bool_or(
        TryAsRef::<Map>::try_as_ref(tls).and_then(|m| m.get("verify")),
        true
    ));

    let simple = merge(MergeMode::Simple, &earlier, &later).unwrap();
    let tls = TryAsRef::<Map>::try_as_ref(&simple)
        .and_then(|m| m.get("tls"))
        .expect("the tls key survives");
    assert!(
        TryAsRef::<Map>::try_as_ref(tls)
            .and_then(|m| m.get("ca"))
            .is_none(),
        "Simple's shallow update DELETES the sibling -- the case it gets wrong"
    );

    // `Simple` is right and `Substitute` is wrong: patching element 1 of
    // a fixed-arity list while leaving the rest alone.
    let earlier = l([s("red"), s("green"), s("blue")]);
    let later = l([null(), s("GREEN")]);

    let simple = merge(MergeMode::Simple, &earlier, &later).unwrap();
    assert!(
        same(&simple, &l([null(), s("GREEN"), s("blue")])),
        "Simple patches in place and keeps the tail: {:?}",
        simple
    );

    let substituted = merge(MergeMode::Substitute, &earlier, &later).unwrap();
    assert!(
        same(&substituted, &l([null(), s("GREEN")])),
        "Substitute drops the tail -- the case it gets wrong: {:?}",
        substituted
    );
}

/// The asymmetry that makes `Substitute` the default: a list can be
/// SHRUNK under it and provably cannot be under `Deep`.
///
/// The whole argument for the default, asserted rather than argued.
/// `Deep` has no operation that removes an item.
#[test]
fn a_list_shrinks_under_substitute_and_provably_cannot_under_deep() {
    let inherited = l([s("prod"), s("legacy"), s("eu-west")]);
    let shorter = l([s("prod")]);

    let substituted = merge(MergeMode::Substitute, &inherited, &shorter).unwrap();
    assert!(
        same(&substituted, &l([s("prod")])),
        "a later layer's list IS the list: {:?}",
        substituted
    );

    let deep = merge(MergeMode::Deep, &inherited, &shorter).unwrap();
    assert!(
        same(&deep, &l([s("prod"), s("legacy"), s("eu-west")])),
        "Deep can only ever grow: `legacy` cannot be removed by any later layer: {:?}",
        deep
    );
    // And no ordering of layers helps -- the union is monotone.
    let twice = merge(MergeMode::Deep, &deep, &shorter).unwrap();
    assert_eq!(elements(&twice).len(), 3, "still monotone on a second pass");
}

/// A type mismatch returns `Err` and never silently replaces: a
/// silent replacement means the user set an option, the shapes
/// disagreed, the earlier value survived, and nothing said so.
#[test]
fn a_type_mismatch_is_an_error_and_never_a_silent_replacement() {
    // A map arriving where the earlier layer has a scalar, under
    // `Substitute` -- the reference raises here.
    let err = merge(MergeMode::Substitute, &s("a string"), &m([("k", n(1))]))
        .expect_err("a map replacing a scalar must not pass silently");
    let MergeError::Kind { mode, later, .. } = err else {
        panic!("the shapes disagreed, so this is a Kind error and not a Build one")
    };
    assert_eq!(mode, MergeMode::Substitute);
    assert_eq!(later, Some(Tag::GUATIAO_MAP));

    // A list arriving where the earlier layer has a map, under `Deep`,
    // with a non-map element in it.
    let err = merge(MergeMode::Deep, &m([("a", n(1))]), &l([s("not a map")]))
        .expect_err("a scalar cannot be folded into a map");
    let MergeError::Kind { mode, .. } = err else {
        panic!("the shapes disagreed, so this is a Kind error and not a Build one")
    };
    assert_eq!(mode, MergeMode::Deep);

    // A list on the earlier side and a map on the later, under `Deep`:
    // no combination exists.
    let err = merge(MergeMode::Deep, &l([n(1)]), &m([("a", n(1))]))
        .expect_err("a map has no meaning merged into a list");
    let MergeError::Kind { earlier, later, .. } = err else {
        panic!("the shapes disagreed, so this is a Kind error and not a Build one")
    };
    assert_eq!(
        (earlier, later),
        (Some(Tag::GUATIAO_LIST), Some(Tag::GUATIAO_MAP))
    );

    // The error names WHERE, so an operator can find the key.
    let err = merge(
        MergeMode::Deep,
        &m([("outer", m([("inner", l([n(1)]))]))]),
        &m([("outer", m([("inner", m([("x", n(1))]))]))]),
    )
    .expect_err("nested mismatch");
    let MergeError::Kind { path, .. } = &err else {
        panic!("the shapes disagreed, so this is a Kind error and not a Build one")
    };
    assert_eq!(
        path.as_str(),
        "outer.inner",
        "the error must name the path: {err}"
    );
    assert!(err.to_string().contains("outer.inner"));
}

/// `Substitute` must report the kind the earlier side ACTUALLY had.
///
/// The arm raising it has already moved `earlier`, so it is easy to
/// report a fixed kind instead -- right only when that value is a
/// string and wrong for the five other kinds reaching the same arm.
/// `Deep` derives the field from the value it was handed, so this
/// pins the two modes against each other rather than against a
/// literal. `Null` is excluded because `Deep` treats a null on the
/// left as "replace me" and returns `Ok`, the asymmetry pinned just
/// below.
#[test]
fn substitute_names_the_earlier_kind_it_was_given_and_agrees_with_deep() {
    let later = m([("k", n(1))]);

    for (earlier, expected, cross_check_deep) in [
        (null(), Tag::GUATIAO_NULL, false),
        (b(true), Tag::GUATIAO_BOOL, true),
        (n(1), Tag::GUATIAO_NUMBER, true),
        (s("text"), Tag::GUATIAO_STRING, true),
        (
            Value::from(Buffer::new(&[0xde, 0xad])),
            Tag::GUATIAO_BYTES,
            true,
        ),
        (l([n(1)]), Tag::GUATIAO_LIST, true),
    ] {
        let err = merge(MergeMode::Substitute, &earlier, &later)
            .expect_err("a map arriving over a non-map is a mismatch under Substitute");
        let MergeError::Kind {
            earlier: reported,
            later: later_kind,
            ..
        } = err
        else {
            panic!("the shapes disagreed, so this is a Kind error and not a Build one")
        };
        assert_eq!(
            reported,
            Some(expected),
            "Substitute reported {reported:?} for an earlier value that is {expected:?}"
        );
        assert_eq!(later_kind, Some(Tag::GUATIAO_MAP));

        if cross_check_deep {
            let deep =
                merge(MergeMode::Deep, &earlier, &later).expect_err("Deep refuses the same pair");
            let MergeError::Kind {
                earlier: deep_reported,
                ..
            } = deep
            else {
                panic!("the shapes disagreed, so this is a Kind error and not a Build one")
            };
            assert_eq!(
                deep_reported, reported,
                "the two modes must not disagree about what the earlier value WAS"
            );
        }
    }
}

/// The `a is None` handling differs between the modes, deliberately:
/// `Deep` returns the later value when the earlier side is nothing,
/// `Substitute` only when the earlier side is not a map.
#[test]
fn the_a_is_none_handling_differs_between_deep_and_substitute() {
    // Earlier side is a stored nothing, later is a map.
    let later = m([("k", n(1))]);
    assert_eq!(
        merge(MergeMode::Deep, &null(), &later).map(|v| int_at(&v, "k")),
        Ok(1),
        "Deep: a null on the left takes whatever arrives"
    );
    assert!(
        merge(MergeMode::Substitute, &null(), &later).is_err(),
        "Substitute: a map arriving over a non-map is a mismatch, not a replacement"
    );

    // Both agree that a scalar over a null replaces.
    assert_merges_to!(merge(MergeMode::Deep, &null(), &n(7)), n(7));
    assert_merges_to!(merge(MergeMode::Substitute, &null(), &n(7)), n(7));
}

/// Absent means "no opinion", never "delete".
#[test]
fn a_key_absent_from_the_later_layer_leaves_the_earlier_value_standing() {
    let earlier = m([("kept", s("yes")), ("changed", s("before"))]);
    let later = m([("changed", s("after"))]);
    for mode in [MergeMode::Simple, MergeMode::Substitute, MergeMode::Deep] {
        let merged = merge(mode, &earlier, &later).unwrap();
        assert_eq!(
            str_at(&merged, "kept"),
            "yes",
            "{mode:?} must not delete an unmentioned key"
        );
        assert_eq!(str_at(&merged, "changed"), "after");
    }
}

/// A stored null on the LATER side is an opinion and does overwrite —
/// the distinction between "the key is not there" and "the key is there
/// and its value is nothing", which this crate draws explicitly (see
/// `Tag::GUATIAO_ABSENT` versus `Tag::GUATIAO_NULL`).
#[test]
fn a_stored_null_on_the_later_side_overwrites_rather_than_being_ignored() {
    let merged = merge(
        MergeMode::Substitute,
        &m([("k", s("something"))]),
        &m([("k", null())]),
    )
    .unwrap();
    // A key that is not there answers `None`, which is a different
    // statement from a key holding a stored null.
    assert_eq!(
        TryAsRef::<Map>::try_as_ref(&merged)
            .and_then(|m| m.get("k"))
            .and_then(kind),
        Some(Tag::GUATIAO_NULL),
        "a caller who wrote null meant it -- unlike an absent key"
    );
}

/// A replaced key keeps its position, so merged output does not re-order
/// a caller's rendered form. Map's own `set` guarantees this for a direct
/// set; the merge has to preserve it through remove/re-insert.
#[test]
fn merging_preserves_the_earlier_maps_key_order() {
    let merged = merge(
        MergeMode::Substitute,
        &m([("alpha", n(1)), ("bravo", n(2)), ("charlie", n(3))]),
        &m([("bravo", n(99)), ("delta", n(4))]),
    )
    .unwrap();
    let keys: Vec<String> = TryAsRef::<Map>::try_as_ref(&merged)
        .map(Map::entries)
        .unwrap_or(&[])
        .iter()
        .map(|e| (e.key(), e.value()))
        .map(|(key, _)| String::from_utf8_lossy(key).into_owned())
        .collect();
    assert_eq!(
        keys,
        ["alpha", "bravo", "charlie", "delta"],
        "an overridden key must not jump to the end, and a new one appends"
    );
}

// --- per-path overrides (the `x-merge` channel) ------------------------

/// An override changes the mode for **that key only**, leaving every
/// other key on the call-site default -- the mechanism a schema
/// layer's per-key annotations ride on.
#[test]
fn a_per_path_override_beats_the_call_site_mode_for_that_key_only() {
    let earlier = m([
        ("tags", l([s("prod"), s("eu")])),
        ("fallbacks", l([s("a"), s("b")])),
    ]);
    let later = m([("tags", l([s("canary")])), ("fallbacks", l([s("c")]))]);

    // `tags` is declared as an unordered set, so it unions; `fallbacks`
    // takes the call-site default and is replaced wholesale.
    let overrides = MergeOverrides::new().with("tags", MergeMode::Deep);
    let merged = merge_with(
        MergeMode::Substitute,
        &earlier,
        &later,
        MergeOptions::new(),
        Some(&overrides),
    )
    .unwrap();

    let tags = TryAsRef::<Map>::try_as_ref(&merged)
        .and_then(|m| m.get("tags"))
        .expect("the merge keeps `tags`");
    assert!(
        tags == &l([s("prod"), s("eu"), s("canary")]),
        "the declared Deep mode unions this key"
    );
    let fallbacks = TryAsRef::<Map>::try_as_ref(&merged)
        .and_then(|m| m.get("fallbacks"))
        .expect("the merge keeps `fallbacks`");
    assert!(
        fallbacks == &l([s("c")]),
        "every other key stays on the call-site Substitute"
    );
}

/// An override applies at a NESTED path too, spelled with dots.
#[test]
fn an_override_applies_at_a_nested_path() {
    let earlier = m([("tls", m([("ciphers", l([s("aes")]))]))]);
    let later = m([("tls", m([("ciphers", l([s("chacha")]))]))]);

    let overrides = MergeOverrides::new().with("tls.ciphers", MergeMode::Deep);
    let merged = merge_with(
        MergeMode::Substitute,
        &earlier,
        &later,
        MergeOptions::new(),
        Some(&overrides),
    )
    .unwrap();

    let tls = TryAsRef::<Map>::try_as_ref(&merged)
        .and_then(|m| m.get("tls"))
        .expect("the merge keeps `tls`");
    let ciphers = TryAsRef::<Map>::try_as_ref(tls)
        .and_then(|m| m.get("ciphers"))
        .expect("the merge keeps `tls.ciphers`");
    assert!(ciphers == &l([s("aes"), s("chacha")]));
}

/// A merge with no overrides at all is fully usable — the property that
/// keeps this crate free of any dependency on a schema crate.
#[test]
fn a_merge_with_no_overrides_is_fully_usable() {
    assert!(MergeOverrides::new().is_empty());
    let earlier = m([("a", n(1))]);
    let later = m([("b", n(2))]);
    let merged = merge(MergeMode::default(), &earlier, &later).unwrap();
    assert_eq!((int_at(&merged, "a"), int_at(&merged, "b")), (1, 2));
}

/// The documented default is `Substitute`, not `Deep`.
#[test]
fn the_default_mode_is_substitute() {
    assert_eq!(MergeMode::default(), MergeMode::Substitute);
}

// --- provenance --------------------------------------------------------

/// Provenance survives a RECURSIVE merge and reports *mixed* where it
/// must: after recursing, `tls.verify` came from the user layer while
/// `tls.ca` came from the system one, so `tls` has no single source
/// and answering with either would be a confident lie.

#[test]
fn provenance_is_per_leaf_and_reports_mixed_for_an_interior_node() {
    let system = m([
        ("tls", m([("ca", s("/etc/ca.pem")), ("verify", b(true))])),
        ("host", s("system.example")),
    ]);
    let user = m([("tls", m([("verify", b(false))])), ("theme", s("dark"))]);

    let (merged, provenance) = MergeMode::Substitute
        .merge_layers([("system", &system), ("user", &user)], alloc())
        .unwrap();

    let tls = TryAsRef::<Map>::try_as_ref(&merged)
        .and_then(|m| m.get("tls"))
        .expect("the merged map still carries `tls`");
    assert!(!crate::value::read::bool_or(
        TryAsRef::<Map>::try_as_ref(tls).and_then(|m| m.get("verify")),
        true
    ));
    assert_eq!(
        str_or(
            TryAsRef::<Map>::try_as_ref(tls).and_then(|m| m.get("ca")),
            ""
        ),
        "/etc/ca.pem"
    );

    assert_eq!(provenance.source_of("tls.verify"), Source::Layer("user"));
    assert_eq!(provenance.source_of("tls.ca"), Source::Layer("system"));
    assert_eq!(
        provenance.source_of("tls"),
        Source::Mixed,
        "the map was drawn from both layers; naming one would be a lie"
    );
    assert_eq!(provenance.source_of("host"), Source::Layer("system"));
    assert_eq!(provenance.source_of("theme"), Source::Layer("user"));
    assert_eq!(provenance.source_of("nothing.here"), Source::Absent);
}

/// An interior node whose leaves ALL came from one layer answers with
/// that layer, not `Mixed` — `Mixed` has to mean something.
#[test]
fn an_interior_node_from_a_single_layer_names_that_layer() {
    let system = m([("db", m([("host", s("a")), ("port", n(1))]))]);
    let user = m([("db", m([("host", s("b")), ("port", n(2))]))]);

    let (_, provenance) = MergeMode::Substitute
        .merge_layers([("system", &system), ("user", &user)], alloc())
        .unwrap();
    assert_eq!(
        provenance.source_of("db"),
        Source::Layer("user"),
        "every leaf under `db` was overridden, so `user` is the honest answer"
    );
}

/// The prefix match must not fold in a SIBLING key that merely starts
/// with the same characters. `tls` and `tlsv2` are unrelated keys.
#[test]
fn an_interior_lookup_does_not_match_a_sibling_with_a_shared_prefix() {
    let system = m([("tls", m([("ca", s("x"))])), ("tlsv2", m([("ca", s("y"))]))]);
    let user = m([("tlsv2", m([("ca", s("z"))]))]);

    let (_, provenance) = MergeMode::Substitute
        .merge_layers([("system", &system), ("user", &user)], alloc())
        .unwrap();
    assert_eq!(
        provenance.source_of("tls"),
        Source::Layer("system"),
        "`tlsv2` is a different key and must not make `tls` look mixed"
    );
    assert_eq!(provenance.source_of("tlsv2"), Source::Layer("user"));
}

/// A single layer attributes everything to itself, and no layers at all
/// is an empty map rather than an error.
#[test]
fn one_layer_and_no_layers_are_both_well_defined() {
    let only = m([("k", s("v"))]);
    let (merged, provenance) = MergeMode::Substitute
        .merge_layers([("only", &only)], alloc())
        .unwrap();
    assert_eq!(str_at(&merged, "k"), "v");
    assert_eq!(provenance.source_of("k"), Source::Layer("only"));

    let (empty, provenance) = MergeMode::Substitute.merge_layers([], alloc()).unwrap();
    assert_eq!(len_of(&empty), 0);
    assert_eq!(provenance.source_of("anything"), Source::Absent);
}

/// A third layer overriding a leaf takes provenance from the second, not
/// from the first — later really does win, in the record as well as in
/// the value.
#[test]
fn a_later_layer_takes_provenance_from_the_one_before_it() {
    let system = m([("k", s("1")), ("untouched", s("s"))]);
    let site = m([("k", s("2"))]);
    let user = m([("k", s("3"))]);
    let (merged, provenance) = MergeMode::Substitute
        .merge_layers(
            [("system", &system), ("site", &site), ("user", &user)],
            alloc(),
        )
        .unwrap();
    assert_eq!(str_at(&merged, "k"), "3");
    assert_eq!(provenance.source_of("k"), Source::Layer("user"));
    assert_eq!(provenance.source_of("untouched"), Source::Layer("system"));
}

/// A later layer replacing a subtree with a differently-shaped value must
/// not leave the old subtree's leaf paths recorded — `source_of` would
/// then answer for keys that no longer exist.
#[test]
fn replacing_a_subtree_forgets_the_paths_that_no_longer_exist() {
    let system = m([("thing", m([("a", n(1)), ("b", n(2))]))]);
    let user = m([("thing", s("just a string now"))]);
    let (merged, provenance) = MergeMode::Deep
        .merge_layers([("system", &system), ("user", &user)], alloc())
        .unwrap();

    assert_eq!(str_at(&merged, "thing"), "just a string now");
    assert_eq!(provenance.source_of("thing"), Source::Layer("user"));
    assert_eq!(
        provenance.source_of("thing.a"),
        Source::Absent,
        "the old leaf is gone from the value and must be gone from the record"
    );
}

/// Provenance is recorded for list contents by index, so a caller can ask
/// about `peers.0.name` rather than only about `peers`.
#[test]
fn list_elements_get_indexed_leaf_paths() {
    let system = m([("peers", l([m([("name", s("a"))])]))]);
    let (_, provenance) = MergeMode::Substitute
        .merge_layers([("system", &system)], alloc())
        .unwrap();
    assert_eq!(
        provenance.source_of("peers.0.name"),
        Source::Layer("system")
    );
    assert_eq!(provenance.source_of("peers"), Source::Layer("system"));
}

/// A merge that fails reports the error rather than producing a partial
/// map — `merge_layers` returns `Err` and no half-merged result escapes.
#[test]
fn a_failed_layer_merge_yields_an_error_not_a_partial_map() {
    let system = m([("k", l([n(1)]))]);
    let user = m([("k", m([("x", n(1))]))]);
    let result = MergeMode::Deep.merge_layers([("system", &system), ("user", &user)], alloc());
    let err = result.expect_err("a list and a map cannot combine");
    // The error is an enum now, so the path comes out of the `Kind`
    // variant rather than off a struct field.
    let MergeError::Kind { path, .. } = err else {
        panic!("a shape disagreement, not a failure to build the result")
    };
    assert_eq!(path, "k");
}

// --- what nothing pinned before ----------------------------------------
//
// Three behaviours the port had to decide: the `mergelists` default,
// the order a mixed later list comes out in, and what the two extra
// spellings of "nothing" mean. Every mergelists test above passes the
// flag explicitly, so a flipped default would leave them all green.

/// `mergelists` is **off** by default, which is one of the behaviours
/// this module's specification names.
#[test]
fn mergelists_is_off_by_default() {
    assert!(!MergeOptions::new().mergelists);
    assert!(!MergeOptions::default().mergelists);

    // And the behaviour that follows: two positionally matching maps that
    // even share a key are APPENDED rather than folded together.
    let earlier = l([m([("a", n(1))])]);
    let later = l([m([("a", n(2))])]);
    assert_merges_to!(
        merge(MergeMode::Deep, &earlier, &later),
        l([m([("a", n(1))]), m([("a", n(2))])])
    );
}

/// With `mergelists` off, a later list's unique non-map items are
/// appended **before** its maps, whatever order it spelled them in. Two
/// passes, and the ordering is visible to any caller that renders the
/// list.
#[test]
fn a_later_lists_scalars_are_appended_before_its_maps() {
    let earlier = l([n(0)]);
    let later = l([m([("k", n(1))]), s("x")]);
    assert_merges_to!(
        merge(MergeMode::Deep, &earlier, &later),
        l([n(0), s("x"), m([("k", n(1))])])
    );
}

/// The stored **absent** sentinel means "no opinion" on either side; a
/// stored **null** is a value and still overwrites. Getting this wrong
/// would let a producer's "I have nothing to say" erase a value.
#[test]
fn a_stored_absent_is_no_opinion_and_a_stored_null_is_a_value() {
    for mode in [MergeMode::Simple, MergeMode::Substitute, MergeMode::Deep] {
        assert_merges_to!(merge(mode, &s("kept"), &absent()), s("kept"));
        assert_merges_to!(merge(mode, &absent(), &s("taken")), s("taken"));
        assert_merges_to!(merge(mode, &s("replaced"), &null()), null());
    }
}

/// A value whose tag this build does not know **stops** the merge.
///
/// Not because the merge refuses to reason about it -- it classifies
/// as a scalar and the later side wins -- but because putting it in
/// the result means copying it, and bit-copying a payload this build
/// cannot interpret would produce two owners of whatever it points
/// at. The one place the "skip what you cannot read" rule cannot
/// apply.
#[test]
fn a_kind_this_build_cannot_read_stops_the_merge_rather_than_being_copied() {
    // A tag no build knows, on a node that owns nothing -- which is what
    // makes writing it here safe.
    let mut future = Value::null();
    future.tag = 9_999;

    let err = merge(MergeMode::Substitute, &s("here"), &future)
        .expect_err("a value that cannot be copied cannot be merged");
    assert_eq!(err, MergeError::Build(ValueError::UnknownTag(9_999)));

    // And it is the COPY that fails, not the classification: the same tag
    // on the earlier side, where the later value wins outright, merges
    // without ever touching it.
    assert_merges_to!(merge(MergeMode::Substitute, &future, &s("wins")), s("wins"));
}
