// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Schemas written by other tools: the shapes a Rust generator that
//! describes serde types emits for an optional field, a unit-variant enum
//! and a shared definition are read, validated and skipped the way the
//! reader's rules say, without a rewrite pass of the host's own.

use guatiao::schema::read::{Kind, SchemaRef};
use guatiao::schema::validate::{validate_map, validate_text};
use guatiao::schema::{inline_refs, vocab};
use guatiao::value::convert::TryAsRef;
use guatiao::{List, Map, Number, Value};

fn obj(pairs: Vec<(&str, Value)>) -> Value {
    let mut m = Map::new();
    for (k, v) in pairs {
        m.set(k, v);
    }
    m.into()
}

fn arr(items: Vec<Value>) -> Value {
    let mut l = List::new();
    for i in items {
        l.push(i);
    }
    l.into()
}

fn s(text: &str) -> Value {
    Value::from(text)
}

fn n(text: &str) -> Value {
    Number::new(text).expect("a JSON number").into()
}

fn typed(name: &str) -> Value {
    obj(vec![("type", s(name))])
}

fn reference(name: &str) -> Value {
    obj(vec![("$ref", s(&format!("#/$defs/{name}")))])
}

/// What serde's generator writes for a struct with an optional string, an
/// optional bounded integer, a unit-variant enum, an optional nested
/// struct and a recursive one.
fn generated() -> Value {
    obj(vec![
        ("$schema", s(vocab::DIALECT)),
        ("title", s("Settings")),
        ("type", s("object")),
        (
            "properties",
            obj(vec![
                ("name", typed("string")),
                (
                    "nick",
                    obj(vec![("type", arr(vec![s("string"), s("null")]))]),
                ),
                (
                    "retries",
                    obj(vec![(
                        "anyOf",
                        arr(vec![
                            obj(vec![("type", s("integer")), ("minimum", n("0"))]),
                            typed("null"),
                        ]),
                    )]),
                ),
                ("mode", reference("Mode")),
                (
                    "tls",
                    obj(vec![("anyOf", arr(vec![reference("Tls"), typed("null")]))]),
                ),
                ("tree", reference("Node")),
            ]),
        ),
        ("required", arr(vec![s("name"), s("mode")])),
        (
            "$defs",
            obj(vec![
                (
                    "Mode",
                    obj(vec![(
                        "oneOf",
                        arr(vec![
                            obj(vec![
                                ("description", s("Quick")),
                                ("type", s("string")),
                                ("const", s("fast")),
                            ]),
                            obj(vec![
                                ("title", s("Slow")),
                                ("type", s("string")),
                                ("const", s("slow")),
                            ]),
                        ]),
                    )]),
                ),
                (
                    "Tls",
                    obj(vec![
                        ("type", s("object")),
                        ("properties", obj(vec![("verify", typed("boolean"))])),
                        ("required", arr(vec![s("verify")])),
                    ]),
                ),
                (
                    "Node",
                    obj(vec![
                        ("type", s("object")),
                        ("properties", obj(vec![("child", reference("Node"))])),
                    ]),
                ),
            ]),
        ),
    ])
}

fn doc(pairs: Vec<(&str, Value)>) -> Value {
    obj(pairs)
}

#[test]
fn an_optional_value_reads_as_the_value_it_holds() {
    let schema = generated();
    let read = SchemaRef::new(&schema).unwrap();

    let nick = read.find("nick").unwrap();
    assert!(matches!(nick.kind(), Kind::Str { .. }));
    assert!(nick.is_nullable());

    let retries = read.find("retries").unwrap();
    assert!(matches!(
        retries.kind(),
        Kind::Int {
            min: Some(0),
            max: None,
            ..
        }
    ));
    assert!(retries.is_nullable());

    assert!(!read.find("name").unwrap().is_nullable());
}

#[test]
fn a_reference_this_view_cannot_follow_is_skipped_not_refused() {
    let schema = generated();
    let read = SchemaRef::new(&schema).unwrap();
    // A skipped field accepts: the provider has the last word.
    for key in ["mode", "tls", "tree"] {
        let field = read.find(key).unwrap();
        assert!(
            matches!(field.kind(), Kind::Unknown(vocab::REF)),
            "{key} is a reference"
        );
    }
    let ok = doc(vec![("name", s("a")), ("mode", s("anything at all"))]);
    assert!(validate_map(read, &ok).is_ok());
}

#[test]
fn shapes_the_reader_does_not_know_are_skipped_never_missing() {
    let schema = obj(vec![
        ("type", s("object")),
        (
            "properties",
            obj(vec![
                (
                    "either",
                    obj(vec![("type", arr(vec![s("string"), s("integer")]))]),
                ),
                (
                    "bare",
                    obj(vec![(
                        "oneOf",
                        arr(vec![typed("string"), typed("integer")]),
                    )]),
                ),
                ("levels", obj(vec![("enum", arr(vec![n("1"), n("2")]))])),
            ]),
        ),
    ]);
    let read = SchemaRef::new(&schema).unwrap();
    assert!(matches!(
        read.find("either").unwrap().kind(),
        Kind::Unknown(vocab::TYPE)
    ));
    assert!(matches!(
        read.find("bare").unwrap().kind(),
        Kind::Unknown(vocab::ONE_OF)
    ));
    let levels = read.find("levels").unwrap().kind();
    assert!(
        matches!(levels, Kind::Enum(_)),
        "an untyped enum is a choice"
    );
    assert_eq!(
        levels.choices().map(|c| c.value()).collect::<Vec<_>>(),
        ["1", "2"]
    );
    assert!(validate_text(read.find("levels").unwrap(), "2").is_ok());
    assert!(validate_text(read.find("levels").unwrap(), "3").is_err());
}

#[test]
fn a_generated_schema_is_read_and_checked_end_to_end() {
    let inlined = inline_refs(&generated()).expect("it inlines");
    let read = SchemaRef::new(&inlined).unwrap();

    // The definitions are spent, and nothing refers to them any more.
    let root = TryAsRef::<Map>::try_as_ref(&inlined).unwrap();
    assert!(root.get("$defs").is_none());

    let mode = read.find("mode").unwrap().kind();
    assert!(matches!(mode, Kind::Enum(_)));
    let choices: Vec<(&str, &str)> = mode.choices().map(|c| (c.value(), c.label())).collect();
    assert_eq!(choices, [("fast", "fast"), ("slow", "Slow")]);

    let tls = read.find("tls").unwrap();
    assert!(matches!(tls.kind(), Kind::Map(_)));
    assert!(tls.is_nullable());

    let good = doc(vec![
        ("name", s("a")),
        ("mode", s("fast")),
        ("nick", Value::null()),
        ("retries", Value::null()),
        ("tls", Value::null()),
    ]);
    assert_eq!(validate_map(read, &good), Ok(()));

    let full = doc(vec![
        ("name", s("a")),
        ("mode", s("slow")),
        ("nick", s("al")),
        ("retries", n("3")),
        ("tls", obj(vec![("verify", Value::from(true))])),
        ("tree", obj(vec![])),
    ]);
    assert_eq!(validate_map(read, &full), Ok(()));

    let refused: Vec<(&str, Value)> = vec![
        ("nick", n("5")),
        ("retries", n("-1")),
        ("retries", s("3")),
        ("mode", s("medium")),
        ("tls", obj(vec![("verify", s("yes"))])),
        ("tls", obj(vec![])),
    ];
    for (key, value) in refused {
        let mut pairs = vec![("name", s("a")), ("mode", s("fast"))];
        pairs.retain(|(k, _)| *k != key);
        pairs.push((key, value));
        assert!(
            validate_map(read, &doc(pairs)).is_err(),
            "{key} should be refused"
        );
    }
    assert!(
        validate_map(read, &doc(vec![("name", s("a"))])).is_err(),
        "mode is required"
    );
}

#[test]
fn a_recursive_definition_is_expanded_once_and_then_skipped() {
    let inlined = inline_refs(&generated()).unwrap();
    let read = SchemaRef::new(&inlined).unwrap();
    let tree = read.find("tree").unwrap().kind();
    let Kind::Map(_) = tree else {
        panic!("the first level is expanded");
    };
    let child = tree.fields().next().unwrap().kind();
    assert!(
        matches!(child, Kind::Unknown(vocab::REF)),
        "the cycle stays a reference to skip"
    );
}

#[test]
fn a_reference_that_goes_nowhere_is_left_as_written() {
    let schema = obj(vec![
        ("type", s("object")),
        (
            "properties",
            obj(vec![
                ("gone", reference("Nowhere")),
                (
                    "remote",
                    obj(vec![("$ref", s("https://example.org/schema.json"))]),
                ),
            ]),
        ),
    ]);
    let inlined = inline_refs(&schema).unwrap();
    let read = SchemaRef::new(&inlined).unwrap();
    for key in ["gone", "remote"] {
        assert!(matches!(
            read.find(key).unwrap().kind(),
            Kind::Unknown(vocab::REF)
        ));
    }
}

#[test]
fn siblings_of_a_reference_win_over_its_target() {
    let schema = obj(vec![
        ("type", s("object")),
        (
            "properties",
            obj(vec![(
                "port",
                obj(vec![
                    ("$ref", s("#/$defs/Port")),
                    ("description", s("The listener")),
                ]),
            )]),
        ),
        (
            "$defs",
            obj(vec![(
                "Port",
                obj(vec![("type", s("integer")), ("description", s("Any port"))]),
            )]),
        ),
    ]);
    let inlined = inline_refs(&schema).unwrap();
    let port = SchemaRef::new(&inlined).unwrap().find("port").unwrap();
    assert!(matches!(port.kind(), Kind::Int { .. }));
    assert_eq!(port.description(), "The listener");
}

#[test]
fn definitions_that_name_each_other_twice_do_not_explode() {
    // Twenty levels, two references each: a million schemas if expanded in
    // full. The budget stops it, and what is left stays a reference.
    let mut defs = Map::new();
    defs.set("d0", typed("string"));
    for i in 1..20 {
        let previous = format!("d{}", i - 1);
        defs.set(
            &format!("d{i}"),
            obj(vec![
                ("type", s("object")),
                (
                    "properties",
                    obj(vec![
                        ("a", reference(&previous)),
                        ("b", reference(&previous)),
                    ]),
                ),
            ]),
        );
    }
    let schema = obj(vec![
        ("type", s("object")),
        ("properties", obj(vec![("top", reference("d19"))])),
        ("$defs", defs.into()),
    ]);
    let inlined = inline_refs(&schema).expect("it finishes");

    // The last branch is expanded after the budget is spent.
    let read = SchemaRef::new(&inlined).unwrap();
    let mut kind = read.find("top").unwrap().kind();
    let mut skipped = false;
    for _ in 0..25 {
        match kind {
            Kind::Map(_) => kind = kind.fields().nth(1).unwrap().kind(),
            Kind::Unknown(vocab::REF) => {
                skipped = true;
                break;
            }
            _ => break,
        }
    }
    assert!(skipped, "past the budget a reference stays as written");
}

/// `{"description": .., "allOf": [X]}` is how a generator puts a
/// description beside a reference, and reads as `X`. A schema that
/// constrains nothing -- `{}`, prose alone, `true`, an `allOf` of several
/// -- reads as a kind to skip, which accepts any value, and never as
/// `Kind::Missing`.
#[test]
fn all_of_empty_and_true_schemas_read_as_what_json_schema_means() {
    let lone = obj(vec![
        ("description", s("The name")),
        ("allOf", arr(vec![typed("string")])),
    ]);
    let several = obj(vec![(
        "allOf",
        arr(vec![typed("string"), obj(vec![("maxLength", n("3"))])]),
    )]);
    let schema = obj(vec![
        ("type", s("object")),
        (
            "properties",
            obj(vec![
                ("lone", lone),
                ("several", several),
                ("empty", obj(vec![])),
                ("prose", obj(vec![("description", s("Anything"))])),
                ("any", Value::from(true)),
            ]),
        ),
    ]);
    let read = SchemaRef::new(&schema).unwrap();
    let lone = read.find("lone").unwrap();
    assert!(matches!(lone.kind(), Kind::Str { .. }));
    assert_eq!(lone.description(), "The name");
    assert!(matches!(
        read.find("several").unwrap().kind(),
        Kind::Unknown("allOf")
    ));
    for key in ["empty", "prose", "any"] {
        let field = read
            .find(key)
            .expect("a schema that accepts anything is a field");
        assert!(
            matches!(field.kind(), Kind::Unknown(_)),
            "{key} reads as {:?}",
            field.kind()
        );
    }

    let good = doc(vec![
        ("lone", s("ok")),
        ("several", n("5")),
        ("empty", s("x")),
        ("prose", n("5")),
        ("any", obj(vec![("deep", arr(vec![]))])),
    ]);
    assert_eq!(validate_map(read, &good), Ok(()));
    assert!(validate_map(read, &doc(vec![("lone", n("5"))])).is_err());
}

/// The shape a generator writes for a documented field whose type is a
/// shared definition: `allOf` around a reference, under the older
/// `definitions`. Inlined, it is checked as the definition.
#[test]
fn a_documented_reference_is_checked_once_inlined() {
    let schema = obj(vec![
        ("type", s("object")),
        (
            "properties",
            obj(vec![(
                "inner",
                obj(vec![
                    ("description", s("d")),
                    (
                        "allOf",
                        arr(vec![obj(vec![("$ref", s("#/definitions/Inner"))])]),
                    ),
                ]),
            )]),
        ),
        (
            "definitions",
            obj(vec![(
                "Inner",
                obj(vec![
                    ("type", s("object")),
                    ("properties", obj(vec![("a", typed("integer"))])),
                    ("required", arr(vec![s("a")])),
                ]),
            )]),
        ),
    ]);
    let inlined = inline_refs(&schema).unwrap();
    let read = SchemaRef::new(&inlined).unwrap();
    assert!(matches!(read.find("inner").unwrap().kind(), Kind::Map(_)));
    let good = doc(vec![("inner", obj(vec![("a", n("1"))]))]);
    assert_eq!(validate_map(read, &good), Ok(()));
    let bad = doc(vec![("inner", obj(vec![("a", s("1"))]))]);
    assert!(validate_map(read, &bad).is_err());
}

/// What copying a tree costs, as `inline_refs` counts it: one per node
/// and one per byte of text, a key's included.
fn units(v: &Value) -> usize {
    let mut total = 0;
    let mut stack = vec![v];
    while let Some(v) = stack.pop() {
        total += 1;
        if let Some(m) = TryAsRef::<Map>::try_as_ref(v) {
            for e in m {
                total += e.key().len();
                stack.push(e.value());
            }
        } else if let Some(l) = TryAsRef::<List>::try_as_ref(v) {
            stack.extend(l.iter());
        } else if let Some(text) = TryAsRef::<str>::try_as_ref(v) {
            total += text.len();
        }
    }
    total
}

/// Definitions that each name the one before twice, over a leaf holding
/// ten kilobytes of description: a few kilobytes written, gigabytes if
/// expanded in full. What is copied stays within the budget.
#[test]
fn expansion_is_bounded_by_what_it_copies() {
    let mut defs = Map::new();
    defs.set(
        "a20",
        obj(vec![
            ("type", s("string")),
            ("description", s(&"x".repeat(10_000))),
        ]),
    );
    for i in (0..20).rev() {
        let next = format!("a{}", i + 1);
        defs.set(
            &format!("a{i}"),
            obj(vec![(
                "anyOf",
                arr(vec![reference(&next), reference(&next)]),
            )]),
        );
    }
    let schema = obj(vec![
        ("type", s("object")),
        ("properties", obj(vec![("x", reference("a0"))])),
        ("$defs", defs.into()),
    ]);
    let written = units(&schema);
    let inlined = inline_refs(&schema).expect("it finishes");
    let copied = units(&inlined);
    assert!(
        copied <= 32 * written + 256 * 1024 + written,
        "{written} units written became {copied}"
    );
}

/// A reference naming a reference is followed where it stands, so a chain
/// of aliases longer than the nesting bound still resolves.
#[test]
fn a_chain_of_aliases_resolves_without_counting_as_nesting() {
    let mut defs = Map::new();
    for i in 0..300 {
        defs.set(&format!("c{i}"), reference(&format!("c{}", i + 1)));
    }
    defs.set(
        "c300",
        obj(vec![("type", s("string")), ("maxLength", n("2"))]),
    );
    let schema = obj(vec![
        ("type", s("object")),
        ("properties", obj(vec![("x", reference("c0"))])),
        ("$defs", defs.into()),
    ]);
    let inlined = inline_refs(&schema).expect("a chain is not nesting");
    let read = SchemaRef::new(&inlined).unwrap();
    assert!(matches!(read.find("x").unwrap().kind(), Kind::Str { .. }));
    assert!(validate_map(read, &doc(vec![("x", s("abc"))])).is_err());
}

/// `default`, `const`, `examples` and `enum` hold instances, and a
/// `{"$ref": ..}` in one is data. A property NAMED `default` is a schema,
/// and its reference is followed.
#[test]
fn a_reference_inside_an_instance_is_data() {
    let data = || obj(vec![("$ref", s("#/$defs/A"))]);
    let schema = obj(vec![
        ("type", s("object")),
        (
            "properties",
            obj(vec![
                (
                    "p",
                    obj(vec![
                        ("type", s("object")),
                        ("default", data()),
                        ("examples", arr(vec![data()])),
                    ]),
                ),
                ("q", obj(vec![("const", data())])),
                ("r", obj(vec![("enum", arr(vec![data()]))])),
                ("default", reference("A")),
            ]),
        ),
        ("$defs", obj(vec![("A", typed("string"))])),
    ]);
    let inlined = inline_refs(&schema).unwrap();
    let read = SchemaRef::new(&inlined).unwrap();
    let p = read.find("p").unwrap();
    assert_eq!(p.default(), Some(&data()));
    assert_eq!(p.examples(), [data()]);
    let field =
        |key: &str| TryAsRef::<Map>::try_as_ref(read.find(key).unwrap().as_value()).unwrap();
    assert_eq!(field("q").get("const"), Some(&data()));
    assert_eq!(field("r").get("enum"), Some(&arr(vec![data()])));
    assert!(matches!(
        read.find("default").unwrap().kind(),
        Kind::Str { .. }
    ));
}
