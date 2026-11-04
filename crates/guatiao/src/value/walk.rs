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
use super::value::{Owned, arm};

// --- copying ------------------------------------------------------------

/// How a copy is made: the allocator asked for, and whether a node that
/// lives in this build's wiping allocator is copied there instead (every
/// copy but a kind's answer moving into the carrier its caller named).
#[derive(Clone, Copy)]
struct Copy {
    alloc: Alloc,
    keeps_secrets: bool,
}

impl Copy {
    /// Where a copy of what `recorded` made goes: the wiping allocator for
    /// a secret, unless this copy is exact; else the one asked for.
    fn home(self, recorded: *const crate::value::alloc::Allocator) -> Alloc {
        if self.keeps_secrets && super::types::lives_in_wiping(recorded) {
            Alloc::wiping()
        } else {
            self.alloc
        }
    }
}

/// A container being copied.
struct Copying<'a> {
    built: Built,
    left: Left<'a>,
    /// The key this container goes under in its parent, when the parent
    /// is a map.
    key: Option<&'a str>,
    /// Where this container's copy, its array and its keys, is built.
    alloc: Alloc,
}

enum Built {
    List(List),
    Map(Map),
}

/// What is left of the source to copy.
enum Left<'a> {
    List(core::slice::Iter<'a, Value>),
    Map(core::slice::Iter<'a, Entry>),
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
    fn list(src: &'a List, key: Option<&'a str>, how: Copy) -> Result<Self, ValueError> {
        let alloc = how.home(src.alloc);
        Ok(Copying {
            built: Built::List(List::with_capacity_in(alloc, src.len())?),
            left: Left::List(src.iter()),
            key,
            alloc,
        })
    }

    /// A map whose array or any of whose keys lives in the wiping
    /// allocator is copied there whole, its keys with it.
    fn map(src: &'a Map, key: Option<&'a str>, how: Copy) -> Result<Self, ValueError> {
        let secret_key = how.keeps_secrets
            && src
                .entries()
                .iter()
                .any(|e| super::types::lives_in_wiping(e.key.alloc));
        let alloc = if secret_key {
            Alloc::wiping()
        } else {
            how.home(src.alloc)
        };
        Ok(Copying {
            built: Built::Map(Map::with_capacity_in(alloc, src.len())?),
            left: Left::Map(src.iter()),
            key,
            alloc,
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
    fn attach(&mut self, key: Option<&str>, value: Value) -> Result<(), ValueError> {
        match &mut self.built {
            Built::List(list) => list.push_node(value, self.alloc),
            Built::Map(map) => {
                let key = key.expect("a map's child is reached with its key");
                map.insert_node(key, value, self.alloc)
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

fn step<'a>(node: &'a Value, key: Option<&'a str>, how: Copy) -> Result<Step<'a>, ValueError> {
    Ok(Step::Leaf(match node.tag()? {
        Tag::GUATIAO_LIST => return Ok(Step::Open(Copying::list(arm(node)?, key, how)?)),
        Tag::GUATIAO_MAP => return Ok(Step::Open(Copying::map(text(node)?, key, how)?)),
        Tag::GUATIAO_ABSENT => Value::absent(),
        Tag::GUATIAO_NULL => Value::null(),
        Tag::GUATIAO_BOOL => Value::from(*arm::<bool>(node)?),
        Tag::GUATIAO_STRING => {
            let t = text::<Text>(node)?;
            Text::new_in(how.home(t.alloc), t)?.into()
        }
        Tag::GUATIAO_NUMBER => {
            let n = arm::<Number>(node)?;
            n.copy_in(how.home(n.recorded()))?.into()
        }
        Tag::GUATIAO_BYTES => {
            let b = arm::<Buffer>(node)?;
            Buffer::new_in(how.home(b.alloc), b)?.into()
        }
    }))
}

/// Fills `root` and every container under it. A failure part-way drops
/// the stack, and with it everything built so far.
fn copy_tree(root: Copying<'_>, how: Copy) -> Result<Built, ValueError> {
    let mut stack = vec![root];
    loop {
        let top = stack
            .last_mut()
            .expect("the root leaves the stack only to return");
        match top.next() {
            Some((key, node)) => match step(node, key, how)? {
                Step::Leaf(copy) => top.attach(key, copy)?,
                Step::Open(container) => stack.push(container),
            },
            None => {
                let done = stack.pop().expect("the top was just read");
                match stack.last_mut() {
                    Some(parent) => parent.attach(done.key, done.built.into_value())?,
                    None => return Ok(done.built),
                }
            }
        }
    }
}

fn copy_value_how(node: &Value, how: Copy) -> Result<Value, ValueError> {
    match step(node, None, how)? {
        Step::Leaf(copy) => Ok(copy),
        Step::Open(root) => Ok(copy_tree(root, how)?.into_value()),
    }
}

fn copy_list_how(list: &List, how: Copy) -> Result<List, ValueError> {
    match copy_tree(Copying::list(list, None, how)?, how)? {
        Built::List(copy) => Ok(copy),
        Built::Map(_) => unreachable!("a list's copy is a list"),
    }
}

fn copy_map_how(map: &Map, how: Copy) -> Result<Map, ValueError> {
    match copy_tree(Copying::map(map, None, how)?, how)? {
        Built::Map(copy) => Ok(copy),
        Built::List(_) => unreachable!("a map's copy is a map"),
    }
}

/// A deep copy of `node` through `alloc`; a node that lives in the wiping
/// allocator is copied there instead, so a copy of a secret stays one.
pub(crate) fn copy_value(node: &Value, alloc: Alloc) -> Result<Value, ValueError> {
    copy_value_how(
        node,
        Copy {
            alloc,
            keeps_secrets: true,
        },
    )
}

/// A deep copy of `list`, as [`copy_value`].
pub(crate) fn copy_list(list: &List, alloc: Alloc) -> Result<List, ValueError> {
    copy_list_how(
        list,
        Copy {
            alloc,
            keeps_secrets: true,
        },
    )
}

/// A deep copy of `map`, as [`copy_value`].
pub(crate) fn copy_map(map: &Map, alloc: Alloc) -> Result<Map, ValueError> {
    copy_map_how(
        map,
        Copy {
            alloc,
            keeps_secrets: true,
        },
    )
}

/// A deep copy of `node` entirely in `alloc`: how a kind's answer moves
/// into the carrier its caller named, which is the caller's to choose.
pub(crate) fn copy_value_exact(node: &Value, alloc: Alloc) -> Result<Value, ValueError> {
    copy_value_how(
        node,
        Copy {
            alloc,
            keeps_secrets: false,
        },
    )
}

/// [`copy_value_exact`] for a list.
pub(crate) fn copy_list_exact(list: &List, alloc: Alloc) -> Result<List, ValueError> {
    copy_list_how(
        list,
        Copy {
            alloc,
            keeps_secrets: false,
        },
    )
}

/// [`copy_value_exact`] for a map.
pub(crate) fn copy_map_exact(map: &Map, alloc: Alloc) -> Result<Map, ValueError> {
    copy_map_how(
        map,
        Copy {
            alloc,
            keeps_secrets: false,
        },
    )
}

// --- where a tree lives ---------------------------------------------------

/// Whether every container in the tree, every map key included, records
/// `alloc`: nothing in it needs another allocator to grow or be freed. A
/// container recording none, or a tag this build does not know, answers
/// `false`. Each container is judged before its children are visited, so
/// a tree that is not homed is told at its first container.
///
/// Allocates nothing for a tree up to [`SHALLOW`] containers deep, judged
/// on the thread's stack; a deeper one is judged again over a heap stack,
/// so any depth is answered and none can overflow the thread.
///
/// A node that lives in this build's wiping allocator counts as home
/// whatever `alloc` is: a secret stays there through every move but a kind
/// answer's, which asks [`recorded_by_exact`].
pub(crate) fn recorded_by(node: &Value, alloc: *const crate::value::alloc::Allocator) -> bool {
    value_home(node, Home::keeping(alloc))
}

/// [`recorded_by`], every node in exactly `alloc`.
pub(crate) fn recorded_by_exact(
    node: &Value,
    alloc: *const crate::value::alloc::Allocator,
) -> bool {
    value_home(node, Home::exactly(alloc))
}

fn value_home(node: &Value, alloc: Home) -> bool {
    shallow(node, alloc, SHALLOW).unwrap_or_else(|| {
        let mut stack = Vec::new();
        homed(node, alloc, &mut stack) && drain(stack, alloc)
    })
}

/// [`recorded_by`] for a map.
pub(crate) fn map_recorded_by(map: &Map, alloc: *const crate::value::alloc::Allocator) -> bool {
    map_home(map, Home::keeping(alloc))
}

/// [`recorded_by_exact`] for a map.
pub(crate) fn map_recorded_by_exact(
    map: &Map,
    alloc: *const crate::value::alloc::Allocator,
) -> bool {
    map_home(map, Home::exactly(alloc))
}

fn map_home(map: &Map, alloc: Home) -> bool {
    shallow_map(map, alloc, SHALLOW).unwrap_or_else(|| {
        let mut stack = Vec::new();
        map_homed(map, alloc, &mut stack) && drain(stack, alloc)
    })
}

/// [`recorded_by`] for a list.
pub(crate) fn list_recorded_by(list: &List, alloc: *const crate::value::alloc::Allocator) -> bool {
    list_home(list, Home::keeping(alloc))
}

/// [`recorded_by_exact`] for a list.
pub(crate) fn list_recorded_by_exact(
    list: &List,
    alloc: *const crate::value::alloc::Allocator,
) -> bool {
    list_home(list, Home::exactly(alloc))
}

fn list_home(list: &List, alloc: Home) -> bool {
    shallow_list(list, alloc, SHALLOW).unwrap_or_else(|| {
        let mut stack = Vec::new();
        list_homed(list, alloc, &mut stack) && drain(stack, alloc)
    })
}

/// What a walk counts as a tree's home: the allocator named, and, unless
/// exact, this build's wiping allocator, where a secret stays.
///
/// The wiping allocator counts as home by its address, [`Alloc::wiping`]'s
/// static: a container recording another struct with its `ctx` (a copy
/// `guatiao_alloc_wiping` filled in a caller's memory, which may go away)
/// is not home, and the settle points it at the static instead.
#[derive(Clone, Copy)]
struct Home {
    alloc: *const crate::value::alloc::Allocator,
    /// [`Alloc::wiping`]'s address, or null when exact.
    wiping: *const crate::value::alloc::Allocator,
}

/// What the settle does with a block, by the allocator it recorded.
enum Settle {
    /// Home: left as it is.
    Keep,
    /// In this build's wiping allocator through a struct other than its
    /// static: left where it is, the container pointed at the static.
    Repoint,
    /// Moved into the allocator named.
    Move,
}

impl Home {
    fn keeping(alloc: *const crate::value::alloc::Allocator) -> Home {
        Home {
            alloc,
            wiping: Alloc::wiping().as_raw(),
        }
    }

    fn exactly(alloc: *const crate::value::alloc::Allocator) -> Home {
        Home {
            alloc,
            wiping: core::ptr::null(),
        }
    }

    /// Whether a container that recorded `recorded` is home.
    fn holds(self, recorded: *const crate::value::alloc::Allocator) -> bool {
        is_recorded(recorded, self.alloc) || is_recorded(recorded, self.wiping)
    }

    /// What the settle does with a block that recorded `recorded`.
    fn settle(self, recorded: *const crate::value::alloc::Allocator) -> Settle {
        if self.holds(recorded) {
            Settle::Keep
        } else if !self.wiping.is_null() && super::types::lives_in_wiping(recorded) {
            Settle::Repoint
        } else {
            Settle::Move
        }
    }
}

/// How many containers deep a tree is judged by recursion: deeper than a
/// configuration nests, and few enough frames to cost nothing.
const SHALLOW: usize = 16;

/// [`recorded_by`] by recursion no more than `depth` containers deep, or
/// `None` for a tree deeper than that.
fn shallow(node: &Value, alloc: Home, depth: usize) -> Option<bool> {
    match node.tag() {
        Ok(Tag::GUATIAO_MAP) => match text::<Map>(node) {
            Ok(map) => shallow_map(map, alloc, depth),
            Err(_) => Some(false),
        },
        Ok(Tag::GUATIAO_LIST) => match arm::<List>(node) {
            Ok(list) => shallow_list(list, alloc, depth),
            Err(_) => Some(false),
        },
        // A leaf leaves nothing on the stack, and an empty one allocates
        // nothing.
        _ => Some(homed(node, alloc, &mut Vec::new())),
    }
}

fn shallow_map(map: &Map, alloc: Home, depth: usize) -> Option<bool> {
    if !alloc.holds(map.alloc) {
        return Some(false);
    }
    let below = depth.checked_sub(1)?;
    for entry in map.iter() {
        if !alloc.holds(entry.key.alloc) || !shallow(&entry.value, alloc, below)? {
            return Some(false);
        }
    }
    Some(true)
}

fn shallow_list(list: &List, alloc: Home, depth: usize) -> Option<bool> {
    if !alloc.holds(list.alloc) {
        return Some(false);
    }
    let below = depth.checked_sub(1)?;
    for item in list.iter() {
        if !shallow(item, alloc, below)? {
            return Some(false);
        }
    }
    Some(true)
}

/// [`recorded_by`] for a text.
pub(crate) fn text_recorded_by(text: &Text, alloc: *const crate::value::alloc::Allocator) -> bool {
    Home::keeping(alloc).holds(text.alloc)
}

/// [`recorded_by`] for a buffer.
pub(crate) fn buffer_recorded_by(
    buffer: &Buffer,
    alloc: *const crate::value::alloc::Allocator,
) -> bool {
    Home::keeping(alloc).holds(buffer.alloc)
}

/// Whether a container recorded `alloc` itself: the addresses compared,
/// so nothing is read through either; `alloc` is a checked allocator's.
fn is_recorded(
    recorded: *const crate::value::alloc::Allocator,
    alloc: *const crate::value::alloc::Allocator,
) -> bool {
    !recorded.is_null() && core::ptr::eq(recorded, alloc)
}

fn map_homed<'a>(map: &'a Map, alloc: Home, stack: &mut Vec<&'a Value>) -> bool {
    if !alloc.holds(map.alloc) {
        return false;
    }
    for entry in map.iter() {
        if !alloc.holds(entry.key.alloc) || !child_homed(&entry.value, alloc, stack) {
            return false;
        }
    }
    true
}

fn list_homed<'a>(list: &'a List, alloc: Home, stack: &mut Vec<&'a Value>) -> bool {
    if !alloc.holds(list.alloc) {
        return false;
    }
    list.iter().all(|item| child_homed(item, alloc, stack))
}

/// A child judged on the spot when it is a leaf, or left on `stack` when
/// it is a container: a tree with no nested container is judged without
/// the stack allocating.
fn child_homed<'a>(node: &'a Value, alloc: Home, stack: &mut Vec<&'a Value>) -> bool {
    match node.tag() {
        Ok(Tag::GUATIAO_MAP | Tag::GUATIAO_LIST) => {
            stack.push(node);
            true
        }
        _ => homed(node, alloc, stack),
    }
}

/// One node judged, its children left on `stack` for [`drain`].
fn homed<'a>(node: &'a Value, alloc: Home, stack: &mut Vec<&'a Value>) -> bool {
    match node.tag() {
        Ok(Tag::GUATIAO_MAP) => text::<Map>(node).is_ok_and(|map| map_homed(map, alloc, stack)),
        Ok(Tag::GUATIAO_LIST) => arm::<List>(node).is_ok_and(|list| list_homed(list, alloc, stack)),
        Ok(Tag::GUATIAO_STRING) => text::<Text>(node).is_ok_and(|t| alloc.holds(t.alloc)),
        Ok(Tag::GUATIAO_NUMBER) => arm::<Number>(node).is_ok_and(|n| alloc.holds(n.recorded())),
        Ok(Tag::GUATIAO_BYTES) => arm::<Buffer>(node).is_ok_and(|b| alloc.holds(b.alloc)),
        Ok(Tag::GUATIAO_ABSENT | Tag::GUATIAO_NULL | Tag::GUATIAO_BOOL) => true,
        Err(_) => false,
    }
}

fn drain(mut stack: Vec<&Value>, alloc: Home) -> bool {
    while let Some(node) = stack.pop() {
        if !homed(node, alloc, &mut stack) {
            return false;
        }
    }
    true
}

// --- moving a tree's storage, in place ---------------------------------------

/// Moves every block of the tree that `alloc` did not make into one it
/// did, overwriting the old block and freeing it through its own
/// allocator; a block in this build's wiping allocator stays where it is,
/// a secret kept a secret, its container pointed at [`Alloc::wiping`]'s
/// static when it recorded another copy of that vtable: a container's
/// array with its elements moved bitwise, a key's or a leaf's bytes. A
/// node whose tag this build does not know owns nothing it can see and is
/// left as it is. One loop over a heap stack, so any depth.
///
/// A failure stops the walk with the tree whole: every block is either
/// moved or where it was.
pub(crate) fn settle_value(node: &mut Value, alloc: Alloc) -> Result<(), ValueError> {
    let mut stack = Vec::new();
    settle_child(node, alloc, &mut stack)?;
    settle_drain(stack, alloc)
}

/// [`settle_value`] for a map.
pub(crate) fn settle_map(map: &mut Map, alloc: Alloc) -> Result<(), ValueError> {
    let mut stack = Vec::new();
    settle_map_into(map, alloc, &mut stack)?;
    settle_drain(stack, alloc)
}

/// [`settle_value`] for a list.
pub(crate) fn settle_list(list: &mut List, alloc: Alloc) -> Result<(), ValueError> {
    let mut stack = Vec::new();
    settle_list_into(list, alloc, &mut stack)?;
    settle_drain(stack, alloc)
}

/// [`settle_value`] for a text.
pub(crate) fn settle_text(text: &mut Text, alloc: Alloc) -> Result<(), ValueError> {
    let home = Home::keeping(alloc.as_raw());
    match home.settle(text.alloc) {
        Settle::Keep => {}
        Settle::Repoint => text.alloc = home.wiping,
        Settle::Move => text.rehome(alloc)?,
    }
    Ok(())
}

/// [`settle_value`] for a buffer.
pub(crate) fn settle_buffer(buffer: &mut Buffer, alloc: Alloc) -> Result<(), ValueError> {
    let home = Home::keeping(alloc.as_raw());
    match home.settle(buffer.alloc) {
        Settle::Keep => {}
        Settle::Repoint => buffer.alloc = home.wiping,
        Settle::Move => buffer.rehome(alloc)?,
    }
    Ok(())
}

/// What the settle does with a container's array: moved when another
/// allocator made it, or when it is not owned at all (`cap == 0`), since
/// its elements are about to be written and a literal's array may be
/// read-only.
fn settle_array(
    recorded: *const crate::value::alloc::Allocator,
    len: usize,
    cap: usize,
    home: Home,
) -> Settle {
    if cap == 0 && len > 0 {
        Settle::Move
    } else {
        home.settle(recorded)
    }
}

fn settle_map_into<'a>(
    map: &'a mut Map,
    alloc: Alloc,
    stack: &mut Vec<&'a mut Value>,
) -> Result<(), ValueError> {
    let home = Home::keeping(alloc.as_raw());
    match settle_array(map.alloc, map.len, map.cap, home) {
        Settle::Keep => {}
        Settle::Repoint => map.alloc = home.wiping,
        Settle::Move => map.rehome(alloc)?,
    }
    for entry in map.entries_mut() {
        settle_text(&mut entry.key, alloc)?;
        settle_child(&mut entry.value, alloc, stack)?;
    }
    Ok(())
}

fn settle_list_into<'a>(
    list: &'a mut List,
    alloc: Alloc,
    stack: &mut Vec<&'a mut Value>,
) -> Result<(), ValueError> {
    let home = Home::keeping(alloc.as_raw());
    match settle_array(list.alloc, list.len, list.cap, home) {
        Settle::Keep => {}
        Settle::Repoint => list.alloc = home.wiping,
        Settle::Move => list.rehome(alloc)?,
    }
    for item in list.iter_mut() {
        settle_child(item, alloc, stack)?;
    }
    Ok(())
}

/// A leaf settled on the spot, a container left on `stack`.
fn settle_child<'a>(
    node: &'a mut Value,
    alloc: Alloc,
    stack: &mut Vec<&'a mut Value>,
) -> Result<(), ValueError> {
    if matches!(node.owned_mut(), Owned::Map(_) | Owned::List(_)) {
        stack.push(node);
        return Ok(());
    }
    match node.owned_mut() {
        Owned::Text(text) => settle_text(text, alloc),
        Owned::Buffer(buffer) => settle_buffer(buffer, alloc),
        _ => Ok(()),
    }
}

fn settle_drain(mut stack: Vec<&mut Value>, alloc: Alloc) -> Result<(), ValueError> {
    while let Some(node) = stack.pop() {
        match node.owned_mut() {
            Owned::Map(map) => settle_map_into(map, alloc, &mut stack)?,
            Owned::List(list) => settle_list_into(list, alloc, &mut stack)?,
            _ => unreachable!("only a container is left on the stack"),
        }
    }
    Ok(())
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

/// Feeds `value`, and everything under it, to `state` so that two values
/// [`equal`] calls the same feed it the same input: kinds, lengths, keys
/// in order, the bytes of a text, number or buffer. A hash of the result
/// finds equal values without comparing every pair.
pub(crate) fn hash<H: core::hash::Hasher>(value: &Value, state: &mut H) {
    let mut stack = Vec::new();
    let mut next = Some(value);
    while let Some(node) = next.take().or_else(|| stack.pop()) {
        state.write_u32(node.tag);
        match node.tag() {
            Ok(Tag::GUATIAO_ABSENT | Tag::GUATIAO_NULL) | Err(_) => {}
            Ok(Tag::GUATIAO_BOOL) => {
                state.write_u8(TryAsRef::<bool>::try_as_ref(node).map_or(2, |b| u8::from(*b)));
            }
            Ok(tag @ (Tag::GUATIAO_STRING | Tag::GUATIAO_NUMBER)) => {
                bytes(node.text_bytes(tag).unwrap_or(&[]), state);
            }
            Ok(Tag::GUATIAO_BYTES) => {
                bytes(TryAsRef::<[u8]>::try_as_ref(node).unwrap_or(&[]), state);
            }
            Ok(Tag::GUATIAO_LIST) => {
                let items = arm::<List>(node).map_or(&[][..], |list| &list[..]);
                state.write_usize(items.len());
                stack.extend(items.iter().rev());
            }
            Ok(Tag::GUATIAO_MAP) => {
                let entries = node.map_arm().map_or(&[][..], Map::entries);
                state.write_usize(entries.len());
                for entry in entries {
                    bytes(entry.key_bytes(), state);
                }
                stack.extend(entries.iter().rev().map(Entry::value));
            }
        }
    }

    /// Bytes with their length first, so where one run ends is part of
    /// the input.
    fn bytes<H: core::hash::Hasher>(bytes: &[u8], state: &mut H) {
        state.write_usize(bytes.len());
        state.write(bytes);
    }
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
