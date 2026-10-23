// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! serde and the visitor with no tree: a document reads into a derived
//! type and a derived type writes a document straight through the
//! bridge, with the same text, value and refusal as the tree path; a
//! serde type walks and reads through its own impls; a document's
//! events reach any sink.

#![cfg(feature = "json")]

extern crate alloc;

use alloc::collections::BTreeMap;

use guatiao::value::visit::{Build, Flat, FromWalk, TreeSink, Visitor, Walk, Walker};
use guatiao::value::wire::{Encoder, decode_as, to_bytes, to_bytes_of};
use guatiao::{Alloc, FromValue, Kinds, MapError, ToValue, Value, ValueError};
use guatiao_serde::text::{Error, json};
use guatiao_serde::{
    BridgeError, Presentation, Serde, Walked, deserialize_into, from_deserializer,
};

#[derive(ToValue, FromValue, Debug, PartialEq, Clone)]
struct Server {
    host: String,
    port: u16,
    weight: f64,
    tags: Vec<String>,
}

#[derive(ToValue, FromValue, Debug, PartialEq, Clone)]
#[map(rename_all = "kebab-case")]
struct Config {
    name: String,
    enabled: bool,
    ratio: f64,
    servers: Vec<Server>,
    by_zone: BTreeMap<String, u32>,
    note: Option<String>,
    #[map(default = 30)]
    timeout_secs: u32,
    extra: Serde<serde_json::Value>,
}

fn config() -> Config {
    Config {
        name: "front door".into(),
        enabled: true,
        ratio: 0.75,
        servers: (0..3u16)
            .map(|i| Server {
                host: format!("10.0.0.{i}"),
                port: 5900 + i,
                weight: 1.5 + f64::from(i),
                tags: vec!["a".into()],
            })
            .collect(),
        by_zone: [("east".to_string(), 2), ("west".to_string(), 1)]
            .into_iter()
            .collect(),
        note: None,
        timeout_secs: 45,
        extra: Serde::new(serde_json::json!({"k": [1, 2.5, "x", null, true]})),
    }
}

fn how() -> Presentation {
    Presentation::new()
}

#[test]
fn a_derived_type_writes_the_text_its_tree_writes() {
    let c = config();
    let tree = c.to_value().unwrap();
    assert_eq!(
        json::to_string_of(&c, how()).unwrap(),
        json::to_string(&tree, how()).unwrap()
    );
    assert_eq!(
        json::to_string_pretty_of(&c, how()).unwrap(),
        json::to_string_pretty(&tree, how()).unwrap()
    );
}

#[test]
fn a_document_reads_into_a_derived_type_as_the_tree_reads_it() {
    let text = json::to_string_of(&config(), how()).unwrap();
    let streamed: Config = json::from_str_as(&text, how()).unwrap();
    let tree = json::from_str(&text, Alloc::rust(), how()).unwrap();
    assert_eq!(streamed, Config::from_value(&tree).unwrap());
    assert_eq!(streamed, config());
}

#[test]
fn a_refusal_is_the_tree_readers_and_quotes_nothing() {
    for doc in [
        r#"{"name": 5}"#,
        r#"{"name": "n", "enabled": true, "ratio": 1, "servers": [{"host": "h", "port": 70000}]}"#,
        r#"{"name": "n", "enabled": true, "ratio": 1, "servers": [], "by-zone": {"z": -1}}"#,
        r#"[]"#,
    ] {
        let tree = json::from_str(doc, Alloc::rust(), how()).unwrap();
        let expected = Config::from_value(&tree).unwrap_err();
        match json::from_str_as::<Config>(doc, how()) {
            Err(Error::Refused { format, error }) => {
                assert_eq!(format, "json");
                assert_eq!(error, expected, "{doc}");
            }
            other => panic!("{doc}: {other:?}"),
        }
    }
    // A document that does not parse is the format's refusal, with where.
    match json::from_str_as::<Config>(r#"{"name": "#, how()) {
        Err(Error::Syntax { position, .. }) => assert!(position.is_some()),
        other => panic!("{other:?}"),
    }
    // Something after the value is refused, as `from_str` refuses it.
    assert!(json::from_str_as::<Vec<u8>>("[1] [2]", how()).is_err());
}

/// The same documents through TOML and YAML.
#[test]
#[cfg(all(feature = "toml", feature = "yaml"))]
fn toml_and_yaml_carry_a_derived_type() {
    use guatiao_serde::text::{toml, yaml};

    let mut c = config();
    // TOML has no null; the extra map holds none of it there.
    c.extra = Serde::new(serde_json::json!({"k": [1, 2.5, "x", true]}));
    let tree = c.to_value().unwrap();

    let t = toml::to_string_of(&c, how()).unwrap();
    assert_eq!(t, toml::to_string(&tree, how()).unwrap());
    assert_eq!(toml::from_str_as::<Config>(&t, how()).unwrap(), c);

    let y = yaml::to_string_of(&c, how()).unwrap();
    assert_eq!(y, yaml::to_string(&tree, how()).unwrap());
    assert_eq!(yaml::from_str_as::<Config>(&y, how()).unwrap(), c);
}

// --- with no tree at all --------------------------------------------------------

/// Reads and writes itself; its tree impls must never run.
#[derive(Debug, PartialEq)]
struct OnlyStreams(u32);

impl ToValue for OnlyStreams {
    fn to_value_in(&self, _: Alloc) -> Result<Value, ValueError> {
        panic!("a field's tree was built")
    }
}

impl FromValue for OnlyStreams {
    fn from_value(_: &Value) -> Result<OnlyStreams, MapError> {
        panic!("a field's tree was read")
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

    fn u64(self, v: u64) -> Result<OnlyStreams, MapError> {
        u32::try_from(v)
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
    many: Vec<OnlyStreams>,
    some: Option<OnlyStreams>,
}

#[test]
fn a_document_reads_into_a_derived_type_with_no_tree() {
    let holder: Holder =
        json::from_str_as(r#"{"one": 1, "many": [2, 3], "some": 4}"#, how()).unwrap();
    assert_eq!(
        holder,
        Holder {
            one: OnlyStreams(1),
            many: vec![OnlyStreams(2), OnlyStreams(3)],
            some: Some(OnlyStreams(4)),
        }
    );
    assert_eq!(
        json::to_string_of(&holder, how()).unwrap(),
        r#"{"one":1,"many":[2,3],"some":4}"#
    );
}

// --- serde types, both ways ------------------------------------------------------

#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq, Clone)]
enum Mode {
    Fast,
    Weighted(f64),
    Pair(u8, String),
    Shaped { side: u32, label: Option<String> },
}

#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq, Clone)]
struct Limits {
    max: u32,
    modes: Vec<Mode>,
    nested: BTreeMap<String, Vec<Option<i64>>>,
    when: Option<u128>,
}

fn limits() -> Limits {
    Limits {
        max: 7,
        modes: vec![
            Mode::Fast,
            Mode::Weighted(1.0),
            Mode::Pair(2, "b".into()),
            Mode::Shaped {
                side: 3,
                label: None,
            },
        ],
        nested: [("a".to_string(), vec![Some(-1), None])]
            .into_iter()
            .collect(),
        when: Some(u128::MAX),
    }
}

#[test]
fn a_serde_type_walks_as_its_tree_and_reads_back() {
    let wrapped: Serde<Limits> = Serde::new(limits());
    let tree = wrapped.to_value().unwrap();

    let mut sink = TreeSink::new();
    wrapped.walk(Flat(&mut sink)).unwrap();
    assert_eq!(sink.into_value().unwrap(), tree);

    let bytes = to_bytes(&tree).unwrap();
    assert_eq!(to_bytes_of(&wrapped).unwrap(), bytes);
    assert_eq!(decode_as::<Serde<Limits>>(&bytes).unwrap(), wrapped);
    assert_eq!(Serde::<Limits>::from_walk(&tree).unwrap(), wrapped);
}

#[test]
fn a_serde_readers_refusal_names_the_key() {
    let mut tree = Serde::<Limits>::new(limits()).to_value().unwrap();
    let map: &mut guatiao::Map = guatiao::TryAsMut::try_as_mut(&mut tree).unwrap();
    map.set("max", "many");
    let walked = Serde::<Limits>::from_walk(&tree).unwrap_err();
    let tree_read = Serde::<Limits>::from_value(&tree).unwrap_err();
    assert_eq!(walked, tree_read);
    assert_eq!(walked.key(), "max");
}

/// Any guatiao walk, written by a format this crate never names.
#[test]
fn any_walk_is_written_by_any_serde_format() {
    let c = config();
    let packed = rmp_serde::to_vec(&Walked::new(&c, how())).unwrap();
    let mut de = rmp_serde::Deserializer::new(&packed[..]);
    let back: Config = from_deserializer(&mut de, how()).unwrap();
    assert_eq!(back, c);
}

/// A document's events reach any sink: here the wire encoder, writing
/// the bytes of the tree the document is.
#[test]
fn a_document_feeds_a_sink_with_no_tree() {
    let text = json::to_string_of(&config(), how()).unwrap();
    let mut bytes = Vec::new();
    let mut encoder = Encoder::new(&mut bytes);
    let mut de = serde_json::Deserializer::from_str(&text);
    deserialize_into(&mut de, &mut encoder, how()).unwrap();
    let tree = json::from_str(&text, Alloc::rust(), how()).unwrap();
    assert_eq!(bytes, to_bytes(&tree).unwrap());

    // The sink's refusal comes back as the sink's.
    let mut sink = TreeSink::new();
    guatiao::value::visit::Sink::null(&mut sink).unwrap();
    let mut de = serde_json::Deserializer::from_str("1");
    match deserialize_into(&mut de, &mut sink, how()) {
        Err(BridgeError::Refused(ValueError::WrongKind)) => {}
        other => panic!("{other:?}"),
    }
}

/// A document nested past `MAX_DEPTH` is refused by a typed read too,
/// through a format with no limit of its own.
#[test]
fn a_typed_read_follows_no_deeper_than_max_depth() {
    fn packed(depth: usize) -> Vec<u8> {
        let mut bytes = vec![0x91u8; depth];
        bytes.push(0xc0);
        bytes
    }
    let deep = packed(300);
    let mut de = rmp_serde::Deserializer::new(&deep[..]);
    assert!(from_deserializer::<Value, _>(&mut de, how()).is_err());
    let shallow = packed(100);
    let mut de = rmp_serde::Deserializer::new(&shallow[..]);
    assert!(from_deserializer::<Value, _>(&mut de, how()).is_ok());
}
