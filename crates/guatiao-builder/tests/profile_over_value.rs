// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A guatiao value judged through `ValueView` lists the keys, and for each the reason, that
//! `schema::differences` lists for the same value against the same declaration's defaults.
//! The values shown differ on purpose (`Type(text)` against bare text), so only key paths and
//! reasons are compared.

extern crate alloc;

mod support;

use alloc::collections::BTreeMap;

use coqui_builder::{
    Check, Describe, Expect, Profile, Reason, Report, Shown, differences as coqui_differences,
};
use guatiao::schema::SchemaRef;
use guatiao::schema::differences as guatiao_differences;
use guatiao::{FromValue, Schema, ToValue, Value};
use guatiao_builder::{ValueView, to_value};
use support::*;

#[derive(Describe, ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
struct Tuned {
    #[map(default = 10)]
    total: u32,
    #[map(default)]
    debug: bool,
    #[map(default = Level::Warning)]
    level: Level,
    #[map(default)]
    ports: Vec<u16>,
    #[map(default)]
    env: BTreeMap<String, String>,
    #[map(default = Waiting { total: 32, per_source: Some(8) })]
    waiting: Waiting,
    #[map(default = Some(5u16), none = "off")]
    cap: Option<u16>,
    #[map(default = Credential::Anonymous)]
    auth: Credential,
    #[schema(sensitive)]
    #[map(default)]
    token: String,
}

fn base() -> Tuned {
    Tuned {
        total: 10,
        debug: false,
        level: Level::Warning,
        ports: Vec::new(),
        env: BTreeMap::new(),
        waiting: Waiting {
            total: 32,
            per_source: Some(8),
        },
        cap: Some(5),
        auth: Credential::Anonymous,
        token: String::new(),
    }
}

fn changed() -> Tuned {
    let mut value = base();
    value.total = 40;
    value.level = Level::Off;
    value.ports = vec![80, 443];
    value.env = BTreeMap::from([("a.b".to_owned(), "1".to_owned())]);
    value.waiting.per_source = None;
    value.cap = None;
    value.token = "hunter2".into();
    value
}

/// The keys guatiao lists for `value`, with a value present.
fn guatiao_keys(value: &Value) -> Vec<String> {
    let schema = Tuned::schema().unwrap();
    let mut keys: Vec<String> =
        guatiao_differences(SchemaRef::new(&schema).unwrap(), value, None, "***")
            .unwrap()
            .iter()
            .map(|d| d.key())
            .collect();
    keys.sort();
    keys
}

fn coqui_keys(report: &Report) -> Vec<String> {
    let mut keys: Vec<String> = report.unmet().map(|f| f.key().to_string()).collect();
    keys.sort();
    keys
}

#[test]
fn the_same_keys_differ_from_the_defaults() {
    let value = to_value(&changed()).unwrap();
    let theirs = guatiao_keys(&value);
    assert!(!theirs.is_empty());
    let description = Tuned::description();
    let report = coqui_differences(&description, &ValueView::new(&description, &value), None);
    let ours = coqui_keys(&report);
    assert_eq!(ours, theirs, "(bridge | guatiao)");
    assert!(report.unmet().all(|f| f.reason() == Some(&Reason::Differs)));
}

/// A changed arm is one place in both; guatiao names the field and coqui-builder its tag.
#[test]
fn a_changed_arm_is_one_place_at_the_field_or_its_tag() {
    let mut value = base();
    value.auth = Credential::KeyFile { path: "/k".into() };
    let written = to_value(&value).unwrap();
    assert_eq!(guatiao_keys(&written), ["auth"]);
    let description = Tuned::description();
    let report = coqui_differences(&description, &ValueView::new(&description, &written), None);
    assert_eq!(coqui_keys(&report), ["auth.kind"]);
}

#[test]
fn the_default_value_differs_nowhere() {
    let value = to_value(&base()).unwrap();
    assert_eq!(guatiao_keys(&value), Vec::<String>::new());
    let description = Tuned::description();
    let report = coqui_differences(&description, &ValueView::new(&description, &value), None);
    assert_eq!(coqui_keys(&report), Vec::<String>::new());
}

#[test]
fn a_value_judged_by_the_default_profile_fails_where_it_differs() {
    let value = to_value(&changed()).unwrap();
    let description = Tuned::description();
    let profile = Profile::defaults::<Tuned>();
    let mut report = Report::new(&description);
    profile.judge(
        &description,
        &ValueView::new(&description, &value),
        &mut report,
    );
    let ours = coqui_keys(&report);
    // Every key guatiao lists is among the unmet, except the two places a default profile
    // says nothing about: the entries of an open map, and a secret, which it only requires.
    for key in guatiao_keys(&value) {
        if key.starts_with("env[") || key == "token" {
            continue;
        }
        assert!(ours.contains(&key), "{key} is not unmet in {ours:?}");
    }
    let mut met = Report::new(&description);
    let still = to_value(&base()).unwrap();
    profile.judge(
        &description,
        &ValueView::new(&description, &still),
        &mut met,
    );
    assert!(met.is_met(), "{:?}", coqui_keys(&met));
}

#[test]
fn a_secret_is_listed_and_never_shown() {
    let value = to_value(&changed()).unwrap();
    let description = Tuned::description();
    let report = coqui_differences(&description, &ValueView::new(&description, &value), None);
    let token = report
        .unmet()
        .find(|f| f.key().to_string() == "token")
        .expect("the changed token is listed");
    let Shown::One(datum) = token.value() else {
        panic!("a value: {:?}", token.value());
    };
    assert!(datum.is_sensitive());
    assert!(!datum.to_string().contains("hunter2"));
    let schema = Tuned::schema().unwrap();
    let theirs =
        guatiao_differences(SchemaRef::new(&schema).unwrap(), &value, None, "***").unwrap();
    let theirs = theirs.iter().find(|d| d.key() == "token").unwrap();
    assert_eq!(
        <&str>::try_from(theirs.value.as_ref().unwrap()).unwrap(),
        "***"
    );
}

#[test]
fn a_profile_expectation_reads_the_leafs_own_text() {
    let value = to_value(&changed()).unwrap();
    let description = Tuned::description();
    let profile = Profile::new()
        .expecting("total", Expect::Equal("40".into()))
        .unwrap()
        .expecting("level", Expect::OneOf(vec!["Off".into(), "On".into()]))
        .unwrap();
    let mut report = Report::new(&description);
    profile.judge(
        &description,
        &ValueView::new(&description, &value),
        &mut report,
    );
    assert!(report.is_met(), "{:?}", coqui_keys(&report));
}

#[allow(dead_code)]
fn unused(_: &dyn Check<Tuned>) {}

#[derive(Describe)]
#[map(tag = "k")]
enum Share {
    A { port: u16 },
    B { port: String },
}

#[test]
fn a_key_arms_share_is_read_as_the_arm_the_value_names() {
    use coqui_builder::{Inspect, Segment};
    let description = Share::description();
    let value = parse(r#"{"k": "B", "port": "x"}"#);
    let view = ValueView::new(&description, &value);
    assert_eq!(
        view.at(&[Segment::Key("port".into())]).unwrap().ty(),
        "String"
    );
    let value = parse(r#"{"k": "A", "port": 80}"#);
    let view = ValueView::new(&description, &value);
    assert_eq!(view.at(&[Segment::Key("port".into())]).unwrap().ty(), "u16");
}
