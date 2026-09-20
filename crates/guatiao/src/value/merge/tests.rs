// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Merge tests.
//!
//! # The first block is a PORT, not a fresh test suite
//!
//! `port_of_the_reference_suite` below is a case-for-case translation of
//! the prior implementation's own tests. Those tests are the closest
//! thing to a written specification of the three strategies — the summary
//! table in the module doc does not capture the positional list rule, the
//! `mergelists` overlap condition, or the map-absorbs-list-of-maps
//! behaviour, and all three are pinned there.
//!
//! Keeping them as a distinct, named block matters: a divergence from the
//! reference should show up as a **red test in the port**, not as a
//! surprise in some unrelated behaviour years later. Each test carries
//! the reference test's own name in a comment so the two can be diffed
//! by hand.
//!
//! The second block is what this port needs and the reference did not
//! have: the `Result` instead of an exception, the leaf-path provenance,
//! and the per-path mode override.
//!
//! # Every value here is built on Rust's own heap
//!
//! The short-form constructors — `Value::string`, `Map::owned` — build
//! through Rust's allocator and name none, so no helper below has to
//! thread one through its own literals. A merge still takes one
//! explicitly, because its result is a new tree and something has to say
//! what that grows through.

use super::*;

use crate::value::alloc::Alloc;
use crate::value::read::{entries, int_or, str_or};

/// The allocator a merge builds its result through. The values handed to
/// it need none — they are already on Rust's heap.
fn alloc() -> Alloc {
    Alloc::rust()
}

/// Builds a map from `(key, value)` pairs.
fn m(pairs: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    let mut built = Value::map();
    for (key, value) in pairs {
        built.set(key, value).expect("a test map is small");
    }
    built
}

/// Builds a list from values.
fn l(values: impl IntoIterator<Item = Value>) -> Value {
    let mut built = Value::list();
    for value in values {
        built.push(value).expect("a test list is small");
    }
    built
}

/// A string value, for brevity in the cases below.
fn s(text: &str) -> Value {
    Value::string(text)
}

/// A number value from an integer, for brevity in the cases below.
///
/// Numbers are stored as text, so `n(1)` is the value whose exact
/// spelling is `"1"`. Merging never looks inside a number, so which
/// spelling a case uses does not matter to it -- only that two values
/// that should differ do.
fn n(value: i64) -> Value {
    Value::from(value)
}

/// The stored nothing.
fn null() -> Value {
    Value::null()
}

/// A boolean value.
fn b(value: bool) -> Value {
    Value::bool(value)
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
/// number. There is no `PartialEq` on a value, and there should not be —
/// comparing two trees is a walk, not a field comparison.
fn same(left: &Value, right: &Value) -> bool {
    equal(left, right)
}

/// The integer under `key`, or zero.
fn int_at(value: &Value, key: &str) -> i64 {
    int_or(value.get(key), 0)
}

/// The text under `key`, or the empty string.
fn str_at<'v>(value: &'v Value, key: &str) -> &'v str {
    str_or(value.get(key), "")
}

/// How many entries a map has.
fn len_of(value: &Value) -> usize {
    entries(value).count()
}

/// The elements of a list, for comparison.
fn elements(value: &Value) -> Vec<&Value> {
    items(value).collect()
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
        let b = merged.get("b").expect("the merged map keeps `b`");
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
        let b = merged.get("b").expect("the merged map keeps `b`");
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
        assert_eq!(int_or(got[0].get("k"), 0), 1);
        assert_eq!(int_or(got[1].get("k"), 0), 2);
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
        assert_eq!(str_or(got[0].get("v"), ""), "b");
    }
}

// ----------------------------------------------------------------------
// Beyond the reference: what this port has that the original did not.
// ----------------------------------------------------------------------

/// The overlap condition on `mergelists`, which the reference's own suite
/// only tests in the positive.
///
/// Two positionally-matching maps with **no** shared key are two records
/// that happen to be adjacent, not one record described twice — so the
/// later one is appended rather than folded in. Without this the merge
/// would silently fuse unrelated records, which is worse than a duplicate
/// because the duplicate is visible.
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
    assert_eq!(str_or(got[0].get("name"), ""), "alpha");
    assert_eq!(str_or(got[1].get("other"), ""), "beta");
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
    assert_eq!(str_or(elements(&on)[0].get("v"), ""), "new");
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
    let outer = simple.get("outer").expect("the outer key survives");
    // The reference asserted `ValueType::Absent` here; this model spells
    // the same thing as the key simply not being in the map, since a
    // merge never writes the stored-absent sentinel.
    assert!(outer.get("kept").is_none(), "Simple is shallow");
    assert_eq!(int_or(outer.get("changed"), 0), 99);

    for mode in [MergeMode::Substitute, MergeMode::Deep] {
        let merged = merge(mode, &map_earlier, &map_later).unwrap();
        let outer = merged.get("outer").expect("the outer key survives");
        assert_eq!(
            int_or(outer.get("kept"), 0),
            1,
            "{mode:?} recurses, so `kept` survives"
        );
        assert_eq!(int_or(outer.get("changed"), 0), 99);
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
    let tls = substituted.get("tls").expect("the tls key survives");
    assert_eq!(
        str_or(tls.get("ca"), ""),
        "/etc/ca.pem",
        "the sibling survives"
    );
    assert!(!crate::value::read::bool_or(tls.get("verify"), true));

    let simple = merge(MergeMode::Simple, &earlier, &later).unwrap();
    let tls = simple.get("tls").expect("the tls key survives");
    assert!(
        tls.get("ca").is_none(),
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
/// This is the whole argument for the default, asserted rather than
/// argued in a comment. `Deep` has no operation that removes an item, so
/// an inherited tag could never be taken back without inventing a null
/// sentinel — a worse problem than the one it would solve.
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

/// A type mismatch returns `Err` and never silently replaces.
///
/// The failure being guarded is not untidiness: a silent replacement
/// means the user set an option, the merge decided the shapes disagreed,
/// the earlier value survived, and nothing anywhere said so.
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
/// The arm that raises this error sits in the `else` of a `let ... else`,
/// which has already moved `earlier` — so the first version of it could not
/// call `value_type()` and hardcoded `ValueType::Str` instead. That is
/// right only when the earlier value happens to be a string and wrong for
/// the five other kinds that reach the same arm (`Null`, `Bool`, `Number`,
/// `Bytes` and, least obviously, `List`). Nothing caught it because the one
/// test exercising the arm passed a string.
///
/// `Deep` derives the same field from the value it was handed, so for
/// identical input the two modes disagreed about what went wrong. This
/// pins them against each other rather than against a literal: a mode that
/// starts guessing again stops matching its neighbour, which is a sharper
/// signal than a hardcoded expectation would be.
///
/// `Null` is excluded from the cross-check on purpose — `Deep` treats a
/// null on the left as "replace me" and returns `Ok`, which is the
/// deliberate asymmetry pinned by the test just below. `Substitute` still
/// has to name it correctly.
#[test]
fn substitute_names_the_earlier_kind_it_was_given_and_agrees_with_deep() {
    let later = m([("k", n(1))]);

    for (earlier, expected, cross_check_deep) in [
        (null(), Tag::GUATIAO_NULL, false),
        (b(true), Tag::GUATIAO_BOOL, true),
        (n(1), Tag::GUATIAO_NUMBER, true),
        (s("text"), Tag::GUATIAO_STRING, true),
        (Value::bytes(&[0xde, 0xad]), Tag::GUATIAO_BYTES, true),
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

/// The `a is None` handling differs between the modes, deliberately.
///
/// `Deep` returns the later value when the earlier side is nothing.
/// `Substitute` returns it only when the earlier side is not a map — so a
/// map on the left is never overwritten by a scalar, it is an error.
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
        merged.get("k").and_then(kind),
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
    let keys: Vec<String> = entries(&merged)
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
/// other key on the call-site default.
///
/// This is the mechanism a schema layer's per-key merge annotations ride on. The
/// declarer of an option knows whether its list is an unordered tag set
/// or an ordered fallback chain; the caller merging two maps does not.
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

    let tags = merged.get("tags").expect("the merge keeps `tags`");
    assert!(
        equal(tags, &l([s("prod"), s("eu"), s("canary")])),
        "the declared Deep mode unions this key"
    );
    let fallbacks = merged
        .get("fallbacks")
        .expect("the merge keeps `fallbacks`");
    assert!(
        equal(fallbacks, &l([s("c")])),
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

    let tls = merged.get("tls").expect("the merge keeps `tls`");
    let ciphers = tls.get("ciphers").expect("the merge keeps `tls.ciphers`");
    assert!(equal(ciphers, &l([s("aes"), s("chacha")])));
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
/// must.
///
/// The wrinkle a naive per-top-level-key design gets wrong: after
/// recursing, `tls` has no single source — `tls.verify` came from the
/// user layer while `tls.ca` came from the system one. Answering with
/// either would be a confident lie.

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

    let tls = merged
        .get("tls")
        .expect("the merged map still carries `tls`");
    assert!(!crate::value::read::bool_or(tls.get("verify"), true));
    assert_eq!(str_or(tls.get("ca"), ""), "/etc/ca.pem");

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
// Three behaviours the port had to decide and the suite above does not
// assert: the `mergelists` default, the order a mixed later list comes
// out in, and what the C form's two extra spellings of "nothing" mean.
// Every mergelists test above passes the flag explicitly, so a port that
// flipped its default would have left all 43 of them green.

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
/// appended **before** its maps, whatever order it spelled them in.
///
/// Two passes rather than one, and the ordering is load-bearing: it is
/// visible to any caller that renders the list, so it is written down
/// here rather than left to be rediscovered by someone reading output
/// that does not match their input.
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
/// stored **null** is a value and still overwrites.
///
/// The C form can put absent in a container and the owned model this
/// replaced could not, so the rule the module states for a key the later
/// layer does not carry is stated again for the sentinel that spells the
/// same thing. Getting this wrong would let a producer's "I have nothing
/// to say" erase a value.
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
/// Not because the merge refuses to reason about it — it classifies as a
/// scalar and the later side simply wins — but because putting it in the
/// result means copying it, and a node whose payload this build cannot
/// interpret is one it cannot copy: bit-copying it would produce two
/// owners of whatever it points at. So the failure is
/// [`ValueError::UnknownTag`], reported rather than papered over.
///
/// This is the one place the C form's "skip the value you cannot read,
/// render the rest" rule cannot apply, and saying so out loud is the
/// point of the test.
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
