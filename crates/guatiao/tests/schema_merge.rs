// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A field declaring how its own value layers: the `x-merge` annotation.
//!
//! These live out here rather than beside the code because the merge
//! module forbids `unsafe`, and a test file is free of that rule. The
//! thing under test contains no `unsafe` at all.

use guatiao::schema::build::{FieldBuilder, KindBuilder, SchemaBuilder};
use guatiao::schema::merge::{
    DeclaredMerge, X_MERGE, annotation, declared_for, merge_options, merge_overrides,
    merge_with_schema, parse_mode,
};
use guatiao::schema::read::SchemaRef;
use guatiao::value::alloc::Alloc;
use guatiao::value::read::items;
use guatiao::{MergeMode, MergeOptions, Value};

/// A schema carrying one string field per pair, annotated whenever the
/// declaration is not empty.
///
/// The schema builders still name an allocator — a schema is a value a
/// host may have to own — while the annotation itself is an ordinary
/// Rust-heap string, which is why it arrives already built, wrapped in
/// the `Ok` the builder's error-carrying signature expects.
fn schema_with(alloc: Alloc, pairs: &[(&str, &str)]) -> Value {
    let mut builder = SchemaBuilder::new_in(alloc);
    for (key, declaration) in pairs {
        let mut field = FieldBuilder::new_in(alloc, key, KindBuilder::string_in(alloc));
        if !declaration.is_empty() {
            field = field.option(X_MERGE, Value::string(declaration));
        }
        builder = builder.field(field);
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
    let schema = SchemaBuilder::new_in(alloc)
        .field(
            FieldBuilder::new_in(alloc, "k", KindBuilder::string_in(alloc))
                .option(X_MERGE, Value::from(2i64)),
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

/// `mergelists` resolves to one flag for the whole merge, on if any field
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

// --- declarations below the top level ------------------------------------

/// A declaration on a field of a NESTED object governs that field's own
/// dotted path.
///
/// It was read from top-level fields only, so `x-merge` two levels down
/// did nothing at all — silently, which is the worst way for a
/// declaration to fail. The doc claimed the opposite: that a dotted field
/// KEY declared a nested path, which `flat::check_keys` refuses at
/// declaration and `SchemaBuilder::finish` now refuses too.
#[test]
fn a_declaration_on_a_nested_field_is_read_as_its_dotted_path() {
    let alloc = Alloc::rust();

    let schema = SchemaBuilder::new_in(alloc)
        .field(FieldBuilder::new_in(
            alloc,
            "tls",
            KindBuilder::map_in(
                alloc,
                vec![
                    FieldBuilder::new_in(alloc, "ciphers", KindBuilder::string_in(alloc))
                        .option(X_MERGE, Value::string("substitute")),
                    FieldBuilder::new_in(alloc, "roots", KindBuilder::string_in(alloc)),
                ],
            ),
        ))
        .field(
            FieldBuilder::new_in(alloc, "tags", KindBuilder::string_in(alloc))
                .option(X_MERGE, Value::string("deep+mergelists")),
        )
        .finish()
        .expect("a schema this small builds");

    let read = SchemaRef::new(&schema).expect("a schema is a map");
    let overrides = merge_overrides(read);
    assert_eq!(
        overrides.get("tls.ciphers"),
        Some(MergeMode::Substitute),
        "a nested field declares the path it occupies"
    );
    assert_eq!(overrides.get("tags"), Some(MergeMode::Deep));
    assert_eq!(
        overrides.get("tls"),
        None,
        "an owner declares nothing of its own"
    );
    assert_eq!(overrides.get("tls.roots"), None);

    // And the nested declaration is what the merge then applies: `deep`
    // would union the two lists, `substitute` replaces them.
    let earlier = nested("ciphers", &["a", "b"]);
    let later = nested("ciphers", &["c"]);
    let merged = merge_with_schema(read, MergeMode::Deep, &earlier, &later, alloc)
        .expect("the two agree in shape");
    let tls = merged.get("tls").expect("the nested map survives");
    assert_eq!(
        strings_at(tls, "ciphers"),
        ["c"],
        "the nested declaration won over the call-site mode"
    );
}

/// A `mergelists` asked for below the top level is still asked for.
#[test]
fn a_nested_mergelists_declaration_is_resolved_across_the_schema() {
    let alloc = Alloc::rust();
    let schema = SchemaBuilder::new_in(alloc)
        .field(FieldBuilder::new_in(
            alloc,
            "tls",
            KindBuilder::map_in(
                alloc,
                vec![
                    FieldBuilder::new_in(alloc, "ciphers", KindBuilder::string_in(alloc))
                        .option(X_MERGE, Value::string("deep+mergelists")),
                ],
            ),
        ))
        .finish()
        .expect("a schema this small builds");

    let read = SchemaRef::new(&schema).expect("a schema is a map");
    assert_eq!(
        merge_options(read),
        MergeOptions::new().with_mergelists(true)
    );
}

/// A map of one nested map holding one list.
fn nested(key: &str, values: &[&str]) -> Value {
    let mut inner = Value::map();
    inner.set(key, list(values)).unwrap();
    let mut outer = Value::map();
    outer.set("tls", inner).unwrap();
    outer
}
