// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The path grammar: what parses, what does not, and where it says the
//! trouble is.
//!
//! Every malformation is refused **at its byte offset**, the same
//! contract the wire decoder has, because a path comes from a command
//! line or a stored record and "that path is wrong" without a position is
//! a bug report nobody can act on.

use std::borrow::Cow;

use guatiao::path::{self, PathError, Segment};
use guatiao::{List, Map, Value};

fn segments(text: &str) -> Vec<Segment<'_>> {
    path::parse(text)
        .unwrap_or_else(|e| panic!("`{text}` should parse: {e}"))
        .segments()
        .collect()
}

fn field(name: &str) -> Segment<'_> {
    Segment::Field(name)
}

fn key(text: &str) -> Segment<'_> {
    Segment::Key(Cow::Borrowed(text))
}

/// The charter's own example, segment by segment.
#[test]
fn the_shape_the_charter_asked_for() {
    assert_eq!(
        segments("agent[1].name[name2].value"),
        [
            field("agent"),
            Segment::Index(1),
            field("name"),
            key("name2"),
            field("value"),
        ]
    );
}

#[test]
fn one_field_is_a_path() {
    assert_eq!(segments("host"), [field("host")]);
}

/// A bracket may open a path, for a value that is a list or a map at its
/// root. A schema's root is an object, so this is for walking a value.
#[test]
fn a_path_may_start_with_a_bracket() {
    assert_eq!(segments("[0].host"), [Segment::Index(0), field("host")]);
}

/// Digits and only digits are a position. Everything else in brackets is
/// a key, and a key is never read as a position.
#[test]
fn what_counts_as_an_index() {
    assert_eq!(segments("a[0]"), [field("a"), Segment::Index(0)]);
    assert_eq!(segments("a[007]"), [field("a"), Segment::Index(7)]);
    assert_eq!(segments("a[+1]"), [field("a"), key("+1")]);
    assert_eq!(segments("a[ 1]"), [field("a"), key(" 1")]);
    assert_eq!(segments("a[1x]"), [field("a"), key("1x")]);
    assert_eq!(
        segments("a[\"1\"]"),
        [field("a"), key("1")],
        "quoting is what says `a key, never a position`"
    );
}

/// A `.` inside brackets needs no quoting: the `]` already says where the
/// segment ends. A `[`, a `]` or a leading `\"` is what quoting is for.
#[test]
fn a_key_may_hold_a_delimiter() {
    assert_eq!(segments("env[a.b]"), [field("env"), key("a.b")]);
    assert_eq!(segments("env[\"a.b\"]"), [field("env"), key("a.b")]);
    assert_eq!(segments("env[\"a[b]\"]"), [field("env"), key("a[b]")]);
    assert_eq!(
        segments(r#"env["say \"hi\""]"#),
        [field("env"), Segment::Key(Cow::Owned("say \"hi\"".into()))]
    );
    assert_eq!(
        segments(r#"env["c:\\tmp"]"#),
        [field("env"), Segment::Key(Cow::Owned("c:\\tmp".into()))]
    );
}

/// Every refusal, by its kind and its offset.
#[test]
fn every_malformation_says_where() {
    let refused = |text: &str| path::parse(text).expect_err(&format!("`{text}` is not a path"));

    assert_eq!(refused(""), PathError::Empty);
    assert_eq!(refused(".a"), PathError::EmptyField { at: 0 });
    assert_eq!(refused("a..b"), PathError::EmptyField { at: 2 });
    assert_eq!(refused("a."), PathError::EmptyField { at: 2 });
    assert_eq!(refused("a[]"), PathError::EmptyIndex { at: 1 });
    assert_eq!(refused("a[0"), PathError::UnclosedIndex { at: 1 });
    assert_eq!(refused("a]"), PathError::UnexpectedClose { at: 1 });
    assert_eq!(refused("a[\"k]"), PathError::UnclosedQuote { at: 2 });
    assert_eq!(refused("a[\"k\"x]"), PathError::TrailingQuote { at: 5 });
    // The offset is the `\` itself: `a [ " k \ x " ]` puts it at 4.
    assert_eq!(refused(r#"a["k\x"]"#), PathError::BadEscape { at: 4 });
    assert_eq!(refused(r#"a["k\"#), PathError::BadEscape { at: 4 });

    // And the offset is reachable without matching on the variant.
    assert_eq!(refused("a..b").offset(), 2);
}

/// A segment renders as the spelling it parses back from, which is what
/// `flatten` builds a store key out of. A key that would read as
/// something else is quoted, and nothing else is.
#[test]
fn a_segment_renders_as_what_it_parses_from() {
    let render = |s: Segment<'_>| s.to_string();

    assert_eq!(render(field("host")), "host");
    assert_eq!(render(Segment::Index(3)), "[3]");
    assert_eq!(render(key("name2")), "[name2]");
    assert_eq!(render(key("a.b")), "[a.b]");
    assert_eq!(
        render(key("1")),
        "[\"1\"]",
        "or it would read as a position"
    );
    assert_eq!(render(key("")), "[\"\"]");
    assert_eq!(render(key("a[b]")), "[\"a[b]\"]");
    assert_eq!(
        render(key("say \"hi\"")),
        "[say \"hi\"]",
        "only a LEADING quote opens a quoted key, so this one needs none"
    );
    assert_eq!(render(key("\"hi\"")), r#"["\"hi\""]"#, "this one does");

    // Rendered, then parsed, is the same segment.
    for original in [
        key("1"),
        key(""),
        key("a[b]"),
        key("say \"hi\""),
        key("\"hi\""),
        key("a.b"),
    ] {
        let text = format!("x{original}");
        assert_eq!(
            segments(&text)[1],
            original,
            "`{text}` should parse back to what rendered it"
        );
    }
}

/// The path itself keeps the text it was parsed from.
#[test]
fn a_path_keeps_its_text() {
    let parsed = path::parse("agent[1].name").expect("a path");
    assert_eq!(parsed.as_str(), "agent[1].name");
    assert_eq!(parsed.to_string(), "agent[1].name");
}

/// One segment off the front, and the rest as a path of its own: what a
/// sub-form over one field's members needs.
#[test]
fn a_path_splits_into_a_head_and_the_rest() {
    let parsed = path::parse("agent[1].name").expect("a path");
    let (first, rest) = parsed.split_first().expect("a path has a first segment");
    assert_eq!(first, field("agent"));
    let rest = rest.expect("there is more");
    assert_eq!(rest.as_str(), "[1].name");

    let (first, rest) = rest.split_first().expect("still a path");
    assert_eq!(first, Segment::Index(1));
    assert_eq!(rest.expect("one more").as_str(), "name");

    let last = path::parse("name").expect("a path");
    assert_eq!(last.split_first().expect("a segment").1, None);
}

/// A tree three containers deep, walked.
fn tree() -> Value {
    let mut inner = Map::new();
    inner.set("host", "10.0.0.1").unwrap();
    inner.set("port", 5900i64.to_string().as_str()).unwrap();

    let mut env = Map::new();
    env.set("PATH", "/bin").unwrap();
    env.set("0", "a key that looks like a position").unwrap();
    inner.set("env", env).unwrap();

    let mut agents = List::new();
    agents.push(inner).unwrap();

    let mut root = Map::new();
    root.set("agent", agents).unwrap();
    root.into()
}

#[test]
fn walking_finds_what_is_there() {
    let value = tree();
    let at = |text: &str| {
        path::get(&value, path::parse(text).expect("a path"))
            .and_then(|v| TryInto::<&str>::try_into(v).ok())
    };

    assert_eq!(at("agent[0].host"), Some("10.0.0.1"));
    assert_eq!(at("agent[0].env[PATH]"), Some("/bin"));
    assert_eq!(
        at("agent[0].env[0]"),
        Some("a key that looks like a position"),
        "a number applied to a map is that map's key"
    );
    assert_eq!(
        at("agent[0].env[\"0\"]"),
        Some("a key that looks like a position"),
        "and quoting says so outright"
    );
}

#[test]
fn walking_answers_none_for_everything_that_is_not_there() {
    let value = tree();
    let missing = |text: &str| path::get(&value, path::parse(text).expect("a path")).is_none();

    assert!(missing("nothing"));
    assert!(missing("agent[1]"), "past the end of the list");
    assert!(missing("agent[0].host.more"), "a text has no members");
    assert!(missing("agent[0].env[NOPE]"));
    assert!(missing("agent[first]"), "a key applied to a list");
}

/// Writing through a path changes the tree in place, and creates nothing.
#[test]
fn writing_through_a_path_changes_what_is_there() {
    let mut value = tree();
    let at = path::parse("agent[0].env[PATH]").expect("a path");
    *path::get_mut(&mut value, at).expect("it is there") = Value::from(true);
    assert_eq!(
        path::get(&value, at).map(|v| v.tag()),
        Some(Ok(guatiao::Tag::GUATIAO_BOOL))
    );

    let absent = path::parse("agent[0].env[NEW]").expect("a path");
    assert!(
        path::get_mut(&mut value, absent).is_none(),
        "a path names a place; it does not make one"
    );
}

/// A path as deep as its text, which is what a sender controls. This one
/// is far past `MAX_DEPTH`: walking loops, so there is no stack to run
/// out of.
#[test]
fn a_path_deeper_than_any_recursion_would_survive() {
    let depth = 10_000;
    let mut value = Value::from("leaf");
    for _ in 0..depth {
        let mut wrap = Map::new();
        wrap.set("n", value).unwrap();
        value = wrap.into();
    }

    let text = std::iter::repeat_n("n", depth)
        .collect::<Vec<_>>()
        .join(".");
    let parsed = path::parse(&text).expect("a long path is still a path");
    assert_eq!(parsed.segments().count(), depth);
    assert_eq!(
        path::get(&value, parsed).and_then(|v| TryInto::<&str>::try_into(v).ok()),
        Some("leaf")
    );
}
