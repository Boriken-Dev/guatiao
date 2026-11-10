// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `Described<T>` holds a described type as a guatiao value, schema and field, and
//! `with_module!` serves a field of one inside a struct guatiao derives.

mod support;

use core::time::Duration;

use coqui_builder::{Describe, Secret};
use guatiao::schema::read::SchemaRef;
use guatiao::schema::validate::validate_map;
use guatiao::{FromValue, Schema, ToValue};
use guatiao_builder::{Described, schema, with_module};
use support::*;

#[test]
fn it_round_trips_through_to_value_and_from_value() {
    let held = Described(everything());
    let value = held.to_value().unwrap();
    assert_eq!(Described::<Everything>::from_value(&value).unwrap(), held);
    // The value is the one guatiao's own derive writes.
    assert!(value == everything().to_value().unwrap());
}

#[test]
fn its_schema_is_the_one_made_from_the_description() {
    let made = Described::<Everything>::schema().unwrap();
    assert!(made == schema(&Everything::description()).unwrap());
    // And what it writes validates against it.
    let written = Described(everything()).to_value().unwrap();
    validate_map(SchemaRef::new(&made).unwrap(), &written).unwrap();
}

#[test]
fn a_refusal_is_guatiaos_under_the_key_and_never_quotes_the_value() {
    let value = parse(r#"{"port": 70000}"#);
    let error = Described::<Everything>::from_value(&value).expect_err("not a u16");
    let shown = error.to_string();
    assert!(shown.contains("port"), "{shown}");
    assert!(!shown.contains("70000"), "{shown}");

    let error = Described::<Everything>::from_value(&parse("{}")).expect_err("missing keys");
    assert!(!error.to_string().is_empty());
}

// ---- a field of a described type inside a struct guatiao derives ----

#[derive(Describe, Debug, Clone, PartialEq)]
struct Listener {
    host: String,
    #[map(default = 8080)]
    port: u16,
    #[map(default = Duration::from_secs(30))]
    idle: Duration,
}

with_module!(listener, Listener);

#[derive(ToValue, FromValue, Schema, Debug, PartialEq)]
struct Server {
    name: String,
    #[map(with = listener)]
    listener: Listener,
}

#[test]
fn with_module_serves_a_field() {
    let server = Server {
        name: "s".into(),
        listener: Listener {
            host: "h".into(),
            port: 1,
            idle: Duration::from_millis(250),
        },
    };
    let value = server.to_value().unwrap();
    assert!(
        value == parse(r#"{"name": "s", "listener": {"host": "h", "port": 1, "idle": "250ms"}}"#)
    );
    assert_eq!(Server::from_value(&value).unwrap(), server);

    let document = Server::schema().unwrap();
    let root = SchemaRef::new(&document).unwrap();
    assert!(root.find("listener").unwrap().is_required());
    assert!(matches!(
        root.find("listener").unwrap().kind(),
        guatiao::schema::KindRef::Map(_)
    ));
}

// ---- a secret ----

#[derive(ToValue, FromValue, Schema, Debug, PartialEq)]
struct Login {
    user: String,
    password: Described<Secret<String>>,
}

#[test]
fn a_secret_is_served_by_described_and_marked_in_the_schema() {
    let login = Login {
        user: "u".into(),
        password: Described(Secret::new("hunter2".to_owned())),
    };
    let value = login.to_value().unwrap();
    assert!(value == parse(r#"{"user": "u", "password": "hunter2"}"#));
    assert_eq!(Login::from_value(&value).unwrap(), login);
    let document = Login::schema().unwrap();
    let password = SchemaRef::new(&document).unwrap().find("password").unwrap();
    assert!(password.is_sensitive());
}

#[test]
fn a_refusal_deep_in_the_value_names_its_key() {
    let value = parse(r#"{"ports": [[1]]}"#);
    let error = Described::<Everything>::from_value(&value).expect_err("a list of lists");
    assert!(error.to_string().contains("ports"), "{error}");
    let missing = Described::<Everything>::from_value(&parse("{}")).expect_err("missing");
    assert!(missing.to_string().contains("host"), "{missing}");
    assert!(missing.to_string().contains("port") || missing.to_string().contains("auth"));
}
