// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The visitor model: a value walked into a builder is itself, the
//! streaming wire codec writes and reads exactly the tree codec's bytes
//! for every shared vector, and a read through a walk refuses what the
//! tree reader refuses, with the same error.

extern crate alloc;

use alloc::collections::{BTreeMap, BTreeSet};

use guatiao::value::visit::{
    Build, Flat, FromWalk, ListWalker, MapVisitor, MapWalker, Sink, TreeSink, Visitor, Walk, Walker,
};
use guatiao::value::wire::{
    Decoder, Encoded, Encoder, WireError, decode, decode_as, to_bytes, to_bytes_of,
};
use guatiao::{
    Bytes, FromValue, Kinds, List, MAX_DEPTH, Map, MapError, Number, ToValue, TryAsMut, TryAsRef,
    Value,
    ValueError,
};
use serde_json::Value as Json;

#[path = "vectors/read.rs"]
mod vectors;
use vectors::{hex, tree, wire_file};

fn list<'a>(doc: &'a Json, key: &str) -> &'a [Json] {
    doc[key].as_array().expect("a list of vectors")
}

/// A walk of anything, built into a tree.
fn built<T: Walk + ?Sized>(value: &T) -> Result<Value, ValueError> {
    let mut sink = TreeSink::new();
    value.walk(Flat(&mut sink))?;
    sink.into_value()
}

#[test]
fn every_vector_streams_to_its_bytes_and_back() {
    let doc = wire_file();
    for v in list(&doc, "values") {
        let name = v["name"].as_str().unwrap();
        let bytes = hex(v["hex"].as_str().unwrap());
        let expected = tree(&v["tree"]);

        assert_eq!(
            built(&expected).unwrap(),
            expected,
            "{name}: walked into a tree"
        );
        assert_eq!(
            to_bytes_of(&expected).unwrap(),
            bytes,
            "{name}: streamed encode"
        );
        assert_eq!(to_bytes(&expected).unwrap(), bytes, "{name}: tree encode");
        assert_eq!(
            decode_as::<Value>(&bytes).unwrap_or_else(|e| panic!("{name}: {e}")),
            expected,
            "{name}: streamed decode"
        );
        assert_eq!(
            built(&Encoded(&bytes)).unwrap(),
            expected,
            "{name}: bytes walked"
        );
        assert_eq!(
            to_bytes_of(&Encoded(&bytes)).unwrap(),
            bytes,
            "{name}: bytes walked into bytes"
        );
        assert_eq!(
            Value::from_walk(&expected).unwrap(),
            expected,
            "{name}: read from a tree"
        );
    }
}

fn offset_of(error: &WireError) -> Option<usize> {
    match error {
        WireError::Truncated { at }
        | WireError::UnknownTag { at, .. }
        | WireError::Overlong { at }
        | WireError::NotUtf8 { at }
        | WireError::NotANumber { at }
        | WireError::NotABool { at, .. }
        | WireError::DuplicateKey { at }
        | WireError::Trailing { at }
        | WireError::UnknownFrame { at, .. } => Some(*at),
        _ => None,
    }
}

#[test]
fn every_malformed_vector_is_refused_where_the_tree_decoder_refuses_it() {
    let doc = wire_file();
    for v in list(&doc, "malformed") {
        let name = v["name"].as_str().unwrap();
        let bytes = hex(v["hex"].as_str().unwrap());
        let tree_error = decode(&bytes).unwrap_err();
        let stream_error = decode_as::<Value>(&bytes).unwrap_err();
        assert_eq!(stream_error, tree_error, "{name}: streamed decode");
        match built(&Encoded(&bytes)) {
            Err(ValueError::Unparsed { at, .. }) => {
                assert_eq!(Some(at), offset_of(&tree_error), "{name}: bytes walked")
            }
            other => panic!("{name}: bytes walked: {other:?}"),
        }
    }
}

/// A map of `n` keys, `k000000` on, as wire bytes, with the key at
/// `copy_to` (when given) made the same as the one at `copy_from`; and
/// where that repeat starts.
fn wide_map(n: usize, repeat: Option<(usize, usize)>) -> (Vec<u8>, usize) {
    let mut map = Map::new();
    for i in 0..n {
        map.set(&format!("k{i:06}"), i as i64);
    }
    let mut bytes = to_bytes(&map.into()).expect("encodes");
    let Some((from, to)) = repeat else {
        return (bytes, 0);
    };
    let find = |key: &str| {
        bytes
            .windows(key.len())
            .position(|w| w == key.as_bytes())
            .expect("the key is in the bytes")
    };
    let at = find(&format!("k{to:06}"));
    let from = format!("k{from:06}");
    bytes[at..at + from.len()].copy_from_slice(from.as_bytes());
    // The repeat is reported at its length's byte, one before its text.
    (bytes, at - 1)
}

/// A map past a few keys checks each new key against a set, not every key
/// before it: the reader of bytes anyone wrote stays linear in them.
#[test]
fn a_wide_map_is_read_in_linear_time_and_a_repeat_anywhere_is_found() {
    let n = 100_000;
    let (bytes, _) = wide_map(n, None);
    let started = std::time::Instant::now();
    let tree = decode(&bytes).expect("decodes");
    let streamed = decode_as::<Value>(&bytes).expect("streams");
    let walked = built(&Encoded(&bytes)).expect("walks");
    let took = started.elapsed();
    let map: &Map = TryAsRef::try_as_ref(&tree).expect("a map");
    assert_eq!(map.len(), n);
    assert_eq!(streamed, tree);
    assert_eq!(walked, tree);
    // A scan per key is some 5e9 comparisons here, minutes in a debug build.
    assert!(took.as_secs() < 20, "{n} keys took {took:?}");

    for (from, to) in [(3, 7), (3, 40), (17, 99_999), (99_998, 99_999)] {
        let (bytes, at) = wide_map(n, Some((from, to)));
        let tree_error = decode(&bytes).unwrap_err();
        assert_eq!(tree_error, WireError::DuplicateKey { at }, "k{from} at k{to}");
        assert_eq!(decode_as::<Value>(&bytes).unwrap_err(), tree_error, "streamed");
        match built(&Encoded(&bytes)) {
            Err(ValueError::Unparsed { at: walked, .. }) => assert_eq!(walked, at, "walked"),
            other => panic!("walked: {other:?}"),
        }
    }
}

// --- a type that walks: derived, and written by hand -------------------------------

#[derive(Debug, Clone, PartialEq, ToValue, FromValue)]
struct Server {
    host: String,
    port: u16,
    weight: f64,
    tags: Vec<String>,
    labels: BTreeMap<String, u32>,
    spare: Option<Bytes>,
}

/// The same struct, its walk written by hand as the derive writes it.
#[derive(Debug, Clone, PartialEq)]
struct ByHand(Server);

impl Walk for ByHand {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        let s = &self.0;
        let mut map = visitor.map(Some(5 + usize::from(s.spare.is_some())))?;
        map.entry("host", &s.host)?;
        map.entry("port", &s.port)?;
        map.entry("weight", &s.weight)?;
        map.entry("tags", &s.tags)?;
        map.entry("labels", &s.labels)?;
        if let Some(spare) = &s.spare {
            map.entry("spare", spare)?;
        }
        map.end()
    }
}

struct ByHandBuild;

impl Build for ByHandBuild {
    type Out = ByHand;

    fn kinds(&self) -> Kinds {
        Kinds::MAP
    }

    fn map<M: MapWalker>(self, mut entries: M) -> Result<ByHand, MapError> {
        let mut host = None;
        let mut port = None;
        let mut weight = None;
        let mut tags = None;
        let mut labels = None;
        let mut spare = None;
        while let Some(key) = entries.next_key(|key| match key {
            "host" => 0,
            "port" => 1,
            "weight" => 2,
            "tags" => 3,
            "labels" => 4,
            "spare" => 5,
            _ => 6,
        })? {
            let named = |name: &'static str| move |e: MapError| e.under(name);
            match key {
                0 => host = Some(entries.value().map_err(named("host"))?),
                1 => port = Some(entries.value().map_err(named("port"))?),
                2 => weight = Some(entries.value().map_err(named("weight"))?),
                3 => tags = Some(entries.value().map_err(named("tags"))?),
                4 => labels = Some(entries.value().map_err(named("labels"))?),
                5 => spare = entries.value().map_err(named("spare"))?,
                _ => entries.skip_value()?,
            }
        }
        Ok(ByHand(Server {
            host: host.ok_or_else(|| MapError::missing("host"))?,
            port: port.ok_or_else(|| MapError::missing("port"))?,
            weight: weight.ok_or_else(|| MapError::missing("weight"))?,
            tags: tags.ok_or_else(|| MapError::missing("tags"))?,
            labels: labels.ok_or_else(|| MapError::missing("labels"))?,
            spare,
        }))
    }
}

impl FromWalk for ByHand {
    fn from_walk<W: Walker>(walker: W) -> Result<ByHand, MapError> {
        walker.build(ByHandBuild)
    }
}

impl FromValue for ByHand {
    fn from_value(value: &Value) -> Result<ByHand, MapError> {
        Server::from_value(value).map(ByHand)
    }
}

fn servers(n: usize) -> Vec<Server> {
    (0..n)
        .map(|i| Server {
            host: format!("10.0.0.{i}"),
            port: 5900 + i as u16,
            weight: 0.5 + i as f64,
            tags: vec!["a".into(), format!("rack-{i}")],
            labels: [("zone".to_string(), i as u32)].into_iter().collect(),
            spare: (i % 2 == 0).then(|| Bytes(vec![i as u8, 0xff])),
        })
        .collect()
}

#[test]
fn a_type_streams_to_the_bytes_its_tree_encodes_to_and_back() {
    let value = servers(40);
    let tree = value.to_value().unwrap();
    let tree_bytes = to_bytes(&tree).unwrap();

    assert_eq!(to_bytes_of(&value).unwrap(), tree_bytes, "derived");
    assert_eq!(decode_as::<Vec<Server>>(&tree_bytes).unwrap(), value);
    assert_eq!(Vec::<Server>::from_walk(&tree).unwrap(), value);
    assert_eq!(built(&value).unwrap(), tree);

    let by_hand: Vec<ByHand> = value.iter().cloned().map(ByHand).collect();
    assert_eq!(to_bytes_of(&by_hand).unwrap(), tree_bytes, "by hand");
    assert_eq!(decode_as::<Vec<ByHand>>(&tree_bytes).unwrap(), by_hand);
}

/// A walk names what it refuses as the tree reader does: the same error
/// for every input refused here.
#[test]
fn a_walk_refuses_what_the_tree_reader_refuses_with_the_same_error() {
    fn same<T: FromWalk + FromValue + core::fmt::Debug>(value: Value) {
        let tree = T::from_value(&value).expect_err("refused by the tree reader");
        let walk = T::from_walk(&value).expect_err("refused through a walk");
        assert_eq!(walk, tree, "for {value:?}");
        let bytes = to_bytes(&value).unwrap();
        assert_eq!(
            decode_as::<T>(&bytes).unwrap_err(),
            WireError::Refused { error: tree },
            "from bytes, for {value:?}"
        );
    }

    same::<u16>(Value::from(70_000));
    same::<u16>(Number::new("1.5").unwrap().into());
    same::<u16>(Value::from("80"));
    same::<i8>(Number::new("1e2").unwrap().into());
    same::<f32>(Number::new("1e39").unwrap().into());
    same::<bool>(Value::null());
    same::<String>(Value::from(5));
    same::<char>(Value::from("ab"));
    same::<Bytes>(Value::from("x"));
    same::<()>(Value::from(false));
    same::<Vec<u8>>(vec![1i64, 2, 300].to_value().unwrap());
    same::<BTreeSet<u8>>(vec![1i64, 1].to_value().unwrap());
    same::<[u8; 2]>(vec![1i64, 2, 3].to_value().unwrap());
    same::<(u8, String)>(vec![1i64].to_value().unwrap());
    same::<BTreeMap<u32, String>>({
        let mut m = Map::new();
        m.set("01", "x");
        m.into()
    });
    same::<BTreeMap<String, u8>>({
        let mut m = Map::new();
        m.set("PATH", 999);
        m.into()
    });
    same::<Server>(Value::from("not a map"));
    same::<ByHand>(Value::from("not a map"));
    same::<ByHand>(Map::new().into());
    same::<Server>(Map::new().into());
    same::<Server>({
        let mut tree = servers(1)[0].to_value().unwrap();
        let map: &mut Map = TryAsMut::<Map>::try_as_mut(&mut tree).unwrap();
        map.set("tags", vec![Value::from(1)].into_iter().collect::<List>());
        tree
    });
    same::<Map>(Value::from(1));
    same::<core::time::Duration>(Value::from("soon"));
    same::<core::net::IpAddr>(Value::from("not an address"));
}

// --- the encoder's own checks -------------------------------------------------------

#[test]
fn the_encoder_refuses_a_walk_that_breaks_the_encoding() {
    let mut out = Vec::new();

    let mut e = Encoder::new(&mut out);
    e.list_begin(Some(2)).unwrap();
    e.null().unwrap();
    assert_eq!(
        e.list_end(),
        Err(ValueError::WrongKind),
        "fewer than promised"
    );

    let mut e = Encoder::new(&mut out);
    e.list_begin(Some(1)).unwrap();
    e.null().unwrap();
    assert_eq!(e.null(), Err(ValueError::WrongKind), "more than promised");

    let mut e = Encoder::new(&mut out);
    e.map_begin(None).unwrap();
    e.key("a").unwrap();
    e.null().unwrap();
    assert_eq!(e.key("a"), Err(ValueError::DuplicateKey));

    let mut e = Encoder::new(&mut out);
    assert_eq!(e.number("1,5"), Err(ValueError::NotANumber));

    let mut e = Encoder::new(&mut out);
    e.map_begin(None).unwrap();
    assert_eq!(
        e.null(),
        Err(ValueError::WrongKind),
        "a value where a key is due"
    );
    assert_eq!(e.list_end(), Err(ValueError::WrongKind), "the wrong end");

    let mut e = Encoder::new(&mut out);
    e.null().unwrap();
    assert_eq!(
        e.null(),
        Err(ValueError::WrongKind),
        "a second value at the top"
    );
}

/// Keys past the linear check's reach are still checked.
#[test]
fn a_repeated_key_is_refused_in_a_large_map() {
    let mut out = Vec::new();
    let mut e = Encoder::new(&mut out);
    e.map_begin(None).unwrap();
    for i in 0..100 {
        e.key(&format!("k{i}")).unwrap();
        e.null().unwrap();
    }
    assert_eq!(e.key("k7"), Err(ValueError::DuplicateKey));
    assert_eq!(e.key("k99"), Err(ValueError::DuplicateKey));
    e.key("k100").unwrap();
}

/// A count given only at the end, long enough to need two LEB128 bytes,
/// moves what follows it and leaves the same bytes a promised one would.
#[test]
fn a_count_given_at_the_end_is_written_where_a_promised_one_would_be() {
    struct Unsized(Vec<String>);

    impl Walk for Unsized {
        fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
            let mut map = visitor.map(None)?;
            for (i, text) in self.0.iter().enumerate() {
                map.entry(&format!("{i}"), text)?;
            }
            map.end()
        }
    }

    for n in [0, 1, 127, 128, 300, 20_000] {
        let texts: Vec<String> = (0..n).map(|i| format!("t{i}")).collect();
        let tree: Map = texts
            .iter()
            .enumerate()
            .map(|(i, t)| (format!("{i}"), t.as_str()))
            .collect();
        assert_eq!(
            to_bytes_of(&Unsized(texts)).unwrap(),
            to_bytes(&tree.into()).unwrap(),
            "{n} entries"
        );
    }
}

// --- depth ----------------------------------------------------------------------------

fn nested(depth: usize) -> Value {
    let mut value = Value::null();
    for _ in 0..depth {
        let mut list = List::new();
        list.push(value);
        value = list.into();
    }
    value
}

#[test]
fn a_walk_follows_no_deeper_than_max_depth() {
    let limit = MAX_DEPTH as usize;
    let deepest = nested(limit);
    let bytes = to_bytes(&deepest).unwrap();
    assert_eq!(to_bytes_of(&deepest).unwrap(), bytes);
    assert_eq!(decode_as::<Value>(&bytes).unwrap(), deepest);
    assert_eq!(built(&Encoded(&bytes)).unwrap(), deepest);

    let deeper = nested(limit + 1);
    let bytes = to_bytes(&deeper).unwrap();
    assert_eq!(
        decode(&bytes).unwrap(),
        deeper,
        "the tree codec has no limit"
    );
    assert_eq!(to_bytes_of(&deeper), Err(ValueError::TooDeep));
    assert_eq!(
        decode_as::<Value>(&bytes),
        // Each list is its tag and a one-byte count.
        Err(WireError::TooDeep { at: 2 * limit })
    );
    assert_eq!(built(&Encoded(&bytes)), Err(ValueError::TooDeep));
    assert!(matches!(
        Value::from_walk(&deeper),
        Err(MapError::BadValue { .. })
    ));
}

// --- a builder that reads less than it was given -------------------------------------

/// Reads a map's first entry's value and stops; the decoder passes over
/// the rest, so what follows the map is read where it is.
struct First;

struct FirstBuild;

impl Build for FirstBuild {
    type Out = First;

    fn kinds(&self) -> Kinds {
        Kinds::MAP.union(Kinds::LIST)
    }

    fn map<M: MapWalker>(self, mut entries: M) -> Result<First, MapError> {
        entries.next_key(|_| ())?;
        Ok(First)
    }

    fn list<L: ListWalker>(self, mut items: L) -> Result<First, MapError> {
        items.next::<Value>()?;
        Ok(First)
    }
}

impl FromWalk for First {
    fn from_walk<W: Walker>(walker: W) -> Result<First, MapError> {
        walker.build(FirstBuild)
    }
}

#[test]
fn what_a_builder_leaves_unread_is_passed_over() {
    let mut inner = Map::new();
    inner.set("a", vec![1i64, 2, 3].to_value().unwrap());
    inner.set("b", "two");
    let mut outer = List::new();
    outer.push(Value::from(inner));
    outer.push(
        vec![Value::from(1), Value::from("x")]
            .into_iter()
            .collect::<List>(),
    );
    outer.push(true);
    let bytes = to_bytes(&outer.into()).unwrap();

    let read = decode_as::<(First, First, bool)>(&bytes).unwrap();
    assert!(read.2);
}

/// Passing over a value costs heap, never the thread's stack, whatever
/// its depth.
#[test]
fn a_value_passed_over_may_be_of_any_depth() {
    let depth = if cfg!(miri) { 2_000 } else { 200_000 };
    let mut map = Map::new();
    map.set("deep", nested(depth));
    let bytes = to_bytes(&map.into()).unwrap();
    assert!(decode_as::<First>(&bytes).is_ok());
}

/// A key repeated in bytes is refused by the decoder even inside a map a
/// builder passes over.
#[test]
fn a_repeated_key_is_refused_even_where_it_is_skipped() {
    // [ {"a": null, "a": null} , true ] written by hand.
    let bytes = [6, 2, 7, 2, 1, b'a', 1, 1, b'a', 1, 2, 1];
    assert!(matches!(
        decode_as::<(First, bool)>(&bytes),
        Err(WireError::DuplicateKey { at: 7 })
    ));
    assert!(matches!(
        decode(&bytes),
        Err(WireError::DuplicateKey { at: 7 })
    ));
}

#[test]
fn a_decoder_reports_its_own_refusal_and_trailing_bytes() {
    let mut decoder = Decoder::new(&[3, 1, b'x']);
    assert!(matches!(
        u8::from_walk(&mut decoder),
        Err(MapError::Stopped { .. })
    ));
    assert_eq!(decoder.failure(), Some(&WireError::NotANumber { at: 1 }));
    assert_eq!(
        decode_as::<bool>(&[2, 1, 0]),
        Err(WireError::Trailing { at: 2 })
    );
}

/// A sink sees every event in order, which is what the C visitor table
/// receives.
#[test]
fn a_sink_sees_the_events_in_order() {
    #[derive(Default)]
    struct Log(Vec<String>);

    impl Sink for Log {
        type Error = ValueError;
        fn absent(&mut self) -> Result<(), ValueError> {
            self.0.push("absent".into());
            Ok(())
        }
        fn null(&mut self) -> Result<(), ValueError> {
            self.0.push("null".into());
            Ok(())
        }
        fn bool(&mut self, v: bool) -> Result<(), ValueError> {
            self.0.push(format!("bool {v}"));
            Ok(())
        }
        fn number(&mut self, t: &str) -> Result<(), ValueError> {
            self.0.push(format!("number {t}"));
            Ok(())
        }
        fn string(&mut self, t: &str) -> Result<(), ValueError> {
            self.0.push(format!("string {t}"));
            Ok(())
        }
        fn bytes(&mut self, b: &[u8]) -> Result<(), ValueError> {
            self.0.push(format!("bytes {b:?}"));
            Ok(())
        }
        fn list_begin(&mut self, n: Option<usize>) -> Result<(), ValueError> {
            self.0.push(format!("list {n:?}"));
            Ok(())
        }
        fn list_end(&mut self) -> Result<(), ValueError> {
            self.0.push("end list".into());
            Ok(())
        }
        fn map_begin(&mut self, n: Option<usize>) -> Result<(), ValueError> {
            self.0.push(format!("map {n:?}"));
            Ok(())
        }
        fn key(&mut self, k: &str) -> Result<(), ValueError> {
            self.0.push(format!("key {k}"));
            Ok(())
        }
        fn map_end(&mut self) -> Result<(), ValueError> {
            self.0.push("end map".into());
            Ok(())
        }
    }

    let mut log = Log::default();
    let mut map = BTreeMap::new();
    map.insert("x".to_string(), vec![1.5f64, -2.0]);
    (map, Some(true), 'c', u128::MAX)
        .walk(Flat(&mut log))
        .unwrap();
    assert_eq!(
        log.0,
        [
            "list Some(4)",
            "map Some(1)",
            "key x",
            "list Some(2)",
            "number 1.5",
            "number -2",
            "end list",
            "end map",
            "bool true",
            "string c",
            "number 340282366920938463463374607431768211455",
            "end list",
        ]
    );
}

/// A visitor that walks every element twice, as a format that looks at a
/// value before writing it does, sees the same events both times.
#[test]
fn bytes_walked_twice_give_the_same_events() {
    use guatiao::value::visit::{ListVisitor, MapVisitor};

    struct Twice<'a>(&'a mut TreeSink);

    struct TwiceList<'a>(guatiao::value::visit::FlatList<'a, TreeSink>);
    struct TwiceMap<'a>(guatiao::value::visit::FlatMap<'a, TreeSink>);

    fn looked_at<W: Walk + ?Sized>(w: &W) -> Result<(), ValueError> {
        let mut scratch = TreeSink::new();
        w.walk(Flat(&mut scratch))
    }

    impl ListVisitor for TwiceList<'_> {
        type Ok = ();
        type Error = ValueError;
        fn item<W: Walk + ?Sized>(&mut self, w: &W) -> Result<(), ValueError> {
            looked_at(w)?;
            self.0.item(w)
        }
        fn end(self) -> Result<(), ValueError> {
            self.0.end()
        }
    }

    impl MapVisitor for TwiceMap<'_> {
        type Ok = ();
        type Error = ValueError;
        fn entry<W: Walk + ?Sized>(&mut self, k: &str, w: &W) -> Result<(), ValueError> {
            looked_at(w)?;
            self.0.entry(k, w)
        }
        fn end(self) -> Result<(), ValueError> {
            self.0.end()
        }
    }

    impl<'a> Visitor for Twice<'a> {
        type Ok = ();
        type Error = ValueError;
        type List = TwiceList<'a>;
        type Map = TwiceMap<'a>;
        fn absent(self) -> Result<(), ValueError> {
            Flat(self.0).absent()
        }
        fn null(self) -> Result<(), ValueError> {
            Flat(self.0).null()
        }
        fn bool(self, v: bool) -> Result<(), ValueError> {
            Flat(self.0).bool(v)
        }
        fn number(self, t: &str) -> Result<(), ValueError> {
            Flat(self.0).number(t)
        }
        fn string(self, t: &str) -> Result<(), ValueError> {
            Flat(self.0).string(t)
        }
        fn bytes(self, b: &[u8]) -> Result<(), ValueError> {
            Flat(self.0).bytes(b)
        }
        fn list(self, n: Option<usize>) -> Result<TwiceList<'a>, ValueError> {
            Flat(self.0).list(n).map(TwiceList)
        }
        fn map(self, n: Option<usize>) -> Result<TwiceMap<'a>, ValueError> {
            Flat(self.0).map(n).map(TwiceMap)
        }
    }

    let doc = wire_file();
    for v in list(&doc, "values") {
        let bytes = hex(v["hex"].as_str().unwrap());
        let mut sink = TreeSink::new();
        Encoded(&bytes).walk(Twice(&mut sink)).unwrap();
        assert_eq!(
            sink.into_value().unwrap(),
            tree(&v["tree"]),
            "{}",
            v["name"]
        );
    }
}

#[test]
fn a_float_that_is_not_finite_is_refused() {
    assert_eq!(to_bytes_of(&f64::NAN), Err(ValueError::NotANumber));
    assert_eq!(to_bytes_of(&f32::INFINITY), Err(ValueError::NotANumber));
}

#[test]
fn a_tree_sink_refuses_events_out_of_order() {
    let mut sink = TreeSink::new();
    assert_eq!(sink.list_end(), Err(ValueError::WrongKind));
    sink.map_begin(None).unwrap();
    assert_eq!(sink.string("v"), Err(ValueError::WrongKind));
    sink.key("k").unwrap();
    assert_eq!(sink.key("k2"), Err(ValueError::WrongKind));
    assert_eq!(sink.map_end(), Err(ValueError::WrongKind));
    sink.number("1").unwrap();
    sink.map_end().unwrap();
    assert_eq!(sink.null(), Err(ValueError::WrongKind));
    let value = sink.into_value().unwrap();
    let map: &Map = (&value).try_into().unwrap();
    assert_eq!(map.len(), 1);
    assert_eq!(TreeSink::new().into_value(), Err(ValueError::WrongKind));
}
