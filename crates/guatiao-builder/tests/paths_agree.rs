// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! coqui-builder's key paths and guatiao's: the grammar `guatiao-intake` reads, and the
//! spelling `schema::differences` writes for a path.

use coqui_builder::path::{KeyPath, Segment};
use guatiao::schema::Difference;
use guatiao_intake::path::{self, Segment as Theirs};

/// Spellings both read, which both write back the same way.
const SHARED: &[&str] = &[
    "a",
    "a.b",
    "waiting.per_source",
    "a[0]",
    "peers[1].name",
    "peers[1][2]",
    "env[PATH]",
    "env[\"a.b\"]",
    "env[\"say \\\"hi\\\"\"]",
    "a.b[3].c.d",
];

/// Spellings both refuse.
const REFUSED: &[&str] = &["", ".a", "a.", "a[", "a]", "a[0]b", "a.[0]", "a[]", "a[\"x"];

#[test]
fn both_read_a_spelling_as_the_same_steps_and_write_it_back_the_same() {
    for text in SHARED {
        let ours: KeyPath = text.parse().unwrap_or_else(|e| panic!("{text}: {e}"));
        let theirs = path::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"));
        let steps: Vec<Theirs<'_>> = theirs.into_iter().collect();
        assert_eq!(ours.len(), steps.len(), "{text}");
        for (mine, other) in ours.segments().iter().zip(&steps) {
            match (mine, other) {
                (Segment::Key(a), Theirs::Field(b)) => assert_eq!(a, b, "{text}"),
                (Segment::Key(a), Theirs::Key(b)) => assert_eq!(a, b.as_ref(), "{text}"),
                (Segment::Index(a), Theirs::Index(b)) => assert_eq!(a, b, "{text}"),
                (a, b) => panic!("{text}: {a:?} against {b:?}"),
            }
        }
        // What is written here parses back to the same steps in both.
        let written = ours.to_string();
        assert_eq!(written.parse::<KeyPath>().unwrap(), ours, "{text}");
        let back: Vec<Theirs<'_>> = path::parse(&written).unwrap().into_iter().collect();
        assert_eq!(named(&back), named(&steps), "{text} written as {written}");
        // A spelling with no `[name]` is written alike by both.
        if !steps.iter().any(|s| matches!(s, Theirs::Key(_))) {
            assert_eq!(written, theirs.to_string(), "{text}");
        }
    }
}

/// Steps with a field and a map key told apart by nothing: each is a name or a position.
fn named(steps: &[Theirs<'_>]) -> Vec<(bool, String)> {
    steps
        .iter()
        .map(|step| match step {
            Theirs::Field(name) => (false, (*name).to_owned()),
            Theirs::Key(name) => (false, name.to_string()),
            Theirs::Index(at) => (true, at.to_string()),
            _ => unreachable!("a step this test does not know"),
        })
        .collect()
}

#[test]
fn both_refuse_the_same_spellings() {
    for text in REFUSED {
        assert!(text.parse::<KeyPath>().is_err(), "{text:?} is read here");
        assert!(path::parse(text).is_err(), "{text:?} is read by intake");
    }
}

#[test]
fn a_position_is_a_position_and_a_digit_key_is_a_key_in_both() {
    let ours: KeyPath = "a[01]".parse().unwrap();
    let theirs: Vec<Theirs<'_>> = path::parse("a[01]").unwrap().into_iter().collect();
    assert!(matches!(ours.segments()[1], Segment::Key(ref k) if k == "01"));
    assert!(matches!(&theirs[1], Theirs::Key(k) if k == "01"));
}

/// What `schema::differences` writes for a list of segments, a position being digits.
fn spelled_by_guatiao(segments: &[&str]) -> String {
    Difference {
        path: segments.iter().map(|s| (*s).to_owned()).collect(),
        value: None,
        baseline: None,
    }
    .key()
}

fn spelled_here(segments: &[&str]) -> String {
    KeyPath::new(
        segments
            .iter()
            .map(|s| match s.parse::<usize>() {
                Ok(index) if s.bytes().all(|b| b.is_ascii_digit()) => Segment::Index(index),
                _ => Segment::Key((*s).to_owned()),
            })
            .collect(),
    )
    .to_string()
}

#[test]
fn a_path_is_written_as_differences_writes_it() {
    let cases: &[&[&str]] = &[
        &["a"],
        &["waiting", "per_source"],
        &["peers", "0", "name"],
        &["peers", "12"],
        &["env", "a.b"],
        &["env", "PATH"],
        &["env", "say \"hi\""],
        &["a", "b", "c"],
    ];
    for segments in cases {
        assert_eq!(
            spelled_here(segments),
            spelled_by_guatiao(segments),
            "{segments:?}"
        );
    }
}
