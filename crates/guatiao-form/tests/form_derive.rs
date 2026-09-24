// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `#[derive(Form)]` beside `#[derive(Schema)]`: the derived form fits the
//! derived schema, and says what the attributes said.

#![cfg(feature = "derive")]
#![allow(dead_code)] // the types exist to be described, not built

use guatiao::schema::read::SchemaRef;
use guatiao::value::convert::TryAsRef;
use guatiao::{Alloc, Map, Schema, Value};
use guatiao_form::{Form, FormField, FormRef, Screen, check, is_visible, layout, vocab};

/// How a session authenticates. An arm field carries a hint of its own.
#[derive(Schema, Form)]
#[map(tag = "method")]
enum Auth {
    /// Whatever the environment provides.
    Ambient,
    /// A name and a secret.
    UserPass {
        username: String,
        #[form(widget = "password")]
        password: String,
    },
}

/// A connection: two named sections, a guarded field, a nested member.
#[derive(Schema, Form)]
#[form(
    section(id = "net", label = "Network", help = "Where to connect"),
    section(id = "login", label = "Login")
)]
struct Connection {
    #[schema(section = "net")]
    host: String,
    #[schema(section = "net", order = 1)]
    #[form(widget = "number")]
    port: i64,
    verify: bool,
    #[form(
        placeholder = "/etc/ssl/ca.pem",
        visible_when(field = "verify", equals = true)
    )]
    #[map(rename = "ca-file")]
    ca: Option<String>,
    #[schema(section = "login")]
    #[form(nested)]
    auth: Auth,
}

fn pair() -> (Value, Value) {
    let schema = Connection::schema(Alloc::rust()).expect("the schema builds");
    let form = Connection::form(Alloc::rust()).expect("the form builds");
    (schema, form)
}

/// The derived form names only paths the derived schema declares, and
/// every condition can be met: `check` is the test that the two agree.
#[test]
fn a_derived_form_fits_its_derived_schema() {
    let (schema, form) = pair();
    let (s, f) = (
        SchemaRef::new(&schema).expect("a schema"),
        FormRef::new(&form).expect("a form"),
    );
    check(s, f).expect("the derived pair agrees");

    // The sections, in declaration order, with what they are called.
    let ids: Vec<(&str, &str, &str)> = f
        .sections()
        .map(|s| (s.id(), s.label(), s.help()))
        .collect();
    assert_eq!(
        ids,
        [
            ("net", "Network", "Where to connect"),
            ("login", "Login", "")
        ]
    );

    // The hints, under the keys the value uses: `#[map(rename)]` and the
    // nested member's `owner.member`.
    assert_eq!(f.hints("port").widget(), "number");
    assert_eq!(f.hints("ca-file").placeholder(), "/etc/ssl/ca.pem");
    let when = f.hints("ca-file").visible_when().expect("guarded");
    assert_eq!(when.field(), "verify");
    assert_eq!(when.equals().try_into().ok(), Some(true));
    assert_eq!(f.hints("auth.password").widget(), "password");
    assert!(f.hints("auth.username").is_empty(), "nothing was said");
    assert!(f.hints("host").is_empty());
    assert_eq!(f.fields().count(), 3, "only fields with hints are named");

    // The layout groups as the schema places and the form titles.
    let groups = layout(s, f);
    let titled: Vec<(Option<&str>, Vec<&str>)> = groups
        .iter()
        .map(|g| {
            (
                g.section.map(|s| s.label()),
                g.fields.iter().map(|p| p.field.key()).collect(),
            )
        })
        .collect();
    assert_eq!(
        titled,
        [
            (None, vec!["verify", "ca-file"]),
            (Some("Network"), vec!["port", "host"]),
            (Some("Login"), vec!["auth"]),
        ]
    );

    // And the guard holds.
    let mut values = Map::new();
    values.set("verify", false).unwrap();
    assert!(!is_visible(s, f, "ca-file", &values.into()).unwrap());
    let mut values = Map::new();
    values.set("verify", true).unwrap();
    assert!(is_visible(s, f, "ca-file", &values.into()).unwrap());
}

/// **The derive writes the keys this crate names.** `#[derive(Schema)]`
/// lives in `guatiao-derive` and emits `"x-section"` and its siblings as
/// literals, because a crate deriving a schema must not have to depend on
/// this one. That literal and [`vocab::X_SECTION`] are the same key in two
/// places, and this is what notices when they stop being.
#[test]
fn the_schema_derive_writes_the_keys_this_crate_names() {
    let schema = Connection::schema(Alloc::rust()).expect("the schema builds");
    let s = SchemaRef::new(&schema).expect("a schema");

    let host = s.find("host").expect("host is declared");
    assert_eq!(host.section(), "net");
    assert_eq!(host.order(), 0, "nothing said, so no ordering information");
    assert!(!host.is_advanced());

    let port = s.find("port").expect("port is declared");
    assert_eq!(port.section(), "net");
    assert_eq!(port.order(), 1);

    // Read the raw keys too: the trait is a convenience over them, and a
    // renamed constant that both sides moved together would pass above.
    let raw = |key: &str| {
        TryAsRef::<Map>::try_as_ref(port.as_value())
            .and_then(|m| m.get(key))
            .is_some()
    };
    assert!(raw("x-section") && raw("x-order"));
    assert_eq!(vocab::X_SECTION, "x-section");
    assert_eq!(vocab::X_ORDER, "x-order");
    assert_eq!(vocab::X_ADVANCED, "x-advanced");
}

/// A member's hints compose under the owner's key, and a hand-written
/// owner can compose them the same way the derive does.
#[test]
fn a_member_s_hints_compose_under_its_key() {
    let form = Auth::hints("session.auth.", Form::new())
        .finish()
        .expect("builds");
    let f = FormRef::new(&form).expect("a form");
    assert_eq!(f.hints("session.auth.password").widget(), "password");
    assert!(
        TryAsRef::<Map>::try_as_ref(&form)
            .and_then(|m| m.get(vocab::SECTIONS))
            .is_none(),
        "a member adds no sections"
    );
}
