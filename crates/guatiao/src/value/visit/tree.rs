// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The tree as one implementation among others: a value walks itself,
//! `&Value` is a walker, a value is read from any walk, and
//! [`TreeSink`] builds one from events in the allocator it is given.

#![forbid(unsafe_code)]

use alloc::string::String;
use alloc::vec::Vec;

use super::{
    Build, FromWalk, ListVisitor, ListWalker, MapVisitor, MapWalker, Seed, Sink, Visitor, Walk,
    Walker,
};
use crate::value::alloc::Alloc;
use crate::value::convert::{Kinds, MapError, TryAsRef, copied};
use crate::value::error::{MAX_DEPTH, ValueError};
use crate::value::types::map::Filling;
use crate::value::types::{Buffer, Entry, List, Map, Number, Tag, Text, Value};
use crate::value::wire::keys::Seeded;

/// How many elements a length hint may reserve room for.
const RESERVE: usize = 1024;

// --- a value walks itself --------------------------------------------------

/// A node and how many containers are open above it.
struct Deep<'a>(&'a Value, u32);

/// Refuses opening a container past [`MAX_DEPTH`].
fn room(depth: u32) -> Result<u32, ValueError> {
    if depth >= MAX_DEPTH {
        return Err(ValueError::TooDeep);
    }
    Ok(depth + 1)
}

impl Walk for Deep<'_> {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        let Deep(node, depth) = *self;
        // A node a foreign producer wrote is refused as the wire encoder
        // refuses it: an unknown tag, or a payload its door refuses.
        match node.tag()? {
            Tag::GUATIAO_ABSENT => visitor.absent(),
            Tag::GUATIAO_NULL => visitor.null(),
            Tag::GUATIAO_BOOL => {
                let b = TryAsRef::<bool>::try_as_ref(node).ok_or(ValueError::WrongKind)?;
                visitor.bool(*b)
            }
            Tag::GUATIAO_NUMBER => {
                let n = TryAsRef::<Number>::try_as_ref(node).ok_or(ValueError::NotANumber)?;
                visitor.number(n)
            }
            Tag::GUATIAO_STRING => {
                visitor.string(TryAsRef::<str>::try_as_ref(node).ok_or(ValueError::NotUtf8)?)
            }
            Tag::GUATIAO_BYTES => {
                visitor.bytes(TryAsRef::<[u8]>::try_as_ref(node).ok_or(ValueError::WrongKind)?)
            }
            Tag::GUATIAO_LIST => {
                let list = TryAsRef::<List>::try_as_ref(node).ok_or(ValueError::WrongKind)?;
                walk_list(list, depth, visitor)
            }
            Tag::GUATIAO_MAP => {
                let map = TryAsRef::<Map>::try_as_ref(node).ok_or(ValueError::NotUtf8)?;
                walk_map(map, depth, visitor)
            }
        }
    }
}

fn walk_list<V: Visitor>(list: &List, depth: u32, visitor: V) -> Result<V::Ok, V::Error> {
    let inside = room(depth)?;
    let mut items = visitor.list(Some(list.len()))?;
    for item in list.iter() {
        items.item(&Deep(item, inside))?;
    }
    items.end()
}

fn walk_map<V: Visitor>(map: &Map, depth: u32, visitor: V) -> Result<V::Ok, V::Error> {
    let inside = room(depth)?;
    let mut entries = visitor.map(Some(map.len()))?;
    for entry in map.iter() {
        entries.entry(entry.key(), &Deep(entry.value(), inside))?;
    }
    entries.end()
}

/// Every node, as the wire encoder writes it. A tree nested deeper than
/// [`MAX_DEPTH`] is refused with [`ValueError::TooDeep`], since a walk
/// recurses once per container.
impl Walk for Value {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        Deep(self, 0).walk(visitor)
    }
}

impl Walk for Map {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        walk_map(self, 0, visitor)
    }
}

impl Walk for List {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        walk_list(self, 0, visitor)
    }
}

impl Walk for Text {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        visitor.string(self)
    }
}

impl Walk for Number {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        visitor.number(self)
    }
}

impl Walk for Buffer {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        visitor.bytes(self)
    }
}

// --- a value as a walker ---------------------------------------------------------

/// A borrowed node to read from, and the containers open above it.
#[derive(Clone, Copy)]
struct Node<'a> {
    value: &'a Value,
    depth: u32,
}

/// What a node of kind `tag` whose payload cannot be read is refused
/// with: the wrong kind for a builder that does not take it, else the
/// content, as the tree readers refuse it.
fn malformed(kinds: Kinds, tag: Tag) -> MapError {
    if !kinds.contains(tag) {
        return MapError::wrong_kind(kinds, tag);
    }
    MapError::bad_value(match tag {
        Tag::GUATIAO_STRING => "text that is UTF-8",
        Tag::GUATIAO_NUMBER => "a number in the JSON number grammar",
        Tag::GUATIAO_MAP => "a map whose keys are text that is UTF-8",
        _ => "a value whose payload matches its kind",
    })
}

impl Walker for Node<'_> {
    fn build<B: Build>(self, build: B) -> Result<B::Out, MapError> {
        let node = self.value;
        let Ok(tag) = node.tag() else {
            return Err(MapError::bad_value(alloc::format!(
                "{}, not a kind this build does not know",
                build.kinds()
            )));
        };
        let bad = |build: &B| malformed(build.kinds(), tag);
        match tag {
            Tag::GUATIAO_ABSENT => build.absent(),
            Tag::GUATIAO_NULL => build.null(),
            Tag::GUATIAO_BOOL => match TryAsRef::<bool>::try_as_ref(node) {
                Some(b) => build.bool(*b),
                None => Err(bad(&build)),
            },
            Tag::GUATIAO_NUMBER => match TryAsRef::<Number>::try_as_ref(node) {
                Some(n) => build.number(n),
                None => Err(bad(&build)),
            },
            Tag::GUATIAO_STRING => match TryAsRef::<str>::try_as_ref(node) {
                Some(text) => build.string(text),
                None => Err(bad(&build)),
            },
            Tag::GUATIAO_BYTES => match TryAsRef::<[u8]>::try_as_ref(node) {
                Some(bytes) => build.bytes(bytes),
                None => Err(bad(&build)),
            },
            Tag::GUATIAO_LIST => match TryAsRef::<List>::try_as_ref(node) {
                Some(list) => build.list(Items {
                    items: list.iter(),
                    depth: deeper(self.depth)?,
                }),
                None => Err(bad(&build)),
            },
            Tag::GUATIAO_MAP => match TryAsRef::<Map>::try_as_ref(node) {
                Some(map) => build.map(Entries {
                    entries: map.iter(),
                    pending: None,
                    depth: deeper(self.depth)?,
                }),
                None => Err(bad(&build)),
            },
        }
    }
}

/// The depth inside a container opened at `depth`, or the refusal.
fn deeper(depth: u32) -> Result<u32, MapError> {
    room(depth).map_err(|_| {
        MapError::bad_value(alloc::format!(
            "a value nested no deeper than {MAX_DEPTH} levels"
        ))
    })
}

/// A value as the source of a read: what [`FromValue`](crate::FromValue)
/// reads, reached through a walk.
impl Walker for &Value {
    fn build<B: Build>(self, build: B) -> Result<B::Out, MapError> {
        Node {
            value: self,
            depth: 0,
        }
        .build(build)
    }
}

struct Items<'a> {
    items: core::slice::Iter<'a, Value>,
    depth: u32,
}

impl ListWalker for Items<'_> {
    fn len_hint(&self) -> Option<usize> {
        Some(self.items.len())
    }

    fn next_seed<S: Seed>(&mut self, seed: S) -> Result<Option<S::Out>, MapError> {
        match self.items.next() {
            Some(value) => seed
                .read(Node {
                    value,
                    depth: self.depth,
                })
                .map(Some),
            None => Ok(None),
        }
    }
}

struct Entries<'a> {
    entries: core::slice::Iter<'a, Entry>,
    pending: Option<&'a Value>,
    depth: u32,
}

impl MapWalker for Entries<'_> {
    fn len_hint(&self) -> Option<usize> {
        Some(self.entries.len() + usize::from(self.pending.is_some()))
    }

    fn next_key<R>(&mut self, read: impl FnOnce(&str) -> R) -> Result<Option<R>, MapError> {
        match self.entries.next() {
            Some(entry) => {
                self.pending = Some(entry.value());
                Ok(Some(read(entry.key())))
            }
            None => Ok(None),
        }
    }

    fn value_seed<S: Seed>(&mut self, seed: S) -> Result<S::Out, MapError> {
        let value = self
            .pending
            .take()
            .ok_or_else(|| MapError::bad_value("a key read before its value"))?;
        seed.read(Node {
            value,
            depth: self.depth,
        })
    }

    fn skip_value(&mut self) -> Result<(), MapError> {
        self.pending.take();
        Ok(())
    }
}

// --- a value read from any walk ---------------------------------------------------

/// Builds the value a walk holds, in Rust's allocator.
struct ValueBuild;

/// A node built in Rust's allocator: a refusal of the content is the
/// reader's, an allocation failure aborts, as `clone` does.
fn built<T: Into<Value>>(node: Result<T, ValueError>) -> Result<Value, MapError> {
    match node {
        Ok(node) => Ok(node.into()),
        Err(ValueError::NotANumber) => {
            Err(MapError::bad_value("a number in the JSON number grammar"))
        }
        Err(e) => copied(Err(e)),
    }
}

impl Build for ValueBuild {
    type Out = Value;

    fn kinds(&self) -> Kinds {
        Kinds::ANY
    }

    fn absent(self) -> Result<Value, MapError> {
        Ok(Value::absent())
    }

    fn null(self) -> Result<Value, MapError> {
        Ok(Value::null())
    }

    fn bool(self, value: bool) -> Result<Value, MapError> {
        Ok(Value::from(value))
    }

    fn number(self, text: &str) -> Result<Value, MapError> {
        built(Number::new_in(Alloc::rust(), text))
    }

    fn string(self, text: &str) -> Result<Value, MapError> {
        built(Text::new_in(Alloc::rust(), text))
    }

    fn bytes(self, bytes: &[u8]) -> Result<Value, MapError> {
        built(Buffer::new_in(Alloc::rust(), bytes))
    }

    fn list<L: ListWalker>(self, mut items: L) -> Result<Value, MapError> {
        let alloc = Alloc::rust();
        let mut list = List::new_in(alloc);
        while let Some(item) = items.next::<Value>()? {
            copied(list.push_in(item, alloc))?;
        }
        Ok(list.into())
    }

    fn map<M: MapWalker>(self, mut entries: M) -> Result<Value, MapError> {
        let alloc = Alloc::rust();
        if entries.keys_checked().is_some() {
            // The walker refused every repeated key itself: each is new.
            let mut map = Map::new_in(alloc);
            each_entry(&mut entries, |key, value| {
                map.insert_node(key, value, alloc)
            })?;
            return Ok(map.into());
        }
        // A repeated key replaces, as a map's `set` does.
        let mut map = Filling::with_room(Map::new_in(alloc), entries.len_hint().unwrap_or(0));
        each_entry(&mut entries, |key, value| map.set_in(key, value, alloc))?;
        Ok(map.into_map().into())
    }
}

/// Reads each entry of `entries` as a value in Rust's allocator and hands
/// it to `store` with its key.
fn each_entry<M: MapWalker>(
    entries: &mut M,
    mut store: impl FnMut(&str, Value) -> Result<(), ValueError>,
) -> Result<(), MapError> {
    let mut key = String::new();
    while entries
        .next_key(|text| {
            key.clear();
            key.push_str(text);
        })?
        .is_some()
    {
        let value = entries.value::<Value>()?;
        copied(store(&key, value))?;
    }
    Ok(())
}

/// A copy in Rust's allocator of the value the walk holds, as
/// `FromValue for Value` makes one.
impl FromWalk for Value {
    fn from_walk<W: Walker>(walker: W) -> Result<Value, MapError> {
        walker.build(ValueBuild)
    }
}

macro_rules! own_container {
    ($($ty:ident => $tag:ident),* $(,)?) => {$(
        /// Read as a value, refused when it is another kind.
        impl FromWalk for $ty {
            fn from_walk<W: Walker>(walker: W) -> Result<$ty, MapError> {
                $ty::try_from(Value::from_walk(walker)?)
                    .map_err(|other| MapError::wrong_type(Tag::$tag, &other))
            }
        }
    )*};
}

own_container!(
    Map => GUATIAO_MAP,
    List => GUATIAO_LIST,
    Text => GUATIAO_STRING,
    Number => GUATIAO_NUMBER,
    Buffer => GUATIAO_BYTES,
);

// --- a value built from events -------------------------------------------------

/// A container being built, and for a map the key its next value goes
/// under.
#[derive(Debug)]
enum Open {
    List(List),
    Map {
        map: Filling<Map>,
        key: String,
        keyed: bool,
    },
}

/// Builds the value a stream of events describes, through the allocator
/// it was made with: the [`Sink`] a walk of anything writes a tree into.
///
/// It keeps its own stack, so any depth costs heap rather than the
/// thread's stack. Events out of order (a value where a key is due, an
/// end that matches nothing, a second value at the top) are refused with
/// [`ValueError::WrongKind`]; a number outside the grammar with
/// [`ValueError::NotANumber`]. A repeated key replaces the first.
///
/// ```
/// use guatiao::value::visit::{Flat, TreeSink, Walk};
///
/// let mut sink = TreeSink::new();
/// vec![1u16, 2, 3].walk(Flat(&mut sink))?;
/// let value = sink.into_value()?;
/// let list: &guatiao::List = (&value).try_into().unwrap();
/// assert_eq!(list.len(), 3);
/// # Ok::<(), guatiao::ValueError>(())
/// ```
#[derive(Debug)]
pub struct TreeSink {
    alloc: Alloc,
    open: Vec<Open>,
    done: Option<Value>,
    /// What every wide map of this build hashes its keys under.
    seeds: Option<Seeded>,
}

impl TreeSink {
    /// Builds in Rust's allocator.
    pub fn new() -> TreeSink {
        TreeSink::new_in(Alloc::rust())
    }

    /// Builds through `alloc`.
    pub fn new_in(alloc: Alloc) -> TreeSink {
        TreeSink {
            alloc,
            open: Vec::new(),
            done: None,
            seeds: None,
        }
    }

    /// The value the events built. Refused with [`ValueError::WrongKind`]
    /// when no value was given or a container is still open.
    pub fn into_value(mut self) -> Result<Value, ValueError> {
        match (self.open.is_empty(), self.done.take()) {
            (true, Some(value)) => Ok(value),
            _ => Err(ValueError::WrongKind),
        }
    }

    /// Whether a value may come next: at the top before the one value,
    /// in a list, or in a map after a key.
    fn ready(&self) -> Result<(), ValueError> {
        match self.open.last() {
            None if self.done.is_some() => Err(ValueError::WrongKind),
            Some(Open::Map { keyed: false, .. }) => Err(ValueError::WrongKind),
            _ => Ok(()),
        }
    }

    /// Puts a finished value where it goes.
    fn place(&mut self, value: Value) -> Result<(), ValueError> {
        let alloc = self.alloc;
        match self.open.last_mut() {
            None => {
                if self.done.is_some() {
                    return Err(ValueError::WrongKind);
                }
                self.done = Some(value);
                Ok(())
            }
            Some(Open::List(list)) => list.push_in(value, alloc),
            Some(Open::Map { map, key, keyed }) => {
                if !*keyed {
                    return Err(ValueError::WrongKind);
                }
                *keyed = false;
                map.set_in_seeded(key, value, alloc, &mut self.seeds)
            }
        }
    }

    fn leaf(&mut self, value: Result<Value, ValueError>) -> Result<(), ValueError> {
        self.ready()?;
        self.place(value?)
    }
}

impl Default for TreeSink {
    fn default() -> TreeSink {
        TreeSink::new()
    }
}

impl Sink for TreeSink {
    type Error = ValueError;

    fn absent(&mut self) -> Result<(), ValueError> {
        self.leaf(Ok(Value::absent()))
    }

    fn null(&mut self) -> Result<(), ValueError> {
        self.leaf(Ok(Value::null()))
    }

    fn bool(&mut self, value: bool) -> Result<(), ValueError> {
        self.leaf(Ok(Value::from(value)))
    }

    fn number(&mut self, text: &str) -> Result<(), ValueError> {
        self.ready()?;
        let number = Number::new_in(self.alloc, text)?;
        self.place(number.into())
    }

    fn string(&mut self, text: &str) -> Result<(), ValueError> {
        self.ready()?;
        let text = Text::new_in(self.alloc, text)?;
        self.place(text.into())
    }

    fn bytes(&mut self, bytes: &[u8]) -> Result<(), ValueError> {
        self.ready()?;
        let buffer = Buffer::new_in(self.alloc, bytes)?;
        self.place(buffer.into())
    }

    fn list_begin(&mut self, len: Option<usize>) -> Result<(), ValueError> {
        self.ready()?;
        let list = List::with_capacity_in(self.alloc, len.unwrap_or(0).min(RESERVE))?;
        self.open.push(Open::List(list));
        Ok(())
    }

    fn list_end(&mut self) -> Result<(), ValueError> {
        match self.open.pop() {
            Some(Open::List(list)) => self.place(list.into()),
            Some(other) => {
                self.open.push(other);
                Err(ValueError::WrongKind)
            }
            None => Err(ValueError::WrongKind),
        }
    }

    fn map_begin(&mut self, len: Option<usize>) -> Result<(), ValueError> {
        self.ready()?;
        let len = len.unwrap_or(0);
        let map = Map::with_capacity_in(self.alloc, len.min(RESERVE))?;
        self.open.push(Open::Map {
            map: Filling::with_room(map, len),
            key: String::new(),
            keyed: false,
        });
        Ok(())
    }

    fn key(&mut self, text: &str) -> Result<(), ValueError> {
        match self.open.last_mut() {
            Some(Open::Map { key, keyed, .. }) if !*keyed => {
                key.clear();
                key.push_str(text);
                *keyed = true;
                Ok(())
            }
            _ => Err(ValueError::WrongKind),
        }
    }

    fn map_end(&mut self) -> Result<(), ValueError> {
        match self.open.pop() {
            Some(Open::Map {
                map, keyed: false, ..
            }) => self.place(map.into_map().into()),
            Some(other) => {
                self.open.push(other);
                Err(ValueError::WrongKind)
            }
            None => Err(ValueError::WrongKind),
        }
    }
}
