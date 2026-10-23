// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The derives' streaming half: `#[derive(ToValue)]` also writes `Walk`
//! and `#[derive(FromValue)]` also `FromWalk`. For every shape and field
//! attribute, the walk writes exactly the bytes the tree encodes to, the
//! read gives back the value, and a refusal is the tree reader's, error
//! for error. A field that only builds a tree still works, and one that
//! walks is never built as one.

#![cfg(feature = "derive")]

extern crate alloc;

use alloc::collections::BTreeMap;
use core::fmt;
use core::str::FromStr;

use guatiao::value::visit::{Build, Visitor, Walker};
use guatiao::value::wire::{WireError, decode_as, to_bytes, to_bytes_of};
use guatiao::{
    Alloc, FromValue, FromWalk, Kinds, List, Map, MapError, Number, ToValue, Value, ValueError,
    Walk,
};

/// What every derived type here must do: the same bytes as the tree, and
/// back to itself through bytes and through a tree.
fn round_trips<T>(value: &T)
where
    T: ToValue + FromValue + Walk + FromWalk + PartialEq + fmt::Debug,
{
    let tree = value.to_value().expect("the tree is built");
    let bytes = to_bytes(&tree).expect("the tree encodes");
    assert_eq!(to_bytes_of(value).unwrap(), bytes, "walked: {value:?}");
    assert_eq!(&decode_as::<T>(&bytes).unwrap(), value, "read from bytes");
    assert_eq!(&T::from_walk(&tree).unwrap(), value, "read from a tree");
}

/// A refusal through a walk is the tree reader's.
fn refuses_alike<T: FromValue + FromWalk + fmt::Debug>(value: Value) {
    let tree = T::from_value(&value).expect_err("the tree reader refuses it");
    assert_eq!(T::from_walk(&value).unwrap_err(), tree, "through a walk");
    assert_eq!(
        decode_as::<T>(&to_bytes(&value).unwrap()).unwrap_err(),
        WireError::Refused { error: tree },
        "from bytes"
    );
}

fn map(entries: &[(&str, Value)]) -> Value {
    let mut map = Map::new();
    for (key, value) in entries {
        map.set(key, value.clone());
    }
    map.into()
}

// --- a struct with every field attribute --------------------------------------

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

impl FromStr for Version {
    type Err = &'static str;

    fn from_str(text: &str) -> Result<Version, &'static str> {
        let (major, minor) = text.split_once('.').ok_or("a version is MAJOR.MINOR")?;
        Ok(Version {
            major: major.parse().map_err(|_| "a number before the dot")?,
            minor: minor.parse().map_err(|_| "a number after the dot")?,
        })
    }
}

/// A duration as whole seconds, through a `with` module.
mod secs {
    use core::time::Duration;
    use guatiao::{Alloc, FromValue, MapError, ToValue, Value, ValueError};

    pub fn to_value_in(d: &Duration, alloc: Alloc) -> Result<Value, ValueError> {
        d.as_secs().to_value_in(alloc)
    }

    pub fn from_value(v: &Value) -> Result<Duration, MapError> {
        u64::from_value(v).map(Duration::from_secs)
    }
}

/// A type with hand-written tree impls only, as a field of a derived one.
#[derive(Debug, PartialEq, Clone, Copy)]
struct Port(u16);

impl ToValue for Port {
    fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
        self.0.to_value_in(alloc)
    }
}

impl FromValue for Port {
    fn from_value(value: &Value) -> Result<Port, MapError> {
        u16::from_value(value).map(Port)
    }
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

#[derive(ToValue, FromValue, Debug, PartialEq, Clone)]
#[map(rename_all = "kebab-case")]
struct Everything {
    host_name: String,
    #[map(rename = "PORT")]
    port: Port,
    ports: Vec<Port>,
    motd: Option<String>,
    #[map(skip)]
    cache: u8,
    #[map(default)]
    retries: u32,
    #[map(default = 30)]
    timeout: u32,
    #[map(skip_serializing_if = is_zero, default)]
    weight: u32,
    #[map(as_text)]
    version: Version,
    #[map(as_text)]
    minimum: Option<Version>,
    #[map(with = secs)]
    idle: core::time::Duration,
    labels: BTreeMap<String, Option<f64>>,
    nested: Option<Box<Everything>>,
    any: Value,
}

fn everything(depth: u32) -> Everything {
    Everything {
        host_name: "h".into(),
        port: Port(5900),
        ports: vec![Port(1), Port(2)],
        motd: depth.is_multiple_of(2).then(|| "hi".into()),
        cache: 0,
        retries: depth,
        timeout: 30 + depth,
        weight: depth,
        version: Version { major: 2, minor: 7 },
        minimum: (!depth.is_multiple_of(2)).then_some(Version { major: 1, minor: 0 }),
        idle: core::time::Duration::from_secs(90),
        labels: [("a".to_string(), Some(1.5)), ("b".to_string(), None)]
            .into_iter()
            .collect(),
        nested: (depth > 0).then(|| Box::new(everything(depth - 1))),
        any: vec![Value::from(1), Value::from("x")]
            .into_iter()
            .collect::<List>()
            .into(),
    }
}

#[test]
fn every_field_attribute_streams_as_its_tree() {
    for depth in 0..4 {
        round_trips(&everything(depth));
    }
}

#[test]
fn an_absent_key_reads_as_the_tree_reader_reads_it() {
    let full = everything(0).to_value().unwrap();
    let mut entries: Map = TryFrom::try_from(full).unwrap();
    for key in ["retries", "timeout", "weight", "motd", "minimum", "nested"] {
        entries.remove(key);
    }
    let value: Value = entries.into();
    assert_eq!(
        Everything::from_walk(&value).unwrap(),
        Everything::from_value(&value).unwrap()
    );
    let read = Everything::from_walk(&value).unwrap();
    assert_eq!((read.retries, read.timeout, read.weight), (0, 30, 0));
}

#[test]
fn a_refusal_through_a_walk_is_the_tree_readers() {
    let full = everything(1).to_value().unwrap();
    let with = |key: &str, v: Value| {
        let mut entries: Map = TryFrom::try_from(full.clone()).unwrap();
        entries.set(key, v);
        Value::from(entries)
    };
    let without = |key: &str| {
        let mut entries: Map = TryFrom::try_from(full.clone()).unwrap();
        entries.remove(key);
        Value::from(entries)
    };
    refuses_alike::<Everything>(Value::from("not a map"));
    refuses_alike::<Everything>(without("host-name"));
    refuses_alike::<Everything>(without("PORT"));
    refuses_alike::<Everything>(with("PORT", Value::from(70_000)));
    refuses_alike::<Everything>(with(
        "ports",
        vec![Value::from(1), Value::from("x")]
            .into_iter()
            .collect::<List>()
            .into(),
    ));
    refuses_alike::<Everything>(with("version", Value::from("2")));
    refuses_alike::<Everything>(with("version", Value::from(2)));
    refuses_alike::<Everything>(with("minimum", Value::from("x.1")));
    refuses_alike::<Everything>(with("idle", Value::from("soon")));
    refuses_alike::<Everything>(with("labels", map(&[("a", Value::from(true))])));
    refuses_alike::<Everything>(with("nested", map(&[("host-name", Value::from(1))])));
}

#[test]
fn a_stored_null_reads_as_none() {
    let full = everything(0).to_value().unwrap();
    let mut entries: Map = TryFrom::try_from(full).unwrap();
    entries.set("motd", Value::null());
    entries.set("minimum", Value::null());
    let value: Value = entries.into();
    let read = Everything::from_walk(&value).unwrap();
    assert_eq!(read, Everything::from_value(&value).unwrap());
    assert_eq!((read.motd, read.minimum), (None, None));
}

// --- strictness, and the shapes that are not a map ---------------------------------

#[derive(ToValue, FromValue, Debug, PartialEq)]
#[map(deny_unknown_fields)]
struct Strict {
    a: u8,
    b: Option<u8>,
}

#[test]
fn an_unknown_key_is_refused_alike() {
    round_trips(&Strict { a: 1, b: Some(2) });
    round_trips(&Strict { a: 1, b: None });
    refuses_alike::<Strict>(map(&[("a", Value::from(1)), ("c", Value::from(2))]));
}

#[derive(ToValue, FromValue, Debug, PartialEq)]
struct Pair(String, Option<Port>);

#[derive(ToValue, FromValue, Debug, PartialEq)]
struct Meters(f64);

#[derive(ToValue, FromValue, Debug, PartialEq, Default)]
struct Nothing;

#[derive(ToValue, FromValue, Debug, PartialEq)]
#[map(transparent)]
struct Wrapped {
    inner: Vec<u8>,
    #[map(skip)]
    seen: bool,
}

#[derive(ToValue, FromValue, Debug, PartialEq)]
struct Labelled<T> {
    label: String,
    value: T,
    values: Vec<T>,
}

#[test]
fn the_other_shapes_stream_as_their_trees() {
    round_trips(&Pair("x".into(), Some(Port(1))));
    round_trips(&Pair("x".into(), None));
    round_trips(&Meters(1.25));
    round_trips(&Nothing);
    round_trips(&Wrapped {
        inner: vec![1, 2],
        seen: false,
    });
    round_trips(&Labelled {
        label: "l".into(),
        value: 1u8,
        values: vec![2, 3],
    });
    round_trips(&Labelled {
        label: "l".into(),
        value: Meters(2.0),
        values: vec![],
    });

    refuses_alike::<Pair>(vec![Value::from("x")].into_iter().collect::<List>().into());
    refuses_alike::<Pair>(
        vec![Value::from("x"), Value::null(), Value::null()]
            .into_iter()
            .collect::<List>()
            .into(),
    );
    refuses_alike::<Pair>(
        vec![Value::from(1), Value::null()]
            .into_iter()
            .collect::<List>()
            .into(),
    );
    refuses_alike::<Meters>(Value::from("far"));
    refuses_alike::<Nothing>(Value::from(0));
    refuses_alike::<Wrapped>(Value::from(true));
    refuses_alike::<Labelled<u8>>(map(&[("label", Value::from("l"))]));
}

// --- what goes through a tree: enums and a flattened member --------------------------

#[derive(ToValue, FromValue, Debug, PartialEq)]
enum Level {
    Off,
    #[map(rename = "warn")]
    Warning,
}

#[derive(ToValue, FromValue, Debug, PartialEq)]
#[map(tag = "kind")]
enum Transport {
    Tcp { port: u16 },
    Unix { path: String },
}

#[derive(ToValue, FromValue, Debug, PartialEq)]
#[map(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

#[derive(ToValue, FromValue, Debug, PartialEq)]
struct Common {
    id: u32,
}

#[derive(ToValue, FromValue, Debug, PartialEq)]
struct Flattened {
    #[map(flatten)]
    common: Common,
    level: Level,
    transport: Transport,
    names: OneOrMany,
}

#[test]
fn enums_and_flattened_members_stream_through_their_trees() {
    round_trips(&Level::Warning);
    round_trips(&Transport::Unix { path: "/s".into() });
    round_trips(&OneOrMany::Many(vec!["a".into()]));
    let value = Flattened {
        common: Common { id: 7 },
        level: Level::Off,
        transport: Transport::Tcp { port: 1 },
        names: OneOrMany::One("n".into()),
    };
    round_trips(&value);
    refuses_alike::<Level>(Value::from("loud"));
    refuses_alike::<Transport>(map(&[("kind", Value::from("Udp"))]));
    refuses_alike::<OneOrMany>(Value::from(1));
    refuses_alike::<Flattened>(map(&[("id", Value::from(7))]));
}

// --- a field that walks is never built as a tree ------------------------------------

/// Walks and reads itself; its tree impls must never run.
#[derive(Debug, PartialEq)]
struct OnlyStreams(u32);

impl ToValue for OnlyStreams {
    fn to_value_in(&self, _: Alloc) -> Result<Value, ValueError> {
        panic!("a derived walk built a field's tree")
    }
}

impl FromValue for OnlyStreams {
    fn from_value(_: &Value) -> Result<OnlyStreams, MapError> {
        panic!("a derived read read a field's tree")
    }
}

impl Walk for OnlyStreams {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        visitor.u64(u64::from(self.0))
    }
}

struct OnlyStreamsBuild;

impl Build for OnlyStreamsBuild {
    type Out = OnlyStreams;

    fn kinds(&self) -> Kinds {
        Kinds::NUMBER
    }

    fn number(self, text: &str) -> Result<OnlyStreams, MapError> {
        text.parse()
            .map(OnlyStreams)
            .map_err(|_| MapError::bad_value("a u32"))
    }
}

impl FromWalk for OnlyStreams {
    fn from_walk<W: Walker>(walker: W) -> Result<OnlyStreams, MapError> {
        walker.build(OnlyStreamsBuild)
    }
}

#[derive(ToValue, FromValue, Debug, PartialEq)]
struct Holder {
    one: OnlyStreams,
    some: Option<OnlyStreams>,
    many: Vec<OnlyStreams>,
    by_name: BTreeMap<String, OnlyStreams>,
    pair: Pair2,
}

#[derive(ToValue, FromValue, Debug, PartialEq)]
struct Pair2(OnlyStreams, Option<OnlyStreams>);

#[test]
fn a_derived_type_goes_to_bytes_and_back_with_no_tree() {
    let holder = Holder {
        one: OnlyStreams(1),
        some: Some(OnlyStreams(2)),
        many: vec![OnlyStreams(3), OnlyStreams(4)],
        by_name: [("x".to_string(), OnlyStreams(5))].into_iter().collect(),
        pair: Pair2(OnlyStreams(6), None),
    };
    let bytes = to_bytes_of(&holder).unwrap();
    assert_eq!(decode_as::<Holder>(&bytes).unwrap(), holder);

    // The bytes are the encoding of the tree a person would write.
    let mut pair = List::new();
    pair.push(Value::from(Number::new("6").unwrap()));
    pair.push(Value::null());
    let expected = map(&[
        ("one", Value::from(1)),
        ("some", Value::from(2)),
        (
            "many",
            vec![Value::from(3), Value::from(4)]
                .into_iter()
                .collect::<List>()
                .into(),
        ),
        ("by_name", map(&[("x", Value::from(5))])),
        ("pair", pair.into()),
    ]);
    assert_eq!(bytes, to_bytes(&expected).unwrap());
}
