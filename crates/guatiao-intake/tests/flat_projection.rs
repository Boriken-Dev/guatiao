// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A tagged value projected onto flat `key -> text` storage.
//!
//! The discriminant lands under the field's own key and each payload
//! field under `<key>.<field>`. That is the URI spelling too —
//! `?auth=userpass&auth.username=alice` — which is what lets a command
//! line select an arm at all.
extern crate alloc;

use alloc::collections::BTreeMap;
use guatiao::value::convert::TryAsRef;

use guatiao::schema::build::{ArmBuilder, FieldBuilder, KindBuilder, SchemaBuilder};
use guatiao::schema::read::{Kind, SchemaRef};
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
    m.set("auth", chosen);
    for (k, v) in fields {
        m.set(k, *v);
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
fn unflatten_answers_none_when_there_is_nothing_to_read() {
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
        flat::unflatten(alloc, host, &store_of(&[])).is_none(),
        "nothing stored under it"
    );

    // A plain field reads back its own entry. This used to answer `None`
    // for anything that was not tagged, back when the projection only
    // knew variants; it walks every kind now.
    let back = flat::unflatten(alloc, host, &store_of(&[("host", "x")]))
        .expect("a plain field is a value like any other");
    assert_eq!(TryInto::<&str>::try_into(&back).ok(), Some("x"));
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

    // A path that reads through a SCALAR is refused at that scalar. It
    // used to come back as "unknown option `host.min`", from a resolver
    // that could only walk one dot; now the walk goes as far as it can
    // and names where it stopped, which for a deeper path is the only
    // message a person can act on without bisecting it.
    let refused = flat::resolve_in(s, "host.min", selected(&[])).expect_err("host is text");
    match refused {
        guatiao::schema::ValidationError::BadValue { key, expected } => {
            assert_eq!(key, "host", "the step that failed");
            assert!(expected.contains("members"), "{expected}");
        }
        other => panic!("expected the bad-value shape: {other}"),
    }

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

/// **A field key holding a delimiter is legal.** The grammar has brackets
/// and quoting, so the key has an unambiguous spelling of its own and the
/// schema describes the struct exactly as it is.
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
    values.set(
        "auth.username",
        Text::new_in(alloc, "ana").map(Value::from).unwrap(),
    );
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

// --- a path resolves through everything a value can nest ----------------

/// A schema that nests every way a path can walk: an object inside an
/// object, a list of objects, an open map, and a variant with an arm
/// that adds fields.
fn nested(alloc: Alloc) -> Value {
    let tls = KindBuilder::map_in(
        alloc,
        vec![
            FieldBuilder::new_in(alloc, "verify", KindBuilder::bool_in(alloc)),
            FieldBuilder::new_in(alloc, "ca", KindBuilder::string_in(alloc)).sensitive(),
        ],
    );
    let connection = KindBuilder::map_in(
        alloc,
        vec![
            FieldBuilder::new_in(alloc, "host", KindBuilder::string_in(alloc)),
            FieldBuilder::new_in(alloc, "tls", tls),
        ],
    );
    let agent = KindBuilder::map_in(
        alloc,
        vec![
            FieldBuilder::new_in(alloc, "name", KindBuilder::string_in(alloc)),
            FieldBuilder::new_in(alloc, "secret", KindBuilder::string_in(alloc)).sensitive(),
        ],
    );

    SchemaBuilder::new_in(alloc)
        .field(FieldBuilder::new_in(alloc, "connection", connection))
        .field(FieldBuilder::new_in(
            alloc,
            "agent",
            KindBuilder::list_in(alloc, agent),
        ))
        .field(FieldBuilder::new_in(
            alloc,
            "env",
            KindBuilder::map_of_in(alloc, KindBuilder::string_in(alloc)),
        ))
        // A field whose own name holds a separator: legal since
        // `check_keys` went, and reachable only by quoting.
        .field(FieldBuilder::new_in(
            alloc,
            "a.b",
            KindBuilder::int_in(alloc),
        ))
        .finish()
        .expect("a schema this small does not exhaust an allocator")
}

/// Every step the grammar can take, against the schema above.
#[test]
fn a_path_resolves_through_objects_lists_and_open_maps() {
    let alloc = Alloc::rust();
    let doc = nested(alloc);
    let s = SchemaRef::new(&doc).expect("a schema is a map");
    let at = |key: &str| flat::resolve(s, key);

    // Two declared objects deep.
    assert_eq!(
        at("connection.tls.ca").map(|f| f.key()),
        Some("ca"),
        "a field two objects down is the field itself"
    );
    assert!(at("connection.tls.ca").expect("it resolves").is_sensitive());
    assert!(matches!(
        at("connection.tls").expect("it resolves").kind(),
        guatiao::schema::read::Kind::Map(_)
    ));

    // Through a list: the position is a step, and every element has the
    // one schema, so which index cannot matter.
    assert_eq!(at("agent[1].name").map(|f| f.key()), Some("name"));
    assert_eq!(at("agent[0].name").map(|f| f.key()), Some("name"));
    assert!(at("agent[7].secret").expect("it resolves").is_sensitive());
    assert_eq!(
        at("agent[0]").map(|f| f.key()),
        Some(""),
        "an element has no declared name; the path is the caller's"
    );
    assert!(at("agent[first]").is_none(), "a list is indexed, not keyed");

    // Through an open map: the keys are data, so any segment reaches the
    // one value schema.
    assert!(matches!(
        at("env[PATH]").expect("it resolves").kind(),
        guatiao::schema::read::Kind::Str { .. }
    ));
    assert!(
        at("env[0]").is_some(),
        "a number is a key here, not a position"
    );
    assert!(at("env[\"a.b\"]").is_some(), "and so is a quoted one");

    // Stopping: a scalar has no members.
    assert!(at("connection.tls.ca.x").is_none());
    assert!(at("connection.nonesuch").is_none());
    assert!(at("nonesuch.anything").is_none());

    // A declared key that holds a separator means itself, and a walk
    // reaching a member of that name has to quote it.
    assert_eq!(
        at("a.b").map(|f| f.key()),
        Some("a.b"),
        "the whole key names a field, so it is that field"
    );
}

/// The variant walk still works, and now works at depth.
#[test]
fn an_arms_member_resolves_at_any_depth() {
    let alloc = Alloc::rust();
    let flat_doc = schema(alloc);
    let s = SchemaRef::new(&flat_doc).expect("a schema");
    assert_eq!(
        flat::resolve(s, "auth.password").map(|f| f.key()),
        Some("password"),
        "one level, as it always did"
    );
    assert!(
        flat::resolve(s, "auth.password")
            .expect("it resolves")
            .is_sensitive()
    );
    assert!(flat::resolve(s, "auth.nonesuch").is_none());

    // And the same field reached through a list of objects.
    let session = KindBuilder::map_in(
        alloc,
        vec![FieldBuilder::new_in(
            alloc,
            "auth",
            KindBuilder::variant_in(
                alloc,
                "auth",
                vec![
                    ArmBuilder::new_in(alloc, "sso", "Single sign-on"),
                    ArmBuilder::new_in(alloc, "userpass", "Username and password").field(
                        FieldBuilder::new_in(alloc, "password", KindBuilder::string_in(alloc))
                            .sensitive(),
                    ),
                ],
            ),
        )],
    );
    let doc = SchemaBuilder::new_in(alloc)
        .field(FieldBuilder::new_in(
            alloc,
            "sessions",
            KindBuilder::list_in(alloc, session),
        ))
        .finish()
        .expect("it builds");
    let s = SchemaRef::new(&doc).expect("a schema");
    assert!(
        flat::resolve(s, "sessions[2].auth.password")
            .expect("a list, an object, a variant and its arm's field")
            .is_sensitive(),
        "is_sensitive follows the whole path, which is what a store asks"
    );
    assert!(flat::is_sensitive(s, "sessions[2].auth.password"));
    assert!(!flat::is_sensitive(s, "sessions[2].auth.nonesuch"));
}

/// `resolve_in` refuses at the segment that failed, and names it.
#[test]
fn a_store_refusal_names_the_prefix_that_failed() {
    let alloc = Alloc::rust();
    let doc = nested(alloc);
    let s = SchemaRef::new(&doc).expect("a schema");
    let nothing = |_: &str| None;

    assert_eq!(
        flat::resolve_in(s, "connection.tls.verify", nothing)
            .expect("it resolves")
            .key(),
        "verify"
    );

    let refused = flat::resolve_in(s, "connection.tls.nonesuch", nothing)
        .expect_err("tls declares no such member");
    match refused {
        guatiao::schema::ValidationError::UnknownOption { key, known } => {
            assert_eq!(key, "connection.tls.nonesuch");
            assert_eq!(known, ["verify", "ca"], "the members AT that step");
        }
        other => panic!("expected the unknown-key shape: {other}"),
    }

    let refused =
        flat::resolve_in(s, "connection.host.deeper", nothing).expect_err("a text has no members");
    match refused {
        guatiao::schema::ValidationError::BadValue { key, expected } => {
            assert_eq!(
                key, "connection.host",
                "the prefix that failed, not the whole"
            );
            assert!(expected.contains("members"), "{expected}");
        }
        other => panic!("expected the bad-value shape: {other}"),
    }
}

// --- flattening to every scalar leaf ------------------------------------

/// A value for the `nested` schema: two agents, a nested object, an open
/// map.
fn nested_value(alloc: Alloc) -> Value {
    let mut tls = Map::new_in(alloc);
    tls.set("verify", true);
    tls.set(
        "ca",
        Text::new_in(alloc, "/etc/ca.pem").map(Value::from).unwrap(),
    );
    let mut connection = Map::new_in(alloc);
    connection.set(
        "host",
        Text::new_in(alloc, "10.0.0.1").map(Value::from).unwrap(),
    );
    connection.set("tls", tls);

    let mut one = Map::new_in(alloc);
    one.set("name", Text::new_in(alloc, "one").map(Value::from).unwrap());
    one.set(
        "secret",
        Text::new_in(alloc, "s1").map(Value::from).unwrap(),
    );
    let mut two = Map::new_in(alloc);
    two.set("name", Text::new_in(alloc, "two").map(Value::from).unwrap());
    let mut agents = guatiao::List::new_in(alloc);
    agents.push_in(one, alloc).unwrap();
    agents.push_in(two, alloc).unwrap();

    let mut env = Map::new_in(alloc);
    env.set(
        "PATH",
        Text::new_in(alloc, "/bin").map(Value::from).unwrap(),
    );
    env.set(
        "a.b",
        Text::new_in(alloc, "dotted").map(Value::from).unwrap(),
    );

    let mut root = Map::new_in(alloc);
    root.set("connection", connection);
    root.set("agent", agents);
    root.set("env", env);
    root.into()
}

/// One entry per scalar leaf, each under the path that reaches it.
#[test]
fn flatten_writes_one_entry_per_scalar_leaf() {
    let alloc = Alloc::rust();
    let doc = nested(alloc);
    let s = SchemaRef::new(&doc).expect("a schema");
    let value = nested_value(alloc);
    let held = |key: &str| {
        TryAsRef::<Map>::try_as_ref(&value)
            .unwrap()
            .get(key)
            .unwrap()
    };

    let mut store = BTreeMap::new();
    assert!(flat::flatten(
        s.find("connection").unwrap(),
        held("connection"),
        &mut store
    ));
    assert!(flat::flatten(
        s.find("agent").unwrap(),
        held("agent"),
        &mut store
    ));
    assert!(flat::flatten(
        s.find("env").unwrap(),
        held("env"),
        &mut store
    ));

    let entries: Vec<(&str, &str)> = store
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(
        entries,
        [
            ("agent[0].name", "one"),
            ("agent[0].secret", "s1"),
            ("agent[1].name", "two"),
            ("connection.host", "10.0.0.1"),
            ("connection.tls.ca", "/etc/ca.pem"),
            ("connection.tls.verify", "true"),
            ("env[PATH]", "/bin"),
            // A key holding a separator needs NO quoting inside the
            // brackets: the `]` already says where the segment ends, so
            // this reads back as the one key `a.b`.
            ("env[a.b]", "dotted"),
        ],
        "a list indexes, an object dots, an open map brackets"
    );
}

/// What was written reads back as what it was.
#[test]
fn a_deep_value_survives_the_round_trip() {
    let alloc = Alloc::rust();
    let doc = nested(alloc);
    let s = SchemaRef::new(&doc).expect("a schema");
    let value = nested_value(alloc);
    let held = |key: &str| {
        TryAsRef::<Map>::try_as_ref(&value)
            .unwrap()
            .get(key)
            .unwrap()
            .clone_in(alloc)
            .unwrap()
    };

    for key in ["connection", "agent", "env"] {
        let original = held(key);
        let field = s.find(key).unwrap();
        let mut store = BTreeMap::new();
        assert!(flat::flatten(field, &original, &mut store));
        let back = flat::unflatten(alloc, field, &store).expect("it reads back");

        // Every leaf is text in a store, so the two are compared by
        // flattening again rather than by value: `verify` left as `true`
        // and comes back as `"true"`, which is what a flat store IS.
        let mut again = BTreeMap::new();
        assert!(flat::flatten(field, &back, &mut again));
        assert_eq!(store, again, "`{key}` did not survive the round trip");
    }
}

/// An empty container stores nothing, so it reads back absent. The one
/// fact a `key -> text` store cannot carry, stated rather than hidden.
#[test]
fn an_empty_container_stores_nothing() {
    let alloc = Alloc::rust();
    let doc = nested(alloc);
    let s = SchemaRef::new(&doc).expect("a schema");

    let empty_list = Value::from(guatiao::List::new_in(alloc));
    let mut store = BTreeMap::new();
    assert!(flat::flatten(
        s.find("agent").unwrap(),
        &empty_list,
        &mut store
    ));
    assert!(store.is_empty(), "nothing to write: {store:?}");
    assert!(
        flat::unflatten(alloc, s.find("agent").unwrap(), &store).is_none(),
        "and nothing to read, so it comes back absent rather than empty"
    );

    let empty_map = Value::from(Map::new_in(alloc));
    let mut store = BTreeMap::new();
    assert!(flat::flatten(
        s.find("env").unwrap(),
        &empty_map,
        &mut store
    ));
    assert!(store.is_empty());
}

/// Positions are taken in order and closed up: a store cannot hold a
/// hole, because a list has none to leave.
#[test]
fn a_gap_in_the_positions_closes_up() {
    let alloc = Alloc::rust();
    let doc = nested(alloc);
    let s = SchemaRef::new(&doc).expect("a schema");
    let store = store_of(&[("agent[0].name", "one"), ("agent[2].name", "three")]);

    let back = flat::unflatten(alloc, s.find("agent").unwrap(), &store).expect("two elements");
    let list = TryAsRef::<guatiao::List>::try_as_ref(&back).expect("a list");
    assert_eq!(list.len(), 2);
    let names: Vec<&str> = list
        .iter()
        .map(|v| {
            str_or(
                TryAsRef::<Map>::try_as_ref(v).and_then(|m| m.get("name")),
                "",
            )
        })
        .collect();
    assert_eq!(names, ["one", "three"]);
}

/// The clearing rule generalises: everything AT or UNDER the path goes
/// first, and a key that merely starts with the same letters does not.
#[test]
fn writing_clears_everything_under_the_path_first() {
    let alloc = Alloc::rust();
    let doc = nested(alloc);
    let s = SchemaRef::new(&doc).expect("a schema");

    let mut store = store_of(&[
        ("agent[0].name", "stale"),
        ("agent[3].secret", "stale"),
        ("agentry", "left alone"),
        ("connection.host", "left alone"),
    ]);

    let mut one = Map::new_in(alloc);
    one.set(
        "name",
        Text::new_in(alloc, "fresh").map(Value::from).unwrap(),
    );
    let mut agents = guatiao::List::new_in(alloc);
    agents.push_in(one, alloc).unwrap();

    assert!(flat::flatten(
        s.find("agent").unwrap(),
        &Value::from(agents),
        &mut store
    ));

    let keys: Vec<&str> = store.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        ["agent[0].name", "agentry", "connection.host"],
        "the old element is gone; a key that only shares a prefix is not"
    );
    assert_eq!(
        store.get("agent[0].name").map(String::as_str),
        Some("fresh")
    );
}

/// `clear_under` on its own, since a caller may want it without writing.
#[test]
fn clear_under_takes_a_subtree_and_nothing_else() {
    let mut store = store_of(&[
        ("auth", "userpass"),
        ("auth.username", "ana"),
        ("authority", "kept"),
        ("other", "kept"),
    ]);
    flat::clear_under(&mut store, "auth");
    let keys: Vec<&str> = store.keys().map(String::as_str).collect();
    assert_eq!(keys, ["authority", "other"]);
}

// --- the store round-trips ----------------------------------------------

fn texts_of(value: &Value) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for entry in TryAsRef::<Map>::try_as_ref(value).unwrap().entries() {
        out.push((
            entry.key().to_string(),
            str_or(Some(entry.value()), "").to_string(),
        ));
    }
    out
}

/// A sibling whose name sorts between `agent.` and `agent[` -- a capital, a
/// digit, a hyphen, an underscore -- must not hide the list or the open map
/// from `unflatten`.
#[test]
fn a_sibling_that_sorts_between_a_field_and_its_elements_loses_nothing() {
    let schema = SchemaBuilder::new()
        .field(FieldBuilder::new(
            "agent",
            KindBuilder::list(KindBuilder::string()),
        ))
        .field(FieldBuilder::new("agentName", KindBuilder::string()))
        .field(FieldBuilder::new("agent0", KindBuilder::string()))
        .field(FieldBuilder::new("agent-x", KindBuilder::string()))
        .field(FieldBuilder::new("agent_x", KindBuilder::string()))
        .field(FieldBuilder::new(
            "env",
            KindBuilder::map_of(KindBuilder::string()),
        ))
        .field(FieldBuilder::new("env-file", KindBuilder::string()))
        .field(FieldBuilder::new("envZ", KindBuilder::string()))
        .finish()
        .unwrap();
    let s = SchemaRef::new(&schema).unwrap();
    let alloc = Alloc::rust();

    let mut agents = guatiao::List::new();
    agents.push("one");
    agents.push("two");
    let mut env = Map::new();
    env.set("PATH", "/bin");
    env.set("HOME", "/root");

    let mut store = BTreeMap::new();
    assert!(flat::flatten(
        s.find("agent").unwrap(),
        &Value::from(agents),
        &mut store
    ));
    assert!(flat::flatten(
        s.find("env").unwrap(),
        &Value::from(env),
        &mut store
    ));
    for sibling in [
        "agentName",
        "agent0",
        "agent-x",
        "agent_x",
        "env-file",
        "envZ",
    ] {
        store.insert(sibling.to_string(), "sibling".to_string());
    }

    let agents = flat::unflatten(alloc, s.find("agent").unwrap(), &store)
        .expect("the list is still there beside its siblings");
    let list = TryAsRef::<guatiao::List>::try_as_ref(&agents).unwrap();
    assert_eq!(list.len(), 2);

    let env = flat::unflatten(alloc, s.find("env").unwrap(), &store)
        .expect("the open map is still there beside its siblings");
    assert_eq!(
        texts_of(&env),
        [
            ("HOME".to_string(), "/root".to_string()),
            ("PATH".to_string(), "/bin".to_string())
        ]
    );
    assert_eq!(guatiao_intake::validate_texts(s, &store), Ok(()));
}

/// A field called `a.b` and a field `a` with a member `b` are two keys in
/// a store, not one.
#[test]
fn a_dotted_top_level_field_is_stored_apart_from_a_member() {
    let schema = SchemaBuilder::new()
        .field(FieldBuilder::new(
            "a",
            KindBuilder::map(vec![FieldBuilder::new("b", KindBuilder::string())]),
        ))
        .field(FieldBuilder::new("a.b", KindBuilder::string()))
        .finish()
        .unwrap();
    let s = SchemaRef::new(&schema).unwrap();
    let alloc = Alloc::rust();

    let mut inner = Map::new();
    inner.set("b", "member");
    let mut store = BTreeMap::new();
    assert!(flat::flatten(
        s.find("a.b").unwrap(),
        &Value::from("top"),
        &mut store
    ));
    assert!(flat::flatten(
        s.find("a").unwrap(),
        &Value::from(inner),
        &mut store
    ));
    assert_eq!(
        store,
        store_of(&[("[a.b]", "top"), ("a.b", "member")]),
        "the dotted field is spelled as a bracket key"
    );

    let top = flat::unflatten(alloc, s.find("a.b").unwrap(), &store).unwrap();
    assert_eq!(TryAsRef::<str>::try_as_ref(&top), Some("top"));
    let member = flat::unflatten(alloc, s.find("a").unwrap(), &store).unwrap();
    assert_eq!(texts_of(&member), [("b".to_string(), "member".to_string())]);

    assert_eq!(flat::keys(s.find("a.b").unwrap()), ["[a.b]"]);
    assert_eq!(
        flat::resolve(s, "[a.b]").map(|f| f.key()),
        Some("a.b"),
        "the spelling the store uses names the field"
    );
}

/// `keys` and `split` read the key the way `flatten` writes it.
#[test]
fn keys_and_split_agree_with_flatten_for_a_member_that_needs_quoting() {
    let schema = SchemaBuilder::new()
        .field(FieldBuilder::new(
            "auth",
            KindBuilder::variant(
                "auth",
                vec![
                    ArmBuilder::new("x", "X")
                        .field(FieldBuilder::new("c.d", KindBuilder::string()))
                        .field(FieldBuilder::new(r#""q""#, KindBuilder::string()))
                        .field(FieldBuilder::new("plain", KindBuilder::string())),
                ],
            ),
        ))
        .finish()
        .unwrap();
    let s = SchemaRef::new(&schema).unwrap();
    let auth = s.find("auth").unwrap();

    let value = tagged(
        "x",
        &[("c.d", "one"), (r#""q""#, "two"), ("plain", "three")],
    );
    let mut store = BTreeMap::new();
    assert!(flat::flatten(auth, &value, &mut store));
    assert_eq!(store.len(), 4, "{store:?}");

    let mut written: Vec<&str> = store.keys().map(String::as_str).collect();
    written.sort_unstable();
    let mut offered = flat::keys(auth);
    offered.sort();
    assert_eq!(offered, written, "keys lists what flatten writes");

    assert_eq!(
        flat::split("auth[c.d]"),
        Some(("auth", "c.d".into())),
        "split reads the bracket as one member"
    );
    assert_eq!(
        flat::split(r#"auth["\"q\""]"#),
        Some(("auth", r#""q""#.into())),
        "and unescapes a quoted one"
    );
    assert_eq!(flat::split("auth.plain"), Some(("auth", "plain".into())));
    assert_eq!(
        flat::split("agent[0].name"),
        Some(("agent[0]", "name".into()))
    );
    assert_eq!(flat::split("auth"), None);
    assert_eq!(flat::split("not[a path"), None);
}

/// A spelling the grammar refuses is not a key the store can hold for a
/// field: validation names it and reading skips it.
#[test]
fn a_store_key_off_the_grammar_is_refused_and_not_read() {
    let schema = SchemaBuilder::new()
        .field(FieldBuilder::new(
            "agent",
            KindBuilder::list(KindBuilder::string()),
        ))
        .field(FieldBuilder::new(
            "env",
            KindBuilder::map_of(KindBuilder::string()),
        ))
        .finish()
        .unwrap();
    let s = SchemaRef::new(&schema).unwrap();
    let alloc = Alloc::rust();
    let agent = s.find("agent").unwrap();

    for off in ["agent.[0]", "agent[0]x", "agent[a[b]"] {
        let store = store_of(&[(off, "x")]);
        assert!(
            guatiao_intake::validate_texts(s, &store).is_err(),
            "`{off}` is refused"
        );
        assert!(
            flat::unflatten(alloc, agent, &store).is_none(),
            "`{off}` is not an element"
        );
    }

    // A leading zero makes a key, and a key under a list is no element.
    assert!(flat::unflatten(alloc, agent, &store_of(&[("agent[01]", "x")])).is_none());
    // Under an open map it is an ordinary key, and survives a round trip.
    let env = s.find("env").unwrap();
    let read = flat::unflatten(alloc, env, &store_of(&[("env[01]", "z")])).unwrap();
    assert_eq!(texts_of(&read), [("01".to_string(), "z".to_string())]);
    let mut again = BTreeMap::new();
    assert!(flat::flatten(env, &read, &mut again));
    assert_eq!(again, store_of(&[("env[01]", "z")]));
}

/// A value nested past the bound answers `false` and does not recurse or
/// grow without limit.
#[test]
fn flatten_stops_at_the_depth_bound() {
    let nest = |levels: usize| {
        let mut kind = KindBuilder::string();
        for _ in 0..levels {
            kind = KindBuilder::list(kind);
        }
        let schema = SchemaBuilder::new()
            .field(FieldBuilder::new("x", kind))
            .finish()
            .unwrap();
        let mut value = Value::from("leaf");
        for _ in 0..levels {
            let mut list = guatiao::List::new();
            list.push(value);
            value = list.into();
        }
        (schema, value)
    };

    let (schema, shallow) = nest(20);
    let s = SchemaRef::new(&schema).unwrap();
    let mut store = BTreeMap::new();
    assert!(flat::flatten(s.find("x").unwrap(), &shallow, &mut store));
    assert_eq!(store.len(), 1);

    let (schema, deep) = nest(300);
    let s = SchemaRef::new(&schema).unwrap();
    let mut store = BTreeMap::new();
    assert!(
        !flat::flatten(s.find("x").unwrap(), &deep, &mut store),
        "a value nested past the bound does not fit"
    );
    assert!(store.is_empty(), "and nothing past the bound is written");
}

/// An open map's key reads back in any spelling the grammar reads (`01`
/// quoted, `env["01"]`), and so does a variant under such a key, whose
/// discriminant is found where the store spelled it; where both spellings
/// are present, the one `flatten` writes wins. A top-level field holding
/// a `.` is read under `[a.b]` alone: its bare name (`a.b`, `x.y[k]`)
/// reaches nothing.
#[test]
fn a_store_reads_every_spelling_of_a_key_and_only_the_bracketed_dotted_field() {
    let schema = SchemaBuilder::new()
        .field(FieldBuilder::new(
            "env",
            KindBuilder::map_of(KindBuilder::string()),
        ))
        .field(FieldBuilder::new("a.b", KindBuilder::string()))
        .field(FieldBuilder::new(
            "x.y",
            KindBuilder::map_of(KindBuilder::string()),
        ))
        .field(FieldBuilder::new(
            "m",
            KindBuilder::map_of(KindBuilder::variant(
                "kind",
                vec![
                    ArmBuilder::new("userpass", "User and password")
                        .field(FieldBuilder::new("username", KindBuilder::string())),
                ],
            )),
        ))
        .finish()
        .unwrap();
    let s = SchemaRef::new(&schema).unwrap();
    let alloc = Alloc::rust();
    let read = |field: &str, store: &BTreeMap<String, String>| {
        flat::unflatten(alloc, s.find(field).unwrap(), store)
    };

    let quoted = store_of(&[
        ("env[\"01\"]", "old"),
        ("env[PATH]", "/bin"),
        ("m[\"01\"]", "userpass"),
        ("m[\"01\"].username", "alice"),
    ]);
    assert_eq!(guatiao_intake::validate_texts(s, &quoted), Ok(()));
    assert_eq!(
        texts_of(&read("env", &quoted).expect("the open map reads")),
        [
            ("01".to_string(), "old".to_string()),
            ("PATH".to_string(), "/bin".to_string())
        ]
    );
    let m = read("m", &quoted).expect("the variant under a quoted key reads");
    let one = TryAsRef::<Map>::try_as_ref(&m)
        .and_then(|m| m.get("01"))
        .expect("under its key");
    assert_eq!(
        texts_of(one),
        [
            ("kind".to_string(), "userpass".to_string()),
            ("username".to_string(), "alice".to_string())
        ]
    );

    let bare = store_of(&[("a.b", "dotted"), ("x.y[k]", "v")]);
    assert_eq!(
        read("a.b", &bare),
        None,
        "a bare dotted name is not the field's key"
    );
    assert_eq!(read("x.y", &bare), None, "nor is a path under one");
    assert!(flat::resolve(s, "x.y[k]").is_none());
    assert!(
        guatiao_intake::validate_texts(s, &store_of(&[("x.y[k]", "v")])).is_err(),
        "a key under a bare dotted name is undeclared"
    );
    assert!(
        guatiao_intake::validate_texts(s, &store_of(&[("a.b", "dotted")])).is_err(),
        "and so is the bare name itself, which nothing reads back"
    );
    let bracketed = store_of(&[("[a.b]", "dotted"), ("[x.y][k]", "v")]);
    assert_eq!(guatiao_intake::validate_texts(s, &bracketed), Ok(()));
    let dotted = read("a.b", &bracketed).expect("the bracketed key reads");
    assert_eq!(TryAsRef::<str>::try_as_ref(&dotted), Some("dotted"));
    assert_eq!(
        texts_of(&read("x.y", &bracketed).expect("so does a container under it")),
        [("k".to_string(), "v".to_string())]
    );

    let both = store_of(&[("env[\"01\"]", "old"), ("env[01]", "new")]);
    assert_eq!(guatiao_intake::validate_texts(s, &both), Ok(()));
    assert_eq!(
        texts_of(&read("env", &both).unwrap()),
        [("01".to_string(), "new".to_string())]
    );
}

/// A tuple stores each position under its index, reads the positions back
/// in order, and a path into one reaches that position's own schema.
#[test]
fn a_tuple_flattens_by_position() {
    let alloc = Alloc::rust();
    let schema = SchemaBuilder::new_in(alloc)
        .field(FieldBuilder::new_in(
            alloc,
            "pair",
            KindBuilder::tuple_in(
                alloc,
                vec![KindBuilder::string_in(alloc), KindBuilder::int_in(alloc)],
            ),
        ))
        .finish()
        .unwrap();
    let s = SchemaRef::new(&schema).unwrap();
    let pair = s.find("pair").unwrap();
    assert!(matches!(pair.kind(), Kind::Tuple(_)));

    let mut list = guatiao::List::new();
    list.push("a");
    list.push(7i64);
    let mut store = BTreeMap::new();
    assert!(flat::flatten(pair, &list.into(), &mut store));
    assert_eq!(store_of(&[("pair[0]", "a"), ("pair[1]", "7")]), store);

    let back = flat::unflatten(alloc, pair, &store).unwrap();
    let back = <&guatiao::List>::try_from(&back).unwrap();
    assert_eq!(back.len(), 2);
    assert_eq!(str_or(back.get(1), ""), "7");

    assert!(matches!(
        flat::resolve(s, "pair[1]").unwrap().kind(),
        Kind::Int { .. }
    ));
    assert!(flat::resolve(s, "pair[2]").is_none());
    assert!(flat::resolve(s, "pair.x").is_none());

    // A gap ends the tuple: position 1 cannot move up into 0.
    let gap = store_of(&[("pair[1]", "7")]);
    assert!(flat::unflatten(alloc, pair, &gap).is_none());
}
