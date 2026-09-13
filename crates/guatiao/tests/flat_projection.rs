// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A tagged value projected onto flat `key -> text` storage.
//!
//! The discriminant lands under the option's own key and each payload
//! field under `<key>.<field>`. That is the URI spelling too —
//! `?auth=userpass&auth.username=alice` — which is what lets a command
//! line select an arm at all.

use std::collections::BTreeMap;

use guatiao::Value;
use guatiao::schema::build::{ArmBuilder, KindBuilder, OptionBuilder, SchemaBuilder};
use guatiao::schema::flat;
use guatiao::schema::read::SchemaRef;
use guatiao::value::alloc::Alloc;
use guatiao::value::read::str_or;

/// A schema with one tagged option: two arms, one of them empty.
fn schema(alloc: Alloc) -> Value {
    SchemaBuilder::new_in(alloc)
        .option(OptionBuilder::new_in(
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
                        .field(OptionBuilder::new_in(
                            alloc,
                            "username",
                            KindBuilder::string_in(alloc),
                        ))
                        .field(
                            OptionBuilder::new_in(alloc, "password", KindBuilder::string_in(alloc))
                                .sensitive(),
                        ),
                ],
            ),
        ))
        .option(OptionBuilder::new_in(
            alloc,
            "host",
            KindBuilder::string_in(alloc),
        ))
        .finish()
        .expect("a schema this small does not exhaust an allocator")
}

/// A tagged value: the discriminant plus whatever fields are given.
fn tagged(chosen: &str, fields: &[(&str, &str)]) -> Value {
    let mut m = Value::map();
    m.set("auth", chosen).unwrap();
    for (k, v) in fields {
        m.set(k, *v).unwrap();
    }
    m
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
    assert_eq!(str_or(back.get("auth"), ""), "sso");
    assert!(
        back.get("username").is_none() && back.get("password").is_none(),
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
        "an option that is not tagged at all"
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
        "an untagged option occupies only its own key"
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

/// A separator inside an option's own key would make a payload key
/// ambiguous with an option key, so it is caught at declaration rather
/// than tolerated at read time.
#[test]
fn an_option_key_containing_the_separator_is_rejected() {
    let alloc = Alloc::rust();

    let good = schema(alloc);
    assert_eq!(flat::check_keys(SchemaRef::new(&good).unwrap()), Ok(()));

    let bad = SchemaBuilder::new_in(alloc)
        .option(OptionBuilder::new_in(
            alloc,
            "auth.username",
            KindBuilder::string_in(alloc),
        ))
        .finish()
        .unwrap();
    assert_eq!(
        flat::check_keys(SchemaRef::new(&bad).unwrap()),
        Err("auth.username".to_string())
    );
}
