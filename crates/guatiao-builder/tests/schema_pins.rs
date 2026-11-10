// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What the schema made from a description says where it differs from guatiao's own derive
//! on purpose, each against a committed JSON text:
//!
//! - a duration is a number of seconds or the duration text, where guatiao's `Duration` is a
//!   `[secs, nanos]` pair;
//! - an untagged enum's text side is a string, with no mapping from a boolean or a number;
//! - a type's own doc comment is the description of every field of that type.

mod support;

use core::time::Duration;

use coqui_builder::Describe;
use guatiao::Value;
use guatiao_builder::schema;
use support::*;

fn pinned<T: Describe>(expected: &str) {
    let made = schema(&T::description()).expect("a schema");
    let expected = parse(expected);
    assert!(
        made == expected,
        "(made | pinned):\n{}\n--- made\n{}",
        diff(&made, &expected).join("\n"),
        json(&made)
    );
}

#[derive(Describe)]
struct Timeouts {
    idle: Duration,
    #[map(default = Duration::from_secs(30))]
    read: Duration,
    #[map(default = Some(Duration::from_millis(1500)), none = "never")]
    write: Option<Duration>,
    #[map(default)]
    pauses: Vec<Duration>,
}

#[test]
fn a_duration_is_seconds_or_its_text() {
    pinned::<Timeouts>(
        r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "properties": {
    "idle": {
      "anyOf": [
        {"type": "number", "minimum": 0},
        {"type": "string", "pattern": "^[0-9]+(\\.[0-9]+)?(ns|us|ms|s|m|h)?$"}
      ]
    },
    "read": {
      "anyOf": [
        {"type": "number", "minimum": 0},
        {"type": "string", "pattern": "^[0-9]+(\\.[0-9]+)?(ns|us|ms|s|m|h)?$"}
      ],
      "default": "30s"
    },
    "write": {
      "anyOf": [
        {"type": "string", "enum": ["never"]},
        {"anyOf": [
          {"type": "number", "minimum": 0},
          {"type": "string", "pattern": "^[0-9]+(\\.[0-9]+)?(ns|us|ms|s|m|h)?$"}
        ]}
      ],
      "default": "1500ms"
    },
    "pauses": {
      "type": "array",
      "items": {
        "anyOf": [
          {"type": "number", "minimum": 0},
          {"type": "string", "pattern": "^[0-9]+(\\.[0-9]+)?(ns|us|ms|s|m|h)?$"}
        ]
      },
      "default": []
    }
  },
  "additionalProperties": false,
  "required": ["idle"]
}"#,
    );
}

#[derive(Describe)]
struct CacheOptions {
    #[map(default = 8)]
    size: u32,
}

#[derive(Describe)]
#[map(untagged)]
enum Cache {
    Enabled(bool),
    Tuned(CacheOptions),
}

#[derive(Describe)]
struct Holder {
    #[map(default = Cache::Enabled(false))]
    cache: Cache,
}

#[test]
fn an_untagged_enums_text_side_is_a_string() {
    pinned::<Holder>(
        r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "properties": {
    "cache": {
      "anyOf": [
        {"type": "string"},
        {"type": "object",
         "properties": {"size": {"type": "integer", "minimum": 0, "maximum": 4294967295, "default": 8}},
         "additionalProperties": false}
      ],
      "default": "false"
    }
  },
  "additionalProperties": false
}"#,
    );
}

/// Limits on who may wait.
#[derive(Describe)]
struct Documented {
    total: u32,
}

#[derive(Describe)]
struct Outer {
    inner: Documented,
}

#[test]
fn a_types_doc_comment_describes_each_field_of_that_type() {
    pinned::<Outer>(
        r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "properties": {
    "inner": {
      "type": "object",
      "properties": {"total": {"type": "integer", "minimum": 0, "maximum": 4294967295}},
      "additionalProperties": false,
      "required": ["total"],
      "description": "Limits on who may wait."
    }
  },
  "additionalProperties": false,
  "required": ["inner"]
}"#,
    );
}

#[test]
fn an_optional_field_accepts_null_as_guatiaos_derive_does() {
    #[derive(Describe)]
    struct Maybe {
        nickname: Option<String>,
    }
    pinned::<Maybe>(
        r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "properties": {"nickname": {"type": ["string", "null"]}},
  "additionalProperties": false
}"#,
    );
}

#[allow(dead_code)]
fn unused(_: Value) {}
