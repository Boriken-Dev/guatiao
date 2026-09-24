// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A type describing the values it accepts.
//!
//! The test that matters most here is the last one: a value the type
//! writes validates against the schema the same type declares. Those are
//! two derives reading one declaration, and if they ever disagree, a
//! provider rejects the exact configuration its own schema asked for.

#![cfg(feature = "derive")]

use guatiao::schema::read::{Kind, SchemaRef};
use guatiao::schema::validate::validate_map;
use guatiao::value::alloc::Alloc;
use guatiao::value::convert::{TryAsMut, TryAsRef};
use guatiao::{Bytes, Map, Number, Schema, Text, ToValue, ValueError};

fn alloc_and<R>(body: impl FnOnce(Alloc) -> R) -> R {
    let alloc = Alloc::rust();
    body(alloc)
}

#[derive(Debug, PartialEq, Schema, ToValue)]
struct Tls {
    /// Whether to check the certificate.
    verify: bool,
    ca: Option<String>,
}

#[derive(Debug, PartialEq, Schema, ToValue)]
struct Connection {
    /// Where to connect.
    ///
    /// A host name or an address.
    host: String,
    port: u16,
    #[schema(title = "Password", sensitive)]
    password: Option<String>,
    #[schema(default = 30i64)]
    timeout: i64,
    tags: Vec<String>,
    ticket: Bytes,
    tls: Tls,
    #[map(rename = "max-size")]
    max_size: u64,
    #[map(skip)]
    cache: Vec<String>,
}

fn sample() -> Connection {
    Connection {
        host: "10.0.0.1".into(),
        port: 5900,
        password: None,
        timeout: 30,
        tags: vec!["prod".into()],
        ticket: Bytes(vec![1, 2, 3]),
        tls: Tls {
            verify: true,
            ca: None,
        },
        max_size: u64::MAX,
        cache: Vec::new(),
    }
}

#[test]
fn the_schema_lists_the_stored_fields_in_declaration_order() {
    alloc_and(|alloc| {
        let declared = Connection::schema(alloc).unwrap();
        let s = SchemaRef::new(&declared).unwrap();
        let keys: Vec<String> = s.fields().map(|o| o.key().to_string()).collect();
        assert_eq!(
            keys,
            [
                "host", "port", "password", "timeout", "tags", "ticket", "tls", "max-size"
            ],
            "declaration order, the rename applied, and the skipped field absent"
        );
    });
}

/// A Rust width already states its bounds, so the schema does not ask the
/// author to restate them.
#[test]
fn an_integer_kind_carries_the_bounds_of_its_width() {
    alloc_and(|alloc| {
        let declared = Connection::schema(alloc).unwrap();
        let s = SchemaRef::new(&declared).unwrap();

        assert!(matches!(
            s.find("port").unwrap().kind(),
            Kind::Int {
                min: Some(0),
                max: Some(65535)
            }
        ));
        // `u64::MAX` has no `i64` to be written as, so the upper bound is
        // left off rather than clamped to something nobody declared.
        assert!(matches!(
            s.find("max-size").unwrap().kind(),
            Kind::Int {
                min: Some(0),
                max: None
            }
        ));
        assert!(matches!(
            s.find("timeout").unwrap().kind(),
            Kind::Int {
                min: Some(i64::MIN),
                max: Some(i64::MAX)
            }
        ));
    });
}

/// `Option<T>` is how a field says it may be left out, and nothing else
/// says it.
#[test]
fn required_comes_from_the_type_not_from_an_attribute() {
    alloc_and(|alloc| {
        let declared = Connection::schema(alloc).unwrap();
        let s = SchemaRef::new(&declared).unwrap();
        assert!(s.find("host").unwrap().is_required());
        assert!(s.find("tls").unwrap().is_required());
        assert!(
            !s.find("password").unwrap().is_required(),
            "an Option field is the one thing that is not required"
        );
    });
}

/// Help text comes from the doc comment, because that is where a person
/// already writes it. Writing it twice is how the two drift apart.
#[test]
fn a_doc_comment_becomes_the_help_text() {
    alloc_and(|alloc| {
        let declared = Connection::schema(alloc).unwrap();
        let s = SchemaRef::new(&declared).unwrap();
        assert_eq!(
            s.find("host").unwrap().description(),
            "Where to connect. A host name or an address.",
            "every line of the comment, joined"
        );
        assert_eq!(
            s.find("port").unwrap().description(),
            "",
            "no comment is no help, not an empty sentence"
        );
    });
}

/// The `#[schema(..)]` keys **this crate owns** reach the field.
///
/// The derive also accepts attributes for presentation keys named
/// elsewhere; those are exercised where their vocabulary lives, in
/// `guatiao-intake`'s `form_derive.rs`, because a test here would have to
/// spell a word this crate deliberately does not know.
#[test]
fn the_schema_attributes_reach_the_option() {
    alloc_and(|alloc| {
        let declared = Connection::schema(alloc).unwrap();
        let s = SchemaRef::new(&declared).unwrap();

        let password = s.find("password").unwrap();
        assert_eq!(password.title(), "Password");
        assert!(password.is_sensitive());

        let timeout = s.find("timeout").unwrap();
        assert!(!timeout.is_sensitive());
        let default: i64 = timeout.default().unwrap().try_into().unwrap();
        assert_eq!(
            default, 30,
            "a default is written in Rust and crosses as a value"
        );
    });
}

/// The three kinds a struct needs that a scalar vocabulary does not have.
#[test]
fn a_sequence_a_blob_and_a_nested_struct_each_have_a_kind() {
    alloc_and(|alloc| {
        let declared = Connection::schema(alloc).unwrap();
        let s = SchemaRef::new(&declared).unwrap();

        let tags = s.find("tags").unwrap().kind();
        assert!(matches!(tags, Kind::List(_)));
        assert!(
            matches!(tags.items(), Kind::Str),
            "a list says what it holds"
        );

        assert!(matches!(s.find("ticket").unwrap().kind(), Kind::Bytes));

        let tls = s.find("tls").unwrap().kind();
        assert!(matches!(tls, Kind::Map(_)));
        let fields: Vec<String> = tls.fields().map(|f| f.key().to_string()).collect();
        assert_eq!(fields, ["verify", "ca"], "a nested struct carries its own");
        assert_eq!(
            tls.fields().next().unwrap().description(),
            "Whether to check the certificate.",
            "and its fields are ordinary fields, doc comments and all"
        );
    });
}

/// **The agreement.** The value a type writes is one its own schema
/// accepts, checked by the validator rather than by eye. Two derives read
/// one declaration; if they ever disagree, a provider rejects exactly the
/// configuration its own schema asked for.
#[test]
fn a_value_the_type_writes_validates_against_the_schema_it_declares() {
    alloc_and(|alloc| {
        let declared = Connection::schema(alloc).unwrap();
        let s = SchemaRef::new(&declared).unwrap();
        let value = sample().to_value(alloc).unwrap();

        assert_eq!(
            validate_map(s, &value),
            Ok(()),
            "what the type wrote is what its schema asked for"
        );
    });
}

/// And the check is not vacuous: a key the schema does not declare is
/// refused, which is what proves the test above is asserting something.
#[test]
fn a_key_the_schema_does_not_declare_is_refused() {
    alloc_and(|alloc| {
        let declared = Connection::schema(alloc).unwrap();
        let s = SchemaRef::new(&declared).unwrap();

        let mut value = sample().to_value(alloc).unwrap();
        TryAsMut::<Map>::try_as_mut(&mut value)
            .ok_or(ValueError::WrongKind)
            .and_then(|m| m.set("nonesuch", guatiao::Value::from(Text::new("x"))))
            .unwrap();
        assert!(validate_map(s, &value).is_err());
    });
}

/// A map whose keys are not all text is refused by name, not read as an
/// empty map that passes.
#[test]
fn a_map_with_a_key_that_is_not_text_is_refused() {
    alloc_and(|alloc| {
        let declared = Connection::schema(alloc).unwrap();
        let s = SchemaRef::new(&declared).unwrap();

        let value = sample().to_value(alloc).unwrap();
        let map = Map::try_from(value).expect("a map");
        let (ptr, len, cap, a) = map.into_raw_parts();
        let (bptr, blen, bcap, ba) = guatiao::Buffer::new_in(alloc, b"\xff")
            .unwrap()
            .into_raw_parts();
        // SAFETY: an entry is `repr(C)` with its key first; the old key is
        // freed before a foreign producer's bytes are written over it.
        let map = unsafe {
            let slot = ptr.cast::<Text>();
            std::ptr::drop_in_place(slot);
            std::ptr::write(slot, Text::from_raw_parts(bptr, blen, bcap, ba));
            Map::from_raw_parts(ptr, len, cap, a)
        };
        let value: guatiao::Value = map.into();
        let refused = validate_map(s, &value).expect_err("refused");
        assert!(
            refused.to_string().contains("keys that are text"),
            "{refused}"
        );
    });
}

/// A declared default lands in the document as the value itself.
///
/// `#[schema(default = <expr>)]` goes out through `ToValue`, so the
/// default is written in Rust and cannot drift from the type it defaults;
/// what a consumer reads is the ordinary `default` key JSON Schema
/// already has.
#[test]
fn a_declared_default_lands_in_the_document() {
    #[derive(Schema, ToValue)]
    struct Listener {
        #[schema(default = 5900)]
        port: u16,
    }

    alloc_and(|alloc| {
        let declared = Listener::schema(alloc).unwrap();
        let s = SchemaRef::new(&declared).unwrap();
        let port = s.find("port").unwrap();

        assert_eq!(
            TryAsRef::<Map>::try_as_ref(port.as_value())
                .and_then(|m| m.get(guatiao::schema::vocab::DEFAULT))
                .and_then(TryAsRef::<Number>::try_as_ref)
                .map(AsRef::<str>::as_ref),
            Some("5900"),
            "the document carries `default: 5900`, not a description of one"
        );
        let read: u16 = port.default().unwrap().try_into().unwrap();
        assert_eq!(read, 5900);
    });
}

/// A struct holding a map keyed by text describes an **open object**, and
/// the three derives agree about it: what it declares, what it writes,
/// and what it reads back.
///
/// The schema's one job is to describe the struct as it is, and a
/// `BTreeMap<String, T>` is a set of keys nobody declared.
#[test]
fn a_map_field_describes_an_open_object() {
    use std::collections::BTreeMap;

    #[derive(Debug, PartialEq, Schema, ToValue, guatiao::FromValue)]
    struct Agent {
        name: String,
        /// The environment it runs with.
        env: BTreeMap<String, String>,
    }

    alloc_and(|alloc| {
        let declared = Agent::schema(alloc).unwrap();
        let s = SchemaRef::new(&declared).unwrap();
        let env = s.find("env").expect("env is declared");
        assert!(matches!(env.kind(), Kind::MapOf(_)));
        assert!(matches!(env.kind().values(), Kind::Str));
        assert_eq!(
            env.description(),
            "The environment it runs with.",
            "a doc comment describes the field, whatever its kind"
        );

        let agent = Agent {
            name: "one".into(),
            env: BTreeMap::from([
                ("PATH".into(), "/bin".into()),
                ("HOME".into(), "/root".into()),
            ]),
        };
        let written = agent.to_value(alloc).expect("it writes");

        // What the type writes validates against what the same type
        // declares. That is the agreement the derives exist for.
        let map = TryAsRef::<Map>::try_as_ref(&written).expect("a map");
        guatiao::schema::validate_value(env, map.get("env").expect("env was written"))
            .expect("what it writes, it accepts");

        // And a BTreeMap writes its keys sorted, so the value is the same
        // every time.
        let env_value = map.get("env").expect("env");
        let keys: Vec<&str> = TryAsRef::<Map>::try_as_ref(env_value)
            .expect("a map")
            .keys()
            .collect();
        assert_eq!(keys, ["HOME", "PATH"]);

        let read = <Agent as guatiao::FromValue>::from_value(&written).expect("it reads back");
        assert_eq!(read, agent);
    });
}
