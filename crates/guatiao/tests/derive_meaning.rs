// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What each spelling of `#[map(..)]` and `#[schema(..)]` means, rendered from
//! `crates/guatiao-derive/tests/vectors/attributes.json`: the vectors a second
//! derive that reads the same attributes is held to as well, so a type can move
//! between the two. Refusals are held by the derive crate's own tests.
//!
//! Between the markers below is generated: `GUATIAO_WRITE_MEANING=1 cargo test
//! -p guatiao --all-features --test derive_meaning` rewrites it, and
//! `the_rendered_cases_are_current` fails when it is not what the vectors render to.

#![cfg(feature = "derive")]
#![allow(dead_code, unused)]
// The cases are the vectors' items as written, which name `std` paths.
#![allow(clippy::std_instead_of_core, clippy::std_instead_of_alloc)]

use std::fs;
use std::path::Path;

use guatiao::schema::read::{FieldRef, Kind, SchemaRef};
use guatiao::{Alloc, Schema, Value};
use serde_json::json;

#[path = "vectors/read.rs"]
mod vectors;

// ---- helpers the rendered cases call ----

/// The kind a type describes, as the one field `_` of a document.
fn kind_of<T: Schema>() -> Value {
    T::kind(Alloc::rust()).finish().expect("a schema")
}

fn root(kind: &Value) -> FieldRef<'_> {
    FieldRef::new("_", kind).expect("a field")
}

fn kind_name(kind: Kind<'_>) -> &'static str {
    match kind {
        Kind::Map(_) => "group",
        Kind::MapOf(_) => "map",
        Kind::Enum(_) => "choice",
        Kind::Variant { .. } => "variant",
        Kind::Union(_) => "union",
        Kind::Int { .. } => "int",
        Kind::Str { .. } => "text",
        Kind::List { .. } => "list",
        _ => "other",
    }
}

fn group<'a>(kind: &'a Value) -> SchemaRef<'a> {
    let Kind::Map(object) = root(kind).kind() else {
        panic!("not a group: {:?}", root(kind).kind());
    };
    SchemaRef::new(object).expect("an object")
}

fn keys(kind: &Value) -> Vec<String> {
    group(kind).fields().map(|f| f.key().to_owned()).collect()
}

fn required(kind: &Value) -> Vec<String> {
    group(kind)
        .fields()
        .filter(|f| f.is_required())
        .map(|f| f.key().to_owned())
        .collect()
}

fn sensitive(kind: &Value) -> Vec<String> {
    group(kind)
        .fields()
        .filter(|f| f.is_sensitive())
        .map(|f| f.key().to_owned())
        .collect()
}

fn field<'a>(kind: &'a Value, key: &str) -> FieldRef<'a> {
    group(kind)
        .find(key)
        .unwrap_or_else(|| panic!("no field {key}"))
}

fn names(kind: &Value) -> Vec<String> {
    let field = root(kind);
    match field.kind() {
        Kind::Enum(_) => field
            .kind()
            .choices()
            .map(|c| c.value().to_owned())
            .collect(),
        // An open enumeration is a string offering its names.
        _ => field.suggestions().map(|c| c.value().to_owned()).collect(),
    }
}

fn tag(kind: &Value) -> String {
    match root(kind).kind() {
        Kind::Variant { tag, .. } => tag.to_owned(),
        other => panic!("not a variant: {other:?}"),
    }
}

fn arms(kind: &Value) -> Vec<String> {
    root(kind)
        .kind()
        .arms()
        .map(|a| a.value().to_owned())
        .collect()
}

fn default_of(kind: &Value, key: &str) -> Value {
    field(kind, key)
        .default()
        .unwrap_or_else(|| panic!("no default for {key}"))
        .clone()
}

/// The text a field writes `None` as: the one text of the first arm of its
/// `anyOf`.
fn none_of(kind: &Value, key: &str) -> Option<String> {
    let field = field(kind, key);
    let Kind::Union(_) = field.kind() else {
        return None;
    };
    let first = field.kind().alternatives().next()?;
    first.choices().next().map(|c| c.value().to_owned())
}

fn closed(kind: &Value) -> bool {
    let Ok(object) = <&guatiao::Map>::try_from(root(kind).as_value()) else {
        return false;
    };
    object.get("additionalProperties") == Some(&Value::from(false))
}

// ---- the rendering ----

const BEGIN: &str = concat!("// BEGIN ", "GENERATED");
const END: &str = concat!("// END ", "GENERATED");

fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches('_').to_owned()
}

/// The name of the last derived item of `item`, and whether it is generic.
fn last_item(item: &str) -> (String, bool) {
    let piece = item.rsplit("{derive}").next().expect("a piece");
    let at = piece
        .find("struct ")
        .map(|i| i + "struct ".len())
        .or_else(|| piece.find("enum ").map(|i| i + "enum ".len()))
        .expect("an item");
    let rest = &piece[at..];
    let end = rest
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    let generic = rest[end..].starts_with('<');
    (rest[..end].to_owned(), generic)
}

fn literal(text: &str) -> String {
    format!("{text:?}")
}

fn strings(value: &serde_json::Value) -> String {
    let items: Vec<String> = value
        .as_array()
        .expect("a list")
        .iter()
        .map(|v| literal(v.as_str().expect("text")))
        .collect();
    if items.is_empty() {
        return "Vec::<&str>::new()".to_owned();
    }
    format!("vec![{}]", items.join(", "))
}

fn render() -> String {
    let vectors: serde_json::Value = serde_json::from_str(include_str!(
        "../../guatiao-derive/tests/vectors/attributes.json"
    ))
    .expect("valid JSON");
    let mut out = String::new();
    for (index, case) in vectors["cases"].as_array().unwrap().iter().enumerate() {
        if case["guatiao"] != "accept" {
            continue;
        }
        let Some(expect) = case.get("expect") else {
            continue;
        };
        // A `#[builder(..)]` attribute is the other derive's, not an attribute
        // this one declares.
        if case["item"].as_str().unwrap().contains("#[builder") {
            continue;
        }
        let name = case["name"].as_str().unwrap();
        let item = case
            .get("guatiao_item")
            .unwrap_or(&case["item"])
            .as_str()
            .unwrap();
        let (ty, generic) = last_item(item);
        let module = format!("case_{index:02}_{}", slug(name));
        let derived = item.replace(
            "{derive}",
            "#[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)]",
        );
        let ty_expr = if generic {
            format!("{ty}::<u16>")
        } else {
            ty.clone()
        };
        out.push_str(&format!(
            "#[rustfmt::skip]\n#[allow(dead_code, unused)]\nmod {module} {{\n"
        ));
        out.push_str("    use super::*;\n\n");
        out.push_str(&format!("    {derived}\n\n"));
        out.push_str("    pub fn check() {\n");
        out.push_str(&format!("        let d = kind_of::<{ty_expr}>();\n"));
        if let Some(kind) = expect.get("kind") {
            out.push_str(&format!(
                "        assert_eq!(kind_name(root(&d).kind()), {});\n",
                literal(kind.as_str().unwrap())
            ));
        }
        if let Some(list) = expect.get("keys") {
            out.push_str(&format!(
                "        assert_eq!(keys(&d), {});\n",
                strings(list)
            ));
        }
        if let Some(list) = expect.get("required") {
            out.push_str(&format!(
                "        assert_eq!(required(&d), {});\n",
                strings(list)
            ));
        }
        if let Some(list) = expect.get("sensitive") {
            out.push_str(&format!(
                "        assert_eq!(sensitive(&d), {});\n",
                strings(list)
            ));
        }
        if let Some(list) = expect.get("names") {
            out.push_str(&format!(
                "        assert_eq!(names(&d), {});\n",
                strings(list)
            ));
        }
        if let Some(tag) = expect.get("tag") {
            out.push_str(&format!(
                "        assert_eq!(tag(&d), {});\n",
                literal(tag.as_str().unwrap())
            ));
        }
        if let Some(list) = expect.get("arms") {
            out.push_str(&format!(
                "        assert_eq!(arms(&d), {});\n",
                strings(list)
            ));
        }
        if expect.get("deny_unknown_fields") == Some(&json!(true)) {
            out.push_str("        assert!(closed(&d));\n");
        }
        for (key, want) in expect
            .get("defaults")
            .and_then(serde_json::Value::as_object)
            .into_iter()
            .flatten()
        {
            out.push_str(&format!(
                "        assert!(default_of(&d, {}) == vectors::to_value(&json!({})));\n",
                literal(key),
                want
            ));
        }
        for (key, want) in expect
            .get("descriptions")
            .and_then(serde_json::Value::as_object)
            .into_iter()
            .flatten()
        {
            out.push_str(&format!(
                "        assert_eq!(field(&d, {}).description(), {});\n",
                literal(key),
                literal(want.as_str().unwrap())
            ));
        }
        for (key, want) in expect
            .get("titles")
            .and_then(serde_json::Value::as_object)
            .into_iter()
            .flatten()
        {
            out.push_str(&format!(
                "        assert_eq!(field(&d, {}).title(), {});\n",
                literal(key),
                literal(want.as_str().unwrap())
            ));
        }
        for (key, want) in expect
            .get("field_kinds")
            .and_then(serde_json::Value::as_object)
            .into_iter()
            .flatten()
        {
            out.push_str(&format!(
                "        assert_eq!(kind_name(field(&d, {}).kind()), {});\n",
                literal(key),
                literal(want.as_str().unwrap())
            ));
        }
        for (key, want) in expect
            .get("none")
            .and_then(serde_json::Value::as_object)
            .into_iter()
            .flatten()
        {
            out.push_str(&format!(
                "        assert_eq!(none_of(&d, {}).as_deref(), Some({}));\n",
                literal(key),
                literal(want.as_str().unwrap())
            ));
        }
        out.push_str("    }\n}\n\n");
        out.push_str(&format!(
            "#[test]\nfn {module}() {{\n    {module}::check();\n}}\n\n"
        ));
    }
    out
}

#[test]
fn the_rendered_cases_are_current() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/derive_meaning.rs");
    let source = fs::read_to_string(&path).unwrap();
    let begin = source.rfind(BEGIN).expect("the begin marker") + BEGIN.len();
    let end = source.rfind(END).expect("the end marker");
    let current = &source[begin..end];
    let rendered = format!("\n{}", render());
    if std::env::var_os("GUATIAO_WRITE_MEANING").is_some() {
        let mut next = String::new();
        next.push_str(&source[..begin]);
        next.push_str(&rendered);
        next.push_str(&source[end..]);
        if next != source {
            fs::write(&path, next).unwrap();
        }
        return;
    }
    assert!(
        current == rendered,
        "tests/derive_meaning.rs is not what attributes.json renders to: run \
         `GUATIAO_WRITE_MEANING=1 cargo test -p guatiao --all-features --test derive_meaning` \
         and commit the result"
    );
}

// BEGIN GENERATED
#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_00_fields_keep_their_names {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] struct S { host: String, port: u16 }

    pub fn check() {
        let d = kind_of::<S>();
        assert_eq!(kind_name(root(&d).kind()), "group");
        assert_eq!(keys(&d), vec!["host", "port"]);
        assert_eq!(required(&d), vec!["host", "port"]);
    }
}

#[test]
fn case_00_fields_keep_their_names() {
    case_00_fields_keep_their_names::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_01_rename_all_kebab_case {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] #[map(rename_all = "kebab-case")] struct S { view_only: bool, max_len: u32 }

    pub fn check() {
        let d = kind_of::<S>();
        assert_eq!(kind_name(root(&d).kind()), "group");
        assert_eq!(keys(&d), vec!["view-only", "max-len"]);
    }
}

#[test]
fn case_01_rename_all_kebab_case() {
    case_01_rename_all_kebab_case::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_02_rename_all_screaming_snake_case {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] #[map(rename_all = "SCREAMING_SNAKE_CASE")] struct S { view_only: bool }

    pub fn check() {
        let d = kind_of::<S>();
        assert_eq!(kind_name(root(&d).kind()), "group");
        assert_eq!(keys(&d), vec!["VIEW_ONLY"]);
    }
}

#[test]
fn case_02_rename_all_screaming_snake_case() {
    case_02_rename_all_screaming_snake_case::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_03_rename_all_lowercase {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] #[map(rename_all = "lowercase")] struct S { view_only: bool }

    pub fn check() {
        let d = kind_of::<S>();
        assert_eq!(kind_name(root(&d).kind()), "group");
        assert_eq!(keys(&d), vec!["view_only"]);
    }
}

#[test]
fn case_03_rename_all_lowercase() {
    case_03_rename_all_lowercase::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_04_rename_all_snake_case {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] #[map(rename_all = "snake_case")] struct S { view_only: bool }

    pub fn check() {
        let d = kind_of::<S>();
        assert_eq!(kind_name(root(&d).kind()), "group");
        assert_eq!(keys(&d), vec!["view_only"]);
    }
}

#[test]
fn case_04_rename_all_snake_case() {
    case_04_rename_all_snake_case::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_05_a_field_s_own_rename_wins_over_rename_all {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] #[map(rename_all = "kebab-case")] struct S { #[map(rename = "Host")] host_name: String, max_len: u32 }

    pub fn check() {
        let d = kind_of::<S>();
        assert_eq!(kind_name(root(&d).kind()), "group");
        assert_eq!(keys(&d), vec!["Host", "max-len"]);
    }
}

#[test]
fn case_05_a_field_s_own_rename_wins_over_rename_all() {
    case_05_a_field_s_own_rename_wins_over_rename_all::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_06_a_skipped_field_is_not_a_key {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] struct S { a: u8, #[map(skip)] b: u8 }

    pub fn check() {
        let d = kind_of::<S>();
        assert_eq!(kind_name(root(&d).kind()), "group");
        assert_eq!(keys(&d), vec!["a"]);
        assert_eq!(required(&d), vec!["a"]);
    }
}

#[test]
fn case_06_a_skipped_field_is_not_a_key() {
    case_06_a_skipped_field_is_not_a_key::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_07_a_default_makes_a_key_optional {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] struct S { #[map(default)] n: u32, #[map(default = 5)] m: u32, #[schema(default = 7)] k: u32 }

    pub fn check() {
        let d = kind_of::<S>();
        assert_eq!(kind_name(root(&d).kind()), "group");
        assert_eq!(keys(&d), vec!["n", "m", "k"]);
        assert_eq!(required(&d), vec!["k"]);
        assert!(default_of(&d, "k") == vectors::to_value(&json!(7)));
        assert!(default_of(&d, "m") == vectors::to_value(&json!(5)));
        assert!(default_of(&d, "n") == vectors::to_value(&json!(0)));
    }
}

#[test]
fn case_07_a_default_makes_a_key_optional() {
    case_07_a_default_makes_a_key_optional::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_08_an_option_is_not_required {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] struct S { a: String, b: Option<String> }

    pub fn check() {
        let d = kind_of::<S>();
        assert_eq!(kind_name(root(&d).kind()), "group");
        assert_eq!(keys(&d), vec!["a", "b"]);
        assert_eq!(required(&d), vec!["a"]);
    }
}

#[test]
fn case_08_an_option_is_not_required() {
    case_08_an_option_is_not_required::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_09_a_doc_comment_is_the_description {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] struct S { /// The host name.
    /// Written in DNS form.
    host: String }

    pub fn check() {
        let d = kind_of::<S>();
        assert_eq!(kind_name(root(&d).kind()), "group");
        assert_eq!(keys(&d), vec!["host"]);
        assert_eq!(field(&d, "host").description(), "The host name. Written in DNS form.");
    }
}

#[test]
fn case_09_a_doc_comment_is_the_description() {
    case_09_a_doc_comment_is_the_description::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_10_schema_description_wins_over_the_doc_comment {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] struct S { /// Ignored.
    #[schema(description = "Stated.", title = "Host")] host: String }

    pub fn check() {
        let d = kind_of::<S>();
        assert_eq!(kind_name(root(&d).kind()), "group");
        assert_eq!(keys(&d), vec!["host"]);
        assert_eq!(field(&d, "host").description(), "Stated.");
        assert_eq!(field(&d, "host").title(), "Host");
    }
}

#[test]
fn case_10_schema_description_wins_over_the_doc_comment() {
    case_10_schema_description_wins_over_the_doc_comment::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_11_a_sensitive_field {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] struct S { user: String, #[schema(sensitive)] token: String }

    pub fn check() {
        let d = kind_of::<S>();
        assert_eq!(kind_name(root(&d).kind()), "group");
        assert_eq!(keys(&d), vec!["user", "token"]);
        assert_eq!(sensitive(&d), vec!["token"]);
    }
}

#[test]
fn case_11_a_sensitive_field() {
    case_11_a_sensitive_field::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_12_section_order_advanced_read_only_deprecated_and_bounds {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] struct S { #[schema(section = "Network", order = 2, advanced, read_only, deprecated, min_length = 1, max_length = 8, pattern = "^a", format = "hostname")] name: String, #[schema(multiple_of = 5, min_items = 1, max_items = 2, examples("a".to_string()))] other: String }

    pub fn check() {
        let d = kind_of::<S>();
        assert_eq!(kind_name(root(&d).kind()), "group");
        assert_eq!(keys(&d), vec!["name", "other"]);
    }
}

#[test]
fn case_12_section_order_advanced_read_only_deprecated_and_bounds() {
    case_12_section_order_advanced_read_only_deprecated_and_bounds::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_13_a_flattened_struct_s_keys_sit_beside_the_others {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] struct Inner { x: u8 } #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] struct S { a: u8, #[map(flatten)] inner: Inner }

    pub fn check() {
        let d = kind_of::<S>();
        assert_eq!(kind_name(root(&d).kind()), "group");
        assert_eq!(keys(&d), vec!["a", "x"]);
        assert_eq!(required(&d), vec!["a", "x"]);
    }
}

#[test]
fn case_13_a_flattened_struct_s_keys_sit_beside_the_others() {
    case_13_a_flattened_struct_s_keys_sit_beside_the_others::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_14_deny_unknown_fields {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] #[map(deny_unknown_fields)] struct S { a: u8 }

    pub fn check() {
        let d = kind_of::<S>();
        assert_eq!(kind_name(root(&d).kind()), "group");
        assert_eq!(keys(&d), vec!["a"]);
        assert!(closed(&d));
    }
}

#[test]
fn case_14_deny_unknown_fields() {
    case_14_deny_unknown_fields::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_15_a_transparent_struct_is_its_field {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] #[map(transparent)] struct S { value: u16 }

    pub fn check() {
        let d = kind_of::<S>();
        assert_eq!(kind_name(root(&d).kind()), "int");
    }
}

#[test]
fn case_15_a_transparent_struct_is_its_field() {
    case_15_a_transparent_struct_is_its_field::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_16_a_newtype_is_its_field {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] struct S(u16);

    pub fn check() {
        let d = kind_of::<S>();
        assert_eq!(kind_name(root(&d).kind()), "int");
    }
}

#[test]
fn case_16_a_newtype_is_its_field() {
    case_16_a_newtype_is_its_field::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_17_a_generic_struct {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] struct S<T> { value: T }

    pub fn check() {
        let d = kind_of::<S::<u16>>();
        assert_eq!(kind_name(root(&d).kind()), "group");
        assert_eq!(keys(&d), vec!["value"]);
        assert_eq!(required(&d), vec!["value"]);
    }
}

#[test]
fn case_17_a_generic_struct() {
    case_17_a_generic_struct::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_18_unit_variants_are_a_choice {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] enum E { Fast, SlowDown }

    pub fn check() {
        let d = kind_of::<E>();
        assert_eq!(kind_name(root(&d).kind()), "choice");
        assert_eq!(names(&d), vec!["Fast", "SlowDown"]);
    }
}

#[test]
fn case_18_unit_variants_are_a_choice() {
    case_18_unit_variants_are_a_choice::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_19_rename_all_on_unit_variants {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] #[map(rename_all = "kebab-case")] enum E { Fast, SlowDown, H264 }

    pub fn check() {
        let d = kind_of::<E>();
        assert_eq!(kind_name(root(&d).kind()), "choice");
        assert_eq!(names(&d), vec!["fast", "slow-down", "h264"]);
    }
}

#[test]
fn case_19_rename_all_on_unit_variants() {
    case_19_rename_all_on_unit_variants::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_20_a_variant_s_own_rename_and_label {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] enum E { #[map(rename = "quick")] #[schema(label = "Quick")] Fast, Slow }

    pub fn check() {
        let d = kind_of::<E>();
        assert_eq!(kind_name(root(&d).kind()), "choice");
        assert_eq!(names(&d), vec!["quick", "Slow"]);
    }
}

#[test]
fn case_20_a_variant_s_own_rename_and_label() {
    case_20_a_variant_s_own_rename_and_label::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_21_other_keeps_a_name_no_variant_has {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] enum E { Fast, #[map(other)] Other(String) }

    pub fn check() {
        let d = kind_of::<E>();
        assert_eq!(kind_name(root(&d).kind()), "text");
        assert_eq!(names(&d), vec!["Fast"]);
    }
}

#[test]
fn case_21_other_keeps_a_name_no_variant_has() {
    case_21_other_keeps_a_name_no_variant_has::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_22_a_tagged_enum_has_arms {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] #[map(tag = "kind")] enum E { Password { user: String, #[schema(sensitive)] password: String }, KeyFile { path: String }, Token { #[schema(sensitive)] token: String }, Anonymous }

    pub fn check() {
        let d = kind_of::<E>();
        assert_eq!(kind_name(root(&d).kind()), "variant");
        assert_eq!(tag(&d), "kind");
        assert_eq!(arms(&d), vec!["Password", "KeyFile", "Token", "Anonymous"]);
    }
}

#[test]
fn case_22_a_tagged_enum_has_arms() {
    case_22_a_tagged_enum_has_arms::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_23_rename_all_names_the_arms {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] #[map(tag = "kind", rename_all = "snake_case")] enum E { KeyFile { path: String }, NoAuth }

    pub fn check() {
        let d = kind_of::<E>();
        assert_eq!(kind_name(root(&d).kind()), "variant");
        assert_eq!(tag(&d), "kind");
        assert_eq!(arms(&d), vec!["key_file", "no_auth"]);
    }
}

#[test]
fn case_23_rename_all_names_the_arms() {
    case_23_rename_all_names_the_arms::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_24_a_newtype_arm_holds_a_type_whose_keys_sit_beside_the_tag {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] struct Pw { user: String } #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] #[map(tag = "kind")] enum E { Pw(Pw), None }

    pub fn check() {
        let d = kind_of::<E>();
        assert_eq!(kind_name(root(&d).kind()), "variant");
        assert_eq!(tag(&d), "kind");
        assert_eq!(arms(&d), vec!["Pw", "None"]);
    }
}

#[test]
fn case_24_a_newtype_arm_holds_a_type_whose_keys_sit_beside_the_tag() {
    case_24_a_newtype_arm_holds_a_type_whose_keys_sit_beside_the_tag::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_25_an_untagged_enum_is_told_apart_by_the_kind_of_value {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] #[map(untagged)] enum E { Auto, Number(u16) }

    pub fn check() {
        let d = kind_of::<E>();
        assert_eq!(kind_name(root(&d).kind()), "union");
    }
}

#[test]
fn case_25_an_untagged_enum_is_told_apart_by_the_kind_of_value() {
    case_25_an_untagged_enum_is_told_apart_by_the_kind_of_value::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_26_as_text_writes_with_display_and_reads_with_fromstr {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] struct S { #[map(as_text)] addr: std::net::Ipv4Addr }

    pub fn check() {
        let d = kind_of::<S>();
        assert_eq!(kind_name(root(&d).kind()), "group");
        assert_eq!(keys(&d), vec!["addr"]);
        assert_eq!(required(&d), vec!["addr"]);
        assert_eq!(kind_name(field(&d, "addr").kind()), "text");
    }
}

#[test]
fn case_26_as_text_writes_with_display_and_reads_with_fromstr() {
    case_26_as_text_writes_with_display_and_reads_with_fromstr::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_27_as_text_on_a_list_is_per_item {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] struct S { #[map(default, as_text)] addrs: Vec<std::net::Ipv4Addr> }

    pub fn check() {
        let d = kind_of::<S>();
        assert_eq!(kind_name(root(&d).kind()), "group");
        assert_eq!(keys(&d), vec!["addrs"]);
        assert_eq!(required(&d), Vec::<&str>::new());
        assert_eq!(kind_name(field(&d, "addrs").kind()), "list");
    }
}

#[test]
fn case_27_as_text_on_a_list_is_per_item() {
    case_27_as_text_on_a_list_is_per_item::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_28_as_text_on_an_optional_list_is_per_item {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] struct S { #[map(as_text)] addrs: Option<Vec<std::net::Ipv4Addr>> }

    pub fn check() {
        let d = kind_of::<S>();
        assert_eq!(kind_name(root(&d).kind()), "group");
        assert_eq!(keys(&d), vec!["addrs"]);
        assert_eq!(required(&d), Vec::<&str>::new());
        assert_eq!(kind_name(field(&d, "addrs").kind()), "list");
    }
}

#[test]
fn case_28_as_text_on_an_optional_list_is_per_item() {
    case_28_as_text_on_an_optional_list_is_per_item::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_29_with_names_a_module_that_converts_the_field_a_list_of_protocol_names {
    use super::*;

    mod alpn_list { use guatiao::{Alloc, FromValue, MapError, Schema, ToValue, Value, ValueError}; use guatiao::schema::KindBuilder; pub fn to_value_in(items: &[String], alloc: Alloc) -> Result<Value, ValueError> { items.to_value_in(alloc) } pub fn from_value(value: &Value) -> Result<Vec<String>, MapError> { Vec::<String>::from_value(value) } pub fn kind(alloc: Alloc) -> KindBuilder { KindBuilder::list_in(alloc, String::kind(alloc)) } } #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] struct S { #[map(default, with = alpn_list)] alpn: Vec<String> }

    pub fn check() {
        let d = kind_of::<S>();
        assert_eq!(kind_name(root(&d).kind()), "group");
        assert_eq!(keys(&d), vec!["alpn"]);
        assert_eq!(required(&d), Vec::<&str>::new());
        assert_eq!(kind_name(field(&d, "alpn").kind()), "list");
    }
}

#[test]
fn case_29_with_names_a_module_that_converts_the_field_a_list_of_protocol_names() {
    case_29_with_names_a_module_that_converts_the_field_a_list_of_protocol_names::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_30_with_names_a_module_that_converts_bytes_held_as_text_a_protocol_name {
    use super::*;

    mod alpn_text { use guatiao::{Alloc, FromValue, MapError, Schema, ToValue, Value, ValueError}; use guatiao::schema::KindBuilder; pub fn to_value_in(alpn: &[u8], alloc: Alloc) -> Result<Value, ValueError> { String::from_utf8(alpn.to_vec()).map_err(|_| ValueError::Unrepresentable)?.to_value_in(alloc) } pub fn from_value(value: &Value) -> Result<Vec<u8>, MapError> { Ok(String::from_value(value)?.into_bytes()) } pub fn kind(alloc: Alloc) -> KindBuilder { String::kind(alloc) } } #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] struct S { #[map(default, with = alpn_text)] alpn: Vec<u8> }

    pub fn check() {
        let d = kind_of::<S>();
        assert_eq!(kind_name(root(&d).kind()), "group");
        assert_eq!(keys(&d), vec!["alpn"]);
        assert_eq!(required(&d), Vec::<&str>::new());
        assert_eq!(kind_name(field(&d, "alpn").kind()), "text");
    }
}

#[test]
fn case_30_with_names_a_module_that_converts_bytes_held_as_text_a_protocol_name() {
    case_30_with_names_a_module_that_converts_bytes_held_as_text_a_protocol_name::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_31_an_option_with_a_written_none {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] struct S { #[map(default = Some(std::time::Duration::from_secs(30)), none = "never")] read_timeout: Option<std::time::Duration> }

    pub fn check() {
        let d = kind_of::<S>();
        assert_eq!(kind_name(root(&d).kind()), "group");
        assert_eq!(keys(&d), vec!["read_timeout"]);
        assert_eq!(required(&d), Vec::<&str>::new());
        assert!(default_of(&d, "read_timeout") == vectors::to_value(&json!([30,0])));
        assert_eq!(none_of(&d, "read_timeout").as_deref(), Some("never"));
    }
}

#[test]
fn case_31_an_option_with_a_written_none() {
    case_31_an_option_with_a_written_none::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_33_crate_on_an_enum {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] #[map(crate = guatiao)] enum E { A, B }

    pub fn check() {
        let d = kind_of::<E>();
        assert_eq!(kind_name(root(&d).kind()), "choice");
        assert_eq!(names(&d), vec!["A", "B"]);
    }
}

#[test]
fn case_33_crate_on_an_enum() {
    case_33_crate_on_an_enum::check();
}

#[rustfmt::skip]
#[allow(dead_code, unused)]
mod case_34_arm_title_and_description {
    use super::*;

    #[derive(::guatiao::ToValue, ::guatiao::FromValue, ::guatiao::Schema)] #[map(tag = "kind")] enum E { #[schema(title = "Password", description = "A user and a password.")] Pw { user: String }, None }

    pub fn check() {
        let d = kind_of::<E>();
        assert_eq!(kind_name(root(&d).kind()), "variant");
        assert_eq!(tag(&d), "kind");
        assert_eq!(arms(&d), vec!["Pw", "None"]);
    }
}

#[test]
fn case_34_arm_title_and_description() {
    case_34_arm_title_and_description::check();
}

// END GENERATED
