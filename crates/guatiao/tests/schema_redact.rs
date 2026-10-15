// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A value is shown without its secrets: everything its schema marks
//! `x-sensitive` or `writeOnly`, at any depth, and anything the schema
//! does not describe.

use guatiao::schema::build::{ArmBuilder, FieldBuilder, KindBuilder, SchemaBuilder};
use guatiao::schema::read::SchemaRef;
use guatiao::schema::redact;
use guatiao::value::convert::TryAsRef;
use guatiao::{List, Map, Value};

const HIDDEN: &str = "***";

fn schema() -> Value {
    SchemaBuilder::new()
        .field(FieldBuilder::new("name", KindBuilder::string()))
        .field(FieldBuilder::new("password", KindBuilder::string()).sensitive())
        // `writeOnly` alone, as a generator writes it.
        .field(FieldBuilder::new("api_token", KindBuilder::string()).option("writeOnly", true))
        .field(FieldBuilder::new(
            "tls",
            KindBuilder::map(vec![
                FieldBuilder::new("ca", KindBuilder::string()),
                FieldBuilder::new("key", KindBuilder::bytes()).sensitive(),
                FieldBuilder::new(
                    "inner",
                    KindBuilder::map(vec![
                        FieldBuilder::new("pin", KindBuilder::string()).sensitive(),
                        FieldBuilder::new("label", KindBuilder::string()),
                    ]),
                ),
            ]),
        ))
        .field(FieldBuilder::new(
            "peers",
            KindBuilder::list(KindBuilder::map(vec![
                FieldBuilder::new("host", KindBuilder::string()),
                FieldBuilder::new("secret", KindBuilder::string()).sensitive(),
            ])),
        ))
        .field(FieldBuilder::new(
            "env",
            KindBuilder::map_of(KindBuilder::string()),
        ))
        .field(FieldBuilder::new("tokens", KindBuilder::map_of(KindBuilder::string())).sensitive())
        .field(FieldBuilder::new(
            "auth",
            KindBuilder::variant(
                "kind",
                vec![
                    ArmBuilder::new("pw", "Password")
                        .field(FieldBuilder::new("user", KindBuilder::string()))
                        .field(FieldBuilder::new("pass", KindBuilder::string()).sensitive()),
                    ArmBuilder::new("key", "Key")
                        .field(FieldBuilder::new("token", KindBuilder::string()).sensitive()),
                ],
            ),
        ))
        .field(FieldBuilder::new(
            "either",
            KindBuilder::union(vec![
                KindBuilder::int(),
                KindBuilder::list(KindBuilder::map(vec![
                    FieldBuilder::new("p", KindBuilder::string()).sensitive(),
                ])),
            ]),
        ))
        .finish()
        .expect("the schema builds")
}

fn map(pairs: Vec<(&str, Value)>) -> Value {
    let mut m = Map::new();
    for (k, v) in pairs {
        m.set(k, v);
    }
    m.into()
}

fn list(items: Vec<Value>) -> Value {
    let mut l = List::new();
    for i in items {
        l.push(i);
    }
    l.into()
}

fn text(v: &str) -> Value {
    Value::from(v)
}

fn secret_value() -> Value {
    map(vec![
        ("name", text("srv")),
        ("password", text("hunter2")),
        ("api_token", text("t0k3n")),
        (
            "tls",
            map(vec![
                ("ca", text("/ca.pem")),
                ("key", Value::from(&b"\x01\x02"[..])),
                (
                    "inner",
                    map(vec![("pin", text("1234")), ("label", text("front"))]),
                ),
            ]),
        ),
        (
            "peers",
            list(vec![
                map(vec![("host", text("a")), ("secret", text("s1"))]),
                map(vec![("host", text("b")), ("secret", text("s2"))]),
            ]),
        ),
        ("env", map(vec![("PATH", text("/bin"))])),
        ("tokens", map(vec![("ci", text("abc"))])),
        (
            "auth",
            map(vec![
                ("kind", text("pw")),
                ("user", text("root")),
                ("pass", text("toor")),
            ]),
        ),
        ("either", list(vec![map(vec![("p", text("hidden?"))])])),
    ])
}

/// The value reached by keys and list positions from `root`.
fn at<'a>(root: &'a Value, path: &[&str]) -> &'a Value {
    let mut v = root;
    for step in path {
        v = match step.parse::<usize>() {
            Ok(i) if TryAsRef::<List>::try_as_ref(v).is_some() => {
                &TryAsRef::<List>::try_as_ref(v).unwrap()[i]
            }
            _ => TryAsRef::<Map>::try_as_ref(v)
                .and_then(|m| m.get(step))
                .unwrap_or_else(|| panic!("no {step} on the way to {path:?}")),
        };
    }
    v
}

fn is_text(v: &Value, expected: &str) -> bool {
    TryAsRef::<str>::try_as_ref(v) == Some(expected)
}

#[test]
fn every_marked_value_is_replaced_at_any_depth() {
    let schema = schema();
    let out = redact(SchemaRef::new(&schema).unwrap(), &secret_value(), HIDDEN).unwrap();

    for hidden in [
        &["password"][..],
        &["api_token"],
        &["tls", "key"],
        &["tls", "inner", "pin"],
        &["peers", "0", "secret"],
        &["peers", "1", "secret"],
        &["tokens"],
        &["auth", "pass"],
        &["either", "0", "p"],
    ] {
        assert!(is_text(at(&out, hidden), HIDDEN), "{hidden:?} is a secret");
    }
}

#[test]
fn everything_unmarked_is_kept_as_it_was() {
    let schema = schema();
    let out = redact(SchemaRef::new(&schema).unwrap(), &secret_value(), HIDDEN).unwrap();

    for (path, kept) in [
        (&["name"][..], "srv"),
        (&["tls", "ca"], "/ca.pem"),
        (&["tls", "inner", "label"], "front"),
        (&["peers", "0", "host"], "a"),
        (&["peers", "1", "host"], "b"),
        (&["env", "PATH"], "/bin"),
        (&["auth", "kind"], "pw"),
        (&["auth", "user"], "root"),
    ] {
        assert!(is_text(at(&out, path), kept), "{path:?}");
    }
}

#[test]
fn a_variant_hides_only_the_selected_arms_secrets() {
    let schema = schema();
    let read = SchemaRef::new(&schema).unwrap();
    let key_arm = map(vec![(
        "auth",
        map(vec![("kind", text("key")), ("token", text("tok"))]),
    )]);
    let out = redact(read, &key_arm, HIDDEN).unwrap();
    assert!(is_text(at(&out, &["auth", "kind"]), "key"));
    assert!(is_text(at(&out, &["auth", "token"]), HIDDEN));

    // The password arm's secret is not the key arm's: a payload key the
    // selected arm does not declare is hidden, whatever its name.
    let mixed = map(vec![(
        "auth",
        map(vec![
            ("kind", text("key")),
            ("pass", text("leak")),
            ("extra", text("also")),
        ]),
    )]);
    let out = redact(read, &mixed, HIDDEN).unwrap();
    assert!(is_text(at(&out, &["auth", "pass"]), HIDDEN));
    assert!(is_text(at(&out, &["auth", "extra"]), HIDDEN));

    // A discriminant naming no arm: nothing says what the payload is.
    let unknown = map(vec![(
        "auth",
        map(vec![("kind", text("future")), ("anything", text("x"))]),
    )]);
    let out = redact(read, &unknown, HIDDEN).unwrap();
    assert!(is_text(at(&out, &["auth", "kind"]), "future"));
    assert!(is_text(at(&out, &["auth", "anything"]), HIDDEN));
}

#[test]
fn what_the_schema_does_not_describe_is_hidden() {
    let schema = schema();
    let read = SchemaRef::new(&schema).unwrap();
    let value = map(vec![
        ("undeclared", text("maybe a secret")),
        ("tls", map(vec![("surprise", text("x")), ("ca", text("c"))])),
        // A container where the schema names a scalar.
        ("name", map(vec![("deep", text("secret"))])),
        // A union no arm accepts.
        ("either", list(vec![text("neither arm")])),
    ]);
    let out = redact(read, &value, HIDDEN).unwrap();
    assert!(is_text(at(&out, &["undeclared"]), HIDDEN));
    assert!(is_text(at(&out, &["tls", "surprise"]), HIDDEN));
    assert!(is_text(at(&out, &["tls", "ca"]), "c"));
    assert!(is_text(at(&out, &["name"]), HIDDEN));
    assert!(is_text(at(&out, &["either"]), HIDDEN));
}

#[test]
fn a_secret_that_is_null_is_hidden_too() {
    let schema = schema();
    let value = map(vec![("password", Value::null()), ("name", Value::null())]);
    let out = redact(SchemaRef::new(&schema).unwrap(), &value, HIDDEN).unwrap();
    assert!(
        is_text(at(&out, &["password"]), HIDDEN),
        "the copy does not say whether a secret was set"
    );
    assert!(at(&out, &["name"]).tag() == Ok(guatiao::Tag::GUATIAO_NULL));
}

#[test]
fn the_input_is_left_alone() {
    let schema = schema();
    let value = secret_value();
    let before = value.clone();
    let _ = redact(SchemaRef::new(&schema).unwrap(), &value, HIDDEN).unwrap();
    assert_eq!(value, before);
}
