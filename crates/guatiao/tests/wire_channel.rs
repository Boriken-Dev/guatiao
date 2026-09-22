// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A channel: the schema arrives first, and every value after it is
//! checked against it before the receiver sees it.

use guatiao::schema::{FieldBuilder, KindBuilder, SchemaBuilder};
use guatiao::value::convert::TryAsRef;
use guatiao::value::wire::WireError;
use guatiao::value::wire::channel::{self, Received, Receiver};
use guatiao::{Map, Value};

fn greeting_schema() -> Value {
    SchemaBuilder::new()
        .field(FieldBuilder::new("greeting", KindBuilder::string()).required())
        .finish()
        .expect("a schema")
}

fn greeting(text: impl Into<Value>) -> Value {
    let mut map = Map::new();
    map.set("greeting", text).unwrap();
    map.into()
}

#[test]
fn a_value_arrives_checked_against_its_channels_schema() {
    let mut receiver = Receiver::new();
    let announced = receiver
        .accept(&channel::announce(7, &greeting_schema()).unwrap())
        .unwrap();
    assert!(matches!(announced, Received::Schema(7)));

    match receiver
        .accept(&channel::send(7, &greeting("hello")).unwrap())
        .unwrap()
    {
        Received::Value { channel, value } => {
            assert_eq!(channel, 7);
            assert_eq!(
                TryAsRef::<Map>::try_as_ref(&value)
                    .and_then(|m| m.get("greeting"))
                    .and_then(TryAsRef::<str>::try_as_ref),
                Some("hello")
            );
        }
        other => panic!("{other:?}"),
    }

    // A value without the field the schema requires: refused, by name.
    // (A scalar is checked by its text form, so `5` under a string field
    // would pass: the text "5" is a string.)
    let refused = receiver
        .accept(&channel::send(7, &Map::new().into()).unwrap())
        .unwrap_err();
    assert!(
        matches!(&refused, WireError::Invalid { channel: 7, .. }),
        "{refused:?}"
    );
    assert!(refused.to_string().contains("greeting"), "{refused}");
}

#[test]
fn a_value_before_its_schema_is_refused() {
    let mut receiver = Receiver::new();
    receiver
        .accept(&channel::announce(7, &greeting_schema()).unwrap())
        .unwrap();
    let refused = receiver
        .accept(&channel::send(8, &greeting("hello")).unwrap())
        .unwrap_err();
    assert_eq!(refused, WireError::NoSchema { channel: 8 });
}

#[test]
fn closing_a_channel_forgets_its_schema() {
    let mut receiver = Receiver::new();
    receiver
        .accept(&channel::announce(3, &greeting_schema()).unwrap())
        .unwrap();
    assert!(matches!(
        receiver.accept(&channel::close(3)).unwrap(),
        Received::Closed(3)
    ));
    assert!(receiver.schema(3).is_none());
    assert_eq!(
        receiver
            .accept(&channel::send(3, &greeting("hello")).unwrap())
            .unwrap_err(),
        WireError::NoSchema { channel: 3 }
    );
}

#[test]
fn a_schema_frame_must_carry_a_schema() {
    let mut receiver = Receiver::new();
    let refused = receiver
        .accept(&channel::announce(1, &Value::from(5i64)).unwrap())
        .unwrap_err();
    assert_eq!(refused, WireError::NotASchema { channel: 1 });
}

/// An offset in an error names a byte of the frame, not of its payload.
#[test]
fn offsets_name_bytes_of_the_whole_frame() {
    let mut receiver = Receiver::new();
    receiver
        .accept(&channel::announce(300, &greeting_schema()).unwrap())
        .unwrap();
    // kind, a two-byte channel (300), then a payload whose own byte 0 is
    // an unknown tag: byte 3 of the frame.
    let frame = [channel::VALUE, 0xac, 0x02, 9];
    assert_eq!(
        receiver.accept(&frame).unwrap_err(),
        WireError::UnknownTag { at: 3, tag: 9 }
    );
    assert_eq!(
        receiver.accept(&[9, 1]).unwrap_err(),
        WireError::UnknownFrame { at: 0, kind: 9 }
    );
    assert_eq!(
        receiver.accept(&[]).unwrap_err(),
        WireError::Truncated { at: 0 }
    );
}
