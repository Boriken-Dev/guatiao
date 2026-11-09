// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `guatiao::value::Inline` prints, for every value JSON can say, the
//! text this crate's JSON writer prints with its default presentation:
//! the line a report can show without a format crate in the build.

#![cfg(feature = "json")]

use guatiao::value::Inline;
use guatiao_serde::Presentation;
use guatiao_serde::text::json;

#[path = "../../guatiao/tests/vectors/read.rs"]
mod vectors;

#[test]
fn inline_text_is_what_the_json_writer_prints() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../guatiao/tests/vectors/inline.json"
    );
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("the vector file is readable"))
            .expect("the vector file is JSON");
    let (mut compared, mut skipped) = (0, 0);
    for case in doc["cases"].as_array().expect("cases") {
        let name = case["name"].as_str().unwrap();
        let value = vectors::tree(&case["tree"]);
        if case.get("json").and_then(serde_json::Value::as_bool) == Some(false) {
            assert!(
                json::to_string(&value, Presentation::new()).is_err(),
                "{name}"
            );
            skipped += 1;
            continue;
        }
        assert_eq!(
            Inline(&value).to_string(),
            json::to_string(&value, Presentation::new()).expect("JSON can say it"),
            "{name}"
        );
        compared += 1;
    }
    assert!(compared >= 20 && skipped == 2, "{compared} {skipped}");
}
