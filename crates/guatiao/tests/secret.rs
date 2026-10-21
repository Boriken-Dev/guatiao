// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `Secret<T>`: printed as `[redacted]`, crossing as `T`, its schema
//! marked so redaction hides it, in Rust and through the C export.

#![cfg(feature = "derive")]

use guatiao::exports::schema::guatiao_schema_redact;
use guatiao::schema::read::SchemaRef;
use guatiao::schema::redact;
use guatiao::value::status::Status;
use guatiao::value::types::{Str, Tag};
use guatiao::{Alloc, FromValue, Map, Schema, Secret, ToValue, Value};

#[derive(Debug, PartialEq, guatiao::ToValue, guatiao::FromValue, guatiao::Schema)]
struct Tls {
    ca: String,
    key: Secret<String>,
}

#[derive(Debug, PartialEq, guatiao::ToValue, guatiao::FromValue, guatiao::Schema)]
struct Login {
    user: String,
    password: Secret<String>,
    pin: Secret<u32>,
    token: Option<Secret<String>>,
    tls: Tls,
    recovery: Vec<Secret<String>>,
}

fn login() -> Login {
    Login {
        user: "ana".into(),
        password: Secret::new("hunter2".into()),
        pin: Secret::new(4321),
        token: Some(Secret::new("tok-9f".into())),
        tls: Tls {
            ca: "/etc/ca.pem".into(),
            key: Secret::new("-----BEGIN KEY-----".into()),
        },
        recovery: vec![Secret::new("r1".into()), Secret::new("r2".into())],
    }
}

fn text_at<'a>(value: &'a Value, key: &str) -> &'a str {
    <&Map>::try_from(value)
        .unwrap()
        .get(key)
        .and_then(|v| <&str>::try_from(v).ok())
        .unwrap_or_else(|| panic!("{key} is text"))
}

#[test]
fn printing_never_shows_it() {
    let l = login();
    for shown in [
        format!("{l:?}"),
        format!("{:?}", l.password),
        l.password.to_string(),
    ] {
        for secret in ["hunter2", "4321", "tok-9f", "BEGIN KEY", "r1"] {
            assert!(!shown.contains(secret), "{shown}");
        }
    }
    assert_eq!(format!("{:?}", l.password), "[redacted]");
    assert_eq!(l.password.to_string(), "[redacted]");
    assert_eq!(l.password.expose(), "hunter2");
    assert_eq!(l.pin.into_inner(), 4321);
    assert_eq!(Secret::from(5u8), Secret::new(5u8));
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Secret<String>>();
}

#[test]
fn it_crosses_as_what_it_holds_and_round_trips() {
    let l = login();
    let value = l.to_value().unwrap();
    assert_eq!(text_at(&value, "password"), "hunter2");
    assert_eq!(Login::from_value(&value).unwrap(), l);
    assert_eq!(
        Secret::new(7u8).to_value().unwrap(),
        7u8.to_value().unwrap()
    );
}

#[test]
fn the_schema_marks_it_and_says_nothing_else_new() {
    let kind = Secret::<String>::kind(Alloc::rust()).finish().unwrap();
    let marks = <&Map>::try_from(&kind).unwrap();
    assert_eq!(marks.get("writeOnly"), Some(&Value::from(true)));
    assert_eq!(marks.get("x-sensitive"), Some(&Value::from(true)));
    assert_eq!(text_at(&kind, "type"), "string");

    let schema = Login::schema().unwrap();
    let s = SchemaRef::new(&schema).unwrap();
    for (key, sensitive) in [
        ("user", false),
        ("password", true),
        ("pin", true),
        ("token", true),
        ("tls", false),
    ] {
        assert_eq!(s.find(key).unwrap().is_sensitive(), sensitive, "{key}");
    }
    guatiao::schema::validate_map(s, &login().to_value().unwrap()).unwrap();
}

#[test]
fn redaction_hides_it_at_any_depth() {
    let schema = Login::schema().unwrap();
    let value = login().to_value().unwrap();
    let shown = redact(SchemaRef::new(&schema).unwrap(), &value, "***").unwrap();
    assert_eq!(text_at(&shown, "user"), "ana");
    assert_eq!(text_at(&shown, "password"), "***");
    assert_eq!(text_at(&shown, "pin"), "***");
    assert_eq!(text_at(&shown, "token"), "***");
    let tls = <&Map>::try_from(&shown).unwrap().get("tls").unwrap();
    assert_eq!(text_at(tls, "ca"), "/etc/ca.pem");
    assert_eq!(text_at(tls, "key"), "***");
    let recovery = <&Map>::try_from(&shown).unwrap().get("recovery").unwrap();
    for item in <&guatiao::List>::try_from(recovery).unwrap().iter() {
        assert_eq!(<&str>::try_from(item).unwrap(), "***");
    }
}

/// The C export hides the same fields, follows a local `$ref` first, and
/// refuses what it cannot use.
#[test]
fn redaction_crosses_the_c_surface() {
    let schema = Login::schema().unwrap();
    let value = login().to_value().unwrap();
    let alloc = Alloc::rust();
    let mut out = Value::absent();
    // SAFETY: well-formed values, a readable view, a writable out.
    let status = unsafe {
        guatiao_schema_redact(
            &schema,
            &value,
            Str::new("[hidden]"),
            alloc.as_raw(),
            &mut out,
        )
    };
    assert_eq!(status, Status::GUATIAO_OK);
    assert_eq!(text_at(&out, "password"), "[hidden]");
    assert_eq!(text_at(&out, "user"), "ana");

    // A mark on a definition reached through `$ref`.
    let mut def = Map::new();
    def.set("type", "string");
    def.set("x-sensitive", true);
    let mut defs = Map::new();
    defs.set("Pass", def);
    let mut by_ref = Map::new();
    by_ref.set("$ref", "#/$defs/Pass");
    let mut props = Map::new();
    props.set("p", by_ref);
    let mut name = Map::new();
    name.set("type", "string");
    props.set("n", name);
    let mut doc = Map::new();
    doc.set("type", "object");
    doc.set("properties", props);
    doc.set("$defs", defs);
    let referring = Value::from(doc);
    let mut config = Map::new();
    config.set("p", "s3cret");
    config.set("n", "visible");
    let config = Value::from(config);
    let mut shown = Value::absent();
    // SAFETY: as above.
    let status = unsafe {
        guatiao_schema_redact(
            &referring,
            &config,
            Str::new("*"),
            alloc.as_raw(),
            &mut shown,
        )
    };
    assert_eq!(status, Status::GUATIAO_OK);
    assert_eq!(text_at(&shown, "p"), "*");
    assert_eq!(text_at(&shown, "n"), "visible");

    let not_a_schema = Value::from(1i64);
    let mut out = Value::from(true);
    // SAFETY: as above.
    let status = unsafe {
        guatiao_schema_redact(
            &not_a_schema,
            &config,
            Str::new("*"),
            alloc.as_raw(),
            &mut out,
        )
    };
    assert_eq!(status, Status::GUATIAO_ERR_WRONG_KIND);
    assert_eq!(out.tag(), Ok(Tag::GUATIAO_ABSENT));
    // SAFETY: null is the case under test.
    let status = unsafe {
        guatiao_schema_redact(
            core::ptr::null(),
            &config,
            Str::new("*"),
            alloc.as_raw(),
            &mut out,
        )
    };
    assert_eq!(status, Status::GUATIAO_ERR_NULL);
}
