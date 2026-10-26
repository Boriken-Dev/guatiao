// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Every way of building a map from many keys stays linear in them: a map
//! has no index of its own, so a builder that looked each key up by
//! scanning would be quadratic. Each test builds or combines maps of
//! 100 000 keys, where a scan per key is some 5e9 comparisons: minutes
//! in a debug build, against well under a second.

extern crate alloc;

use alloc::borrow::Cow;
use alloc::collections::BTreeMap;
use core::time::Duration;
use std::collections::HashMap;
use std::time::Instant;

use guatiao::value::convert::MapKey;
use guatiao::value::merge::{MergeMode, Source};
use guatiao::value::visit::{Sink, TreeSink};
use guatiao::{Alloc, Map, MapError, Number, ToValue, TryAsRef, Value, ValueError};

const N: usize = 100_000;

/// What a linear build of 100 000 keys stays under in a debug build.
const LIMIT: Duration = Duration::from_secs(20);

fn key(i: usize) -> String {
    format!("k{i:06}")
}

fn number(i: usize) -> Value {
    Value::from(Number::new(&i.to_string()).unwrap())
}

/// A map of `key(i) -> i + offset` for each `i` in `keys`, built through
/// a sink, a builder already known to be linear.
fn wide(keys: core::ops::Range<usize>, offset: usize) -> Value {
    let mut sink = TreeSink::new();
    sink.map_begin(None).unwrap();
    for i in keys {
        sink.key(&key(i)).unwrap();
        sink.number(&(i + offset).to_string()).unwrap();
    }
    sink.map_end().unwrap();
    sink.into_value().unwrap()
}

fn map(value: &Value) -> &Map {
    TryAsRef::try_as_ref(value).expect("a map")
}

/// Asserts `map` holds `key(i)` at position `i` for every `i` below `len`.
fn in_order(map: &Map, len: usize) {
    assert_eq!(map.len(), len);
    for (i, entry) in map.iter().enumerate() {
        assert_eq!(entry.key(), key(i), "the order is the first one's");
    }
}

#[test]
fn collecting_pairs_is_linear_and_a_repeat_replaces_in_place() {
    let repeats = [0, 3, 40, N - 1];
    let started = Instant::now();
    let mut collected: Map = (0..N)
        .map(|i| (key(i), number(i)))
        .chain(repeats.iter().map(|&i| (key(i), number(i + N))))
        .collect();
    // Extending a map already wide finds its keys without a scan as well.
    collected.extend(
        (N..N + 10)
            .chain(repeats)
            .map(|i| (key(i), number(i + 2 * N))),
    );
    let took = started.elapsed();
    assert!(took < LIMIT, "{N} keys took {took:?}");

    in_order(&collected, N + 10);
    for i in repeats {
        assert_eq!(collected[key(i).as_str()], number(i + 2 * N));
    }
    assert_eq!(collected[key(1).as_str()], number(1));
    assert_eq!(collected[key(N + 9).as_str()], number(N + 9 + 2 * N));
}

/// A key whose text folds case, so two keys a std map holds apart can
/// write one key.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Folded(String);

impl MapKey for Folded {
    fn to_key(&self) -> Result<Cow<'_, str>, ValueError> {
        Ok(Cow::Owned(self.0.to_lowercase()))
    }

    fn from_key(key: &str) -> Result<Folded, MapError> {
        Ok(Folded(key.to_owned()))
    }
}

#[test]
fn a_wide_std_map_converts_in_linear_time() {
    let ordered: BTreeMap<String, usize> = (0..N).map(|i| (key(i), i)).collect();
    let hashed: HashMap<String, usize> = (0..N).map(|i| (key(i), i)).collect();
    // `K000003` sorts before every `k`, and writes the key `k000003` too.
    let folded: BTreeMap<Folded, usize> = (0..N)
        .map(|i| (Folded(key(i)), i))
        .chain([(Folded(key(3).to_uppercase()), N)])
        .collect();

    let started = Instant::now();
    let from_ordered = ordered.to_value().unwrap();
    let from_hashed = hashed.to_value().unwrap();
    let from_folded = folded.to_value().unwrap();
    let took = started.elapsed();
    assert!(took < LIMIT, "{N} keys took {took:?}");

    in_order(map(&from_ordered), N);
    assert_eq!(map(&from_hashed).len(), N);
    for i in [0, 17, N - 1] {
        assert_eq!(map(&from_hashed)[key(i).as_str()], number(i));
    }
    // The key written twice keeps the place of the first and the value of
    // the second, as a map's `set` does.
    let folded = map(&from_folded);
    assert_eq!(folded.len(), N);
    assert_eq!(folded.entries()[0].key(), key(3));
    assert_eq!(folded[key(3).as_str()], number(3));
    assert_eq!(folded.entries()[1].key(), key(0));
}

#[test]
fn copying_a_wide_map_into_a_wide_map_is_linear() {
    let mut into: Map = Map::try_from(wide(0..N, 0)).unwrap();
    let from = wide(N / 2..N + N / 2, N);
    let started = Instant::now();
    let copied = into.copy_from(map(&from)).unwrap();
    let took = started.elapsed();
    assert!(took < LIMIT, "{N} keys took {took:?}");

    assert_eq!(copied, N);
    in_order(&into, N + N / 2);
    assert_eq!(into[key(0).as_str()], number(0));
    assert_eq!(into[key(N / 2).as_str()], number(N / 2 + N));
    assert_eq!(into[key(N + N / 2 - 1).as_str()], number(N + N / 2 - 1 + N));
}

#[test]
fn layering_two_wide_maps_is_linear_in_every_mode() {
    let earlier = wide(0..N, 0);
    let later = wide(N / 2..N + N / 2, N);
    for mode in [MergeMode::Simple, MergeMode::Substitute, MergeMode::Deep] {
        let started = Instant::now();
        let merged = mode.merge(&earlier, &later, Alloc::rust()).unwrap();
        let (layered, provenance) = mode
            .merge_layers([("a", &earlier), ("b", &later)], Alloc::rust())
            .unwrap();
        let took = started.elapsed();
        assert!(took < LIMIT, "{mode:?}: {N} keys took {took:?}");

        assert_eq!(merged, layered, "{mode:?}");
        let merged = map(&merged);
        in_order(merged, N + N / 2);
        assert_eq!(merged[key(0).as_str()], number(0), "{mode:?}");
        assert_eq!(merged[key(N / 2).as_str()], number(N / 2 + N), "{mode:?}");
        assert_eq!(provenance.source_of([key(0)]), Source::Layer("a"));
        assert_eq!(provenance.source_of([key(N / 2)]), Source::Layer("b"));
        assert_eq!(provenance.source_of([key(N + 1)]), Source::Layer("b"));
        assert_eq!(provenance.source_of::<_, &str>([]), Source::Mixed);
    }
}
