// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The one-line text of nodes a checked builder refuses (`Display for Value`), built
//! over borrowed bytes: here because that takes `unsafe`, which the writer's
//! own module forbids.

use alloc::string::ToString;

use crate::value::types::{Map, Number, Payload, Tag, Text, Value};

fn text(v: Value) -> alloc::string::String {
    v.to_string()
}

fn borrowed(bytes: &[u8]) -> Text {
    // SAFETY: a borrowed view (`cap == 0`, no allocator) of bytes the
    // caller keeps alive; only the content may be wrong.
    unsafe { Text::from_raw_parts(bytes.as_ptr().cast_mut(), bytes.len(), 0, core::ptr::null()) }
}

/// The vectors only a raw construction can make: text, a number or a
/// map key a checked reader would refuse.
#[test]
fn the_unreadable_vectors_print_their_placeholders() {
    let doc: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/vectors/display.json")).unwrap();
    let cases = doc["unreadable"].as_array().unwrap();
    assert!(cases.len() >= 7);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let hex = case["hex"].as_str().unwrap();
        let bytes: alloc::vec::Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let value = match case["kind"].as_str().unwrap() {
            kind @ ("string" | "number") => {
                let tag = if kind == "string" {
                    Tag::GUATIAO_STRING
                } else {
                    Tag::GUATIAO_NUMBER
                };
                // SAFETY: the shape is right; only the content is wrong.
                unsafe { Value::from_raw_parts(u32::from(tag), Payload::text(borrowed(&bytes))) }
            }
            _ => {
                let mut map = Map::new();
                map.set("ok", Number::new("1").unwrap());
                map.set("zz", Number::new("2").unwrap());
                map.entries_mut()[1].key = borrowed(&bytes);
                Value::from(map)
            }
        };
        assert_eq!(text(value), case["text"].as_str().unwrap(), "{name}");
    }
}
