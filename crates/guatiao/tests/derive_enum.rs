// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The derives on an enum.
//!
//! Two shapes, and the test that matters most for each is the one where
//! two derives read one declaration and must agree: a value the enum
//! WRITES is one the schema the same enum DECLARES accepts. If those ever
//! disagree, a provider rejects the exact configuration its own schema
//! asked for.

#![cfg(feature = "derive")]

use guatiao::List;
use guatiao::value::convert::{TryAsMut, TryAsRef};

use guatiao::schema::read::{FieldRef, Kind, SchemaRef};
use guatiao::schema::validate::{validate_map, validate_value};
use guatiao::value::alloc::Alloc;
use guatiao::value::read::str_or;
use guatiao::{FromValue, Map, MapError, Schema, Text, ToValue, Value, ValueError};

/// A choice: every variant is a unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ToValue, FromValue, Schema)]
enum Level {
    /// Nothing at all.
    Off,
    #[map(rename = "warn")]
    #[schema(label = "Warnings only")]
    Warning,
    On,
}

/// A variant: the tag names which one, and each carries its own fields.
#[derive(Debug, Clone, PartialEq, Eq, ToValue, FromValue, Schema)]
#[map(tag = "auth")]
enum Auth {
    /// The ambient credential.
    Ambient,
    #[map(rename = "userpass")]
    #[schema(title = "Username and password")]
    UserPass {
        username: String,
        #[schema(sensitive)]
        password: Option<String>,
        #[map(skip)]
        attempts: u32,
    },
}

/// Both, held by a struct, which is where an enum usually lives.
#[derive(Debug, Clone, PartialEq, Eq, ToValue, FromValue, Schema)]
struct Config {
    level: Level,
    auth: Auth,
    fallback: Option<Level>,
}

fn alloc() -> Alloc {
    Alloc::rust()
}

/// A kind as the field a caller would validate against.
fn field_of<T: Schema>(key: &'static str) -> (Value, &'static str) {
    (T::kind(alloc()).finish().expect("a kind builds"), key)
}

// --- a choice ---------------------------------------------------------

#[test]
fn a_unit_enum_is_stored_as_its_name() {
    for (level, stored) in [
        (Level::Off, "Off"),
        (Level::Warning, "warn"),
        (Level::On, "On"),
    ] {
        let value = level.to_value(alloc()).unwrap();
        assert_eq!(
            TryAsRef::<str>::try_as_ref(&value),
            Some(stored),
            "a rename changes the spelling"
        );
        assert_eq!(Level::from_value(&value).unwrap(), level, "and reads back");
    }
}

#[test]
fn a_spelling_no_variant_declares_names_the_alternatives_but_not_itself() {
    let error = Level::from_value(&Value::from(Text::new("hunter2"))).unwrap_err();
    assert!(
        matches!(&error, MapError::BadValue { expected, .. } if expected == "one of Off, warn, On"),
        "the right kind with the wrong value is BadValue: {error:?}"
    );
    assert!(
        !error.to_string().contains("hunter2"),
        "an error never quotes what it refused -- the value may be a secret: {error}"
    );

    // The variant's own Rust name is not a spelling once it is renamed.
    assert!(Level::from_value(&Value::from(Text::new("Warning"))).is_err());
}

#[test]
fn a_choice_that_is_not_text_is_the_wrong_kind() {
    let error = Level::from_value(&Value::from(1i64)).unwrap_err();
    assert!(
        matches!(error, MapError::WrongType { .. }),
        "a number where a name belongs is a disagreement about kind, not value: {error:?}"
    );
}

#[test]
fn a_unit_enum_describes_itself_as_a_choice() {
    let (kind, _) = field_of::<Level>("level");
    let field = FieldRef::new("level", &kind).unwrap();
    assert!(matches!(field.kind(), Kind::Enum(_)));

    let choices: Vec<(String, String)> = field
        .kind()
        .choices()
        .map(|c| (c.value().to_string(), c.label().to_string()))
        .collect();
    assert_eq!(
        choices,
        [
            // A choice has only a label, so its doc comment is the label.
            ("Off".to_string(), "Nothing at all.".to_string()),
            // An explicit label wins over the rename.
            ("warn".to_string(), "Warnings only".to_string()),
            // No label at all reads as the value.
            ("On".to_string(), "On".to_string()),
        ]
    );

    // And the unlabelled one wrote nothing, rather than `"On": ""`.
    let labels = TryAsRef::<Map>::try_as_ref(&kind)
        .and_then(|m| m.get("x-enum-labels"))
        .expect("two choices have labels");
    assert!(
        TryAsRef::<Map>::try_as_ref(labels)
            .and_then(|m| m.get("On"))
            .is_none(),
        "a label saying nothing is left off"
    );
}

#[test]
fn a_choice_it_writes_is_one_its_schema_accepts() {
    let (kind, key) = field_of::<Level>("level");
    let field = FieldRef::new(key, &kind).unwrap();
    for level in [Level::Off, Level::Warning, Level::On] {
        let value = level.to_value(alloc()).unwrap();
        validate_value(field, &value)
            .unwrap_or_else(|e| panic!("{level:?} wrote a value its own schema refuses: {e}"));
    }
    assert!(
        validate_value(field, &Value::from(Text::new("Warning"))).is_err(),
        "and the schema refuses what the reader refuses"
    );
}

// --- a variant ----------------------------------------------------------

fn userpass() -> Auth {
    Auth::UserPass {
        username: "ana".to_string(),
        password: Some("hunter2".to_string()),
        attempts: 0,
    }
}

#[test]
fn a_tagged_enum_is_a_map_with_its_name_under_the_tag() {
    let value = userpass().to_value(alloc()).unwrap();
    let keys: Vec<&str> = TryAsRef::<Map>::try_as_ref(&value)
        .map(Map::entries)
        .unwrap()
        .iter()
        .map(guatiao::Entry::key)
        .collect();
    assert_eq!(
        keys,
        ["auth", "username", "password"],
        "the tag first, then the fields -- and a skipped field not at all"
    );
    assert_eq!(
        str_or(
            TryAsRef::<Map>::try_as_ref(&value).and_then(|m| m.get("auth")),
            ""
        ),
        "userpass"
    );

    let ambient = Auth::Ambient.to_value(alloc()).unwrap();
    assert_eq!(
        TryAsRef::<Map>::try_as_ref(&ambient)
            .map(Map::entries)
            .unwrap()
            .len(),
        1,
        "a unit arm is its tag and nothing else"
    );
}

#[test]
fn a_tagged_enum_round_trips() {
    let absent_password = Auth::UserPass {
        username: "ana".to_string(),
        password: None,
        attempts: 0,
    };
    for auth in [Auth::Ambient, userpass(), absent_password.clone()] {
        let value = auth.to_value(alloc()).unwrap();
        assert_eq!(Auth::from_value(&value).unwrap(), auth);
    }
    assert!(
        TryAsRef::<Map>::try_as_ref(&absent_password.to_value(alloc()).unwrap())
            .and_then(|m| m.get("password"))
            .is_none(),
        "None omits the key, exactly as it does in a struct"
    );
}

#[test]
fn a_skipped_field_reads_back_as_its_default() {
    let written = Auth::UserPass {
        username: "ana".to_string(),
        password: None,
        attempts: 7,
    };
    let read = Auth::from_value(&written.to_value(alloc()).unwrap()).unwrap();
    assert!(
        matches!(read, Auth::UserPass { attempts: 0, .. }),
        "never stored, so `Default::default()` on the way back: {read:?}"
    );
}

#[test]
fn a_missing_wrong_or_unknown_tag_is_named_under_the_tag() {
    let mut no_tag = Map::new();
    no_tag.set("username", "ana").unwrap();
    assert_eq!(
        Auth::from_value(&Value::from(no_tag)).unwrap_err(),
        MapError::missing("auth"),
        "no tag says which key is missing"
    );

    let mut numeric = Map::new();
    numeric.set("auth", 3).unwrap();
    let error = Auth::from_value(&Value::from(numeric)).unwrap_err();
    assert!(
        matches!(&error, MapError::WrongType { key, .. } if key == "auth"),
        "{error:?}"
    );

    let mut unknown = Map::new();
    unknown.set("auth", "kerberos").unwrap();
    let error = Auth::from_value(&Value::from(unknown)).unwrap_err();
    assert!(
        matches!(&error, MapError::BadValue { key, expected }
            if key == "auth" && expected == "one of Ambient, userpass"),
        "{error:?}"
    );

    let error = Auth::from_value(&Value::from(Text::new("userpass"))).unwrap_err();
    assert!(
        matches!(error, MapError::WrongType { .. }),
        "a tagged enum is a map, so bare text is the wrong kind: {error:?}"
    );
}

#[test]
fn a_tagged_enum_describes_itself_as_a_variant() {
    let (kind, key) = field_of::<Auth>("auth");
    let field = FieldRef::new(key, &kind).unwrap();
    assert!(matches!(field.kind(), Kind::Variant { tag: "auth", .. }));

    let arms: Vec<_> = field.kind().arms().collect();
    assert_eq!(arms.len(), 2);

    assert_eq!(arms[0].value(), "Ambient");
    assert_eq!(
        arms[0].description(),
        "The ambient credential.",
        "an arm has a description as well as a title, so its doc comment is the \
         description"
    );
    assert_eq!(
        arms[0].title(),
        "Ambient",
        "and with no title it shows its value"
    );

    assert_eq!(arms[1].value(), "userpass");
    assert_eq!(arms[1].title(), "Username and password");
    let fields: Vec<(String, bool, bool)> = arms[1]
        .fields()
        .map(|f| (f.key().to_string(), f.is_required(), f.is_sensitive()))
        .collect();
    assert_eq!(
        fields,
        [
            ("username".to_string(), true, false),
            // Optional because `Option<T>`, sensitive because it said so --
            // the same rules a struct field follows.
            ("password".to_string(), false, true),
        ],
        "and never the skipped one"
    );
}

/// **The invariant the whole tagged shape rests on.**
///
/// `schema::validate` fixed what a variant's value looks like before this
/// derive existed: a map carrying the tag plus the chosen arm's fields.
/// The derive has to produce exactly that, or `validate_value` rejects
/// what the enum wrote.
#[test]
fn a_variant_it_writes_is_one_its_schema_accepts() {
    let (kind, key) = field_of::<Auth>("auth");
    let field = FieldRef::new(key, &kind).unwrap();
    for auth in [Auth::Ambient, userpass()] {
        let value = auth.to_value(alloc()).unwrap();
        validate_value(field, &value)
            .unwrap_or_else(|e| panic!("{auth:?} wrote a value its own schema refuses: {e}"));
    }

    let mut foreign = userpass().to_value(alloc()).unwrap();
    TryAsMut::<Map>::try_as_mut(&mut foreign)
        .ok_or(ValueError::WrongKind)
        .and_then(|m| m.set("realm", "EXAMPLE"))
        .unwrap();
    assert!(
        validate_value(field, &foreign).is_err(),
        "a key the chosen arm does not declare is the validator's to refuse"
    );
}

// --- held by a struct -----------------------------------------------------

#[test]
fn a_struct_holding_both_validates_against_its_own_schema() {
    let config = Config {
        level: Level::Warning,
        auth: userpass(),
        fallback: Some(Level::Off),
    };
    let value = config.to_value(alloc()).unwrap();
    let schema = Config::schema(alloc()).unwrap();
    let s = SchemaRef::new(&schema).unwrap();

    validate_map(s, &value).unwrap_or_else(|e| panic!("its own schema refuses it: {e}"));
    assert_eq!(Config::from_value(&value).unwrap(), config);

    assert!(matches!(s.find("level").unwrap().kind(), Kind::Enum(_)));
    assert!(matches!(
        s.find("auth").unwrap().kind(),
        Kind::Variant { tag: "auth", .. }
    ));
    assert!(s.find("level").unwrap().is_required());
    assert!(!s.find("fallback").unwrap().is_required());
}

/// A type whose kind is not an object still describes itself.
///
/// `T::schema()` is what `export_schema!` hands to C, and it carried only
/// `properties` and `required` — so a tagged enum arrived as an object
/// with no fields, its `x-variant-tag` and its arms dropped. Every key the
/// kind finished with is carried now, which is what makes the document
/// readable back.
#[test]
fn a_tagged_enums_own_schema_carries_its_tag_and_its_arms() {
    let declared = Auth::schema(alloc()).unwrap();

    assert_eq!(
        str_or(
            TryAsRef::<Map>::try_as_ref(&declared).and_then(|m| m.get("x-variant-tag")),
            ""
        ),
        "auth",
        "the discriminant's key survives"
    );
    let arms = TryAsRef::<Map>::try_as_ref(&declared)
        .and_then(|m| m.get("oneOf"))
        .expect("the arms survive");
    assert_eq!(
        TryAsRef::<List>::try_as_ref(arms)
            .map(|list| &list[..])
            .unwrap_or(&[])
            .len(),
        2
    );

    // And it reads back as the kind it was built from, through the same
    // reader a consumer uses.
    let field = FieldRef::new("auth", &declared).expect("a schema is a map");
    assert!(matches!(field.kind(), Kind::Variant { tag: "auth", .. }));
    let names: Vec<&str> = field.kind().arms().map(|a| a.value()).collect();
    assert_eq!(names, ["Ambient", "userpass"]);

    // A unit enum is the other non-object kind, and keeps its choices.
    let declared = Level::schema(alloc()).unwrap();
    let field = FieldRef::new("level", &declared).expect("a schema is a map");
    assert!(matches!(field.kind(), Kind::Enum(_)));
    let choices: Vec<&str> = field.kind().choices().map(|c| c.value()).collect();
    assert_eq!(choices, ["Off", "warn", "On"]);

    // A struct still answers what it always did.
    let declared = Config::schema(alloc()).unwrap();
    let s = SchemaRef::new(&declared).expect("a schema is a map");
    let keys: Vec<&str> = s.fields().map(|f| f.key()).collect();
    assert_eq!(keys, ["level", "auth", "fallback"]);
}
