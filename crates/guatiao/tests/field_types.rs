// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Every field type the three derives accept: it writes a value, reads
//! that value back unchanged, and its own schema accepts what it wrote.

#![cfg(feature = "derive")]
extern crate alloc;

use core::fmt::Debug;

use guatiao::schema::read::SchemaRef;
use guatiao::schema::validate::validate_map;
use guatiao::schema::{FieldBuilder, SchemaBuilder};
use guatiao::value::convert::TryAsRef;
use guatiao::{
    Alloc, Buffer, FromValue, List, Map, MapError, Number, Schema, Text, ToValue, Value,
};

/// Writes `v`, reads it back, checks the two agree and that `T`'s own
/// kind accepts the value. Returns the value written.
fn check<T: ToValue + FromValue + Schema + PartialEq + Debug>(v: T) -> Value {
    let value = v.to_value().expect("writes");
    assert_eq!(T::from_value(&value).expect("reads back"), v);
    assert!(accepts::<T>(&value), "the type's own schema accepts {v:?}");
    value
}

/// Whether `T`'s kind, as a required field, accepts `value`.
fn accepts<T: Schema + ?Sized>(value: &Value) -> bool {
    let schema = SchemaBuilder::new()
        .field(FieldBuilder::new("f", T::kind(Alloc::rust())).required())
        .finish()
        .expect("a schema");
    let mut doc = Map::new();
    doc.set("f", value.clone());
    validate_map(SchemaRef::new(&schema).expect("a map"), &Value::from(doc)).is_ok()
}

/// `T`'s kind, finished.
fn kind_of<T: Schema + ?Sized>() -> Value {
    T::kind(Alloc::rust()).finish().expect("a kind")
}

/// The text under `key` of a schema map.
fn text_at<'a>(schema: &'a Value, key: &str) -> Option<&'a str> {
    <&Map>::try_from(schema)
        .ok()?
        .get(key)
        .and_then(TryAsRef::<str>::try_as_ref)
}

fn get<'a>(schema: &'a Value, key: &str) -> Option<&'a Value> {
    <&Map>::try_from(schema).ok()?.get(key)
}

mod own_types {
    use super::*;

    fn sample_map() -> Map {
        let mut inner = List::new();
        inner.push("a");
        inner.push(5900i64);
        let mut map = Map::new();
        map.set("host", "10.0.0.1");
        map.set("tags", inner);
        map.set("nothing", Value::null());
        map
    }

    #[test]
    fn a_map_crosses_as_itself_and_accepts_any_keys() {
        let written = check(sample_map());
        assert_eq!(<&Map>::try_from(&written).unwrap().len(), 3);
        let kind = kind_of::<Map>();
        assert_eq!(text_at(&kind, "type"), Some("object"));
        // An open object: any key, any value.
        assert_eq!(
            <&Map>::try_from(get(&kind, "additionalProperties").unwrap())
                .unwrap()
                .len(),
            0
        );
        assert!(!accepts::<Map>(&Value::from(List::new())));
    }

    #[test]
    fn a_list_crosses_as_itself_and_accepts_any_items() {
        let mut list = List::new();
        list.push(sample_map());
        list.push(true);
        check(list);
        let kind = kind_of::<List>();
        assert_eq!(text_at(&kind, "type"), Some("array"));
        assert!(!accepts::<List>(&Value::from(Map::new())));
    }

    #[test]
    fn a_value_is_anything_and_its_schema_is_empty() {
        check(Value::from(sample_map()));
        check(Value::null());
        check(Value::from(1.5f64.to_string().parse::<Number>().unwrap()));
        assert_eq!(<&Map>::try_from(&kind_of::<Value>()).unwrap().len(), 0);
        // `{}` already accepts null, so an option of it says no more.
        assert_eq!(
            <&Map>::try_from(&kind_of::<Option<Value>>()).unwrap().len(),
            0
        );
    }

    #[test]
    fn text_number_and_buffer_cross_as_their_kinds() {
        let t = check(Text::new("héllo"));
        assert_eq!(TryAsRef::<str>::try_as_ref(&t), Some("héllo"));
        let n = check(Number::new("123456789012345678901234567890.5").unwrap());
        // The exact text survives, past what any float holds.
        assert_eq!(
            AsRef::<str>::as_ref(TryAsRef::<Number>::try_as_ref(&n).unwrap()),
            "123456789012345678901234567890.5"
        );
        check(Buffer::new(&[0, 255, 7]));
        assert_eq!(text_at(&kind_of::<Text>(), "type"), Some("string"));
        assert_eq!(text_at(&kind_of::<Number>(), "type"), Some("number"));
        assert_eq!(text_at(&kind_of::<Buffer>(), "type"), Some("bytes"));
    }

    #[test]
    fn the_wrong_kind_is_refused_by_kind() {
        let number = Value::from(5i64);
        assert!(matches!(
            Map::from_value(&number),
            Err(MapError::WrongType { .. })
        ));
        // A number is not text, though they share storage.
        assert!(matches!(
            Text::from_value(&number),
            Err(MapError::WrongType { .. })
        ));
        assert!(matches!(
            Number::from_value(&Value::from("5")),
            Err(MapError::WrongType { .. })
        ));
    }

    #[derive(Debug, PartialEq, guatiao::ToValue, guatiao::FromValue, guatiao::Schema)]
    struct Holder {
        settings: Map,
        extra: Option<Map>,
        items: List,
        maybe: Option<List>,
        raw: Value,
        many: Vec<Map>,
        named: alloc::collections::BTreeMap<String, Map>,
        label: Text,
        amount: Number,
        blob: Buffer,
    }

    #[test]
    fn a_struct_of_them_derives_all_three_and_validates_what_it_writes() {
        let mut named = alloc::collections::BTreeMap::new();
        named.insert("one".to_string(), sample_map());
        let holder = Holder {
            settings: sample_map(),
            extra: None,
            items: List::new(),
            maybe: Some(List::new()),
            raw: Value::from(true),
            many: vec![sample_map(), Map::new()],
            named,
            label: Text::new("x"),
            amount: Number::new("1e3").unwrap(),
            blob: Buffer::new(b"ab"),
        };
        let value = holder.to_value().unwrap();
        assert_eq!(Holder::from_value(&value).unwrap(), holder);
        let schema = Holder::schema().unwrap();
        validate_map(SchemaRef::new(&schema).unwrap(), &value).unwrap();
    }
}

mod wrappers {
    use super::*;
    use alloc::borrow::Cow;
    use alloc::rc::Rc;
    use alloc::sync::Arc;

    #[test]
    fn a_pointer_crosses_as_what_it_holds() {
        let plain = 5900u32.to_value().unwrap();
        assert_eq!(check(Box::new(5900u32)), plain);
        assert_eq!(check(Arc::new(5900u32)), plain);
        assert_eq!(check(Rc::new(5900u32)), plain);
        assert_eq!(kind_of::<Box<u32>>(), kind_of::<u32>());
        check(Box::new(vec![Arc::new("a".to_string())]));
        check(Box::new(Map::new()));
    }

    #[test]
    fn shared_text_and_slices_cross_as_strings_and_lists() {
        let text = "héllo".to_value().unwrap();
        assert_eq!(check(Box::<str>::from("héllo")), text);
        assert_eq!(check(Arc::<str>::from("héllo")), text);
        assert_eq!(check(Rc::<str>::from("héllo")), text);
        assert_eq!(kind_of::<Arc<str>>(), kind_of::<String>());
        let list = vec![1u8, 2].to_value().unwrap();
        assert_eq!(check(Box::<[u8]>::from(&[1u8, 2][..])), list);
        assert_eq!(check(Arc::<[u8]>::from(&[1u8, 2][..])), list);
        assert_eq!(kind_of::<Box<[u8]>>(), kind_of::<Vec<u8>>());
    }

    #[test]
    fn a_cow_crosses_as_what_it_holds_and_reads_owned() {
        let borrowed: Cow<'static, str> = Cow::Borrowed("x");
        let value = check(borrowed);
        assert!(matches!(
            Cow::<str>::from_value(&value).unwrap(),
            Cow::Owned(_)
        ));
        check(Cow::<[u32]>::Owned(vec![1, 2]));
    }

    #[test]
    fn a_char_is_one_character_of_text() {
        assert_eq!(TryAsRef::<str>::try_as_ref(&check('é')), Some("é"));
        check('🦀');
        assert!(matches!(
            char::from_value(&Value::from("ab")),
            Err(MapError::BadValue { .. })
        ));
        assert!(matches!(
            char::from_value(&Value::from("")),
            Err(MapError::BadValue { .. })
        ));
        assert!(matches!(
            char::from_value(&Value::from(1i64)),
            Err(MapError::WrongType { .. })
        ));
        assert!(!accepts::<char>(&Value::from("ab")));
        assert!(!accepts::<char>(&Value::from("")));
        assert_eq!(char::try_from(&Value::from("z")), Ok('z'));
    }

    #[derive(Debug, PartialEq, guatiao::ToValue, guatiao::FromValue, guatiao::Schema)]
    struct Shared {
        name: Arc<str>,
        tags: Box<[String]>,
        inner: Box<Map>,
        count: Rc<u32>,
        note: Cow<'static, str>,
        initial: char,
        maybe: Option<Box<char>>,
    }

    #[test]
    fn a_struct_of_them_derives_and_validates() {
        let s = Shared {
            name: Arc::from("n"),
            tags: Box::from(vec!["a".to_string()]),
            inner: Box::new(Map::new()),
            count: Rc::new(3),
            note: Cow::Borrowed("hi"),
            initial: 'q',
            maybe: Some(Box::new('r')),
        };
        let value = s.to_value().unwrap();
        assert_eq!(Shared::from_value(&value).unwrap(), s);
        let schema = Shared::schema().unwrap();
        validate_map(SchemaRef::new(&schema).unwrap(), &value).unwrap();
    }
}

mod collections {
    use super::*;
    use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
    use std::collections::{HashMap, HashSet};

    fn list(items: &[Value]) -> Value {
        let mut l = List::new();
        for item in items {
            l.push(item.clone());
        }
        l.into()
    }

    #[test]
    fn a_deque_is_a_list() {
        let d: VecDeque<u32> = [3, 1, 2].into_iter().collect();
        assert_eq!(check(d), vec![3u32, 1, 2].to_value().unwrap());
        assert_eq!(kind_of::<VecDeque<u32>>(), kind_of::<Vec<u32>>());
    }

    #[test]
    fn a_set_is_a_list_of_unique_elements() {
        let s: BTreeSet<String> = ["b", "a"].into_iter().map(String::from).collect();
        // Sorted, so the same every time.
        assert_eq!(check(s), vec!["a", "b"].to_value().unwrap());
        let h: HashSet<u16> = [1, 2, 3].into_iter().collect();
        check(h);
        check(BTreeSet::from([true, false]));
        let kind = kind_of::<BTreeSet<String>>();
        assert_eq!(get(&kind, "uniqueItems"), Some(&Value::from(true)));
        assert_eq!(kind_of::<HashSet<String>>(), kind);

        let repeated = list(&["a".to_value().unwrap(), "a".to_value().unwrap()]);
        let refused = BTreeSet::<String>::from_value(&repeated).unwrap_err();
        assert!(matches!(refused, MapError::BadValue { .. }));
        assert_eq!(refused.key(), "[1]");
        assert!(HashSet::<String>::from_value(&repeated).is_err());
        assert!(!accepts::<BTreeSet<String>>(&repeated));
        // Numbers are equal by what they denote.
        let numbers = list(&[Value::from(1i64), "1.0".parse::<Number>().unwrap().into()]);
        assert!(!accepts::<BTreeSet<f64>>(&numbers));
    }

    #[test]
    fn an_array_is_a_list_of_exactly_its_length() {
        let v = check([10u8, 0, 0, 1]);
        assert_eq!(<&List>::try_from(&v).unwrap().len(), 4);
        check(["a".to_string(), "b".to_string()]);
        let kind = kind_of::<[u8; 4]>();
        assert_eq!(get(&kind, "minItems"), Some(&Value::from(4u64)));
        assert_eq!(get(&kind, "maxItems"), Some(&Value::from(4u64)));
        let short = vec![1u8, 2, 3].to_value().unwrap();
        assert!(matches!(
            <[u8; 4]>::from_value(&short),
            Err(MapError::BadValue { .. })
        ));
        assert!(!accepts::<[u8; 4]>(&short));
        let bad = vec![1u32, 2, 300, 4].to_value().unwrap();
        assert_eq!(<[u8; 4]>::from_value(&bad).unwrap_err().key(), "[2]");
    }

    #[test]
    fn a_tuple_is_a_list_by_position() {
        let v = check((7u8, "x".to_string()));
        assert_eq!(v, list(&[Value::from(7i64), Value::from("x")]));
        check((1u8,));
        check(vec![("a".to_string(), 1u8), ("b".to_string(), 2)]);
        check((
            1u8,
            2u16,
            3u32,
            4u64,
            -1i8,
            -2i16,
            -3i32,
            -4i64,
            0.5f32,
            1.5f64,
            true,
            "twelve".to_string(),
        ));
        let kind = kind_of::<(u8, String)>();
        assert_eq!(text_at(&kind, "type"), Some("array"));
        assert_eq!(get(&kind, "items"), Some(&Value::from(false)));
        let prefix = <&List>::try_from(get(&kind, "prefixItems").unwrap()).unwrap();
        assert_eq!(text_at(&prefix[1], "type"), Some("string"));

        // Each position answers to its own kind, and the length is exact.
        assert!(!accepts::<(u8, String)>(&list(&[
            Value::from("x"),
            Value::from(7i64)
        ])));
        assert!(!accepts::<(u8, String)>(&list(&[Value::from(7i64)])));
        assert!(!accepts::<(u8, String)>(&list(&[
            Value::from(7i64),
            Value::from("x"),
            Value::from("y"),
        ])));
        let wrong = list(&[Value::from("x"), Value::from("y")]);
        assert_eq!(<(u8, String)>::from_value(&wrong).unwrap_err().key(), "[0]");
        assert!(<(u8, String)>::from_value(&list(&[Value::from(1i64)])).is_err());
    }

    #[test]
    fn the_unit_is_null() {
        assert_eq!(check(()), Value::null());
        assert_eq!(text_at(&kind_of::<()>(), "type"), Some("null"));
        assert!(matches!(
            <()>::from_value(&Value::from(0i64)),
            Err(MapError::WrongType { .. })
        ));
    }

    #[test]
    fn a_map_keyed_by_an_integer_is_an_object_keyed_by_its_digits() {
        let m: BTreeMap<u32, String> = [(10, "ten".into()), (2, "two".into())].into();
        let v = check(m);
        let keys: Vec<&str> = <&Map>::try_from(&v).unwrap().keys().collect();
        assert_eq!(keys, ["2", "10"]);
        let h: HashMap<i64, bool> = [(-5, true)].into();
        check(h);
        let kind = kind_of::<BTreeMap<u32, String>>();
        let names = get(&kind, "propertyNames").unwrap();
        assert_eq!(text_at(names, "pattern"), Some("^(0|[1-9][0-9]*)$"));
        assert_eq!(
            text_at(
                get(&kind_of::<BTreeMap<i8, u8>>(), "propertyNames").unwrap(),
                "pattern"
            ),
            Some("^-?(0|[1-9][0-9]*)$")
        );

        let mut odd = Map::new();
        odd.set("01", "x");
        let refused = BTreeMap::<u32, String>::from_value(&Value::from(odd)).unwrap_err();
        assert!(matches!(refused, MapError::BadValue { .. }));
        assert_eq!(refused.key(), "[01]");
        let mut wide = Map::new();
        wide.set("300", "x");
        assert!(BTreeMap::<u8, String>::from_value(&Value::from(wide)).is_err());
        let mut word = Map::new();
        word.set("x", "y");
        // A pattern is enforced only by a build that has the `regex` feature.
        assert_eq!(
            accepts::<BTreeMap<u32, String>>(&word.clone().into()),
            !guatiao::schema::PATTERNS_ENFORCED
        );
        // A string key keeps saying nothing about its keys.
        assert!(get(&kind_of::<BTreeMap<String, u8>>(), "propertyNames").is_none());
    }

    #[test]
    fn char_and_bool_keys_say_what_they_accept() {
        check(BTreeMap::from([('a', 1u8), ('é', 2)]));
        check(BTreeMap::from([(true, 1u8), (false, 0)]));
        let mut two = Map::new();
        two.set("ab", 1i64);
        assert!(!accepts::<BTreeMap<char, u8>>(&two.clone().into()));
        assert!(BTreeMap::<char, u8>::from_value(&two.into()).is_err());
        let mut yes = Map::new();
        yes.set("yes", 1i64);
        assert!(!accepts::<BTreeMap<bool, u8>>(&yes.clone().into()));
        assert!(BTreeMap::<bool, u8>::from_value(&yes.into()).is_err());
        check(BTreeMap::from([(Arc::<str>::from("k"), 1u8)]));
    }

    use alloc::sync::Arc;

    #[derive(Debug, PartialEq, guatiao::ToValue, guatiao::FromValue, guatiao::Schema)]
    struct Shapes {
        order: VecDeque<u32>,
        tags: BTreeSet<String>,
        octets: [u8; 4],
        pair: (u8, String),
        pairs: Vec<(String, u8)>,
        nothing: (),
        by_port: BTreeMap<u16, String>,
        maybe: Option<(u8, u8)>,
    }

    #[test]
    fn a_struct_of_them_derives_and_validates() {
        let s = Shapes {
            order: [1, 2].into(),
            tags: ["a".to_string()].into(),
            octets: [127, 0, 0, 1],
            pair: (1, "one".into()),
            pairs: vec![("x".into(), 9)],
            nothing: (),
            by_port: [(80, "http".to_string())].into(),
            maybe: Some((1, 2)),
        };
        let value = s.to_value().unwrap();
        assert_eq!(Shapes::from_value(&value).unwrap(), s);
        let schema = Shapes::schema().unwrap();
        validate_map(SchemaRef::new(&schema).unwrap(), &value).unwrap();
    }
}

mod std_types {
    use super::*;
    use alloc::collections::BTreeMap;
    use alloc::sync::Arc;
    use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
    use core::num::{NonZeroI64, NonZeroU8, NonZeroU32};
    use core::time::Duration;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn text(v: &Value) -> &str {
        TryAsRef::<str>::try_as_ref(v).unwrap()
    }

    #[test]
    fn a_path_is_its_text() {
        let v = check(PathBuf::from("/etc/ssl/ca.pem"));
        assert_eq!(text(&v), "/etc/ssl/ca.pem");
        check(Box::<Path>::from(Path::new("a/b")));
        check(Arc::<Path>::from(PathBuf::from("c")));
        assert_eq!(kind_of::<PathBuf>(), kind_of::<String>());
        check(BTreeMap::from([(PathBuf::from("x"), 1u8)]));
    }

    #[cfg(windows)]
    #[test]
    fn a_path_that_is_not_utf8_is_refused_on_write() {
        use std::os::windows::ffi::OsStringExt;
        let lone = PathBuf::from(std::ffi::OsString::from_wide(&[0xD800]));
        assert_eq!(lone.to_value(), Err(guatiao::ValueError::NotUtf8));
    }

    #[cfg(unix)]
    #[test]
    fn a_path_that_is_not_utf8_is_refused_on_write() {
        use std::os::unix::ffi::OsStrExt;
        let bad = PathBuf::from(std::ffi::OsStr::from_bytes(&[0xff]));
        assert_eq!(bad.to_value(), Err(guatiao::ValueError::NotUtf8));
    }

    #[test]
    fn an_address_is_its_display_text_with_a_format_or_a_pattern() {
        let v4: Ipv4Addr = "10.0.0.1".parse().unwrap();
        assert_eq!(text(&check(v4)), "10.0.0.1");
        let v6: Ipv6Addr = "::1".parse().unwrap();
        assert_eq!(text(&check(v6)), "::1");
        check(IpAddr::V4(v4));
        check(IpAddr::V6(v6));
        let s4: SocketAddrV4 = "10.0.0.1:5900".parse().unwrap();
        assert_eq!(text(&check(s4)), "10.0.0.1:5900");
        let s6: SocketAddrV6 = "[fe80::1%2]:443".parse().unwrap();
        assert_eq!(text(&check(s6)), "[fe80::1%2]:443");
        check(SocketAddr::V4(s4));
        check(SocketAddr::V6(s6));
        assert_eq!(text_at(&kind_of::<Ipv4Addr>(), "format"), Some("ipv4"));
        assert_eq!(text_at(&kind_of::<Ipv6Addr>(), "format"), Some("ipv6"));
        let either = kind_of::<IpAddr>();
        assert_eq!(text_at(&either, "type"), Some("string"));
        assert_eq!(
            <&List>::try_from(get(&either, "anyOf").unwrap())
                .unwrap()
                .len(),
            2
        );
        assert!(text_at(&kind_of::<SocketAddr>(), "pattern").is_some());

        assert!(matches!(
            Ipv4Addr::from_value(&Value::from("10.0.0")),
            Err(MapError::BadValue { .. })
        ));
        assert!(SocketAddrV4::from_value(&Value::from("10.0.0.1")).is_err());
        assert!(IpAddr::from_value(&Value::from(1i64)).is_err());
        if guatiao::schema::PATTERNS_ENFORCED {
            assert!(!accepts::<SocketAddrV4>(&Value::from("10.0.0.1")));
            assert!(!accepts::<SocketAddr>(&Value::from("::1:80")));
        }
        // `format` is an annotation: never checked.
        assert!(accepts::<Ipv4Addr>(&Value::from("not an address")));

        let h: HashMap<IpAddr, String> = [(IpAddr::V4(v4), "lan".to_string())].into();
        check(h);
        let names = get(&kind_of::<BTreeMap<Ipv4Addr, u8>>(), "propertyNames").cloned();
        assert_eq!(names, Some(kind_of::<Ipv4Addr>()));
    }

    #[test]
    fn a_non_zero_integer_refuses_zero() {
        check(NonZeroU32::new(7).unwrap());
        check(NonZeroI64::new(-7).unwrap());
        check(NonZeroU8::MAX);
        let zero = Value::from(0i64);
        assert!(matches!(
            NonZeroU32::from_value(&zero),
            Err(MapError::BadValue { .. })
        ));
        assert!(NonZeroI64::from_value(&zero).is_err());
        assert!(!accepts::<NonZeroU32>(&zero));
        assert!(!accepts::<NonZeroI64>(&zero));
        assert!(accepts::<NonZeroI64>(&Value::from(i64::MIN)));
        assert!(!accepts::<NonZeroU8>(&Value::from(256i64)));
        let unsigned = kind_of::<NonZeroU32>();
        assert_eq!(get(&unsigned, "minimum"), Some(&Value::from(1i64)));
        let signed = kind_of::<NonZeroI64>();
        let not = get(&signed, "not").unwrap();
        assert_eq!(get(not, "const"), Some(&Value::from(0i64)));
        check(Some(NonZeroU8::new(3).unwrap()));
        assert!(accepts::<Option<NonZeroI64>>(&Value::null()));

        // The text form is judged the same way.
        let field = SchemaBuilder::new()
            .field(FieldBuilder::new("n", NonZeroI64::kind(Alloc::rust())))
            .finish()
            .unwrap();
        let n = SchemaRef::new(&field).unwrap().find("n").unwrap();
        assert!(guatiao::schema::validate_text(n, "0").is_err());
        assert!(guatiao::schema::validate_text(n, "5").is_ok());
    }

    /// The instant, or `None` where this platform's clock cannot hold it
    /// (Windows starts at 1601).
    fn try_at(secs: i64, nanos: u32) -> Option<SystemTime> {
        let whole = if secs >= 0 {
            UNIX_EPOCH.checked_add(Duration::from_secs(secs as u64))
        } else {
            UNIX_EPOCH.checked_sub(Duration::from_secs(secs.unsigned_abs()))
        };
        whole?.checked_add(Duration::from_nanos(nanos.into()))
    }

    fn at(secs: i64, nanos: u32) -> SystemTime {
        try_at(secs, nanos).expect("an instant every platform holds")
    }

    #[test]
    fn a_system_time_is_rfc_3339_in_utc() {
        for (secs, nanos, written) in [
            (0, 0, "1970-01-01T00:00:00Z"),
            (951_825_600, 0, "2000-02-29T12:00:00Z"),
            (-1, 0, "1969-12-31T23:59:59Z"),
            (-1, 500_000_000, "1969-12-31T23:59:59.5Z"),
            (1_792_590_300, 500_000_000, "2026-10-21T13:45:00.5Z"),
            // 100 ns: the finest a Windows `SystemTime` holds.
            (1_792_590_300, 100, "2026-10-21T13:45:00.0000001Z"),
            (-62_167_219_200, 0, "0000-01-01T00:00:00Z"),
            (253_402_300_799, 999_999_900, "9999-12-31T23:59:59.9999999Z"),
        ] {
            let Some(t) = try_at(secs, nanos) else {
                continue;
            };
            assert_eq!(text(&check(t)), written, "{secs}.{nanos}");
        }
        assert_eq!(
            text_at(&kind_of::<SystemTime>(), "format"),
            Some("date-time")
        );
    }

    #[test]
    fn a_system_time_reads_any_offset() {
        let read = |s: &str| SystemTime::from_value(&Value::from(s));
        let expected = at(1_792_590_300, 500_000_000);
        assert_eq!(read("2026-10-21T19:15:00.5+05:30").unwrap(), expected);
        assert_eq!(read("2026-10-21T08:45:00.500-05:00").unwrap(), expected);
        assert_eq!(read("2026-10-21t13:45:00.5z").unwrap(), expected);
        assert_eq!(read("1970-01-01T00:00:00-00:00").unwrap(), UNIX_EPOCH);
        for refused in [
            "2026-10-21",
            "2026-10-21 13:45:00Z",
            "2026-13-01T00:00:00Z",
            "2026-02-29T00:00:00Z",
            "2026-10-21T24:00:00Z",
            "2026-10-21T13:45:60Z",
            "2026-10-21T13:45:00",
            "2026-10-21T13:45:00.Z",
            "2026-10-21T13:45:00.1234567891Z",
            "2026-10-21T13:45:00+0530",
            "2026-10-21T13:45:00Zjunk",
            "+2026-10-21T13:45:00Z",
        ] {
            let e = read(refused).unwrap_err();
            assert!(matches!(e, MapError::BadValue { .. }), "{refused}");
        }
        let e = read("2026-13-01T00:00:00Z").unwrap_err().to_string();
        assert!(e.contains("at byte 5"), "{e}");
        assert!(
            !e.contains("2026"),
            "a refusal does not quote the input: {e}"
        );
    }

    #[test]
    fn a_time_past_year_9999_cannot_be_written() {
        let far = UNIX_EPOCH + Duration::from_secs(253_402_300_800);
        assert_eq!(far.to_value(), Err(guatiao::ValueError::Unrepresentable));
    }

    #[derive(Debug, PartialEq, guatiao::ToValue, guatiao::FromValue, guatiao::Schema)]
    struct Endpoint {
        bind: SocketAddr,
        peer: Option<IpAddr>,
        cert: PathBuf,
        workers: NonZeroU8,
        seen: SystemTime,
    }

    #[test]
    fn a_struct_of_them_derives_and_validates() {
        let e = Endpoint {
            bind: "0.0.0.0:443".parse().unwrap(),
            peer: Some("::1".parse().unwrap()),
            cert: "/etc/cert.pem".into(),
            workers: NonZeroU8::new(4).unwrap(),
            seen: at(1_792_590_300, 0),
        };
        let value = e.to_value().unwrap();
        assert_eq!(Endpoint::from_value(&value).unwrap(), e);
        let schema = Endpoint::schema().unwrap();
        validate_map(SchemaRef::new(&schema).unwrap(), &value).unwrap();
    }
}

mod durations {
    use super::*;
    use core::time::Duration;
    use guatiao::Span;

    fn number(text: &str) -> Value {
        Value::from(text.parse::<Number>().unwrap())
    }

    #[test]
    fn a_duration_writes_seconds_and_nanoseconds() {
        let v = check(Duration::new(90, 500));
        assert_eq!(v, vec![90u64, 500].to_value().unwrap());
        check(Duration::ZERO);
        check(Duration::MAX);
        let kind = kind_of::<Duration>();
        assert_eq!(text_at(&kind, "type"), Some("array"));
        assert_eq!(get(&kind, "items"), Some(&Value::from(false)));
        let prefix = <&List>::try_from(get(&kind, "prefixItems").unwrap()).unwrap();
        assert_eq!(get(&prefix[0], "minimum"), Some(&Value::from(0i64)));
        assert_eq!(
            get(&prefix[1], "maximum"),
            Some(&Value::from(999_999_999i64))
        );
        assert!(!accepts::<Duration>(
            &vec![1u64, 1_000_000_000].to_value().unwrap()
        ));
        assert!(!accepts::<Duration>(&vec![1u64].to_value().unwrap()));
    }

    #[test]
    fn a_duration_reads_seconds_as_a_number_exactly() {
        let read = |text: &str| Duration::from_value(&number(text));
        assert_eq!(read("1.5"), Ok(Duration::new(1, 500_000_000)));
        assert_eq!(read("2e-3"), Ok(Duration::from_millis(2)));
        assert_eq!(read("1.000000001"), Ok(Duration::new(1, 1)));
        assert_eq!(read("1E3"), Ok(Duration::from_secs(1000)));
        assert_eq!(read("0"), Ok(Duration::ZERO));
        assert_eq!(read("-0.0"), Ok(Duration::ZERO));
        assert_eq!(read("18446744073709551615.999999999"), Ok(Duration::MAX));
        for refused in ["-1", "0.0000000001", "18446744073709551616", "1e400"] {
            assert!(
                matches!(read(refused), Err(MapError::BadValue { .. })),
                "{refused}"
            );
        }
    }

    #[test]
    fn a_duration_reads_the_map_serde_writes() {
        let mut m = Map::new();
        m.set("secs", 3i64);
        m.set("nanos", 4i64);
        assert_eq!(Duration::from_value(&m.into()), Ok(Duration::new(3, 4)));
        let mut half = Map::new();
        half.set("secs", 3i64);
        assert!(matches!(
            Duration::from_value(&half.into()),
            Err(MapError::MissingKey { .. })
        ));
        let mut over = Map::new();
        over.set("secs", 3i64);
        over.set("nanos", 1_000_000_000i64);
        assert!(Duration::from_value(&over.into()).is_err());
        assert!(Duration::from_value(&Value::from("PT1S")).is_err());
        let three = vec![1u64, 2, 3].to_value().unwrap();
        assert!(Duration::from_value(&three).is_err());
        let bad = vec![-1i64, 0].to_value().unwrap();
        assert_eq!(Duration::from_value(&bad).unwrap_err().key(), "[0]");
    }

    #[test]
    fn a_span_is_its_iso_text() {
        let span = Span::new().days(1).hours(2);
        let v = check(span);
        assert_eq!(TryAsRef::<str>::try_as_ref(&v), Some("P1DT2H"));
        check(Span::new());
        check(Span::new().months(-3));
        assert_eq!(
            Span::from_value(&Value::from("1h 30m")),
            Ok(Span::new().hours(1).minutes(30))
        );
        let e = Span::from_value(&Value::from("P1X")).unwrap_err();
        assert!(e.to_string().contains("at byte 2"), "{e}");
        assert!(Span::from_value(&Value::from(5i64)).is_err());
        assert_eq!(text_at(&kind_of::<Span>(), "format"), Some("duration"));
    }

    #[derive(Debug, PartialEq, guatiao::ToValue, guatiao::FromValue, guatiao::Schema)]
    struct Timing {
        timeout: Duration,
        retry: Option<Duration>,
        every: Span,
        grace: Option<Span>,
        steps: Vec<Span>,
    }

    #[test]
    fn a_struct_of_them_derives_and_validates() {
        let t = Timing {
            timeout: Duration::from_millis(1500),
            retry: None,
            every: Span::new().weeks(1),
            grace: Some(Span::new().minutes(5)),
            steps: vec![Span::new().seconds(1), Span::new().seconds(2)],
        };
        let value = t.to_value().unwrap();
        assert_eq!(Timing::from_value(&value).unwrap(), t);
        let schema = Timing::schema().unwrap();
        validate_map(SchemaRef::new(&schema).unwrap(), &value).unwrap();
    }
}
