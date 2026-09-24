// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A tagged value projected onto flat `key -> text` storage.
//!
//! The discriminant lands under the field's own key and each payload
//! field under `<key>.<field>`. That is the URI spelling too —
//! `?auth=userpass&auth.username=alice` — which is what lets a command
//! line select an arm at all.

use guatiao::value::convert::TryAsRef;
use std::collections::BTreeMap;

use guatiao::schema::build::{ArmBuilder, FieldBuilder, KindBuilder, SchemaBuilder};
use guatiao::schema::read::SchemaRef;
use guatiao::value::alloc::Alloc;
use guatiao::value::read::str_or;
use guatiao::{Map, Text, Value};
use guatiao_intake::flat;
use guatiao_intake::path;

/// A schema with one tagged field: two arms, one of them empty.
fn schema(alloc: Alloc) -> Value {
    SchemaBuilder::new_in(alloc)
        .field(FieldBuilder::new_in(
            alloc,
            "auth",
            KindBuilder::variant_in(
                alloc,
                "auth",
                vec![
                    // No fields, and that is complete rather than missing:
                    // "use the ambient credential" is the common case.
                    ArmBuilder::new_in(alloc, "sso", "Single sign-on"),
                    ArmBuilder::new_in(alloc, "userpass", "Username and password")
                        .field(FieldBuilder::new_in(
                            alloc,
                            "username",
                            KindBuilder::string_in(alloc),
                        ))
                        .field(
                            FieldBuilder::new_in(alloc, "password", KindBuilder::string_in(alloc))
                                .sensitive(),
                        ),
                ],
            ),
        ))
        .field(FieldBuilder::new_in(
            alloc,
            "host",
            KindBuilder::string_in(alloc),
        ))
        .finish()
        .expect("a schema this small does not exhaust an allocator")
}

/// A tagged value: the discriminant plus whatever fields are given.
fn tagged(chosen: &str, fields: &[(&str, &str)]) -> Value {
    let mut m = Map::new();
    m.set("auth", chosen).unwrap();
    for (k, v) in fields {
        m.set(k, *v).unwrap();
    }
    m.into()
}

fn store_of(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn flatten_writes_the_discriminant_and_each_field() {
    let alloc = Alloc::rust();
    let s = schema(alloc);
    let s = SchemaRef::new(&s).unwrap();
    let auth = s.find("auth").unwrap();

    let mut store = BTreeMap::new();
    let value = tagged("userpass", &[("username", "alice"), ("password", "s3cret")]);
    assert!(flat::flatten(auth, &value, &mut store));

    assert_eq!(store.get("auth").map(String::as_str), Some("userpass"));
    assert_eq!(
        store.get("auth.username").map(String::as_str),
        Some("alice")
    );
    assert_eq!(
        store.get("auth.password").map(String::as_str),
        Some("s3cret")
    );
}

/// The deletion is first and unconditional. Switching to an arm that
/// happens to share a field name must not leave the old arm's value
/// behind, which is what a deletion done after the writes would do.
#[test]
fn switching_arms_clears_the_previous_arms_payload() {
    let alloc = Alloc::rust();
    let s = schema(alloc);
    let s = SchemaRef::new(&s).unwrap();
    let auth = s.find("auth").unwrap();

    let mut store = BTreeMap::new();
    let first = tagged("userpass", &[("username", "alice"), ("password", "s3cret")]);
    assert!(flat::flatten(auth, &first, &mut store));

    // An arm with no fields at all: everything under the prefix goes.
    let second = tagged("sso", &[]);
    assert!(flat::flatten(auth, &second, &mut store));

    assert_eq!(store.get("auth").map(String::as_str), Some("sso"));
    assert!(
        !store.keys().any(|k| k.starts_with("auth.")),
        "an arm with no fields stores nothing, and the previous arm's password is gone: {store:?}"
    );
}

/// Reading back drops any `<key>.*` the selected arm does not declare, so
/// a record that picked up a stale field some other way still reads as the
/// arm says it is.
#[test]
fn unflatten_drops_fields_the_arm_does_not_declare() {
    let alloc = Alloc::rust();
    let s = schema(alloc);
    let s = SchemaRef::new(&s).unwrap();
    let auth = s.find("auth").unwrap();

    let store = store_of(&[
        ("auth", "sso"),
        // Left over from a previous arm, by whatever route.
        ("auth.username", "alice"),
        ("auth.password", "s3cret"),
    ]);
    let back = flat::unflatten(alloc, auth, &store).expect("the discriminant names an arm");
    assert_eq!(
        str_or(
            TryAsRef::<Map>::try_as_ref(&back).and_then(|m| m.get("auth")),
            ""
        ),
        "sso"
    );
    assert!(
        TryAsRef::<Map>::try_as_ref(&back)
            .and_then(|m| m.get("username"))
            .is_none()
            && TryAsRef::<Map>::try_as_ref(&back)
                .and_then(|m| m.get("password"))
                .is_none(),
        "neither field is declared by the sso arm"
    );
}

#[test]
fn unflatten_answers_none_when_there_is_no_tagged_value() {
    let alloc = Alloc::rust();
    let s = schema(alloc);
    let s = SchemaRef::new(&s).unwrap();
    let auth = s.find("auth").unwrap();
    let host = s.find("host").unwrap();

    assert!(
        flat::unflatten(alloc, auth, &store_of(&[])).is_none(),
        "no discriminant at all"
    );
    assert!(
        flat::unflatten(alloc, auth, &store_of(&[("auth", "nonesuch")])).is_none(),
        "a discriminant naming no declared arm"
    );
    assert!(
        flat::unflatten(alloc, host, &store_of(&[("host", "x")])).is_none(),
        "a field that is not tagged at all"
    );
}

#[test]
fn keys_enumerates_every_flat_key_the_option_can_occupy() {
    let alloc = Alloc::rust();
    let s = schema(alloc);
    let s = SchemaRef::new(&s).unwrap();

    let mut keys = flat::keys(s.find("auth").unwrap());
    keys.sort();
    assert_eq!(keys, ["auth", "auth.password", "auth.username"]);

    assert_eq!(
        flat::keys(s.find("host").unwrap()),
        ["host"],
        "an untagged field occupies only its own key"
    );
}

/// `sensitive` on a dotted key resolves to the arm field's own flag, which
/// is what lets a field-level encryption path keep working with no new
/// concept.
#[test]
fn is_sensitive_follows_the_projection_and_an_unknown_key_is_not_a_secret() {
    let alloc = Alloc::rust();
    let s = schema(alloc);
    let s = SchemaRef::new(&s).unwrap();

    assert!(flat::is_sensitive(s, "auth.password"));
    assert!(!flat::is_sensitive(s, "auth.username"));
    assert!(!flat::is_sensitive(s, "auth"));
    assert!(
        !flat::is_sensitive(s, "auth.nonesuch"),
        "an unknown key is not a secret, which is the wrong default to reach by accident \
         and so is stated in one place"
    );
    assert!(!flat::is_sensitive(s, "nonesuch"));
}

#[test]
fn resolve_follows_one_level_of_projection() {
    let alloc = Alloc::rust();
    let s = schema(alloc);
    let s = SchemaRef::new(&s).unwrap();

    assert_eq!(flat::resolve(s, "auth").map(|o| o.key()), Some("auth"));
    assert_eq!(
        flat::resolve(s, "auth.password").map(|o| o.key()),
        Some("password"),
        "a dotted key resolves to the arm's own field"
    );
    assert!(flat::resolve(s, "auth.nonesuch").is_none());
    assert!(flat::resolve(s, "nonesuch").is_none());
}

/// A separator inside a field's own key would make a payload key
/// ambiguous with a field key, so it is caught at declaration rather
/// than tolerated at read time.
/// A store says which arm; a payload key is checked against THAT arm.
///
/// `resolve` alone answers for the schema, where `auth.username` is a
/// field of some arm. In a store that selected the empty arm it is a
/// field of the wrong one, and a text-only front end -- a URI -- has no
/// other way to be told so.
#[test]
fn resolve_in_a_store_follows_the_selected_arm() {
    let alloc = Alloc::rust();
    let value = schema(alloc);
    let s = SchemaRef::new(&value).unwrap();

    let selected = |store: &[(&str, &str)]| {
        let store = store_of(store);
        move |k: &str| store.get(k).cloned()
    };

    let field = flat::resolve_in(s, "auth.username", selected(&[("auth", "userpass")]))
        .expect("the selected arm's own field");
    assert_eq!(field.key(), "username");

    let refused = flat::resolve_in(s, "auth.username", selected(&[("auth", "sso")]))
        .expect_err("a field belonging to an unselected arm");
    assert!(
        refused.to_string().contains("sso") && refused.to_string().contains("auth.username"),
        "names the arm and the key: {refused}"
    );

    // No selection and no default: the payload cannot be checked yet,
    // and the message says what to give.
    let undecided =
        flat::resolve_in(s, "auth.username", selected(&[])).expect_err("nothing chose an arm");
    assert!(
        undecided.to_string().contains("sso, userpass"),
        "{undecided}"
    );

    // A dotted key whose stem is not tagged at all is unknown, listing
    // what does exist.
    let unknown = flat::resolve_in(s, "host.min", selected(&[])).expect_err("host is text");
    assert!(matches!(
        unknown,
        guatiao::schema::ValidationError::UnknownOption { .. }
    ));

    // And a whole store, through the validator a front end calls.
    use guatiao_intake::validate_texts;
    assert!(
        validate_texts(
            s,
            &store_of(&[("auth", "userpass"), ("auth.username", "alice")])
        )
        .is_ok()
    );
    assert!(validate_texts(s, &store_of(&[("auth", "sso")])).is_ok());
    assert!(validate_texts(s, &store_of(&[("auth", "sso"), ("auth.username", "alice")])).is_err());
    assert!(
        validate_texts(
            s,
            &store_of(&[("auth", "userpass"), ("auth.usernme", "alice")])
        )
        .is_err()
    );
}

/// **A field key holding a delimiter is legal**, and used not to be.
///
/// The old rule refused `auth.username` at `SchemaBuilder::finish`,
/// because the only spelling for a nested path was `owner.member` and a
/// key like that made one ambiguous. The grammar has brackets and
/// quoting now, so the key has an unambiguous spelling of its own and
/// the schema is free to describe the struct exactly as it is.
#[test]
fn a_field_key_may_hold_a_delimiter_and_is_named_by_quoting_it() {
    let alloc = Alloc::rust();

    let schema = SchemaBuilder::new_in(alloc)
        .field(FieldBuilder::new_in(
            alloc,
            "auth.username",
            KindBuilder::string_in(alloc),
        ))
        .finish()
        .expect("a schema describes the struct as it is");

    let s = SchemaRef::new(&schema).expect("a schema is a map");
    assert!(
        s.find("auth.username").is_some(),
        "the field is declared under the name it was given"
    );

    // The value it holds is named by quoting: `["auth.username"]` is that
    // one key, and `auth.username` is a two-step path that finds nothing
    // here.
    let mut values = Map::new_in(alloc);
    values
        .set(
            "auth.username",
            Text::new_in(alloc, "ana").map(Value::from).unwrap(),
        )
        .unwrap();
    let values = Value::from(values);

    let quoted = path::parse("[\"auth.username\"]").expect("a path");
    assert_eq!(
        path::get(&values, quoted).and_then(|v| TryInto::<&str>::try_into(v).ok()),
        Some("ana")
    );

    let two_steps = path::parse("auth.username").expect("also a path");
    assert!(
        path::get(&values, two_steps).is_none(),
        "unquoted, it means `auth` then `username`, which is not there"
    );
}
