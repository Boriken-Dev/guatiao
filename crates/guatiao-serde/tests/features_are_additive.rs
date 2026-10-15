// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A default build of this crate changes nothing for the other crates in
//! the same build.
//!
//! Cargo unifies a dependency's features across a build, so a feature here
//! that turns one on in `serde_json` reaches every crate that uses it.
//! `serde_json/arbitrary_precision` is the one that matters: it breaks
//! `#[serde(untagged)]` and `#[serde(flatten)]` numbers, so it is the
//! opt-in `exact-numbers` feature and not part of `json`.

#![cfg(all(feature = "json", not(feature = "exact-numbers")))]

use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Deserialize, PartialEq)]
#[serde(untagged)]
enum Setting {
    Number(f64),
    Text(String),
}

#[derive(Debug, Deserialize, PartialEq)]
struct Outer {
    name: String,
    #[serde(flatten)]
    rest: HashMap<String, f64>,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "kind")]
enum Tagged {
    Port { port: u16 },
}

#[test]
fn an_untagged_number_still_reads() {
    assert_eq!(
        serde_json::from_str::<Setting>("1.5").unwrap(),
        Setting::Number(1.5)
    );
    assert_eq!(
        serde_json::from_str::<Setting>(r#""text""#).unwrap(),
        Setting::Text("text".to_string())
    );
}

#[test]
fn a_flattened_number_still_reads() {
    let read: Outer = serde_json::from_str(r#"{"name":"x","ratio":1.5}"#).unwrap();
    assert_eq!(read.rest.get("ratio"), Some(&1.5));
}

#[test]
fn an_internally_tagged_number_still_reads() {
    assert_eq!(
        serde_json::from_str::<Tagged>(r#"{"kind":"Port","port":5900}"#).unwrap(),
        Tagged::Port { port: 5900 }
    );
}
