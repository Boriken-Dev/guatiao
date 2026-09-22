// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The wire encoding: what goes in comes out, exactly, and what a sender
//! can get wrong is refused with the byte it went wrong at.

use guatiao::value::convert::TryAsRef;
use guatiao::value::error::MAX_DEPTH;
use guatiao::value::wire::{WireError, decode, encode, to_bytes};
use guatiao::{Buffer, List, Map, Number, Text, Value, ValueError};

fn round_trip(value: &Value) -> Value {
    let bytes = to_bytes(value).expect("encodes");
    let back = decode(&bytes).expect("decodes");
    assert_eq!(&back, value, "the same tree comes back");
    back
}

#[test]
fn every_tag_comes_back_as_itself() {
    let mut inner = List::new();
    inner.push(Value::null()).unwrap();
    inner.push(Value::absent()).unwrap();
    let mut map = Map::new();
    map.set("absent", Value::absent()).unwrap();
    map.set("null", Value::null()).unwrap();
    map.set("true", true).unwrap();
    map.set("false", false).unwrap();
    map.set("number", Number::new("1.10").unwrap()).unwrap();
    map.set("string", "hello").unwrap();
    map.set("bytes", Buffer::new(&[0x00, 0xff])).unwrap();
    map.set("list", inner).unwrap();
    map.set("empty list", List::new()).unwrap();
    map.set("empty map", Map::new()).unwrap();
    let back = round_trip(&map.into());

    let back = TryAsRef::<Map>::try_as_ref(&back).unwrap();
    assert_eq!(
        back.get("number")
            .and_then(TryAsRef::<Number>::try_as_ref)
            .map(|n| &**n),
        Some("1.10"),
        "a number is its text"
    );
    assert_eq!(
        back.get("bytes").and_then(TryAsRef::<[u8]>::try_as_ref),
        Some(&[0x00, 0xff][..]),
        "bytes are bytes, never text"
    );
    assert_eq!(
        back.get("string").and_then(TryAsRef::<[u8]>::try_as_ref),
        None,
        "and text is never bytes"
    );
}

#[test]
fn a_number_crosses_as_its_own_text() {
    let big = "9".repeat(200);
    round_trip(&Number::new(&big).unwrap().into());
    round_trip(&Number::new("-2.5E-3").unwrap().into());
}

#[test]
fn a_key_with_a_nul_and_a_thousand_keys_keep_their_order() {
    let mut map = Map::new();
    map.set("a\0b", 1).unwrap();
    for i in 0..1000 {
        map.set(&format!("k{}", 999 - i), i).unwrap();
    }
    let back = round_trip(&map.into());
    let back = TryAsRef::<Map>::try_as_ref(&back).unwrap();
    let keys: Vec<&str> = back.keys().take(3).collect();
    assert_eq!(keys, ["a\0b", "k999", "k998"], "insertion order");
}

#[test]
fn a_tree_deeper_than_any_recursive_walk_decodes() {
    let mut value: Value = List::new().into();
    for _ in 0..MAX_DEPTH * 4 {
        let mut outer = List::new();
        outer.push(value).unwrap();
        value = outer.into();
    }
    round_trip(&value);
}

#[test]
fn the_bytes_are_the_documented_ones() {
    let mut map = Map::new();
    map.set("n", Number::new("1.5").unwrap()).unwrap();
    map.set("b", true).unwrap();
    assert_eq!(
        to_bytes(&map.into()).unwrap(),
        [7, 2, 1, b'n', 3, 3, b'1', b'.', b'5', 1, b'b', 2, 1],
        "map of 2; key n, NUMBER \"1.5\"; key b, BOOL 1"
    );
}

/// Each way a sender can go wrong, refused at the byte it went wrong at.
#[test]
fn what_is_malformed_is_refused_where_it_is() {
    let cases: &[(&[u8], WireError)] = &[
        (&[], WireError::Truncated { at: 0 }),
        (&[9], WireError::UnknownTag { at: 0, tag: 9 }),
        (&[4, 5, b'a'], WireError::Truncated { at: 2 }),
        (&[4, 0x80, 0x00], WireError::Overlong { at: 1 }),
        (&[4, 2, b'a', 0xff], WireError::NotUtf8 { at: 1 }),
        (&[3, 2, b'0', b'1'], WireError::NotANumber { at: 1 }),
        (&[2, 7], WireError::NotABool { at: 1, byte: 7 }),
        (&[7, 1, 1, 0xfe, 1], WireError::NotUtf8 { at: 2 }),
        (
            &[7, 2, 1, b'k', 1, 1, b'k', 1],
            WireError::DuplicateKey { at: 5 },
        ),
        (&[1, 1], WireError::Trailing { at: 1 }),
        (&[6, 3, 1], WireError::Truncated { at: 2 }),
        (
            &[
                4, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f,
            ],
            WireError::Overlong { at: 1 },
        ),
    ];
    for (bytes, expected) in cases {
        assert_eq!(decode(bytes).unwrap_err(), *expected, "{bytes:?}");
    }
}

/// A node its own door refuses cannot be sent: the tree a foreign
/// producer broke stays on its side.
#[test]
fn a_string_a_foreign_producer_wrote_that_is_not_utf8_is_not_sent() {
    use guatiao::value::types::{Payload, Tag};

    let (ptr, len, cap, alloc) = Buffer::new(&[b'a', 0xff]).into_raw_parts();
    // SAFETY: consistent storage; its bytes are the point.
    let text = unsafe { Text::from_raw_parts(ptr, len, cap, alloc) };
    // SAFETY: a STRING tag over the text arm.
    let bad = unsafe { Value::from_raw_parts(u32::from(Tag::GUATIAO_STRING), Payload::text(text)) };
    let mut out = Vec::new();
    assert_eq!(encode(&bad, &mut out), Err(ValueError::NotUtf8));
}

/// Anything at all, decoded: an answer or a refusal, never a panic.
#[test]
fn ten_thousand_random_inputs_decode_without_a_panic() {
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..10_000 {
        let len = (next() % 64) as usize;
        // Biased towards small bytes, so tags and short lengths are common
        // and the decoder gets past the first byte most of the time.
        let bytes: Vec<u8> = (0..len).map(|_| (next() % 12) as u8).collect();
        let _ = decode(&bytes);
    }
}
