// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The whole-tree walks besides drop: a deep copy, and structural
//! equality. Each is one loop over a heap stack, so neither has a depth
//! limit and neither can overflow the thread's stack on a tree a foreign
//! producer made deep. A container supplies only its own steps.

#![forbid(unsafe_code)]

use super::alloc::Alloc;
use super::convert::TryAsRef;
use super::error::ValueError;
use super::types::{Buffer, Entry, List, Map, Number, Tag, Text, Value};

/// The live arm as `T`, for a walk that has already read the tag.
fn arm<T: ?Sized>(node: &Value) -> Result<&T, ValueError>
where
    Value: TryAsRef<T>,
{
    node.try_as_ref().ok_or(ValueError::WrongKind)
}

// --- copying ------------------------------------------------------------

/// A container being copied.
struct Copying<'a> {
    built: Built,
    left: Left<'a>,
    /// The key this container goes under in its parent, when the parent
    /// is a map.
    key: Option<&'a str>,
}

enum Built {
    List(List),
    Map(Map),
}

/// What is left of the source to copy.
enum Left<'a> {
    List(std::slice::Iter<'a, Value>),
    Map(std::slice::Iter<'a, Entry>),
}

impl Built {
    fn into_value(self) -> Value {
        match self {
            Built::List(list) => list.into(),
            Built::Map(map) => map.into(),
        }
    }
}

impl<'a> Copying<'a> {
    fn list(src: &'a List, key: Option<&'a str>, alloc: Alloc) -> Result<Self, ValueError> {
        Ok(Copying {
            built: Built::List(List::with_capacity_in(alloc, src.len())?),
            left: Left::List(src.iter()),
            key,
        })
    }

    fn map(src: &'a Map, key: Option<&'a str>, alloc: Alloc) -> Result<Self, ValueError> {
        Ok(Copying {
            built: Built::Map(Map::with_capacity_in(alloc, src.len())?),
            left: Left::Map(src.iter()),
            key,
        })
    }

    /// The next child, with the key it goes under when this is a map.
    fn next(&mut self) -> Option<(Option<&'a str>, &'a Value)> {
        match &mut self.left {
            Left::List(items) => items.next().map(|item| (None, item)),
            Left::Map(entries) => entries
                .next()
                .map(|entry| (Some(entry.key()), entry.value())),
        }
    }

    /// Adds a finished copy, under `key` when this is a map.
    fn attach(&mut self, key: Option<&str>, value: Value, alloc: Alloc) -> Result<(), ValueError> {
        match &mut self.built {
            Built::List(list) => list.push_node(value, alloc),
            Built::Map(map) => {
                let key = key.expect("a map's child is reached with its key");
                map.insert_node(key, value, alloc)
            }
        }
    }
}

/// A child is either copied on the spot or opened as a container to fill.
enum Step<'a> {
    Leaf(Value),
    Open(Copying<'a>),
}

/// A STRING or MAP under its tag that its door refuses is not UTF-8.
fn text<T: ?Sized>(node: &Value) -> Result<&T, ValueError>
where
    Value: TryAsRef<T>,
{
    node.try_as_ref().ok_or(ValueError::NotUtf8)
}

fn step<'a>(node: &'a Value, key: Option<&'a str>, alloc: Alloc) -> Result<Step<'a>, ValueError> {
    Ok(Step::Leaf(match node.tag()? {
        Tag::GUATIAO_LIST => return Ok(Step::Open(Copying::list(arm(node)?, key, alloc)?)),
        Tag::GUATIAO_MAP => return Ok(Step::Open(Copying::map(text(node)?, key, alloc)?)),
        Tag::GUATIAO_ABSENT => Value::absent(),
        Tag::GUATIAO_NULL => Value::null(),
        Tag::GUATIAO_BOOL => Value::from(*arm::<bool>(node)?),
        Tag::GUATIAO_STRING => text::<Text>(node)?.clone_in(alloc)?.into(),
        Tag::GUATIAO_NUMBER => arm::<Number>(node)?.clone_in(alloc)?.into(),
        Tag::GUATIAO_BYTES => arm::<Buffer>(node)?.clone_in(alloc)?.into(),
    }))
}

/// Fills `root` and every container under it. A failure part-way drops
/// the stack, and with it everything built so far.
fn copy_tree(root: Copying<'_>, alloc: Alloc) -> Result<Built, ValueError> {
    let mut stack = vec![root];
    loop {
        let top = stack
            .last_mut()
            .expect("the root leaves the stack only to return");
        match top.next() {
            Some((key, node)) => match step(node, key, alloc)? {
                Step::Leaf(copy) => top.attach(key, copy, alloc)?,
                Step::Open(container) => stack.push(container),
            },
            None => {
                let done = stack.pop().expect("the top was just read");
                match stack.last_mut() {
                    Some(parent) => parent.attach(done.key, done.built.into_value(), alloc)?,
                    None => return Ok(done.built),
                }
            }
        }
    }
}

/// A deep copy of `node` through `alloc`.
pub(crate) fn copy_value(node: &Value, alloc: Alloc) -> Result<Value, ValueError> {
    match step(node, None, alloc)? {
        Step::Leaf(copy) => Ok(copy),
        Step::Open(root) => Ok(copy_tree(root, alloc)?.into_value()),
    }
}

/// A deep copy of `list` through `alloc`.
pub(crate) fn copy_list(list: &List, alloc: Alloc) -> Result<List, ValueError> {
    match copy_tree(Copying::list(list, None, alloc)?, alloc)? {
        Built::List(copy) => Ok(copy),
        Built::Map(_) => unreachable!("a list's copy is a list"),
    }
}

/// A deep copy of `map` through `alloc`.
pub(crate) fn copy_map(map: &Map, alloc: Alloc) -> Result<Map, ValueError> {
    match copy_tree(Copying::map(map, None, alloc)?, alloc)? {
        Built::Map(copy) => Ok(copy),
        Built::List(_) => unreachable!("a map's copy is a map"),
    }
}

// --- equality -----------------------------------------------------------

/// Two things to compare, of whatever kind the walk has reached.
pub(crate) enum Pair<'a> {
    Values(&'a Value, &'a Value),
    Lists(&'a List, &'a List),
    Maps(&'a Map, &'a Map),
}

/// Whether `first`, and everything under it, is equal: the same kinds,
/// the same keys in the same order, the same bytes. A number is its text,
/// so `1.10` and `1.1` differ. A tag this build does not know is equal
/// exactly when the tags are.
///
/// Allocates only once a container is reached, so comparing two leaves
/// costs no allocation.
pub(crate) fn equal(first: Pair<'_>) -> bool {
    let mut stack = Vec::new();
    let mut next = Some(first);
    while let Some(pair) = next.take().or_else(|| stack.pop()) {
        let same = match pair {
            Pair::Values(a, b) => values(a, b, &mut stack),
            Pair::Lists(a, b) => {
                let same = a.len() == b.len();
                if same {
                    stack.extend(a.iter().zip(b.iter()).map(|(x, y)| Pair::Values(x, y)));
                }
                same
            }
            Pair::Maps(a, b) => {
                a.len() == b.len()
                    && a.iter().zip(b.iter()).all(|(x, y)| {
                        let same = x.key_bytes() == y.key_bytes();
                        if same {
                            stack.push(Pair::Values(x.value(), y.value()));
                        }
                        same
                    })
            }
        };
        if !same {
            return false;
        }
    }
    true
}

/// Compares two nodes, leaving any children on `stack` to compare next.
fn values<'a>(a: &'a Value, b: &'a Value, stack: &mut Vec<Pair<'a>>) -> bool {
    /// Both as `T`, by `T`'s own equality.
    fn same<T: ?Sized + PartialEq>(a: &Value, b: &Value) -> bool
    where
        Value: TryAsRef<T>,
    {
        matches!((a.try_as_ref(), b.try_as_ref()), (Some(x), Some(y)) if x == y)
    }

    if a.tag != b.tag {
        return false;
    }
    match a.tag() {
        Ok(Tag::GUATIAO_ABSENT | Tag::GUATIAO_NULL) | Err(_) => true,
        Ok(Tag::GUATIAO_BOOL) => same::<bool>(a, b),
        // By bytes, so a foreign text or number that is malformed still
        // equals itself.
        Ok(tag @ (Tag::GUATIAO_STRING | Tag::GUATIAO_NUMBER)) => {
            a.text_bytes(tag) == b.text_bytes(tag)
        }
        Ok(Tag::GUATIAO_BYTES) => same::<Buffer>(a, b),
        Ok(Tag::GUATIAO_LIST) => match (arm::<List>(a), arm::<List>(b)) {
            (Ok(x), Ok(y)) => {
                stack.push(Pair::Lists(x, y));
                true
            }
            _ => false,
        },
        Ok(Tag::GUATIAO_MAP) => match (a.map_arm(), b.map_arm()) {
            (Some(x), Some(y)) => {
                stack.push(Pair::Maps(x, y));
                true
            }
            _ => false,
        },
    }
}
