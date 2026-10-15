// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The standard traits the containers implement, each doing what the
//! standard library's own type does with it.

extern crate alloc;

use core::fmt::Write as _;
use std::collections::HashSet;
use std::io::Write as _;

use guatiao::{Buffer, List, Map, Number, Text, TryAsRef, Value};

fn text_of(value: &Value) -> Option<&str> {
    TryAsRef::<str>::try_as_ref(value)
}

#[test]
fn a_list_is_its_slice_of_values() {
    let mut list: List = ["b", "a", "c"].into_iter().collect();
    assert_eq!(list.len(), 3);
    assert!(!list.is_empty());
    assert_eq!(text_of(&list[0]), Some("b"));
    assert_eq!(list.get(1).and_then(text_of), Some("a"));
    assert!(list.get(9).is_none());
    assert_eq!(
        list.iter().filter_map(text_of).collect::<Vec<_>>(),
        ["b", "a", "c"]
    );

    // Mutably, the length is fixed and what is swapped moves whole.
    list.sort_by(|x, y| text_of(x).cmp(&text_of(y)));
    assert_eq!(
        list.iter().filter_map(text_of).collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
    list.swap(0, 2);
    assert_eq!(text_of(&list[0]), Some("c"));

    let slice: &[Value] = list.as_ref();
    assert_eq!(slice.len(), 3);
}

#[test]
fn a_list_hands_its_values_out_in_order() {
    let list: List = [1, 2, 3].into_iter().collect();
    let numbers: Vec<i64> = list
        .into_iter()
        .map(|v| i64::try_from(&v).unwrap())
        .collect();
    assert_eq!(numbers, [1, 2, 3]);

    let mut list: List = [1, 2].into_iter().collect();
    for value in &mut list {
        *value = Value::from(i64::try_from(&*value).unwrap() * 10);
    }
    assert_eq!(list, [10, 20].into_iter().collect::<List>());
}

#[test]
fn a_buffer_is_its_bytes() {
    let mut buffer = Buffer::new(b"abc");
    assert_eq!(&buffer[..], b"abc");
    assert_eq!(buffer.len(), 3);
    buffer[0] = b'x';
    assert_eq!(buffer.as_ref() as &[u8], b"xbc");

    write!(buffer, "{}", 12).unwrap();
    assert_eq!(&buffer[..], b"xbc12");
}

#[test]
fn text_takes_formatting_and_hands_out_its_bytes() {
    let mut text = Text::new("port ");
    write!(text, "{}", 5900).unwrap();
    assert_eq!(&*text, "port 5900");
    assert_eq!(text.as_ref() as &[u8], b"port 5900");
    assert_eq!(text.as_ref() as &str, "port 5900");
    assert_eq!(text.len(), 9, "the str's length, in bytes");
    assert!(text.starts_with("port"));
    assert_eq!(text.to_string(), "port 5900", "Display");
    assert_eq!(&*Text::default(), "");
}

#[test]
fn a_number_is_its_text() {
    let number: Number = "1.10".parse().unwrap();
    assert_eq!(<Number as AsRef<str>>::as_ref(&number), "1.10");
    assert_eq!(<Number as AsRef<[u8]>>::as_ref(&number), b"1.10");
}

#[test]
fn leaves_hash_by_their_bytes_as_they_compare() {
    let texts: HashSet<Text> = [Text::new("a"), Text::new("a"), Text::new("b")]
        .into_iter()
        .collect();
    assert_eq!(texts.len(), 2);

    let buffers: HashSet<Buffer> = [Buffer::new(b"x"), Buffer::new(b"x")].into_iter().collect();
    assert_eq!(buffers.len(), 1);

    // A number is its text, so `1.10` and `1.1` are two keys.
    let numbers: HashSet<Number> = ["1.10", "1.1", "1.10"]
        .into_iter()
        .map(|t| t.parse().unwrap())
        .collect();
    assert_eq!(numbers.len(), 2);
}

#[test]
fn a_map_is_indexed_by_key() {
    let map: Map = [("host", "h"), ("port", "p")].into_iter().collect();
    assert_eq!(text_of(&map["host"]), Some("h"));
}

#[test]
#[should_panic(expected = "no value under `missing`")]
fn indexing_a_missing_key_panics_as_hashmap_does() {
    let map = Map::new();
    let _ = &map["missing"];
}

#[test]
fn a_map_hands_its_pairs_out_in_order() {
    let map: Map = [("b", 1), ("a", 2)].into_iter().collect();
    let pairs: Vec<(String, i64)> = map
        .into_iter()
        .map(|(k, v)| (k.to_string(), i64::try_from(&v).unwrap()))
        .collect();
    assert_eq!(pairs, [("b".to_string(), 1), ("a".to_string(), 2)]);

    let mut map: Map = [("n", 1), ("m", 2)].into_iter().collect();
    for (key, value) in &mut map {
        if &**key == "n" {
            *value = Value::from(7);
        }
    }
    assert_eq!(i64::try_from(&map["n"]).unwrap(), 7);
    let keys: Vec<&str> = map.iter_mut().rev().map(|(k, _)| &**k).collect();
    assert_eq!(
        keys,
        ["m", "n"],
        "the keys came through unchanged, in order"
    );
}

#[test]
fn the_views_and_the_owned_types_convert_as_std_does() {
    use guatiao::value::types::Bytes;
    use guatiao::{Buffer, Str};

    let view: Str = "port".into();
    assert_eq!(view.len(), 4);
    let bytes: Bytes = (b"ab" as &'static [u8]).into();
    assert_eq!(bytes.len(), 2);

    let text: Text = String::from("host").into();
    assert_eq!(String::from(text), "host");
    let buffer: Buffer = vec![0u8, 255].into();
    assert_eq!(Vec::<u8>::from(buffer), [0, 255]);
}

#[test]
fn a_text_changes_in_place_as_a_string_does() {
    let mut text = Text::new("port");
    text.make_ascii_uppercase();
    assert_eq!(&*text, "PORT");
    AsMut::<str>::as_mut(&mut text).make_ascii_lowercase();
    assert_eq!(&*text, "port");
    let mut empty = Text::default();
    assert_eq!(&mut *empty, "");
}

#[test]
fn owned_keys_are_searched_by_their_borrowed_form() {
    use guatiao::Buffer;
    use std::collections::HashMap;

    let texts: HashMap<Text, i32> = [(Text::new("a"), 1)].into_iter().collect();
    assert_eq!(texts.get("a"), Some(&1));
    let numbers: HashMap<Number, i32> = [(Number::new("1.10").unwrap(), 1)].into_iter().collect();
    assert_eq!(numbers.get("1.10"), Some(&1));
    assert_eq!(numbers.get("1.1"), None, "a number is its text");
    let buffers: HashMap<Buffer, i32> = [(Buffer::new(b"ab"), 1)].into_iter().collect();
    assert_eq!(buffers.get(&b"ab"[..]), Some(&1));
}

#[test]
fn a_view_borrows_what_it_views() {
    use guatiao::Str;
    use guatiao::value::types::Bytes;

    let owned = String::from("host");
    let view = Str::new(&owned);
    assert_eq!(view.len(), 4);
    assert_eq!(<&str>::from(view), "host");
    let checked: Str<'_> = core::str::from_utf8(b"host").unwrap().into();
    assert_eq!(<&str>::from(checked), "host");
    let bad = [b'h', 0xff];
    // SAFETY: a readable array that outlives the call.
    let from_c = unsafe { Str::from_ptr(&Str::from_raw_parts(bad.as_ptr(), bad.len())) };
    assert_eq!(
        from_c.map(|_| ()),
        Err(guatiao::ValueError::NotUtf8),
        "refused"
    );
    let bytes = Bytes::new(owned.as_bytes());
    assert_eq!(<&[u8]>::from(bytes), b"host");
    assert!(<&[u8]>::from(Bytes::empty()).is_empty());
}

#[test]
fn what_c_points_at_is_checked_on_the_way_in() {
    use guatiao::Buffer;

    let good = Text::new("1.5");
    // SAFETY: a live text.
    assert_eq!(unsafe { Text::from_ptr(&good) }.map(|t| &**t), Ok("1.5"));
    // SAFETY: a live text; a `Number` is a `Text` in layout.
    let as_number = unsafe { Number::from_ptr((&good as *const Text).cast()) };
    assert_eq!(as_number.map(|n| &**n), Ok("1.5"));

    let not_a_number = Text::new("1,5");
    // SAFETY: as above.
    let refused = unsafe { Number::from_ptr((&not_a_number as *const Text).cast()) };
    assert_eq!(refused.map(|_| ()), Err(guatiao::ValueError::NotANumber));

    // A text a foreign producer wrote, not UTF-8.
    let (ptr, len, cap, alloc) = Buffer::new(&[b'h', 0xff]).into_raw_parts();
    // SAFETY: consistent storage; its bytes are what is under test.
    let foreign = unsafe { Text::from_raw_parts(ptr, len, cap, alloc) };
    // SAFETY: a live text.
    assert_eq!(
        unsafe { Text::from_ptr(&foreign) }.map(|_| ()),
        Err(guatiao::ValueError::NotUtf8)
    );
}

/// `from_ptr` and the grammar behind `Number` run at compile time.
#[test]
fn the_checked_reads_are_const() {
    use guatiao::{Str, ValueError};

    const HOST: Str<'static> = Str::new("host");
    // SAFETY: a live view over a literal.
    const READ: Result<Str<'static>, ValueError> = unsafe { Str::from_ptr(&HOST) };
    assert!(matches!(READ, Ok(view) if view.len() == 4));
    // SAFETY: null is an empty view.
    const NULL: Result<Str<'static>, ValueError> = unsafe { Str::from_ptr(core::ptr::null()) };
    assert!(matches!(NULL, Ok(view) if view.is_empty()));
}

/// `clone()` cannot report content this build refuses, so it panics on it
/// as documented; `clone_in` reports it, and equality still holds.
#[test]
fn clone_panics_on_refused_content_and_clone_in_reports_it() {
    use guatiao::value::Payload;
    use guatiao::{Alloc, ValueError};

    // SAFETY: an unknown tag over a fully initialised payload that owns
    // nothing: the way a newer producer's kind arrives.
    let unknown = unsafe { Value::from_raw_parts(99, Payload::bool(false)) };
    let mut list = List::new();
    list.push(unknown);
    let tree = Value::from(list);

    assert_eq!(
        tree.clone_in(Alloc::rust()).map(|_| ()),
        Err(ValueError::UnknownTag(99))
    );
    let caught = std::panic::catch_unwind(|| tree.clone());
    let message = caught.expect_err("clone panics on it");
    let message = message
        .downcast_ref::<String>()
        .map(String::as_str)
        .unwrap_or("");
    assert!(message.contains("clone_in"), "{message}");
    assert!(tree == tree);
}

/// A text and a number compare with a `str` either way round, a text
/// orders as its `str` does, and an entry clones and compares whole.
#[test]
fn text_compares_and_orders_as_str_and_an_entry_clones() {
    use alloc::collections::BTreeMap;
    use guatiao::Entry;

    let text = Text::new("host");
    assert!(text == "host" && "host" == text);
    assert!(text == *"host" && *"host" == text);
    assert!(text != "port");
    let number = Number::new("1.10").unwrap();
    assert!(number == "1.10" && "1.10" == number);
    assert!(number != "1.1", "a number compares as the text it is");

    let mut sorted: Vec<Text> = ["b", "a", "\u{e9}", "B"].map(Text::new).into();
    sorted.sort();
    assert_eq!(sorted, ["B", "a", "b", "\u{e9}"]);
    let by_key: BTreeMap<Text, i32> = [(Text::new("z"), 1), (Text::new("m"), 2)].into();
    assert_eq!(by_key.keys().map(|k| &**k).collect::<Vec<_>>(), ["m", "z"]);
    assert_eq!(by_key.get("z"), Some(&1), "searched by its borrowed form");

    let entry = Entry::new(Text::new("k"), Value::from(1));
    let copy = entry.clone();
    assert_eq!(copy, entry);
    assert_ne!(copy, Entry::new(Text::new("k"), Value::from(2)));
}
