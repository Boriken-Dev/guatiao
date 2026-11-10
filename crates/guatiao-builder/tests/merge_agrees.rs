// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Laying one profile over another takes the later per key, as `MergeMode::Substitute` lays
//! one value over another, for layers of leaves.

mod support;

use coqui_builder::describe::Kind;
use coqui_builder::path::Segment;
use coqui_builder::{Describe, Description, Expect, Inspect, KeyPath, Profile};
use guatiao::{Alloc, MergeMode, Value};
use guatiao_builder::ValueView;
use support::*;

#[derive(Describe)]
struct Layered {
    x: u32,
    y: String,
    z: bool,
    inner: Inner,
}

#[derive(Describe)]
struct Inner {
    p: u8,
    q: String,
}

/// An `Equal` per leaf a value holds: what a profile of a layer says.
fn profile_of(description: &Description, value: &Value) -> Profile {
    fn walk(
        description: &Description,
        view: &ValueView<'_>,
        at: &mut Vec<Segment>,
        out: &mut Profile,
    ) {
        if let Kind::Group(group) = description.kind() {
            for field in &group.fields {
                at.push(Segment::Key(field.key.clone()));
                walk(&field.description, view, at, out);
                at.pop();
            }
        } else if let Some(datum) = view.at(at) {
            out.insert(
                KeyPath::new(at.clone()),
                Expect::Equal(datum.text().to_owned()),
            );
        }
    }
    let mut profile = Profile::new();
    walk(
        description,
        &ValueView::new(description, value),
        &mut Vec::new(),
        &mut profile,
    );
    profile
}

fn entries(profile: &Profile) -> Vec<(String, Expect)> {
    let mut out: Vec<(String, Expect)> = profile
        .entries()
        .iter()
        .map(|(key, expect)| (key.to_string(), expect.clone()))
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[test]
fn the_later_profile_wins_per_key_as_the_later_value_does() {
    let description = Layered::description();
    let earlier = parse(r#"{"x": 1, "y": "a", "inner": {"p": 1, "q": "b"}}"#);
    let later = parse(r#"{"y": "c", "z": true, "inner": {"q": "d"}}"#);
    let merged = MergeMode::Substitute
        .merge(&earlier, &later, Alloc::rust())
        .unwrap();

    let by_profile = profile_of(&description, &earlier).merge(&profile_of(&description, &later));
    let by_value = profile_of(&description, &merged);
    assert_eq!(entries(&by_profile), entries(&by_value));
    assert_eq!(by_profile.len(), 5);
}

#[test]
fn a_key_only_one_layer_holds_stands() {
    let description = Layered::description();
    let earlier = parse(r#"{"x": 1}"#);
    let later = parse(r#"{"y": "c"}"#);
    let merged = MergeMode::Substitute
        .merge(&earlier, &later, Alloc::rust())
        .unwrap();
    assert_eq!(
        entries(&profile_of(&description, &earlier).merge(&profile_of(&description, &later))),
        entries(&profile_of(&description, &merged))
    );
}
