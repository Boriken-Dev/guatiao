// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `#[map(as_text)]`: a field whose type has only `Display` and `FromStr`
//! crosses as its text, is described as a string (with the `format` or
//! `pattern` `#[schema]` adds), and a refusal carries the parse error's
//! words under the field's key.

#![cfg(feature = "derive")]

use core::fmt;
use core::str::FromStr;

use guatiao::schema::read::{Kind, SchemaRef};
use guatiao::schema::validate::validate_map;
use guatiao::{FromValue, Map, MapError, Schema, ToValue, Value};

/// `MAJOR.MINOR`, and nothing else: no value traits of its own.
#[derive(Debug, PartialEq, Clone)]
struct Version {
    major: u32,
    minor: u32,
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

#[derive(Debug)]
struct NotAVersion;

impl fmt::Display for NotAVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a version is MAJOR.MINOR")
    }
}

impl FromStr for Version {
    type Err = NotAVersion;

    fn from_str(text: &str) -> Result<Version, NotAVersion> {
        let (major, minor) = text.split_once('.').ok_or(NotAVersion)?;
        Ok(Version {
            major: major.parse().map_err(|_| NotAVersion)?,
            minor: minor.parse().map_err(|_| NotAVersion)?,
        })
    }
}

#[derive(ToValue, FromValue, Schema, Debug, PartialEq)]
struct Package {
    name: String,
    #[map(as_text)]
    #[schema(pattern = "^[0-9]+\\.[0-9]+$")]
    version: Version,
    #[map(as_text)]
    minimum: Option<Version>,
    #[map(as_text, default = Version { major: 1, minor: 0 })]
    floor: Version,
}

#[derive(ToValue, FromValue, Schema, Debug, PartialEq)]
#[map(tag = "source")]
enum Source {
    Registry {
        #[map(as_text)]
        at: Version,
    },
    Path {
        dir: String,
    },
}

fn package() -> Package {
    Package {
        name: "core".into(),
        version: Version { major: 2, minor: 7 },
        minimum: Some(Version { major: 2, minor: 0 }),
        floor: Version { major: 1, minor: 4 },
    }
}

fn written(p: &Package) -> Map {
    <&Map>::try_from(&p.to_value().unwrap()).unwrap().clone()
}

#[test]
fn a_field_crosses_as_its_display_text_and_reads_back() {
    let map = written(&package());
    assert_eq!(
        <&str>::try_from(map.get("version").unwrap()).unwrap(),
        "2.7"
    );
    assert_eq!(
        <&str>::try_from(map.get("minimum").unwrap()).unwrap(),
        "2.0"
    );
    let schema = Package::schema().unwrap();
    validate_map(SchemaRef::new(&schema).unwrap(), &map.clone().into()).expect("accepted");
    assert_eq!(Package::from_value(&map.into()).unwrap(), package());
}

#[test]
fn an_option_omits_none_and_reads_null_as_none() {
    let mut p = package();
    p.minimum = None;
    let mut map = written(&p);
    assert!(map.get("minimum").is_none());
    map.set("minimum", Value::null());
    assert_eq!(Package::from_value(&map.into()).unwrap(), p);
}

#[test]
fn an_absent_default_reads_as_it_and_the_schema_carries_its_text() {
    let mut map = written(&package());
    map.remove("floor");
    let read = Package::from_value(&map.into()).unwrap();
    assert_eq!(read.floor, Version { major: 1, minor: 0 });
    let schema = Package::schema().unwrap();
    let s = SchemaRef::new(&schema).unwrap();
    let floor = s.find("floor").unwrap();
    assert!(!floor.is_required());
    let default = <&Map>::try_from(floor.as_value())
        .unwrap()
        .get("default")
        .unwrap();
    assert_eq!(<&str>::try_from(default).unwrap(), "1.0");
}

#[test]
fn the_schema_is_a_string_with_what_schema_adds() {
    let schema = Package::schema().unwrap();
    let s = SchemaRef::new(&schema).unwrap();
    assert!(matches!(
        s.find("version").unwrap().kind(),
        Kind::Str {
            pattern: Some("^[0-9]+\\.[0-9]+$"),
            ..
        }
    ));
    assert!(matches!(
        s.find("minimum").unwrap().kind(),
        Kind::Str { .. }
    ));
    assert!(s.find("minimum").unwrap().is_nullable());
}

#[test]
fn a_refusal_carries_the_parse_error_and_the_key() {
    let mut map = written(&package());
    map.set("version", "two");
    match Package::from_value(&map.into()) {
        Err(MapError::BadValue { key, expected }) => {
            assert_eq!(key, "version");
            assert!(expected.contains("a version is MAJOR.MINOR"), "{expected}");
        }
        other => panic!("{other:?}"),
    }
    let mut map = written(&package());
    map.set("version", 3u8);
    assert!(matches!(
        Package::from_value(&map.into()),
        Err(MapError::WrongType { .. })
    ));
}

#[test]
fn a_variant_field_crosses_as_text_too() {
    let source = Source::Registry {
        at: Version { major: 0, minor: 9 },
    };
    let value = source.to_value().unwrap();
    let map = <&Map>::try_from(&value).unwrap();
    assert_eq!(<&str>::try_from(map.get("at").unwrap()).unwrap(), "0.9");
    assert_eq!(Source::from_value(&value).unwrap(), source);
    let dir = Source::Path { dir: "/src".into() };
    assert_eq!(Source::from_value(&dir.to_value().unwrap()).unwrap(), dir);
}
