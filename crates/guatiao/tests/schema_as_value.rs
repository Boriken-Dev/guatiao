// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A schema is a value, that value is a JSON Schema, and the builder and
//! the reader agree about it.
//!
//! These go through the public API a provider and a consumer each use: one
//! declares a schema, the other reads it back without having seen the
//! declaration. That is the whole contract, so it is what gets tested —
//! plus the document itself, because "it IS a JSON Schema" is a claim
//! about the bytes and not only about the round trip.

use guatiao::value::convert::TryAsRef;
use std::cell::Cell;
use std::ffi::c_void;

use guatiao::schema::FormBuilder;
use guatiao::schema::FormFieldBuilder;
use guatiao::schema::build::{ArmBuilder, FieldBuilder, KindBuilder, SchemaBuilder};
use guatiao::schema::read::{Kind, SchemaRef};
use guatiao::schema::vocab;
use guatiao::value::alloc::{Alloc, Allocator, rust_alloc};
use guatiao::value::read::str_or;
use guatiao::{List, Map, Number, Text, Value};

// A counting allocator, so every test also proves the schema frees.
#[derive(Default)]
struct Counter {
    outstanding: Cell<isize>,
}

unsafe extern "C" fn c_alloc(ctx: *mut c_void, size: usize, align: usize) -> *mut c_void {
    // SAFETY: `ctx` is the `&Counter` installed below, outliving the call.
    let c = unsafe { &*(ctx as *const Counter) };
    c.outstanding.set(c.outstanding.get() + 1);
    let real = rust_alloc().alloc.expect("the rust allocator has an alloc");
    // SAFETY: forwarding an unchanged request.
    unsafe { real(std::ptr::null_mut(), size, align) }
}

unsafe extern "C" fn c_free(ctx: *mut c_void, p: *mut c_void, size: usize, align: usize) {
    // SAFETY: as above.
    let c = unsafe { &*(ctx as *const Counter) };
    c.outstanding.set(c.outstanding.get() - 1);
    let real = rust_alloc().free.expect("the rust allocator has a free");
    // SAFETY: forwarding the same block with the same layout.
    unsafe { real(std::ptr::null_mut(), p, size, align) }
}

fn with_alloc(body: impl FnOnce(Alloc)) {
    let counter = Counter::default();
    let vt = Allocator {
        struct_size: size_of::<Allocator>() as u32,
        ctx: (&counter as *const Counter).cast_mut().cast(),
        alloc: Some(c_alloc),
        free: Some(c_free),
        release: None,
    };
    // SAFETY: `vt` is fully initialised and outlives the closure.
    let alloc = unsafe { Alloc::from_raw(&vt) }.expect("a complete vtable");
    body(alloc);
    assert_eq!(
        counter.outstanding.get(),
        0,
        "a schema is a value, so it frees like one"
    );
}

/// The keys of a map, in order.
fn keys_of(v: &Value) -> Vec<String> {
    TryAsRef::<Map>::try_as_ref(v)
        .map(Map::entries)
        .unwrap_or(&[])
        .iter()
        .map(|e| e.key().to_string())
        .collect()
}

/// The strings in a list.
fn strings_of(v: Option<&Value>) -> Vec<String> {
    v.and_then(TryAsRef::<List>::try_as_ref)
        .map(|list| &list[..])
        .unwrap_or(&[])
        .iter()
        .filter_map(TryAsRef::<str>::try_as_ref)
        .map(str::to_string)
        .collect()
}

/// The value under `key`, when `v` is a map.
fn at<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    TryAsRef::<Map>::try_as_ref(v).and_then(|m| m.get(key))
}

#[test]
fn a_declared_schema_reads_back() {
    with_alloc(|alloc| {
        let schema = SchemaBuilder::new_in(alloc)
            .label("Connection")
            .help("Where to connect, and how.")
            .field(
                FieldBuilder::new_in(alloc, "host", KindBuilder::string_in(alloc))
                    .label("Host")
                    .section("net")
                    .required()
                    .order(1),
            )
            .field(
                FieldBuilder::new_in(alloc, "port", KindBuilder::int_range_in(alloc, 1, 65535))
                    .label("Port")
                    .section("net")
                    .default(
                        Number::new_in(alloc, &5900.to_string())
                            .map(Value::from)
                            .unwrap(),
                    )
                    .order(2),
            )
            .field(
                FieldBuilder::new_in(alloc, "password", KindBuilder::string_in(alloc))
                    .label("Password")
                    .sensitive()
                    .advanced(),
            )
            .finish()
            .expect("a schema this small does not exhaust an allocator");

        let s = SchemaRef::new(&schema).expect("a schema is a map");

        assert_eq!(s.dialect(), vocab::DIALECT);
        assert_eq!(s.label(), "Connection");
        assert_eq!(s.help(), "Where to connect, and how.");

        let keys: Vec<_> = s.fields().map(|o| o.key().to_string()).collect();
        assert_eq!(keys, ["host", "port", "password"], "declaration order");

        let host = s.find("host").expect("host is declared");
        assert_eq!(host.label(), "Host");
        assert_eq!(host.section(), "net");
        assert!(host.is_required());
        assert!(!host.is_sensitive());
        assert!(matches!(host.kind(), Kind::Str));
        assert!(host.default().is_none(), "no default is not a null default");

        let port = s.find("port").expect("port is declared");
        assert!(matches!(
            port.kind(),
            Kind::Int {
                min: Some(1),
                max: Some(65535)
            }
        ));
        assert!(!port.is_required(), "only `host` asked to be required");
        assert_eq!(
            guatiao::value::read::int_or(port.default(), -1),
            5900,
            "the default reads back as the number it was declared as"
        );

        let password = s.find("password").expect("password is declared");
        assert!(password.is_sensitive());
        assert!(password.is_advanced());

        assert!(s.find("nothing").is_none());
    });
}

/// The claim the whole design rests on: what comes out is a JSON Schema
/// document, key for key.
///
/// Asserted on the value rather than on text, because there is no
/// serialiser here -- the value IS the document, and `guatiao-serde`
/// writes it out with no knowledge of schemas at all.
#[test]
fn the_document_is_written_in_json_schemas_own_keys() {
    with_alloc(|alloc| {
        let schema = SchemaBuilder::new_in(alloc)
            .label("Connection")
            .field(
                FieldBuilder::new_in(alloc, "host", KindBuilder::string_in(alloc))
                    .label("Host")
                    .help("Where to connect.")
                    .required(),
            )
            .field(FieldBuilder::new_in(
                alloc,
                "port",
                KindBuilder::int_range_in(alloc, 1, 65535),
            ))
            .field(FieldBuilder::new_in(
                alloc,
                "cert",
                KindBuilder::bytes_in(alloc),
            ))
            .finish()
            .unwrap();

        // The dialect is declared once, on the root, and first.
        assert_eq!(
            keys_of(&schema),
            [
                "$schema",
                "type",
                "title",
                "properties",
                "additionalProperties",
                "required"
            ],
            "the document opens by saying what it is"
        );
        assert_eq!(str_or(at(&schema, "$schema"), ""), vocab::DIALECT);
        assert_eq!(str_or(at(&schema, "type"), ""), "object");

        // A field's name is its key in `properties`, and appears nowhere
        // inside the field.
        let properties = at(&schema, "properties").expect("an object has properties");
        assert_eq!(keys_of(properties), ["host", "port", "cert"]);
        let host = at(properties, "host").expect("host is a property");
        assert_eq!(str_or(at(host, "type"), ""), "string");
        assert_eq!(str_or(at(host, "title"), ""), "Host");
        assert_eq!(str_or(at(host, "description"), ""), "Where to connect.");
        assert!(
            at(host, "key").is_none() && at(host, "kind").is_none(),
            "a field carries neither its own name nor a nested kind"
        );

        // Requiredness is a name in a list on the OBJECT.
        assert_eq!(strings_of(at(&schema, "required")), ["host"]);
        assert!(
            at(host, "required").is_none(),
            "and not a flag on the field"
        );

        let port = at(properties, "port").unwrap();
        assert_eq!(str_or(at(port, "type"), ""), "integer");
        assert_eq!(
            guatiao::value::read::int_or(at(port, "minimum"), -1),
            1,
            "bounds are `minimum` and `maximum`"
        );
        assert_eq!(guatiao::value::read::int_or(at(port, "maximum"), -1), 65535);

        // The one type that is ours. See `vocab::TYPE_BYTES`: this is
        // readable by anything and fails a strict meta-schema check, which
        // is a decision rather than a surprise.
        assert_eq!(
            str_or(at(at(properties, "cert").unwrap(), "type"), ""),
            "bytes"
        );
    });
}

#[test]
fn an_enum_carries_labels_keyed_by_value_not_a_second_list() {
    with_alloc(|alloc| {
        let schema = SchemaBuilder::new_in(alloc)
            .field(FieldBuilder::new_in(
                alloc,
                "level",
                KindBuilder::enumeration_in(alloc, &[("off", "Off"), ("on", "On")]),
            ))
            .finish()
            .unwrap();

        let s = SchemaRef::new(&schema).unwrap();
        let kind = s.find("level").unwrap().kind();
        let choices: Vec<_> = kind
            .choices()
            .map(|c| (c.value().to_string(), c.label().to_string()))
            .collect();
        assert_eq!(
            choices,
            [
                ("off".to_string(), "Off".to_string()),
                ("on".to_string(), "On".to_string())
            ],
            "a label is keyed by its value, so the two cannot drift apart"
        );

        // And on the wire it is a string narrowed by `enum`.
        let level = at(at(&schema, "properties").unwrap(), "level").unwrap();
        assert_eq!(str_or(at(level, "type"), ""), "string");
        assert_eq!(strings_of(at(level, "enum")), ["off", "on"]);
        assert_eq!(
            str_or(at(level, "x-enum-labels").and_then(|m| at(m, "off")), ""),
            "Off",
            "the labels are ours, so they carry the prefix"
        );
    });
}

/// A union is untagged and a variant is tagged, and a reader must not
/// treat them alike: a selector built from a union's arms would store a
/// bare discriminant with no payload.
#[test]
fn a_union_and_a_variant_are_different_features() {
    with_alloc(|alloc| {
        let schema = SchemaBuilder::new_in(alloc)
            .field(FieldBuilder::new_in(
                alloc,
                "port",
                KindBuilder::union_in(
                    alloc,
                    vec![KindBuilder::int_in(alloc), KindBuilder::string_in(alloc)],
                ),
            ))
            .field(FieldBuilder::new_in(
                alloc,
                "auth",
                KindBuilder::variant_in(
                    alloc,
                    "auth",
                    vec![
                        // An arm with no fields is ordinary and complete:
                        // "use the ambient credential" is the common case.
                        ArmBuilder::new_in(alloc, "ambient", "Ambient"),
                        ArmBuilder::new_in(alloc, "userpass", "Username and password")
                            .field(FieldBuilder::new_in(
                                alloc,
                                "username",
                                KindBuilder::string_in(alloc),
                            ))
                            .field(FieldBuilder::new_in(
                                alloc,
                                "password",
                                KindBuilder::string_in(alloc),
                            )),
                    ],
                ),
            ))
            .finish()
            .unwrap();

        let s = SchemaRef::new(&schema).unwrap();

        let union = s.find("port").unwrap().kind();
        assert_eq!(union.alternatives().count(), 2, "a union has arms as kinds");
        assert_eq!(union.arms().count(), 0, "and no tagged arms");

        let variant = s.find("auth").unwrap().kind();
        assert!(matches!(variant, Kind::Variant { tag: "auth", .. }));
        assert_eq!(
            variant.alternatives().count(),
            0,
            "a variant's arms are NOT a union's; reporting them here would make a reader \
             draw a selector that stores a bare discriminant with no payload"
        );

        let arms: Vec<_> = variant.arms().collect();
        assert_eq!(arms.len(), 2);
        assert_eq!(arms[0].value(), "ambient");
        assert_eq!(arms[0].label(), "Ambient");
        assert_eq!(arms[0].fields().count(), 0, "an empty arm is complete");
        assert_eq!(arms[1].value(), "userpass");
        let fields: Vec<_> = arms[1].fields().map(|f| f.key().to_string()).collect();
        assert_eq!(
            fields,
            ["username", "password"],
            "the discriminant is a property of the arm, never one of its fields"
        );

        // The document: `anyOf` for the untagged one, `oneOf` plus a
        // `const` discriminant for the tagged one.
        let properties = at(&schema, "properties").unwrap();
        let port = at(properties, "port").unwrap();
        assert_eq!(
            at(port, "anyOf")
                .and_then(TryAsRef::<List>::try_as_ref)
                .map(|list| &list[..])
                .unwrap()
                .len(),
            2
        );
        assert!(
            at(port, "type").is_none(),
            "a union of an integer and a string has no single type to name"
        );

        let auth = at(properties, "auth").unwrap();
        assert_eq!(str_or(at(auth, "type"), ""), "object");
        assert_eq!(str_or(at(auth, "x-variant-tag"), ""), "auth");
        let one_of = at(auth, "oneOf")
            .and_then(TryAsRef::<List>::try_as_ref)
            .map(|list| &list[..])
            .unwrap();
        assert_eq!(one_of.len(), 2);
        let userpass = &one_of[1];
        assert_eq!(str_or(at(userpass, "title"), ""), "Username and password");
        assert_eq!(
            keys_of(at(userpass, "properties").unwrap()),
            ["auth", "username", "password"],
            "the discriminant goes in first, so the arm reads as what it selects"
        );
        assert_eq!(
            str_or(
                at(userpass, "properties")
                    .and_then(|p| at(p, "auth"))
                    .and_then(|t| at(t, "const")),
                ""
            ),
            "userpass"
        );
        assert_eq!(
            strings_of(at(userpass, "required")),
            ["auth"],
            "an arm without its discriminant is not that arm"
        );
    });
}

/// The whole forward-compatibility story: a kind from a newer producer
/// leaves its field readable, and every other field untouched.
///
/// Built by hand rather than through the builder, because that is what a
/// newer producer's document looks like arriving here.
#[test]
fn an_unknown_kind_leaves_the_field_readable_and_the_rest_intact() {
    with_alloc(|alloc| {
        let mut known = Map::new_in(alloc);
        known
            .set(
                vocab::TYPE,
                Text::new_in(alloc, "string").map(Value::from).unwrap(),
            )
            .unwrap();
        known
            .set(
                vocab::TITLE,
                Text::new_in(alloc, "Known").map(Value::from).unwrap(),
            )
            .unwrap();

        // A type this build has never heard of, on a field that is
        // otherwise ordinary.
        let mut future = Map::new_in(alloc);
        future
            .set(
                vocab::TYPE,
                Text::new_in(alloc, "duration").map(Value::from).unwrap(),
            )
            .unwrap();
        future
            .set(
                vocab::TITLE,
                Text::new_in(alloc, "Timeout").map(Value::from).unwrap(),
            )
            .unwrap();

        let mut properties = Map::new_in(alloc);
        properties.set("known", known).unwrap();
        properties.set("timeout", future).unwrap();
        let mut schema = Map::new_in(alloc);
        schema
            .set(
                vocab::TYPE,
                Text::new_in(alloc, "object").map(Value::from).unwrap(),
            )
            .unwrap();
        schema.set(vocab::PROPERTIES, properties).unwrap();

        let schema = Value::from(schema);
        let s = SchemaRef::new(&schema).unwrap();
        assert_eq!(s.fields().count(), 2, "both fields are still listed");

        let future = s.find("timeout").expect("the field is readable");
        assert!(matches!(future.kind(), Kind::Unknown("duration")));
        assert_eq!(
            future.label(),
            "Timeout",
            "and everything that does not depend on the kind still reads"
        );

        assert_eq!(s.find("known").unwrap().label(), "Known");
    });
}

/// A property that is not a schema is skipped, rather than taking the
/// schema down with it.
#[test]
fn a_malformed_field_is_skipped_not_fatal() {
    with_alloc(|alloc| {
        let mut good = Map::new_in(alloc);
        good.set(
            vocab::TYPE,
            Text::new_in(alloc, "boolean").map(Value::from).unwrap(),
        )
        .unwrap();

        let mut properties = Map::new_in(alloc);
        properties.set("good", good).unwrap();
        // Not a map, so not a schema.
        properties.set("bad", Value::from(true)).unwrap();

        let mut schema = Map::new_in(alloc);
        schema.set(vocab::PROPERTIES, properties).unwrap();

        let schema = Value::from(schema);
        let s = SchemaRef::new(&schema).unwrap();
        let keys: Vec<_> = s.fields().map(|o| o.key().to_string()).collect();
        assert_eq!(keys, ["good"], "the readable field still reads");
        assert!(s.find("bad").is_none());
    });
}

/// Anything outside the vocabulary is an annotation: carried, and never
/// interpreted.
#[test]
fn an_annotation_is_carried_but_not_interpreted() {
    with_alloc(|alloc| {
        let schema = SchemaBuilder::new_in(alloc)
            .field(
                FieldBuilder::new_in(alloc, "host", KindBuilder::string_in(alloc))
                    .sensitive()
                    .option(
                        "x-widget",
                        Text::new_in(alloc, "combo").map(Value::from).unwrap(),
                    ),
            )
            .option(
                "x-origin",
                Text::new_in(alloc, "test").map(Value::from).unwrap(),
            )
            .finish()
            .unwrap();

        let s = SchemaRef::new(&schema).unwrap();
        let host = s.find("host").unwrap();
        assert_eq!(str_or(host.extra("x-widget"), ""), "combo");
        assert_eq!(str_or(s.extra("x-origin"), ""), "test");

        assert!(
            host.extra(vocab::TYPE).is_none(),
            "a keyword is not an annotation, even though it is a key"
        );
        assert!(
            host.is_sensitive() && host.extra(vocab::X_SENSITIVE).is_none(),
            "and neither is one of OURS: `x-sensitive` is set and this crate reads it, \
             so it is a keyword that happens to carry the prefix"
        );

        // And a schema really is just a map, walkable by anything that
        // walks a value.
        assert!(
            TryAsRef::<Map>::try_as_ref(&schema)
                .map(Map::entries)
                .unwrap_or(&[])
                .len()
                >= 2
        );
    });
}

/// The annotations, enumerated: every key the vocabulary does not claim,
/// and none it does -- `type` and `x-sensitive` are read through their
/// own accessors and are not annotations.
#[test]
fn every_annotation_is_enumerated_and_nothing_else_is() {
    let s = SchemaBuilder::new()
        .field(
            FieldBuilder::new("host", KindBuilder::string())
                .sensitive()
                .option("x-widget", "combo")
                .option("x-placeholder", "example.org"),
        )
        .option("x-origin", "test")
        .finish()
        .unwrap();
    let s = SchemaRef::new(&s).unwrap();
    let host = s.find("host").unwrap();

    let mut on_field: Vec<(&str, &str)> = host
        .extras()
        .map(|(k, v)| (k, str_or(Some(v), "")))
        .collect();
    on_field.sort();
    assert_eq!(
        on_field,
        [("x-placeholder", "example.org"), ("x-widget", "combo")]
    );

    let on_schema: Vec<(&str, &str)> = s.extras().map(|(k, v)| (k, str_or(Some(v), ""))).collect();
    assert_eq!(on_schema, [("x-origin", "test")]);
}

/// **Building a schema names no allocator**, the same rule the value API
/// has -- and the two forms build the same thing.
///
/// The plain form is what a schema written by hand should read like; the
/// `_in` form is for a schema built into a host's arena, which is what
/// the example library does.
#[test]
fn the_plain_builders_and_the_in_builders_agree() {
    let by_hand = SchemaBuilder::new()
        .field(
            FieldBuilder::new("port", KindBuilder::int_range(1, 65535))
                .label("Port")
                .required(),
        )
        .field(FieldBuilder::new("name", KindBuilder::string()))
        .finish()
        .expect("a schema this small does not exhaust an allocator");

    let alloc = Alloc::rust();
    let named = SchemaBuilder::new_in(alloc)
        .field(
            FieldBuilder::new_in(alloc, "port", KindBuilder::int_range_in(alloc, 1, 65535))
                .label("Port")
                .required(),
        )
        .field(FieldBuilder::new_in(
            alloc,
            "name",
            KindBuilder::string_in(alloc),
        ))
        .finish()
        .expect("as above");

    assert!(
        by_hand == named,
        "the plain form is the `_in` form with the crate's own allocator"
    );
}
