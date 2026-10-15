// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Projecting a tagged value onto flat `key -> text` storage.
//!
//! The discriminant goes under the field's own key and each payload field
//! under `<key>.<field>`. That is also the URI spelling —
//! `?auth=userpass&auth.username=alice` — which is what lets a command
//! line select an arm at all.
//!
//! # The deletion is first, and unconditional
//!
//! [`flatten`] clears every `<key>.*` entry before it writes, and
//! [`unflatten`] drops any it finds that the selected arm does not
//! declare. That is what makes "an arm with no fields stores nothing" a
//! property of the code rather than a convention somebody has to remember.
//!
//! Move the deletion after the writes, or make it conditional on the new
//! arm, and a field that two arms happen to share keeps the old arm's
//! value. Same bug, longer fuse.

#![forbid(unsafe_code)]

use alloc::borrow::Cow;
use alloc::collections::{BTreeMap, BTreeSet};
use core::ops::Bound;
use guatiao::value::convert::TryAsRef;

use crate::path::{self, Segment};
use guatiao::schema::ValidationError;
use guatiao::schema::read::{FieldRef, Kind, SchemaRef};
use guatiao::schema::validate::text_of;
use guatiao::schema::vocab as schema_vocab;
use guatiao::value::alloc::Alloc;
use guatiao::value::error::MAX_DEPTH;
use guatiao::value::error::ValueError;
use guatiao::value::read::str_or;
use guatiao::value::types::{List, Map, Tag, Text, Value};

/// Between a field's key and one of its payload fields.
pub const SEPARATOR: char = '.';

/// Writes a value into flat storage, one entry per scalar leaf.
///
/// `{"agent": [{"name": "one"}]}` stores `agent[0].name`. A nested
/// object, a list and an open map are each walked; a tagged field keeps
/// its discriminant under its **own** key, because that is the thing a
/// command line selects (`?auth=userpass&auth.username=alice`).
///
/// Answers whether the value fits the field: `false` when the shape
/// contradicts the kind at the top — a list where an object is declared,
/// a discriminant naming no arm — and the store is left cleared but
/// otherwise untouched. **Deeper mismatches are skipped, not refused**:
/// a member that cannot be spelled as text is not written, the way it
/// never was. A value with [`MAX_DEPTH`] or more containers nested one
/// inside the next also answers `false`, and what lies at or past the
/// bound is not written.
///
/// # An empty container stores nothing
///
/// So an empty list and an absent one are the same thing here, and
/// reading back gives the absent one. It is the one fact a flat store
/// cannot carry, and it is cheaper to say than to work around: a store
/// of `key -> text` has no way to write "there is a container here and
/// it is empty".
///
/// # The deletion is first, and unconditional
///
/// Everything at or under the field's key goes before anything is
/// written ([`clear_under`]). See the module note: make it conditional
/// and a field two arms happen to share keeps the old arm's value.
pub fn flatten(field: FieldRef<'_>, value: &Value, store: &mut BTreeMap<String, String>) -> bool {
    let root = root_path(field.key());
    clear_under(store, &root);
    let mut stack = Vec::new();
    // The top frame decides the answer; everything it pushes is walked
    // for what it can offer.
    let mut fits = visit(store, &mut stack, root, field.kind(), value, 0);
    while let Some((path, kind, value, depth)) = stack.pop() {
        if depth >= MAX_DEPTH {
            // Bounded like `unflatten`: a value is somebody else's input.
            fits = false;
            continue;
        }
        let _ = visit(store, &mut stack, path, kind, value, depth);
    }
    fits
}

/// A frame still to be walked: where it is stored, its kind, its value,
/// and how many containers are open above it.
type Frame<'a> = (String, Kind<'a>, &'a Value, u32);

/// Removes every entry **at or under** `path`.
///
/// `auth` takes `auth`, `auth.username` and `auth[0].x` with it, and
/// leaves `authority` alone — a prefix is only a prefix when what
/// follows it starts a segment.
pub fn clear_under(store: &mut BTreeMap<String, String>, path: &str) {
    store.retain(|key, _| !at_or_under(key, path));
}

/// Whether `key` is `path` itself or something inside it.
fn at_or_under(key: &str, path: &str) -> bool {
    if key == path {
        return true;
    }
    key.strip_prefix(path)
        .is_some_and(|rest| rest.starts_with(SEPARATOR) || rest.starts_with('['))
}

/// One node: writes a leaf, or pushes what it contains. Answers whether
/// the value fits the kind at all.
fn visit<'a>(
    store: &mut BTreeMap<String, String>,
    stack: &mut Vec<Frame<'a>>,
    path: String,
    kind: Kind<'a>,
    value: &'a Value,
    depth: u32,
) -> bool {
    let below = depth + 1;
    match kind {
        Kind::Variant { tag, .. } => {
            let Some(map) = TryAsRef::<Map>::try_as_ref(value) else {
                return false;
            };
            let chosen = str_or(map.get(tag), "");
            let Some(arm) = kind.arms().find(|a| a.value() == chosen) else {
                return false;
            };
            // The discriminant under the field's OWN key, which is what
            // makes `?auth=userpass` a thing a person can type.
            store.insert(path.clone(), chosen.to_string());
            for member in arm.fields() {
                if let Some(v) = map.get(member.key()) {
                    stack.push((named(&path, member.key()), member.kind(), v, below));
                }
            }
            true
        }
        Kind::Map(_) => {
            let Some(map) = TryAsRef::<Map>::try_as_ref(value) else {
                return false;
            };
            // Walked in DECLARATION order rather than the value's, so two
            // values of one schema produce keys in one order. The store
            // is sorted anyway; this is for anything reading the walk.
            for declared in kind.fields() {
                if let Some(v) = map.get(declared.key()) {
                    stack.push((named(&path, declared.key()), declared.kind(), v, below));
                }
            }
            true
        }
        Kind::MapOf(_) => {
            let Some(map) = TryAsRef::<Map>::try_as_ref(value) else {
                return false;
            };
            let values = kind.values();
            for entry in map {
                stack.push((keyed(&path, entry.key()), values, entry.value(), below));
            }
            true
        }
        Kind::List { .. } => {
            let Some(list) = TryAsRef::<List>::try_as_ref(value) else {
                return false;
            };
            let items = kind.items();
            for (at, v) in list.iter().enumerate() {
                stack.push((indexed(&path, at), items, v, below));
            }
            true
        }
        _ => match scalar_text(value) {
            Some(text) => {
                store.insert(path, text);
                true
            }
            None => false,
        },
    }
}

/// `path.name`, or `path[na.me]` when the name would not read back as
/// one segment. A declared key may hold a separator, so this is not
/// always a dot.
fn named(path: &str, name: &str) -> String {
    if path::is_bare_name(name) {
        format!("{path}{SEPARATOR}{name}")
    } else {
        keyed(path, name)
    }
}

/// How the store spells a top-level field's own key: the name, or a
/// bracket when the name holds a `.`, a bracket or nothing at all.
///
/// So a field called `a.b` is stored under `[a.b]` and cannot be mistaken
/// for the member `b` of a field called `a`.
pub(crate) fn root_path(name: &str) -> String {
    if path::is_bare_name(name) {
        name.to_string()
    } else {
        keyed("", name)
    }
}

/// `path[key]`, quoted if it would otherwise read as a position.
fn keyed(path: &str, key: &str) -> String {
    format!("{path}{}", Segment::Key(Cow::Borrowed(key)))
}

/// `path[3]`.
fn indexed(path: &str, at: usize) -> String {
    format!("{path}{}", Segment::Index(at))
}

/// Reads a value back out of flat storage, rebuilding what the schema
/// declares.
///
/// The inverse of [`flatten`], and the same rule in reverse: **an entry
/// the schema does not declare is dropped**, so a record that picked up
/// a stale key some other way still reads back as the schema says. A
/// container with nothing under it reads as absent, which is the other
/// half of "an empty container stores nothing".
///
/// `None` when nothing under this field is present at all, or when the
/// stored shape cannot be a value of this kind — a tagged field whose
/// discriminant is missing or names no arm is the case that matters,
/// because "no tagged value here" and "an arm with an empty payload" are
/// different answers and must not be confused.
///
/// # Every spelling the path grammar reads is read
///
/// An open map's key is found however the store spelled it (`env[01]`,
/// `env["01"]`, `env.PATH`), and a top-level field whose name holds a `.`
/// or a bracket under its bare name (`a.b`) as well as under `[a.b]`, so a
/// store [`flatten`] wrote in an earlier spelling reads back. Where two
/// spellings of one key are both present, the one [`flatten`] writes now
/// wins and the other is ignored.
pub fn unflatten(
    alloc: Alloc,
    field: FieldRef<'_>,
    store: &BTreeMap<String, String>,
) -> Option<Value> {
    let kind = field.kind();
    root_spellings(field.key())
        .into_iter()
        .filter(|root| holds(store, root))
        .find_map(|root| rebuild(alloc, &root, kind, store, 0))
}

/// How a top-level field's own key can be spelled in a store: the way
/// [`flatten`] writes it, then the bare name when that differs, which is
/// how a name holding a `.` or a bracket was once written.
///
/// An empty name has no bare spelling: an empty root followed by `[`
/// would read every bracketed key in the store as its own.
fn root_spellings(name: &str) -> Vec<String> {
    let written = root_path(name);
    if written == name || name.is_empty() {
        vec![written]
    } else {
        vec![written, name.to_string()]
    }
}

/// Whether anything is stored at or under `path`.
fn holds(store: &BTreeMap<String, String>, path: &str) -> bool {
    store.contains_key(path)
        || [SEPARATOR, '['].into_iter().any(|opener| {
            let prefix = format!("{path}{opener}");
            store
                .range::<str, _>((Bound::Included(prefix.as_str()), Bound::Unbounded))
                .next()
                .is_some_and(|(k, _)| k.starts_with(&prefix))
        })
}

/// One node of [`unflatten`], bounded like every other walk over two
/// trees a caller supplied.
fn rebuild(
    alloc: Alloc,
    path: &str,
    kind: Kind<'_>,
    store: &BTreeMap<String, String>,
    depth: u32,
) -> Option<Value> {
    if depth >= MAX_DEPTH {
        return None;
    }
    match kind {
        Kind::Variant { tag, .. } => {
            let chosen = store.get(path)?;
            let arm = kind.arms().find(|a| a.value() == chosen)?;
            let mut map = Map::new_in(alloc);
            // The discriminant goes in FIRST, so re-emission is
            // byte-stable: a map is insertion-ordered by contract.
            map.try_set(tag, Text::new_in(alloc, chosen).map(Value::from).ok()?)
                .ok()?;
            for member in arm.fields() {
                let at = named(path, member.key());
                if let Some(v) = rebuild(alloc, &at, member.kind(), store, depth + 1) {
                    map.try_set(member.key(), v).ok()?;
                }
            }
            Some(map.into())
        }
        Kind::Map(_) => {
            let mut map = Map::new_in(alloc);
            let mut any = false;
            for declared in kind.fields() {
                let at = named(path, declared.key());
                if let Some(v) = rebuild(alloc, &at, declared.kind(), store, depth + 1) {
                    map.try_set(declared.key(), v).ok()?;
                    any = true;
                }
            }
            any.then(|| map.into())
        }
        Kind::MapOf(_) => {
            let values = kind.values();
            let mut map = Map::new_in(alloc);
            let mut any = false;
            // A key that reads as a position is still a key here: an open
            // map's keys are data. So `[1]` and `["1"]` are one key, read
            // through the spelling `flatten` writes first.
            let mut keys: Vec<(String, Vec<String>)> = Vec::new();
            for (step, spelled) in children(store, path) {
                let key = match step {
                    Step::Named(key) => key,
                    Step::At(index) => index.to_string(),
                };
                match keys.iter_mut().find(|(k, _)| *k == key) {
                    Some((_, spellings)) => spellings.push(spelled),
                    None => keys.push((key, vec![spelled])),
                }
            }
            for (key, mut spellings) in keys {
                let written = keyed(path, &key);
                spellings.retain(|s| *s != written);
                spellings.insert(0, written);
                let found = spellings
                    .iter()
                    .filter(|at| holds(store, at))
                    .find_map(|at| rebuild(alloc, at, values, store, depth + 1));
                if let Some(v) = found {
                    map.try_set(&key, v).ok()?;
                    any = true;
                }
            }
            any.then(|| map.into())
        }
        Kind::List { .. } => {
            let items = kind.items();
            let mut at: Vec<usize> = children(store, path)
                .into_iter()
                .filter_map(|(step, _)| match step {
                    Step::At(index) => Some(index),
                    // A key under a list is not a position, so there is
                    // no element it could be.
                    Step::Named(_) => None,
                })
                .collect();
            at.sort_unstable();
            at.dedup();
            let mut list = List::new_in(alloc);
            for index in at {
                if let Some(v) = rebuild(alloc, &indexed(path, index), items, store, depth + 1) {
                    // Positions are taken in order and CLOSED UP: a store
                    // holding 0 and 2 rebuilds two elements, because a
                    // list has no hole to leave.
                    list.push_in(v, alloc).ok()?;
                }
            }
            (!list.is_empty()).then(|| list.into())
        }
        _ => {
            let text = store.get(path)?;
            Text::new_in(alloc, text).map(Value::from).ok()
        }
    }
}

/// One step below a path, as the store spells it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Step {
    /// A name or a quoted key: `path.name`, `path["a.b"]`.
    Named(String),
    /// A position, or a key that reads as one: `path[3]`.
    At(usize),
}

/// The distinct next steps under `path`, each with the prefix that spells
/// it (`path.name`, `path[3]`, `path["a.b"]`), in the order the store
/// holds them. One step spelled two ways appears once per spelling.
///
/// The keys of a `BTreeMap` are sorted, so everything under one prefix is
/// contiguous. There are two prefixes, `path.` and `path[`, and a sibling
/// such as `pathName` or `path-file` can sort **between** them, so each is
/// its own scan rather than one scan that stops at the first stranger.
/// `.` sorts before `[`, which keeps the result in store order.
fn children(store: &BTreeMap<String, String>, path: &str) -> Vec<(Step, String)> {
    let mut out: Vec<(Step, String)> = Vec::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for opener in [SEPARATOR, '['] {
        let prefix = format!("{path}{opener}");
        for key in store
            .range::<str, _>((Bound::Included(prefix.as_str()), Bound::Unbounded))
            .map(|(k, _)| k)
            .take_while(|k| k.starts_with(&prefix))
        {
            let Some((step, read)) = first_step(&key[path.len()..]) else {
                continue;
            };
            let spelled = &key[..path.len() + read];
            if seen.insert(spelled) {
                out.push((step, spelled.to_string()));
            }
        }
    }
    out
}

/// The first segment of what follows a prefix (`.name`, `[3]`,
/// `["a.b"]`) and how many bytes spell it, when all of `rest` reads as
/// the path's continuation.
fn first_step(rest: &str) -> Option<(Step, usize)> {
    let mut segments = path::spelled(rest, true)?.into_iter();
    let (segment, read) = segments.next()?;
    let step = match segment {
        Segment::Field(name) => Step::Named(name.to_string()),
        Segment::Key(key) => Step::Named(key.into_owned()),
        Segment::Index(at) => Step::At(at),
    };
    Some((step, read))
}

/// The flat keys a field and its arms' members are stored under: its own,
/// plus one per field of every arm, spelled as [`flatten`] spells them.
///
/// **Not every key [`flatten`] can write.** A member that is itself a
/// container stores its leaves below its key (`auth.inner.plain`,
/// `agent[0]`, `env[PATH]`), and those are open-ended, so they are not
/// listed; everything this field writes is at or under one of these, and
/// all of it under the first. For a caller deciding whether a key it holds
/// belongs to this field — a config reader partitioning a flat record —
/// that is the question to ask of each, as [`clear_under`] asks it.
pub fn keys(field: FieldRef<'_>) -> Vec<String> {
    let own = root_path(field.key());
    let mut out = vec![own.clone()];
    for arm in field.kind().arms() {
        // `member`, not `field`: the inner binding would shadow the
        // parameter and the key would be built from one of them twice.
        // It compiles either way, which is the whole danger.
        for member in arm.fields() {
            // Spelled the way `flatten` writes it, so a member whose name
            // holds a `.` is `owner[a.b]` here as it is in the store.
            let key = named(&own, member.key());
            if !out.contains(&key) {
                out.push(key);
            }
        }
    }
    out
}

/// Splits a flat key into `(owner key, member name)`, if it is one: a key
/// of at least two segments, split before the last.
///
/// Reads the key with the path grammar, so it agrees with [`flatten`] and
/// [`keys`]: `auth.username` is `("auth", "username")` and `auth[c.d]`,
/// the spelling of a member called `c.d`, is `("auth", "c.d")`. The member
/// is a [`Cow`] because a quoted name with an escape is not a slice of the
/// key. `None` for a key that is a single segment or is not a path.
pub fn split(key: &str) -> Option<(&str, Cow<'_, str>)> {
    let (owner, last) = path::split_last(key)?;
    Some((owner, name_of_owned(last)))
}

/// The field governing a path, walking every segment of it.
///
/// `auth.password` reaches the arm field an arm of a variant adds;
/// `connection.tls.ca` walks two declared objects; `agent[1].name`
/// indexes a list and then names a member; `env[PATH]` reaches into an
/// open map. A segment that names nothing answers `None`, and so does a
/// segment applied to something that has no members at all.
///
/// **A key that names a field outright wins**, whatever it contains: a
/// schema may declare a field called `a.b`, and `resolve(schema, "a.b")`
/// means that field rather than a walk. To reach a *member* called `a.b`
/// of a declared object, quote it: `owner["a.b"]`.
///
/// What comes back for a list element or an open map's entry is a
/// [`FieldRef`] over the **element's** schema with an **empty key**: such
/// a thing has no declared name, and the path that reached it is the
/// caller's to keep.
///
/// A key that does not walk but starts with the bare name of a top-level
/// field holding a `.` or a bracket (`a.b[x]` for a field `a.b`) is read
/// under that field, as [`unflatten`] reads it.
pub fn resolve<'a>(schema: SchemaRef<'a>, key: &str) -> Option<FieldRef<'a>> {
    if let Some(direct) = schema.find(key) {
        return Some(direct);
    }
    let walked = path::spelled(key, false).and_then(|segments| {
        let mut segments = segments.into_iter();
        let (first, _) = segments.next()?;
        walk_from(root(schema, &first)?, segments)
    });
    walked.or_else(|| {
        let (field, rest) = bare_root(schema, key)?;
        walk_from(field, path::spelled(rest, true)?.into_iter())
    })
}

/// Steps into `at` by each segment in turn.
fn walk_from<'a, 's>(
    at: FieldRef<'a>,
    segments: impl Iterator<Item = (Segment<'s>, usize)>,
) -> Option<FieldRef<'a>> {
    let mut at = at;
    for (segment, _) in segments {
        at = into(at, &segment)?;
    }
    Some(at)
}

/// The top-level field whose bare name, holding a `.` or a bracket,
/// starts `key` and is followed by a `.` or a `[`, and what follows it.
///
/// The longest such name, so `a.b.c[x]` belongs to a field `a.b.c`
/// before a field `a.b`.
fn bare_root<'a, 'k>(schema: SchemaRef<'a>, key: &'k str) -> Option<(FieldRef<'a>, &'k str)> {
    schema
        .fields()
        .filter(|f| !f.key().is_empty() && !path::is_bare_name(f.key()))
        .filter_map(|f| {
            let rest = key.strip_prefix(f.key())?;
            (rest.starts_with(SEPARATOR) || rest.starts_with('[')).then_some((f, rest))
        })
        .max_by_key(|(f, _)| f.key().len())
}

/// The top-level field a path's first segment names.
///
/// A schema's root is an object, so a bracket there is a key like any
/// other: `["a.b"]` names the field spelled `a.b`, and `[0]` names one
/// spelled `0`.
fn root<'a>(schema: SchemaRef<'a>, first: &Segment<'_>) -> Option<FieldRef<'a>> {
    schema.find(&name_of(first))
}

/// A segment as the text it names, for a container that is keyed.
fn name_of<'a>(segment: &'a Segment<'_>) -> Cow<'a, str> {
    match segment {
        Segment::Field(name) => Cow::Borrowed(name),
        Segment::Key(key) => Cow::Borrowed(key.as_ref()),
        // A number applied to something keyed is that key's text, the
        // same rule `path::get` follows over a value.
        Segment::Index(at) => Cow::Owned(at.to_string()),
    }
}

/// The same, consuming the segment, so a key borrowed from the text
/// stays borrowed from it.
fn name_of_owned(segment: Segment<'_>) -> Cow<'_, str> {
    match segment {
        Segment::Field(name) => Cow::Borrowed(name),
        Segment::Key(key) => key,
        Segment::Index(at) => Cow::Owned(at.to_string()),
    }
}

/// One step INTO a field, by one segment.
fn into<'a>(field: FieldRef<'a>, segment: &Segment<'_>) -> Option<FieldRef<'a>> {
    let kind = field.kind();
    match kind {
        // A declared object, or the members an arm of a variant adds:
        // both are named fields, and the segment names one.
        Kind::Map(_) => kind.fields().find(|f| f.key() == name_of(segment)),
        Kind::Variant { .. } => kind
            .arms()
            .find_map(|arm| arm.fields().find(|f| f.key() == name_of(segment))),
        // A list is indexed, never keyed: `agent[0]` reaches an element
        // and `agent[first]` reaches nothing. Every element has the one
        // schema, so WHICH index it is changes nothing here.
        Kind::List { items, .. } => {
            matches!(segment, Segment::Index(_)).then(|| FieldRef::new("", items))?
        }
        // An open map is the other way round: its keys are data, so any
        // segment reaches its one value schema.
        Kind::MapOf(_) => FieldRef::new("", values_of(field)?),
        _ => None,
    }
}

/// The value schema of an open map, as the value it is written as.
///
/// [`Kind::values`](guatiao::schema::read::Kind::values) answers a kind;
/// a [`FieldRef`] needs the value under it, which is the same place JSON
/// Schema puts it.
fn values_of<'a>(field: FieldRef<'a>) -> Option<&'a Value> {
    TryAsRef::<Map>::try_as_ref(field.as_value())
        .and_then(|m| m.get(schema_vocab::ADDITIONAL_PROPERTIES))
}

/// The field governing a path **in a particular store**: the arm a
/// payload key belongs to must be the one the store selects.
///
/// [`resolve`] answers for the schema alone, and for a schema alone
/// `auth.username` is a field of SOME arm. A store has said which arm --
/// `selected(path)` is the discriminant's text there, or `None` when
/// nothing was written and the field's default decides -- and a payload
/// key the chosen arm does not declare is an error rather than something
/// to drop. Ignoring it is how a credential ends up stored under a key
/// the chosen arm does not have.
///
/// The discriminant is asked for **by the owner's own path**, which is
/// the key it is stored under: `auth` at the root, `agent[0].auth` for a
/// variant inside a list of objects.
///
/// Every refusal names the **prefix that failed**, not the whole path: a
/// path is wrong at one step, and saying which one is the difference
/// between a message a person can act on and one they have to bisect.
pub fn resolve_in<'a>(
    schema: SchemaRef<'a>,
    key: &str,
    selected: impl Fn(&str) -> Option<String>,
) -> Result<FieldRef<'a>, ValidationError> {
    if let Some(direct) = schema.find(key) {
        return Ok(direct);
    }
    let unknown = || ValidationError::UnknownOption {
        key: key.to_string(),
        known: schema.fields().map(|f| f.key().to_string()).collect(),
    };
    let walked = path::spelled(key, false).and_then(|segments| {
        let mut segments = segments.into_iter();
        let (first, end) = segments.next()?;
        let at = root(schema, &first)?;
        Some(walk_in(
            at,
            &first_name(&first),
            end,
            segments,
            key,
            &selected,
        ))
    });
    // A key that does not walk may be spelled under a top-level field's
    // bare name, as `unflatten` reads it; if that does not walk either,
    // the first answer stands.
    let bare = || {
        let (field, rest) = bare_root(schema, key)?;
        let base = key.len() - rest.len();
        let segments = path::spelled(rest, true)?
            .into_iter()
            .map(move |(segment, end)| (segment, base + end));
        walk_in(field, field.key(), base, segments, key, &selected).ok()
    };
    match walked {
        Some(Ok(at)) => Ok(at),
        Some(Err(e)) => bare().ok_or(e),
        None => bare().ok_or_else(unknown),
    }
}

/// A first segment as the name of the top-level field it reaches.
fn first_name(first: &Segment<'_>) -> String {
    name_of(first).into_owned()
}

/// The walk of [`resolve_in`] from a top-level field called `name`, whose
/// spelling in `key` ends at `end`.
fn walk_in<'a, 's>(
    field: FieldRef<'a>,
    name: &str,
    end: usize,
    segments: impl Iterator<Item = (Segment<'s>, usize)>,
    key: &str,
    selected: &impl Fn(&str) -> Option<String>,
) -> Result<FieldRef<'a>, ValidationError> {
    let mut at = field;
    // The path walked so far as `flatten` spells it, which is what a
    // refusal names and where a discriminant is stored; and as `key`
    // spells it, which is where a store written another way holds it.
    let mut walked = root_path(name);
    let mut spelled = end;
    for (segment, end) in segments {
        at = step_in(at, &segment, &walked, &key[..spelled], key, selected)?;
        if !matches!(segment, Segment::Field(_)) {
            walked.push_str(&segment.to_string());
        } else {
            walked.push(SEPARATOR);
            walked.push_str(&segment.to_string());
        }
        spelled = end;
    }
    Ok(at)
}

/// One step of [`resolve_in`], with the arm a store selected.
fn step_in<'a>(
    field: FieldRef<'a>,
    segment: &Segment<'_>,
    walked: &str,
    spelled: &str,
    whole: &str,
    selected: &impl Fn(&str) -> Option<String>,
) -> Result<FieldRef<'a>, ValidationError> {
    let kind = field.kind();
    let Kind::Variant { .. } = kind else {
        // Everything else steps the same way it does without a store:
        // which element or member a path reaches is not something a
        // store's contents can change.
        return into(field, segment).ok_or_else(|| {
            let known: Vec<String> = kind.fields().map(|f| f.key().to_string()).collect();
            if known.is_empty() {
                ValidationError::BadValue {
                    key: walked.to_string(),
                    expected: format!(
                        "something with members — '{whole}' reads through it as if it had them"
                    ),
                }
            } else {
                ValidationError::UnknownOption {
                    key: whole.to_string(),
                    known,
                }
            }
        });
    };

    let chosen = selected(walked)
        .filter(|t| !t.is_empty())
        .or_else(|| selected(spelled).filter(|t| !t.is_empty()))
        .or_else(|| field.default().map(text_of))
        .unwrap_or_default();
    let Some(arm) = kind.arms().find(|a| a.value() == chosen) else {
        // Either the tag names no arm -- which the tag's own entry
        // reports if it was given -- or nothing selected one and there is
        // no default. Naming the tag is the actionable half: the payload
        // cannot be checked until the arm is known.
        let names: Vec<&str> = kind.arms().map(|a| a.value()).collect();
        return Err(ValidationError::BadValue {
            key: walked.to_string(),
            expected: format!(
                "a value naming one of {} — '{whole}' belongs to an arm and cannot be \
                 checked until one is chosen",
                names.join(", ")
            ),
        });
    };
    let wanted = name_of(segment);
    arm.fields().find(|f| f.key() == wanted).ok_or_else(|| {
        // Names the ARM rather than listing keys, for a measured
        // reason: the empty arm is the interesting case, and its list
        // of accepted payload keys is EMPTY -- a sentence that stops
        // mid-air. The key is not wrong in general, it is wrong for
        // what was chosen.
        let accepted: Vec<&str> = arm.fields().map(|f| f.key()).collect();
        ValidationError::BadValue {
            key: whole.to_string(),
            expected: if accepted.is_empty() {
                format!("nothing — the '{chosen}' arm of '{walked}' stores no fields at all")
            } else {
                format!(
                    "nothing — the '{chosen}' arm of '{walked}' accepts only {}",
                    accepted.join(", ")
                )
            },
        }
    })
}

/// Whether a flat key names a secret, following the projection.
///
/// Worth a named function rather than `resolve(..).is_sensitive()` for one
/// reason: an unknown key answers **false**, and that is the wrong default
/// to arrive at by accident. It is stated here once so a caller does not
/// have to.
pub fn is_sensitive(schema: SchemaRef<'_>, key: &str) -> bool {
    resolve(schema, key).is_some_and(|o| o.is_sensitive())
}

/// The text spelling of a scalar, or `None` for a container.
///
/// A nested container inside an arm has no flat spelling, so it is not
/// written rather than written wrongly.
fn scalar_text(value: &Value) -> Option<String> {
    match value.tag() {
        Ok(Tag::GUATIAO_BOOL | Tag::GUATIAO_NUMBER | Tag::GUATIAO_STRING)
        | Ok(Tag::GUATIAO_BYTES) => Some(text_of(value)),
        _ => None,
    }
}

/// The error type a caller of [`unflatten`] sees when an allocation fails.
///
/// Re-exported so a consumer does not have to name the module the value
/// model lives in today, which is moving.
pub type Error = ValueError;
