// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A deep merge unions two lists without comparing every item of one
//! with every item of the other: each later item is looked up among those
//! already held. Each test unions lists of 100 000 items, where a
//! comparison per pair is some 5e9 comparisons: minutes in a debug build,
//! against well under a second.

use core::time::Duration;
use std::time::Instant;

use guatiao::value::merge::{MergeMode, MergeOptions, Source};
use guatiao::{Alloc, List, Map, Number, TryAsRef, Value};

const N: usize = 100_000;

/// What a linear union of 100 000 items stays under in a debug build.
const LIMIT: Duration = Duration::from_secs(20);

fn number(i: usize) -> Value {
    Value::from(Number::new(&i.to_string()).unwrap())
}

/// A one-item list holding `i`: an item compared by its structure.
fn boxed(i: usize) -> Value {
    let mut list = List::new();
    list.push(number(i));
    list.into()
}

fn list_of(items: impl IntoIterator<Item = Value>) -> Value {
    let mut list = List::new();
    for item in items {
        list.push(item);
    }
    list.into()
}

fn items(value: &Value) -> &[Value] {
    &TryAsRef::<List>::try_as_ref(value).expect("a list")[..]
}

/// Unions `earlier` and `later` both ways a host layers them, and checks
/// the two agree and stayed under [`LIMIT`].
fn union(earlier: &Value, later: &Value, mergelists: bool) -> Value {
    let options = MergeOptions::new().with_mergelists(mergelists);
    let started = Instant::now();
    let merged = MergeMode::Deep
        .merge_with(earlier, later, Alloc::rust(), options, None)
        .unwrap();
    let (layered, provenance) = MergeMode::Deep
        .merge_layers_with([("a", earlier), ("b", later)], Alloc::rust(), options, None)
        .unwrap();
    let took = started.elapsed();
    assert!(
        took < LIMIT,
        "mergelists {mergelists}: {N} items took {took:?}"
    );
    assert_eq!(merged, layered);
    assert_eq!(provenance.source_of(["0"]), Source::Layer("a"));
    merged
}

#[test]
fn unioning_two_long_lists_of_scalars_is_linear_and_keeps_the_order() {
    let earlier = list_of((0..N).map(number));
    // Half already held, half new, and every new one repeated once.
    let later = list_of((N / 2..N + N / 2).flat_map(|i| [number(i), number(i)]));
    for mergelists in [false, true] {
        let merged = union(&earlier, &later, mergelists);
        let merged = items(&merged);
        assert_eq!(merged.len(), N + N / 2, "mergelists {mergelists}");
        for (i, item) in merged.iter().enumerate() {
            assert_eq!(*item, number(i), "mergelists {mergelists}: position {i}");
        }
    }
}

#[test]
fn unioning_two_long_lists_of_lists_compares_each_by_its_structure() {
    let earlier = list_of((0..N).map(boxed));
    let mut later: Vec<Value> = (N / 2..N + N / 2).map(boxed).collect();
    // A map among the later items goes after every unique non-map one.
    later.insert(N / 4, Map::new().into());
    let later = list_of(later);
    for mergelists in [false, true] {
        let merged = union(&earlier, &later, mergelists);
        let merged = items(&merged);
        assert_eq!(merged.len(), N + N / 2 + 1, "mergelists {mergelists}");
        for (i, item) in merged[..N + N / 2].iter().enumerate() {
            assert_eq!(*item, boxed(i), "mergelists {mergelists}: position {i}");
        }
        assert_eq!(merged[N + N / 2], Value::from(Map::new()));
    }
}
