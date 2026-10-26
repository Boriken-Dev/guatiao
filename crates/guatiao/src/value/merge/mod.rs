// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Combining two values, later layer winning — the three strategies, and
//! who came from where.
//!
//! Layering asks two independent questions: when both layers hold a
//! **map**, does the later replace it or recurse into it; and when both
//! hold a **list**, does it replace, overwrite positionally, or union?
//!
//! | mode | nested map | list |
//! |---|---|---|
//! | [`MergeMode::Simple`] | shallow, top-level keys only | positional replace, then append excess |
//! | [`MergeMode::Substitute`] (default) | recursive | replaces wholesale |
//! | [`MergeMode::Deep`] | recursive | extends with unique items |
//!
//! **`Substitute` is the default because `Deep` cannot shrink a list**:
//! every override appends, so a later layer can never remove an inherited
//! tag or cipher, and removal would need a null-sentinel convention worse
//! than the problem. `Deep` is right for a genuine unordered set, which a
//! caller has to name. The test
//! `list_shrinks_under_substitute_and_provably_cannot_under_deep` is that
//! asymmetry asserted.
//!
//! **The result is a NEW tree, through the allocator you name.** Neither
//! input is touched and nothing is moved out of either. That keeps the
//! inputs usable, keeps the result freeable on its own — an owned
//! container records the allocator that made it, so a result stitched
//! from two would free half of itself through each — and keeps this
//! module expressible under `forbid(unsafe_code)`.
//!
//! **Absent means "no opinion", never "delete".** A key missing from the
//! later layer leaves the earlier value standing, and so does a stored
//! absent sentinel. A stored **null** overwrites: a caller who wrote null
//! meant it.
//!
//! **A type mismatch is a [`MergeError`], not a guess.** A silent
//! replacement is how a config layer quietly discards a value.
//!
//! **Provenance is per LEAF PATH.** After a recursive merge `tls.verify`
//! may come from the user layer while `tls.ca` came from the system one,
//! so [`Provenance`] is keyed by leaf path and an interior node drawn
//! from several layers answers [`Source::Mixed`].
//!
//! ```
//! use guatiao::{Alloc, Map, MergeMode, Source, Value};
//!
//! let mut system_tls = Map::new();
//! system_tls.set("ca", "/etc/ca.pem");
//! system_tls.set("verify", true);
//! let mut system = Map::new();
//! system.set("tls", system_tls);
//!
//! let mut user_tls = Map::new();
//! user_tls.set("verify", false);
//! let mut user = Map::new();
//! user.set("tls", user_tls);
//!
//! let (system, user) = (Value::from(system), Value::from(user));
//! let (merged, provenance) = MergeMode::Substitute.merge_layers(
//!     [("system", &system), ("user", &user)],
//!     Alloc::rust(),
//! )?;
//!
//! let merged_map: &Map = (&merged).try_into()?;
//! let tls: &Map = merged_map.required("tls")?.try_into()?;
//! let verify: bool = tls.required("verify")?.try_into()?;
//! assert!(!verify);
//! assert_eq!(provenance.source_of(["tls", "verify"]), Source::Layer("user"));
//! assert_eq!(provenance.source_of(["tls", "ca"]), Source::Layer("system"));
//! assert_eq!(provenance.source_of(["tls"]), Source::Mixed);
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```
//!
//! [`MergeMode`] is a **call-site default**. A declarer that knows an
//! option's meaning better than its merger does overrides it per key with
//! a [`MergeOverrides`] map — a plain `path -> MergeMode` lookup this
//! module defines, deliberately not a schema type.
//! [`crate::schema::merge`] builds one from a schema's annotations; the
//! merge itself never learns what a schema is.

#![forbid(unsafe_code)]

pub(crate) mod path;

use alloc::collections::BTreeMap;
use core::hash::{BuildHasher, Hasher};
use std::collections::HashMap;
use std::collections::hash_map::RandomState;

use crate::value::alloc::Alloc;
use crate::value::convert::{ToValue, TryAsRef, describe};
use crate::value::error::{MAX_DEPTH, ValueError};
use crate::value::types::map::{Filling, Lookup};
use crate::value::types::{List, Map, Tag, Value};
use crate::value::walk;

/// Which strategy combines two values.
///
/// The names are the ones consumers already reason in, and they are
/// fixed: a second set of names for one idea is how two mental models
/// drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum MergeMode {
    /// Shallow: top-level keys replace, nested maps are **not** recursed
    /// into, and a list is overwritten element by element with the
    /// earlier list's excess surviving.
    ///
    /// The discriminants are **frozen**, which is why they are explicit
    /// and why they are not in declaration order. They cross a boundary
    /// as integers — a value that moved would silently change the mode a
    /// caller asked for.
    Simple = 1,

    /// Recursive maps, wholesale list replacement. The default, because a
    /// later layer's list **is** the list and every other mode makes
    /// removing an inherited item impossible.
    #[default]
    Substitute = 3,

    /// Recursive maps, lists extended with unique items. Right for a
    /// genuine unordered set, and unable to shorten one.
    Deep = 2,
}

/// The sub-options a mode can take. Today: one, and only [`MergeMode::Deep`]
/// reads it.
///
/// A struct rather than a bare `bool` so a second sub-option can be added
/// without changing every signature that carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct MergeOptions {
    /// Whether [`MergeMode::Deep`] merges map elements of a list **by
    /// position** rather than appending them.
    ///
    /// **Off by default.** On, two maps
    /// at the same index merge only when at least one key overlaps: two
    /// maps with nothing in common are two records that happen to be
    /// adjacent, not one record described twice.
    pub mergelists: bool,
}

impl MergeOptions {
    /// The defaults: `mergelists` off.
    pub fn new() -> MergeOptions {
        MergeOptions { mergelists: false }
    }

    /// Chained setter for [`MergeOptions::mergelists`].
    #[must_use]
    pub fn with_mergelists(mut self, on: bool) -> MergeOptions {
        self.mergelists = on;
        self
    }
}

/// Why a merge could not produce a value.
///
/// Two cases, kept apart because they have different fixes: the inputs
/// disagree about shape, or the result could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MergeError {
    /// The two sides cannot combine under this mode at this path.
    Kind {
        /// Where they disagreed, spelled as `guatiao-intake` spells a path
        /// (`tls.ca`, `peers[0].name`, `["a.b"]` for a key holding a dot),
        /// empty at the root.
        path: String,
        /// The kind on the earlier side, or `None` for a tag this build
        /// does not know.
        earlier: Option<Tag>,
        /// The kind on the later side.
        later: Option<Tag>,
        /// The mode in force at that path, which is what makes the
        /// combination illegal — another mode may well accept it.
        mode: MergeMode,
    },
    /// The result could not be built: the allocator refused, the tree is
    /// nested deeper than [`MAX_DEPTH`], or a key was not UTF-8.
    ///
    /// Separate from a shape disagreement because nothing about the inputs
    /// is wrong — the same merge on a smaller tree, or with an allocator
    /// that had room, would have succeeded.
    Build(ValueError),
}

impl From<ValueError> for MergeError {
    fn from(e: ValueError) -> MergeError {
        MergeError::Build(e)
    }
}

impl core::fmt::Display for MergeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MergeError::Kind {
                path,
                earlier,
                later,
                mode,
            } => {
                write!(
                    f,
                    "cannot {mode:?}-merge {} into {}",
                    describe(*later),
                    describe(*earlier)
                )?;
                if path.is_empty() {
                    Ok(())
                } else {
                    write!(f, " at \"{path}\"")
                }
            }
            MergeError::Build(e) => write!(f, "the merged value could not be built: {e}"),
        }
    }
}

impl core::error::Error for MergeError {}

/// Where a leaf's final value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source<'a> {
    /// One layer, by the caller's own label.
    Layer(&'a str),
    /// An interior node whose leaves came from more than one layer. The
    /// honest answer, and the reason provenance is per leaf.
    Mixed,
    /// Nothing was recorded under this path.
    Absent,
}

/// Which layer each leaf of a merged value came from.
///
/// Keyed by **leaf path** -- `["tls", "ca"]` -- because an interior node
/// has no single answer after a recursive merge. A path is a list of
/// segments, never joined text: the top-level key `tls.ca` and the key
/// `ca` inside `tls` are different leaves. Sorted, so the prefix scan
/// [`Provenance::source_of`] does is a range rather than a full walk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Provenance<'a> {
    leaves: BTreeMap<Vec<String>, &'a str>,
}

impl<'a> Provenance<'a> {
    /// Where the value at `path` came from: the keys from the root, and a
    /// list position as its decimal text (`["peers", "0", "name"]`).
    ///
    /// An exact leaf answers its layer. An interior node answers the layer
    /// every leaf beneath it agrees on, [`Source::Mixed`] when they do
    /// not, and [`Source::Absent`] when nothing is recorded there. The
    /// empty path is the whole value.
    pub fn source_of<I, S>(&self, path: I) -> Source<'a>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let path: Vec<String> = path.into_iter().map(|s| s.as_ref().to_string()).collect();
        if let Some(layer) = self.leaves.get(&path) {
            return Source::Layer(layer);
        }
        let mut found: Option<&'a str> = None;
        for (_, layer) in self
            .leaves
            .range(path.clone()..)
            .take_while(|(k, _)| k.starts_with(&path))
        {
            match found {
                None => found = Some(layer),
                Some(seen) if seen == *layer => {}
                Some(_) => return Source::Mixed,
            }
        }
        match found {
            Some(layer) => Source::Layer(layer),
            None => Source::Absent,
        }
    }

    /// Every recorded leaf path, as its segments, and its layer, in
    /// sorted path order.
    pub fn leaves(&self) -> impl Iterator<Item = (&[String], &'a str)> {
        self.leaves.iter().map(|(k, v)| (k.as_slice(), *v))
    }

    /// Records `layer` as the source of every leaf under `path`.
    ///
    /// An empty map or list is itself a leaf: there is nothing beneath it
    /// to attribute, and attributing nothing would make it invisible to
    /// [`Provenance::source_of`]. An absent value is not: it is no
    /// opinion, so it has no layer.
    fn claim(&mut self, path: &mut Vec<String>, value: &Value, layer: &'a str, depth: u32) {
        if depth >= MAX_DEPTH {
            self.leaves.insert(path.clone(), layer);
            return;
        }
        match kind(value) {
            Some(Tag::GUATIAO_ABSENT) => {}
            Some(Tag::GUATIAO_MAP) => {
                let entries = TryAsRef::<Map>::try_as_ref(value)
                    .map(Map::entries)
                    .unwrap_or(&[]);
                if entries.is_empty() {
                    self.leaves.insert(path.clone(), layer);
                    return;
                }
                for entry in entries {
                    path.push(entry.key().to_string());
                    self.claim(path, entry.value(), layer, depth + 1);
                    path.pop();
                }
            }
            Some(Tag::GUATIAO_LIST) => {
                let items = TryAsRef::<List>::try_as_ref(value)
                    .map(|list| &list[..])
                    .unwrap_or(&[]);
                if items.is_empty() {
                    self.leaves.insert(path.clone(), layer);
                    return;
                }
                for (index, child) in items.iter().enumerate() {
                    path.push(index.to_string());
                    self.claim(path, child, layer, depth + 1);
                    path.pop();
                }
            }
            _ => {
                self.leaves.insert(path.clone(), layer);
            }
        }
    }

    /// Drops `path` and everything beneath it, so a subtree the later
    /// layer replaced stops being attributed to the layer it came from.
    /// The empty path is the root, and drops everything.
    fn forget_subtree(&mut self, path: &[String]) {
        // A path sorts before everything beneath it and those follow it
        // together, so the subtree is one range of the leaves.
        let beneath: Vec<Vec<String>> = self
            .leaves
            .range(path.to_vec()..)
            .map(|(k, _)| k)
            .take_while(|k| k.starts_with(path))
            .cloned()
            .collect();
        for leaf in beneath {
            self.leaves.remove(&leaf);
        }
    }
}

/// `parent` with one more segment: the path of a child.
fn child(parent: &[String], segment: &str) -> Vec<String> {
    let mut path = Vec::with_capacity(parent.len() + 1);
    path.extend_from_slice(parent);
    path.push(segment.to_string());
    path
}

/// A per-path mode override: what a *declarer* of an option knows that
/// its merger does not -- whether a list is an unordered tag set (where
/// [`MergeMode::Deep`] is right) or an ordered fallback chain.
/// Deliberately a plain path-to-mode map and not a schema type.
///
/// A path is a list of segments, matched **exactly**: an override on
/// `["tls"]` does not govern `["tls", "ciphers"]`, which keeps a
/// declaration's blast radius visible, and one on the key `tls.ciphers`
/// (a single segment) does not govern the key `ciphers` inside `tls`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeOverrides {
    by_path: BTreeMap<Vec<String>, MergeMode>,
}

impl MergeOverrides {
    /// No overrides -- every path takes the call-site mode.
    pub fn new() -> MergeOverrides {
        MergeOverrides {
            by_path: BTreeMap::new(),
        }
    }

    /// Declares `mode` for `path`, replacing any previous declaration.
    pub fn set<I, S>(&mut self, path: I, mode: MergeMode)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.by_path
            .insert(path.into_iter().map(Into::into).collect(), mode);
    }

    /// Chained form of [`MergeOverrides::set`].
    #[must_use]
    pub fn with<I, S>(mut self, path: I, mode: MergeMode) -> MergeOverrides
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.set(path, mode);
        self
    }

    /// The mode declared for `path`, if any.
    pub fn get<I, S>(&self, path: I) -> Option<MergeMode>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let path: Vec<String> = path.into_iter().map(|s| s.as_ref().to_string()).collect();
        self.by_path.get(&path).copied()
    }

    /// Whether anything is declared at all.
    pub fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }
}

/// Everything a merge needs beyond the two values: the fallback mode, the
/// per-path overrides, and the mode sub-options.
#[derive(Debug, Clone, Copy, Default)]
struct Context<'o> {
    /// The call-site default, used for every path no override names.
    mode: MergeMode,
    options: MergeOptions,
    overrides: Option<&'o MergeOverrides>,
}

impl Context<'_> {
    /// The mode in force at `path`: a schema-declared override if there
    /// is one, else the call-site default.
    fn mode_at(&self, path: &[String]) -> MergeMode {
        self.overrides
            .and_then(|o| o.by_path.get(path).copied())
            .unwrap_or(self.mode)
    }
}

impl MergeMode {
    /// Merges `later` into `earlier`, later winning, with this mode
    /// applied to every path.
    ///
    /// Neither input is touched. The result is a new tree built through
    /// `alloc`, which is also the allocator it will free through.
    pub fn merge(&self, earlier: &Value, later: &Value, alloc: Alloc) -> Result<Value, MergeError> {
        self.merge_with(earlier, later, alloc, MergeOptions::new(), None)
    }

    /// [`MergeMode::merge`] with the mode sub-options and per-path
    /// overrides spelled out.
    pub fn merge_with(
        &self,
        earlier: &Value,
        later: &Value,
        alloc: Alloc,
        options: MergeOptions,
        overrides: Option<&MergeOverrides>,
    ) -> Result<Value, MergeError> {
        let ctx = Context {
            mode: *self,
            options,
            overrides,
        };
        merge_value(earlier, later, &[], &ctx, alloc, 0)
    }

    /// Folds labelled layers left to right, later winning, recording
    /// which layer each **leaf path** came from. Labels are the caller's,
    /// so a settings form can show "inherited from system" rather than an
    /// index. An empty list yields an empty map and empty provenance.
    pub fn merge_layers<'l>(
        &self,
        layers: impl IntoIterator<Item = (&'l str, &'l Value)>,
        alloc: Alloc,
    ) -> Result<(Value, Provenance<'l>), MergeError> {
        self.merge_layers_with(layers, alloc, MergeOptions::new(), None)
    }

    /// [`MergeMode::merge_layers`] with sub-options and overrides.
    pub fn merge_layers_with<'l>(
        &self,
        layers: impl IntoIterator<Item = (&'l str, &'l Value)>,
        alloc: Alloc,
        options: MergeOptions,
        overrides: Option<&MergeOverrides>,
    ) -> Result<(Value, Provenance<'l>), MergeError> {
        let ctx = Context {
            mode: *self,
            options,
            overrides,
        };
        let mut accumulated: Option<Value> = None;
        let mut provenance = Provenance::default();

        for (label, layer) in layers {
            match accumulated {
                None => {
                    provenance.claim(&mut Vec::new(), layer, label, 0);
                    accumulated = Some(clone_into(layer, alloc)?);
                }
                Some(previous) => {
                    record_claims(&previous, layer, &[], &ctx, label, &mut provenance, 0);
                    accumulated = Some(merge_value(&previous, layer, &[], &ctx, alloc, 0)?);
                }
            }
        }

        Ok((
            accumulated.unwrap_or_else(|| Map::new_in(alloc).into()),
            provenance,
        ))
    }
}

/// The kind of a value, or `None` for a tag this build does not know.
fn kind(value: &Value) -> Option<Tag> {
    value.tag().ok()
}

/// Through the door, so a foreign map whose keys are not text is a leaf,
/// and copying it refuses.
fn is_map(value: &Value) -> bool {
    TryAsRef::<Map>::try_as_ref(value).is_some()
}

fn is_list(value: &Value) -> bool {
    kind(value) == Some(Tag::GUATIAO_LIST)
}

/// Whether a value is a leaf kind — neither a map nor a list. Null
/// counts, and so does a tag this build does not know: that decides only
/// how it is classified. The clone that would put it in the result
/// answers [`ValueError::UnknownTag`] instead, because bit-copying a
/// payload this build cannot interpret would produce two owners of
/// whatever it points at.
fn is_scalar(value: &Value) -> bool {
    !is_map(value) && !is_list(value)
}

/// A deep copy of `value` into `alloc`, which is how anything from either
/// input reaches the result.
fn clone_into(value: &Value, alloc: Alloc) -> Result<Value, MergeError> {
    Ok(value.to_value_in(alloc)?)
}

/// Whether the two maps share at least one key, compared as raw bytes so
/// a key the contract would reject still compares correctly.
fn shares_a_key(a: &Value, b: &Value) -> bool {
    let Some(a) = TryAsRef::<Map>::try_as_ref(a).map(Lookup::new) else {
        return false;
    };
    TryAsRef::<Map>::try_as_ref(b)
        .map(Map::entries)
        .unwrap_or(&[])
        .iter()
        .any(|e| a.contains_key(e.key()))
}

/// Walks the same shape `merge_value` will, recording which leaves the
/// incoming layer is about to claim.
///
/// A second walk rather than a recorder in every arm, which most callers
/// would pay for and never use. The cost is staying in step, which
/// `provenance_is_per_leaf_and_reports_mixed_for_an_interior_node`
/// catches.
fn record_claims<'a>(
    earlier: &Value,
    later: &Value,
    path: &[String],
    ctx: &Context<'_>,
    label: &'a str,
    provenance: &mut Provenance<'a>,
    depth: u32,
) {
    if depth >= MAX_DEPTH {
        return;
    }
    // Absent means no opinion: the merge keeps what the earlier layers
    // had, so this layer claims nothing here.
    if kind(later) == Some(Tag::GUATIAO_ABSENT) {
        return;
    }
    let mode = ctx.mode_at(path);

    if mode == MergeMode::Simple {
        record_simple(earlier, later, path, label, provenance, depth);
        return;
    }

    // Recursive map-into-map: descend, so only the keys the later layer
    // actually carries change hands.
    if matches!(mode, MergeMode::Substitute | MergeMode::Deep) && is_map(earlier) && is_map(later) {
        let earlier_map = TryAsRef::<Map>::try_as_ref(earlier).map(Lookup::new);
        for (key, later_child) in TryAsRef::<Map>::try_as_ref(later)
            .map(Map::entries)
            .unwrap_or(&[])
            .iter()
            .map(|e| (e.key(), e.value()))
        {
            let child_path = child(path, key);
            match earlier_map.as_ref().and_then(|m| m.get(key)) {
                Some(earlier_child) => record_claims(
                    earlier_child,
                    later_child,
                    &child_path,
                    ctx,
                    label,
                    provenance,
                    depth + 1,
                ),
                None => replace(provenance, child_path, later_child, label, depth + 1),
            }
        }
        return;
    }

    // `Deep` UNIONS lists, and which indices the appended items land at
    // is not knowable without redoing the union. So the whole list is
    // claimed only when the earlier one was empty; otherwise its
    // retained leaves keep their own layer and `source_of` reports the
    // mix, which is the honest answer for a union.
    if mode == MergeMode::Deep && is_list(earlier) && is_list(later) {
        if TryAsRef::<List>::try_as_ref(earlier)
            .map(|list| &list[..])
            .unwrap_or(&[])
            .is_empty()
        {
            replace(provenance, path.to_vec(), later, label, depth);
        }
        return;
    }

    // Everything else: the later value wins this path outright, so the old
    // subtree's paths are forgotten and the new one claimed.
    replace(provenance, path.to_vec(), later, label, depth);
}

/// `Simple`'s claims: the shape [`merge_simple`] builds. A map replaces
/// key by key, each key's old subtree and all; a list overwrites by
/// position and leaves the earlier excess where it was; anything else
/// replaces the path outright. The root is never forgotten for a map
/// merge, because the earlier layer's other keys stand.
fn record_simple<'a>(
    earlier: &Value,
    later: &Value,
    path: &[String],
    label: &'a str,
    provenance: &mut Provenance<'a>,
    depth: u32,
) {
    if depth >= MAX_DEPTH || kind(later) == Some(Tag::GUATIAO_ABSENT) {
        return;
    }
    if is_map(earlier) && is_map(later) {
        for entry in TryAsRef::<Map>::try_as_ref(later)
            .map(Map::entries)
            .unwrap_or(&[])
            .iter()
            .filter(|e| kind(e.value()) != Some(Tag::GUATIAO_ABSENT))
        {
            replace(
                provenance,
                child(path, entry.key()),
                entry.value(),
                label,
                depth + 1,
            );
        }
        return;
    }
    if is_list(earlier) && is_list(later) {
        let earlier_items = TryAsRef::<List>::try_as_ref(earlier)
            .map(|list| &list[..])
            .unwrap_or(&[]);
        for (index, later_item) in TryAsRef::<List>::try_as_ref(later)
            .map(|list| &list[..])
            .unwrap_or(&[])
            .iter()
            .enumerate()
        {
            let item_path = child(path, &index.to_string());
            match earlier_items.get(index) {
                Some(earlier_item) => {
                    record_simple(
                        earlier_item,
                        later_item,
                        &item_path,
                        label,
                        provenance,
                        depth + 1,
                    );
                }
                None => replace(provenance, item_path, later_item, label, depth + 1),
            }
        }
        return;
    }
    replace(provenance, path.to_vec(), later, label, depth);
}

/// `later` takes `path` outright: whatever was recorded there is
/// forgotten and every leaf of `later` is claimed for `label`.
fn replace<'a>(
    provenance: &mut Provenance<'a>,
    mut path: Vec<String>,
    later: &Value,
    label: &'a str,
    depth: u32,
) {
    provenance.forget_subtree(&path);
    provenance.claim(&mut path, later, label, depth);
}

/// The core: one merge step, dispatching on the mode in force at `path`.
fn merge_value(
    earlier: &Value,
    later: &Value,
    path: &[String],
    ctx: &Context<'_>,
    alloc: Alloc,
    depth: u32,
) -> Result<Value, MergeError> {
    if depth >= MAX_DEPTH {
        return Err(MergeError::Build(ValueError::TooDeep));
    }

    // "Absent means no opinion", for the one spelling of absence the C
    // form can store in a container. A key the later layer does not carry
    // never reaches this function at all — the map arm only recurses into
    // keys it has — so this is the same rule said a second way, for a
    // producer that spells the same thing with the sentinel.
    if kind(later) == Some(Tag::GUATIAO_ABSENT) {
        return clone_into(earlier, alloc);
    }
    if kind(earlier) == Some(Tag::GUATIAO_ABSENT) {
        return clone_into(later, alloc);
    }

    match ctx.mode_at(path) {
        MergeMode::Simple => merge_simple(earlier, later, path, ctx, alloc, depth),
        MergeMode::Substitute => merge_substitute(earlier, later, path, ctx, alloc, depth),
        MergeMode::Deep => merge_deep(earlier, later, path, ctx, alloc, depth),
    }
}

/// `Simple`: scalars and maps replace, lists overwrite positionally.
///
/// A stored absent sentinel is no opinion here as everywhere else: an
/// absent element or map entry keeps the earlier value. A stored null is a
/// value and *does* overwrite.
fn merge_simple(
    earlier: &Value,
    later: &Value,
    path: &[String],
    ctx: &Context<'_>,
    alloc: Alloc,
    depth: u32,
) -> Result<Value, MergeError> {
    // List elements recurse here directly, so the bound is checked here
    // and not only in `merge_value`.
    if depth >= MAX_DEPTH {
        return Err(MergeError::Build(ValueError::TooDeep));
    }
    if kind(later) == Some(Tag::GUATIAO_ABSENT) {
        return clone_into(earlier, alloc);
    }
    if kind(earlier) == Some(Tag::GUATIAO_ABSENT) {
        return clone_into(later, alloc);
    }
    if is_scalar(later) {
        return clone_into(later, alloc);
    }

    if is_list(later) {
        if !is_list(earlier) {
            // A list replacing a non-list is a plain replacement.
            return clone_into(later, alloc);
        }
        return overwrite_positionally(earlier, later, path, ctx, alloc, depth);
    }

    // A map on the right.
    if !is_map(earlier) {
        return clone_into(later, alloc);
    }

    // SHALLOW: top-level keys only, each replaced outright, which is the
    // whole distinction from `Substitute`. Built as a copy of the earlier
    // map with the later's keys written over it, so a replaced key keeps
    // its position and a new one lands at the end.
    let mut out = Filling::new(
        Map::try_from(clone_into(earlier, alloc)?)
            .map_err(|_| MergeError::from(ValueError::WrongKind))?,
    );
    for entry in <&Map>::try_from(later).map(Map::entries).unwrap_or(&[]) {
        // An absent entry keeps whatever the earlier map holds there; over
        // a key the earlier map lacks it is copied, as `Substitute` does.
        if kind(entry.value()) == Some(Tag::GUATIAO_ABSENT) && out.contains_key(entry.key()) {
            continue;
        }
        out.set_in(entry.key(), clone_into(entry.value(), alloc)?, alloc)?;
    }
    Ok(out.into_map().into())
}

/// `Simple`'s list rule: element `i` of the later replaces element `i`
/// of the earlier, recursing through `Simple` itself; excess later
/// elements are appended and excess EARLIER ones survive.
///
/// That survival -- `[1,2,3]` updated with `[10,20]` is `[10,20,3]` --
/// is what makes `Simple` unable to shorten a list. A caller that wants
/// the later list to *be* the list wants [`MergeMode::Substitute`].
fn overwrite_positionally(
    earlier: &Value,
    later: &Value,
    path: &[String],
    ctx: &Context<'_>,
    alloc: Alloc,
    depth: u32,
) -> Result<Value, MergeError> {
    let earlier_items = TryAsRef::<List>::try_as_ref(earlier)
        .map(|list| &list[..])
        .unwrap_or(&[]);
    let later_items = TryAsRef::<List>::try_as_ref(later)
        .map(|list| &list[..])
        .unwrap_or(&[]);

    let mut out = List::new_in(alloc);
    for (index, later_item) in later_items.iter().enumerate() {
        let combined = match earlier_items.get(index) {
            Some(earlier_item) => merge_simple(
                earlier_item,
                later_item,
                &child(path, &index.to_string()),
                ctx,
                alloc,
                depth + 1,
            )?,
            None => clone_into(later_item, alloc)?,
        };
        out.push_in(combined, alloc)?;
    }
    for leftover in earlier_items.iter().skip(later_items.len()) {
        out.push_in(clone_into(leftover, alloc)?, alloc)?;
    }
    Ok(out.into())
}

/// `Substitute`: recursive maps, wholesale list replacement.
///
/// Replacement happens outright only when the earlier side is **not** a
/// map, so a map on the left is never overwritten by a scalar -- that
/// combination is an error. [`merge_deep`] deliberately differs, and
/// neither should be "fixed" to match the other.
fn merge_substitute(
    earlier: &Value,
    later: &Value,
    path: &[String],
    ctx: &Context<'_>,
    alloc: Alloc,
    depth: u32,
) -> Result<Value, MergeError> {
    // Not a map on the left: a scalar or a list on the right replaces it
    // outright.
    if !is_map(earlier) {
        if is_scalar(later) || is_list(later) {
            return clone_into(later, alloc);
        }
        // A map arriving on top of a non-map: preserved as an error rather
        // than "fixed", because a map replacing a scalar silently is
        // exactly the discard this mode refuses to make.
        return Err(MergeError::Kind {
            path: path::render(path),
            earlier: kind(earlier),
            later: kind(later),
            mode: MergeMode::Substitute,
        });
    }

    if is_map(later) {
        return merge_maps_recursively(earlier, later, path, ctx, alloc, depth);
    }
    if is_list(later) {
        // A map on the left absorbs a LIST OF MAPS on the right, folding
        // each element in sequentially. Non-obvious, and shared with
        // `Deep`.
        return absorb_list_of_maps(
            earlier,
            later,
            path,
            ctx,
            MergeMode::Substitute,
            alloc,
            depth,
        );
    }
    Err(MergeError::Kind {
        path: path::render(path),
        earlier: Some(Tag::GUATIAO_MAP),
        later: kind(later),
        mode: MergeMode::Substitute,
    })
}

/// `Deep`: recursive maps, lists extended with unique items.
///
/// The null handling differs from [`merge_substitute`] on purpose: here a
/// null on the earlier side is simply replaced by the later value.
fn merge_deep(
    earlier: &Value,
    later: &Value,
    path: &[String],
    ctx: &Context<'_>,
    alloc: Alloc,
    depth: u32,
) -> Result<Value, MergeError> {
    // An explicit null is a value with no structure to merge into, so
    // anything replaces it.
    if kind(earlier) == Some(Tag::GUATIAO_NULL) || is_scalar(later) {
        return clone_into(later, alloc);
    }

    if is_list(earlier) && is_list(later) {
        return union_lists(earlier, later, path, ctx, alloc, depth);
    }
    if is_map(earlier) && is_map(later) {
        return merge_maps_recursively(earlier, later, path, ctx, alloc, depth);
    }
    if is_map(earlier) && is_list(later) {
        return absorb_list_of_maps(earlier, later, path, ctx, MergeMode::Deep, alloc, depth);
    }
    Err(MergeError::Kind {
        path: path::render(path),
        earlier: kind(earlier),
        later: kind(later),
        mode: MergeMode::Deep,
    })
}

/// The recursive map rule shared by `Substitute` and `Deep`: keys the
/// later map carries recurse through whatever mode is in force at the
/// CHILD's path, keys only it has are appended, and keys only the
/// earlier map has stand -- "absent means no opinion", enforced by the
/// loop's shape.
///
/// Built earlier-first, so the result keeps the earlier map's key order,
/// which `merging_preserves_the_earlier_maps_key_order` pins.
fn merge_maps_recursively(
    earlier: &Value,
    later: &Value,
    path: &[String],
    ctx: &Context<'_>,
    alloc: Alloc,
    depth: u32,
) -> Result<Value, MergeError> {
    let mut out = Filling::new(Map::new_in(alloc));
    // Each door checks every key, so each side is read once, not per key,
    // and each is searched through its own index, not scanned per key.
    let earlier_map = TryAsRef::<Map>::try_as_ref(earlier);
    let later_map = TryAsRef::<Map>::try_as_ref(later);
    let earlier_keys = earlier_map.map(Lookup::new);
    let later_keys = later_map.map(Lookup::new);

    for (key, earlier_value) in earlier_map
        .map(Map::entries)
        .unwrap_or(&[])
        .iter()
        .map(|e| (e.key(), e.value()))
    {
        let child = match later_keys.as_ref().and_then(|m| m.get(key)) {
            Some(later_value) => merge_value(
                earlier_value,
                later_value,
                &child(path, key),
                ctx,
                alloc,
                depth + 1,
            )?,
            None => clone_into(earlier_value, alloc)?,
        };
        out.set_in(key, child, alloc)?;
    }

    for (key, later_value) in later_map
        .map(Map::entries)
        .unwrap_or(&[])
        .iter()
        .map(|e| (e.key(), e.value()))
    {
        if !earlier_keys.as_ref().is_some_and(|m| m.contains_key(key)) {
            out.set_in(key, clone_into(later_value, alloc)?, alloc)?;
        }
    }

    Ok(out.into_map().into())
}

/// A map on the left absorbing a list of maps on the right, folding each
/// element in sequentially. Both `Substitute` and `Deep` do this.
///
/// A non-map element is an error, not a skip: a list mixing maps and
/// scalars being merged into a map has no reading in which the scalars
/// mean anything, and dropping them silently is the discard these modes
/// refuse to make.
fn absorb_list_of_maps(
    earlier: &Value,
    later: &Value,
    path: &[String],
    ctx: &Context<'_>,
    mode: MergeMode,
    alloc: Alloc,
    depth: u32,
) -> Result<Value, MergeError> {
    let mut result = clone_into(earlier, alloc)?;
    for item in TryAsRef::<List>::try_as_ref(later)
        .map(|list| &list[..])
        .unwrap_or(&[])
        .iter()
    {
        if !is_map(item) {
            return Err(MergeError::Kind {
                path: path::render(path),
                earlier: Some(Tag::GUATIAO_MAP),
                later: kind(item),
                mode,
            });
        }
        let folded = match mode {
            MergeMode::Substitute => merge_substitute(&result, item, path, ctx, alloc, depth + 1)?,
            _ => merge_deep(&result, item, path, ctx, alloc, depth + 1)?,
        };
        result = folded;
    }
    Ok(result)
}

/// `Deep`'s list rule: extend with unique items. With `mergelists` off
/// (the default) unique non-map items are appended and map items always
/// are; with it on, map elements merge by position and only when a key
/// overlaps.
///
/// Uniqueness is structural equality. Nothing here removes anything,
/// which is why `Deep` cannot shrink a list.
fn union_lists(
    earlier: &Value,
    later: &Value,
    path: &[String],
    ctx: &Context<'_>,
    alloc: Alloc,
    depth: u32,
) -> Result<Value, MergeError> {
    let mut result: Vec<Value> = Vec::new();
    for item in TryAsRef::<List>::try_as_ref(earlier)
        .map(|list| &list[..])
        .unwrap_or(&[])
        .iter()
    {
        result.push(clone_into(item, alloc)?);
    }
    let mut held = Held::new();

    if ctx.options.mergelists {
        // Map elements of the later list, by the position they sat at --
        // the candidates for a positional merge.
        let mut later_maps: BTreeMap<usize, &Value> = TryAsRef::<List>::try_as_ref(later)
            .map(|list| &list[..])
            .unwrap_or(&[])
            .iter()
            .enumerate()
            .filter(|(_, item)| is_map(item))
            .collect();

        // Non-map items first, unique only, and before the positional
        // pass. The order is part of the contract: it is what a consumer
        // diffing two merged lists sees.
        for item in TryAsRef::<List>::try_as_ref(later)
            .map(|list| &list[..])
            .unwrap_or(&[])
            .iter()
        {
            if !is_map(item) && !held.contains(&result, item) {
                result.push(clone_into(item, alloc)?);
            }
        }

        // Positional map merge, and ONLY where at least one key overlaps.
        // Two maps with nothing in common are two records that happen to
        // be adjacent, not one record described twice.
        //
        // Decided in one pass and applied in a second, so the scan can
        // borrow `result` while the application replaces entries in it.
        let mergeable: Vec<usize> = result
            .iter()
            .enumerate()
            .filter(|(index, item)| match later_maps.get(index) {
                Some(later_map) => is_map(item) && shares_a_key(item, later_map),
                None => false,
            })
            .map(|(index, _)| index)
            .collect();

        for index in mergeable {
            let Some(later_item) = later_maps.remove(&index) else {
                continue;
            };
            let merged = merge_deep(
                &result[index],
                later_item,
                &child(path, &index.to_string()),
                ctx,
                alloc,
                depth + 1,
            )?;
            result[index] = merged;
        }

        // Anything not merged is appended rather than dropped.
        for (_, value) in later_maps {
            result.push(clone_into(value, alloc)?);
        }
    } else {
        // The fast path, and its ordering is load-bearing: every unique
        // non-map item is appended FIRST, then every map item
        // unconditionally. A single pass would interleave them
        // differently, which is visible to any caller that renders the
        // list.
        for item in TryAsRef::<List>::try_as_ref(later)
            .map(|list| &list[..])
            .unwrap_or(&[])
            .iter()
        {
            if !is_map(item) && !held.contains(&result, item) {
                result.push(clone_into(item, alloc)?);
            }
        }
        for item in TryAsRef::<List>::try_as_ref(later)
            .map(|list| &list[..])
            .unwrap_or(&[])
            .iter()
        {
            if is_map(item) {
                result.push(clone_into(item, alloc)?);
            }
        }
    }

    let mut out = List::new_in(alloc);
    for item in result {
        out.push_in(item, alloc)?;
    }
    Ok(out.into())
}

/// Up to this many items, a later item is looked for by comparing it with
/// every one held.
const SCAN: usize = 16;

/// Finds whether a list being built already holds a value equal to an
/// item, by a seeded hash of the item's structure past [`SCAN`] items, so
/// a union of N and M items is linear in them rather than N*M
/// comparisons. The list may only grow between lookups.
struct Held {
    hasher: RandomState,
    /// The first position of each hash among the items indexed so far.
    first: HashMap<u64, usize>,
    /// How many items of the list are indexed.
    indexed: usize,
}

impl Held {
    fn new() -> Held {
        Held {
            hasher: RandomState::new(),
            first: HashMap::new(),
            indexed: 0,
        }
    }

    fn hash(&self, value: &Value) -> u64 {
        let mut state = self.hasher.build_hasher();
        walk::hash(value, &mut state);
        state.finish()
    }

    /// Whether `built` holds a value equal to `item`.
    fn contains(&mut self, built: &[Value], item: &Value) -> bool {
        if built.len() <= SCAN {
            return built.iter().any(|held| held == item);
        }
        for (at, held) in built.iter().enumerate().skip(self.indexed) {
            let hash = self.hash(held);
            self.first.entry(hash).or_insert(at);
        }
        self.indexed = built.len();
        match self.first.get(&self.hash(item)) {
            None => false,
            Some(&at) if built[at] == *item => true,
            // Another value with the same seeded hash: rare enough that
            // the scan stays off the common path.
            Some(_) => built.iter().any(|held| held == item),
        }
    }
}

#[cfg(test)]
mod tests;
