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
use guatiao::{Alloc, Buffer, FromValue, List, Map, MapError, Number, Schema, Text, ToValue, Value};

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
