// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! An option declaring how its own value layers: the `x-merge` annotation.
//!
//! These live out here rather than beside the code because the merge
//! module forbids `unsafe`, and a test file is free of that rule. The
//! thing under test contains no `unsafe` at all.

use guatiao::schema::build::{KindBuilder, OptionBuilder, SchemaBuilder};
use guatiao::schema::merge::{
    DeclaredMerge, X_MERGE, annotation, declared_for, merge_options, merge_overrides,
    merge_with_schema, parse_mode,
};
use guatiao::schema::read::SchemaRef;
use guatiao::value::alloc::Alloc;
use guatiao::value::read::items;
use guatiao::{MergeMode, MergeOptions, Value};

/// A schema carrying one string option per pair, annotated whenever the
/// declaration is not empty.
///
/// The schema builders still name an allocator — a schema is a value a
/// host may have to own — while the annotation itself is an ordinary
/// Rust-heap string, which is why it arrives already built, wrapped in
/// the `Ok` the builder's error-carrying signature expects.
fn schema_with(alloc: Alloc, pairs: &[(&str, &str)]) -> Value {
    let mut builder = SchemaBuilder::new(alloc);
    for (key, declaration) in pairs {
        let mut option = OptionBuilder::new(alloc, key, KindBuilder::string(alloc));
        if !declaration.is_empty() {
            option = option.extra(X_MERGE, Ok(Value::string(declaration)));
        }
        builder = builder.option(option);
    }
    builder.finish().expect("a schema this small builds")
}

/// A map holding one list per pair, which is the shape every merge case
/// below needs.
fn map_of_lists(pairs: &[(&str, &[&str])]) -> Value {
    let mut built = Value::map();
    for (key, values) in pairs {
        built.set(key, list(values)).unwrap();
    }
    built
}

fn list(values: &[&str]) -> Value {
    let mut built = Value::list();
    for value in values {
        built.push(Value::string(value)).unwrap();
    }
    built
}

/// The strings of the list under `key`.
fn strings_at(value: &Value, key: &str) -> Vec<String> {
    items(value.get(key).expect("the key was merged"))
        .map(|v| v.as_str().unwrap_or("").to_string())
        .collect()
}

#[test]
fn every_mode_spelling_parses() {
    assert_eq!(parse_mode("simple").unwrap().mode, MergeMode::Simple);
    assert_eq!(
        parse_mode("substitute").unwrap().mode,
        MergeMode::Substitute
    );
    assert_eq!(parse_mode("deep").unwrap().mode, MergeMode::Deep);
    // Case and surrounding whitespace are ignored: these are hand-written
    // in declarations and in config files.
    assert_eq!(parse_mode("  DEEP  ").unwrap().mode, MergeMode::Deep);
}

#[test]
fn the_mergelists_suffix_parses_and_only_applies_to_deep_in_practice() {
    let declared = parse_mode("deep+mergelists").unwrap();
    assert_eq!(declared.mode, MergeMode::Deep);
    assert!(declared.options.mergelists);
    assert!(!parse_mode("deep").unwrap().options.mergelists);
}

/// An unrecognised spelling is "not declared", never an error.
///
/// A schema written against a newer vocabulary must stay readable by an
/// older consumer, and the fallback — the call-site mode — is a defined
/// answer.
#[test]
fn an_unrecognised_spelling_falls_back_rather_than_failing() {
    assert_eq!(parse_mode("recursive-ish"), None);
    assert_eq!(parse_mode(""), None);
    assert_eq!(
        parse_mode("substitue"),
        None,
        "a typo is undeclared, not an error"
    );
}

/// A non-string `x-merge` is ignored rather than fatal: an advisory
/// annotation of the wrong kind must not break a working config.
#[test]
fn a_non_string_annotation_is_ignored() {
    let alloc = Alloc::rust();
    let schema = SchemaBuilder::new(alloc)
        .option(
            OptionBuilder::new(alloc, "k", KindBuilder::string(alloc))
                .extra(X_MERGE, Ok(Value::int(2))),
        )
        .finish()
        .unwrap();
    let s = SchemaRef::new(&schema).unwrap();
    assert_eq!(declared_for(s.find("k").unwrap()), None);
}

#[test]
fn overrides_carry_only_the_declared_options() {
    let alloc = Alloc::rust();
    let schema = schema_with(
        alloc,
        &[("tags", "deep"), ("fallbacks", ""), ("mode", "simple")],
    );
    let overrides = merge_overrides(SchemaRef::new(&schema).unwrap());
    assert_eq!(overrides.get("tags"), Some(MergeMode::Deep));
    assert_eq!(overrides.get("mode"), Some(MergeMode::Simple));
    assert_eq!(overrides.get("fallbacks"), None);
    assert_eq!(overrides.get("never-mentioned"), None);
}

#[test]
fn a_schema_that_declares_nothing_produces_no_overrides() {
    let alloc = Alloc::rust();
    let schema = schema_with(alloc, &[("a", ""), ("b", "")]);
    let s = SchemaRef::new(&schema).unwrap();
    assert!(merge_overrides(s).is_empty());
    assert!(!merge_options(s).mergelists);
}

/// **The behaviour the merge design asks for**: a declared mode beats the
/// call-site default for THAT KEY ONLY.
#[test]
fn x_merge_overrides_the_call_site_mode_for_that_key_only() {
    let alloc = Alloc::rust();
    let schema = schema_with(alloc, &[("tags", "deep"), ("fallbacks", "")]);
    let s = SchemaRef::new(&schema).unwrap();

    let earlier = map_of_lists(&[("tags", &["prod", "eu"]), ("fallbacks", &["a", "b"])]);
    let later = map_of_lists(&[("tags", &["canary"]), ("fallbacks", &["c"])]);

    // The call site says Substitute: lists replace. The schema says `tags`
    // is an unordered set, so that key unions instead.
    let merged = merge_with_schema(s, MergeMode::Substitute, &earlier, &later, alloc).unwrap();

    assert_eq!(
        strings_at(&merged, "tags"),
        ["prod", "eu", "canary"],
        "the declaration won for `tags`"
    );
    assert_eq!(
        strings_at(&merged, "fallbacks"),
        ["c"],
        "every undeclared key keeps the call-site mode"
    );
}

/// A key the schema does not mention at all still merges, on the call-site
/// mode. A values map may legitimately carry more than its schema
/// describes, and a merge must not depend on the schema being exhaustive.
#[test]
fn a_key_the_schema_does_not_mention_still_merges() {
    let alloc = Alloc::rust();
    let schema = schema_with(alloc, &[("known", "deep")]);
    let earlier = map_of_lists(&[("unknown", &["a", "b"])]);
    let later = map_of_lists(&[("unknown", &["c"])]);
    let merged = merge_with_schema(
        SchemaRef::new(&schema).unwrap(),
        MergeMode::Substitute,
        &earlier,
        &later,
        alloc,
    )
    .unwrap();
    assert_eq!(
        strings_at(&merged, "unknown"),
        ["c"],
        "call-site Substitute replaced it"
    );
}

/// `mergelists` resolves to one flag for the whole merge, on if any option
/// asked. Applying it slightly more widely than asked beats having a
/// written declaration silently do nothing.
#[test]
fn mergelists_is_on_when_any_option_declares_it() {
    let alloc = Alloc::rust();
    let on = schema_with(alloc, &[("a", "deep+mergelists"), ("b", "deep")]);
    let off = schema_with(alloc, &[("a", "deep"), ("b", "simple")]);
    assert!(merge_options(SchemaRef::new(&on).unwrap()).mergelists);
    assert!(!merge_options(SchemaRef::new(&off).unwrap()).mergelists);
}

/// The annotation helper and the parser are inverses, so a declaration
/// written through the helper cannot be a typo the parser silently ignores.
#[test]
fn annotation_round_trips_through_parse_mode() {
    let alloc = Alloc::rust();
    for mode in [MergeMode::Simple, MergeMode::Substitute, MergeMode::Deep] {
        for mergelists in [false, true] {
            let declared = DeclaredMerge {
                mode,
                options: MergeOptions::new().with_mergelists(mergelists),
            };
            let written = annotation(alloc, declared).unwrap();
            let text = written.as_str().expect("a string annotation");
            let parsed = parse_mode(text).unwrap();
            assert_eq!(parsed, declared, "{text} must parse back to what wrote it");
        }
    }
}
