// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Combining two values, later layer winning — the three strategies, and
//! who came from where.
//!
//! # Why three modes and not one
//!
//! Configuration layering asks two *independent* questions, and an
//! abstract "shallow versus deep" only answers the first:
//!
//! 1. When both layers hold a **map**, does the later one replace it or
//!    recurse into it?
//! 2. When both layers hold a **list**, does the later one replace it,
//!    overwrite it positionally, or union with it?
//!
//! [`MergeMode`] names the three useful combinations:
//!
//! | mode | nested map | list |
//! |---|---|---|
//! | [`MergeMode::Simple`] | shallow update, top-level keys only | positional replace, then append excess |
//! | [`MergeMode::Substitute`] (default) | recursive | replaces wholesale |
//! | [`MergeMode::Deep`] | recursive | extends with unique items |
//!
//! # Why `Substitute` is the default
//!
//! **`Deep` cannot shrink a list.** Every override only ever appends, so
//! a later layer can never *remove* an inherited tag, address or cipher.
//! Removal would need a null-sentinel convention, and inventing one is a
//! worse problem than the one it solves. `Substitute` gives layering what
//! it actually needs — override one inner key without restating the whole
//! map — while leaving lists predictable: a later layer's list **is** the
//! list. `Deep` stays on the enum because it is right for a genuine
//! unordered set, which is exactly the case a caller has to *name*.
//!
//! `list_shrinks_under_substitute_and_provably_cannot_under_deep` in this
//! module's tests is that asymmetry, asserted rather than asserted-about.
//!
//! # The result is a NEW tree, built through the allocator you name
//!
//! Neither input is touched, and nothing is moved out of either: every
//! value the result keeps is deep-copied into the allocator passed in. It
//! costs copies, and it buys three things that matter more.
//!
//! - **The inputs stay usable**, which a merge that consumed them could
//!   not offer. A caller layering four configurations keeps all four.
//! - **The result is freeable on its own.** An owned container records the
//!   allocator that made it, so a result stitched together from two
//!   allocators would free half of itself through each — and an arena
//!   released by a library would leave the host holding pointers into
//!   freed memory.
//! - **It is expressible in safe code.** Moving a subtree out of one
//!   container into another needs the raw functions, every one of which is
//!   `unsafe`, and this module carries `forbid(unsafe_code)` for the same
//!   reason the rest of the crate outside `ffi` does.
//!
//! # Absent means "no opinion", never "delete"
//!
//! A key missing from the later layer leaves the earlier value standing.
//! Removal, if ever needed, gets an explicit call — never a magic value.
//!
//! The C form can also *store* the absent sentinel in a container, which
//! the model this replaced could not, so the rule is now written down
//! twice: a stored absent on the later side leaves the earlier value
//! standing, and one on the earlier side is nothing to merge into. A
//! stored **null** is a different statement and still overwrites — a
//! caller who wrote null meant it.
//!
//! # A type mismatch is an error, not a guess
//!
//! Merging a string into a list has no defensible answer, so it returns
//! [`MergeError`] rather than picking one. A silent replacement here is
//! how a config layer quietly discards a value: the user sets an option,
//! the merge decides the shapes disagree, the earlier value survives, and
//! nothing anywhere says so.
//!
//! # Provenance is per LEAF PATH
//!
//! After a recursive merge, "where did `tls` come from?" has no single
//! answer — `tls.verify` may come from the user layer while `tls.ca` came
//! from the system one. Recording provenance per top-level key would be a
//! confident lie. So [`Provenance`] is keyed by leaf path
//! (`"tls.ca"`), and asking about an interior node whose leaves came from
//! several layers answers [`Source::Mixed`].
//!
//! ```
//! use guatiao::{Alloc, Map, MergeMode, ReadValue, Source, Value};
//!
//! let mut system_tls = Map::new();
//! system_tls.set("ca", "/etc/ca.pem")?;
//! system_tls.set("verify", true)?;
//! let mut system = Map::new();
//! system.set("tls", system_tls)?;
//!
//! let mut user_tls = Map::new();
//! user_tls.set("verify", false)?;
//! let mut user = Map::new();
//! user.set("tls", user_tls)?;
//!
//! // The merge builds a NEW tree, so it is told where to put it. That is
//! // the one allocator this example names, and a host merging into its
//! // own arena is exactly who names a different one.
//! // The layers cross as values, which is the one place a container
//! // becomes a node: `.into()` is a move, so nothing is copied.
//! let (system, user) = (Value::from(system), Value::from(user));
//! let (merged, provenance) = MergeMode::Substitute.merge_layers(
//!     [("system", &system), ("user", &user)],
//!     Alloc::rust(),
//! )?;
//!
//! let verify: bool = merged.get("tls").get("verify").ok_or_missing()?.try_into()?;
//! assert!(!verify);
//! assert_eq!(provenance.source_of("tls.verify"), Source::Layer("user"));
//! assert_eq!(provenance.source_of("tls.ca"), Source::Layer("system"));
//! // The map itself was drawn from both, so no single layer is the honest
//! // answer.
//! assert_eq!(provenance.source_of("tls"), Source::Mixed);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # This crate still has no schema dependency in the direction that matters
//!
//! [`MergeMode`] is a **call-site default**, applied to every key. A
//! declarer that knows an option's meaning better than its merger does
//! can override the mode per key by handing in a
//! [`MergeOverrides`] map — which is a plain `path -> MergeMode` lookup
//! this module defines, deliberately *not* a schema type.
//! [`crate::schema::merge`] reads a schema's own annotations and builds
//! one; the merge itself never learns what a schema is.

// Merging is provably safe: the `unsafe` this crate contains is confined
// to `src/ffi/`. See the crate root.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use crate::value::alloc::Alloc;
use crate::value::convert::ToValue;
use crate::value::mutate::{MAX_DEPTH, ValueError};
use crate::value::read::{entries, equal, items};
use crate::value::types::{Tag, Value};

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
        /// The dotted path where they disagreed, empty at the root.
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

impl std::fmt::Display for MergeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
                    kind_name(*later),
                    kind_name(*earlier)
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

impl std::error::Error for MergeError {}

/// A kind in a sentence, for [`MergeError`]'s message.
fn kind_name(tag: Option<Tag>) -> &'static str {
    let Some(tag) = tag else {
        return "a kind this build does not know";
    };
    match tag {
        Tag::GUATIAO_ABSENT => "absent",
        Tag::GUATIAO_NULL => "null",
        Tag::GUATIAO_BOOL => "a boolean",
        Tag::GUATIAO_NUMBER => "a number",
        Tag::GUATIAO_STRING => "a string",
        Tag::GUATIAO_BYTES => "a byte string",
        Tag::GUATIAO_LIST => "a list",
        Tag::GUATIAO_MAP => "a map",
    }
}

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
/// Keyed by **leaf path** — `"tls.ca"` — because an interior node has no
/// single answer after a recursive merge. Sorted, so the prefix scan
/// [`Provenance::source_of`] does is a range rather than a full walk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Provenance<'a> {
    leaves: BTreeMap<String, &'a str>,
}

impl<'a> Provenance<'a> {
    /// Where the value at `path` came from.
    ///
    /// An exact leaf answers its layer. An interior node answers the layer
    /// every leaf beneath it agrees on, [`Source::Mixed`] when they do
    /// not, and [`Source::Absent`] when nothing is recorded there.
    pub fn source_of(&self, path: &str) -> Source<'a> {
        if let Some(layer) = self.leaves.get(path) {
            return Source::Layer(layer);
        }
        let prefix = format!("{path}.");
        let mut found: Option<&'a str> = None;
        for (_, layer) in self
            .leaves
            .range(prefix.clone()..)
            .take_while(|(k, _)| k.starts_with(&prefix))
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

    /// Every recorded leaf path and its layer, in sorted path order.
    pub fn leaves(&self) -> impl Iterator<Item = (&str, &'a str)> {
        self.leaves.iter().map(|(k, v)| (k.as_str(), *v))
    }

    /// Records `layer` as the source of every leaf under `path`.
    ///
    /// An empty map or list is itself a leaf: there is nothing beneath it
    /// to attribute, and attributing nothing would make it invisible to
    /// [`Provenance::source_of`].
    fn claim(&mut self, path: &str, value: &Value, layer: &'a str, depth: u32) {
        if depth >= MAX_DEPTH {
            self.leaves.insert(path.to_string(), layer);
            return;
        }
        match kind(value) {
            Some(Tag::GUATIAO_MAP) => {
                let entries = value.entries().unwrap_or(&[]);
                if entries.is_empty() {
                    self.leaves.insert(path.to_string(), layer);
                    return;
                }
                for entry in entries {
                    let child = join(path, &path_segment(entry.key()));
                    self.claim(&child, entry.value(), layer, depth + 1);
                }
            }
            Some(Tag::GUATIAO_LIST) => {
                let items = value.items().unwrap_or(&[]);
                if items.is_empty() {
                    self.leaves.insert(path.to_string(), layer);
                    return;
                }
                for (index, child) in items.iter().enumerate() {
                    let child_path = join(path, &index.to_string());
                    self.claim(&child_path, child, layer, depth + 1);
                }
            }
            _ => {
                self.leaves.insert(path.to_string(), layer);
            }
        }
    }

    /// Drops `path` and everything beneath it, so a subtree the later
    /// layer replaced stops being attributed to the layer it came from.
    fn forget_subtree(&mut self, path: &str) {
        let prefix = format!("{path}.");
        self.leaves
            .retain(|k, _| k != path && !k.starts_with(&prefix));
    }
}

/// Joins a parent path and a segment with the dot the whole crate uses.
fn join(parent: &str, segment: &str) -> String {
    if parent.is_empty() {
        segment.to_string()
    } else {
        format!("{parent}.{segment}")
    }
}

/// A key as a path segment, for a diagnostic.
///
/// Lossy on purpose, and the one place in this module that tolerates a key
/// the contract says should be UTF-8: a **path is a label** and a wrong
/// character in one costs nothing, while a **key is data** and writing a
/// changed one into the result would produce a map that is quietly not the
/// one it came from. [`key_text`] is the other half of that split.
fn path_segment(key: &[u8]) -> String {
    String::from_utf8_lossy(key).into_owned()
}

/// A key as text, refusing one that is not UTF-8.
///
/// A `Text` is UTF-8 by contract, so this only rejects a tree
/// some other language built in violation of it. Refusing is what
/// [`Value::copy_from`] already does, and the alternative — a lossy
/// conversion — does not fail, it **renames the key**.
fn key_text(key: &[u8]) -> Result<&str, MergeError> {
    std::str::from_utf8(key).map_err(|_| MergeError::Build(ValueError::NotUtf8))
}

/// A per-path mode override: what a *declarer* of an option knows that
/// its merger does not.
///
/// The caller merging two maps does not know what the values mean. The
/// declarer of an option knows whether its list is an unordered tag set
/// (where [`MergeMode::Deep`]'s union is right) or an ordered fallback
/// chain (where a later layer must replace it wholesale). This is the
/// channel that lets the declarer say so.
///
/// **Deliberately a plain path→mode map defined here, not a schema type.**
/// [`crate::schema::merge`] reads a schema's own annotations and builds one of these;
/// the merge never learns what a schema is. A merge with no overrides at
/// all is fully usable, because the call-site mode covers every path.
///
/// Paths are matched **exactly**, against the same dotted spelling
/// [`Provenance`] uses. An override on `"tls"` governs how the `tls` maps
/// themselves combine; it does not implicitly govern `"tls.ciphers"`,
/// which takes its own entry. Exact matching rather than prefix
/// inheritance is the choice that keeps a declaration's blast radius
/// visible: an option's mode is stated where the option is declared, and
/// nowhere else.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeOverrides {
    by_path: BTreeMap<String, MergeMode>,
}

impl MergeOverrides {
    /// No overrides — every path takes the call-site mode.
    pub fn new() -> MergeOverrides {
        MergeOverrides {
            by_path: BTreeMap::new(),
        }
    }

    /// Declares `mode` for `path`, replacing any previous declaration.
    pub fn set(&mut self, path: impl Into<String>, mode: MergeMode) {
        self.by_path.insert(path.into(), mode);
    }

    /// Chained form of [`MergeOverrides::set`].
    #[must_use]
    pub fn with(mut self, path: impl Into<String>, mode: MergeMode) -> MergeOverrides {
        self.set(path, mode);
        self
    }

    /// The mode declared for `path`, if any.
    pub fn get(&self, path: &str) -> Option<MergeMode> {
        self.by_path.get(path).copied()
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
    fn mode_at(&self, path: &str) -> MergeMode {
        self.overrides
            .and_then(|o| o.get(path))
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
        merge_value(earlier, later, "", &ctx, alloc, 0)
    }

    /// Folds labelled layers left to right, later winning, recording
    /// which layer each **leaf path** ended up coming from.
    ///
    /// Labels are the caller's: `[("system", base), ("user", local)]`
    /// gives an answer a settings form can show — "inherited from system"
    /// — rather than an index nobody can interpret.
    ///
    /// An empty layer list yields an empty map and empty provenance;
    /// a single layer yields a copy of it with every leaf attributed to it.
    pub fn merge_layers<'a, 'l>(
        &self,
        layers: impl IntoIterator<Item = (&'l str, &'l Value)>,
        alloc: Alloc,
    ) -> Result<(Value, Provenance<'l>), MergeError> {
        self.merge_layers_with(layers, alloc, MergeOptions::new(), None)
    }

    /// [`MergeMode::merge_layers`] with sub-options and overrides.
    pub fn merge_layers_with<'a, 'l>(
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
                    provenance.claim("", layer, label, 0);
                    accumulated = Some(clone_into(layer, alloc)?);
                }
                Some(previous) => {
                    record_claims(&previous, layer, "", &ctx, label, &mut provenance, 0);
                    accumulated = Some(merge_value(&previous, layer, "", &ctx, alloc, 0)?);
                }
            }
        }

        Ok((
            accumulated.unwrap_or_else(|| Value::map_in(alloc)),
            provenance,
        ))
    }
}

/// The kind of a value, or `None` for a tag this build does not know.
fn kind(value: &Value) -> Option<Tag> {
    value.tag().ok()
}

fn is_map(value: &Value) -> bool {
    kind(value) == Some(Tag::GUATIAO_MAP)
}

fn is_list(value: &Value) -> bool {
    kind(value) == Some(Tag::GUATIAO_LIST)
}

/// Whether a value is one of the leaf kinds — anything that is neither a
/// map nor a list.
///
/// Null counts as a scalar.
///
/// **A tag this build does not know counts as one too**, which decides
/// only how it is classified, not that it survives: a value this build
/// cannot read is one it cannot copy either, so the clone that would put
/// it in the result answers [`ValueError::UnknownTag`] and the merge
/// reports it. That is the right end — bit-copying a node whose payload
/// this build cannot interpret would produce two owners of whatever it
/// points at — and it is pinned by
/// `a_kind_this_build_cannot_read_stops_the_merge_rather_than_being_copied`.
fn is_scalar(value: &Value) -> bool {
    !is_map(value) && !is_list(value)
}

/// A deep copy of `value` into `alloc`, which is how anything from either
/// input reaches the result.
fn clone_into(value: &Value, alloc: Alloc) -> Result<Value, MergeError> {
    Ok(value.to_value(alloc)?)
}

/// Whether the two maps share at least one key, compared as raw bytes so
/// a key the contract would reject still compares correctly.
fn shares_a_key(a: &Value, b: &Value) -> bool {
    entries(b).any(|(key, _)| entries(a).any(|(other, _)| other == key))
}

/// Walks the same shape `merge_value` will, recording which leaves the
/// incoming layer is about to claim.
///
/// # Why this is a second walk rather than provenance threaded through the merge
///
/// The merge is used far more often without provenance than with it.
/// Threading an optional recorder through every arm would put a branch in
/// the hot path for the benefit of the rarer caller, and would make each
/// arm responsible for remembering to record — the kind of obligation that
/// gets missed when a new arm is added. A separate walk that mirrors the
/// merge's *decisions* keeps the merge itself simple, at the cost of this
/// function having to stay in step with it. The tests that pin recursive
/// provenance (`provenance_is_per_leaf_and_reports_mixed_for_an_interior_node`)
/// are what catch it drifting out of step.
///
/// Where the two could differ is deliberately narrow: this only has to
/// answer "does the incoming value replace this leaf, or recurse past
/// it?", which is a function of the mode and the two kinds — not of the
/// merge's arithmetic.
fn record_claims<'a>(
    earlier: &Value,
    later: &Value,
    path: &str,
    ctx: &Context<'_>,
    label: &'a str,
    provenance: &mut Provenance<'a>,
    depth: u32,
) {
    if depth >= MAX_DEPTH {
        return;
    }
    let mode = ctx.mode_at(path);

    // Recursive map-into-map: descend, so only the keys the later layer
    // actually carries change hands.
    if matches!(mode, MergeMode::Substitute | MergeMode::Deep) && is_map(earlier) && is_map(later) {
        for (key, later_child) in entries(later) {
            let segment = path_segment(key);
            let child_path = join(path, &segment);
            match key_text(key).ok().and_then(|k| earlier.get(k)) {
                Some(earlier_child) => record_claims(
                    earlier_child,
                    later_child,
                    &child_path,
                    ctx,
                    label,
                    provenance,
                    depth + 1,
                ),
                None => {
                    provenance.forget_subtree(&child_path);
                    provenance.claim(&child_path, later_child, label, depth + 1);
                }
            }
        }
        return;
    }

    // `Deep` UNIONS lists, so the earlier list's items keep their own
    // provenance and only the appended ones belong to the later layer.
    // Which indices those appended items land at is not knowable without
    // redoing the union, so this claims the whole list for the later layer
    // ONLY when the earlier one was empty — in which case every item is
    // genuinely the later layer's.
    //
    // A non-empty earlier list is deliberately left partly unclaimed: its
    // retained leaves keep whatever layer they came from, and `source_of`
    // on the list therefore reports that mix. That is the honest answer
    // for a union — the list is not any one layer's — and it is why this
    // arm does not simply claim everything.
    if mode == MergeMode::Deep && is_list(earlier) && is_list(later) {
        if earlier.items().unwrap_or(&[]).is_empty() {
            provenance.forget_subtree(path);
            provenance.claim(path, later, label, depth);
        }
        return;
    }

    // Everything else: the later value wins this path outright, so the old
    // subtree's paths are forgotten and the new one claimed.
    provenance.forget_subtree(path);
    provenance.claim(path, later, label, depth);
}

/// The core: one merge step, dispatching on the mode in force at `path`.
fn merge_value(
    earlier: &Value,
    later: &Value,
    path: &str,
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
/// # The `a is None` question
///
/// Absence and a stored null are different things, and only one of them
/// reaches here. Absence is the key not being in the later map, and the
/// map arm only recurses into keys the later layer carries — so "absent
/// means no opinion" holds structurally rather than by a check. A stored
/// null is a value and *does* overwrite: a caller who wrote null meant
/// it.
fn merge_simple(
    earlier: &Value,
    later: &Value,
    path: &str,
    ctx: &Context<'_>,
    alloc: Alloc,
    depth: u32,
) -> Result<Value, MergeError> {
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

    // SHALLOW: top-level keys only, each replaced outright. This is the
    // whole distinction from `Substitute`, and
    // `simple_and_substitute_each_get_a_case_the_other_gets_right` pins it.
    //
    // Built as a copy of the earlier map with the later map's keys written
    // over it, so a replaced key keeps its position and a new one lands at
    // the end. Insertion order is part of this container's contract —
    // consumers render maps as forms and diff them in tests — and
    // `merging_preserves_the_earlier_maps_key_order` is what catches it.
    let mut out = clone_into(earlier, alloc)?;
    for (key, value) in entries(later) {
        out.set(key_text(key)?, clone_into(value, alloc)?)?;
    }
    Ok(out)
}

/// `Simple`'s list rule: element `i` of the later list replaces element
/// `i` of the earlier, recursing through `Simple` itself so a nested
/// structure inside a list follows the same shallow rule; excess elements
/// of the later list are appended, and excess elements of the *earlier*
/// list SURVIVE.
///
/// That survival is the counter-intuitive half — `[1,2,3]` updated with
/// `[10,20]` is `[10,20,3]` — and it is what makes `Simple` unable to
/// shorten a list. A caller that wants the later list to *be* the list
/// wants [`MergeMode::Substitute`].
fn overwrite_positionally(
    earlier: &Value,
    later: &Value,
    path: &str,
    ctx: &Context<'_>,
    alloc: Alloc,
    depth: u32,
) -> Result<Value, MergeError> {
    let earlier_items = earlier.items().unwrap_or(&[]);
    let later_items = later.items().unwrap_or(&[]);

    let mut out = Value::list_in(alloc);
    for (index, later_item) in later_items.iter().enumerate() {
        let combined = match earlier_items.get(index) {
            Some(earlier_item) => merge_simple(
                earlier_item,
                later_item,
                &join(path, &index.to_string()),
                ctx,
                alloc,
                depth + 1,
            )?,
            None => clone_into(later_item, alloc)?,
        };
        out.push(combined)?;
    }
    for leftover in earlier_items.iter().skip(later_items.len()) {
        out.push(clone_into(leftover, alloc)?)?;
    }
    Ok(out)
}

/// `Substitute`: recursive maps, wholesale list replacement.
///
/// # The `a is None` asymmetry
///
/// Replacement happens outright only when the earlier side is **not** a
/// map; otherwise the mapping branches take over. So a map on the left is
/// never simply overwritten by a scalar — that combination is an error.
///
/// [`merge_deep`] deliberately differs: it returns the later value
/// whenever the earlier side is null. The asymmetry is the point, so
/// neither side should be "fixed" to match the other.
fn merge_substitute(
    earlier: &Value,
    later: &Value,
    path: &str,
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
            path: path.to_string(),
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
        // each element in sequentially. Non-obvious, taken from the prior
        // implementation, and shared with `Deep`.
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
        path: path.to_string(),
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
    path: &str,
    ctx: &Context<'_>,
    alloc: Alloc,
    depth: u32,
) -> Result<Value, MergeError> {
    // A stored null is this crate's nothing and stands in for the prior
    // implementation's `None` on the earlier side -- an explicit null is a
    // value with no structure to merge into, so anything replaces it.
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
        path: path.to_string(),
        earlier: kind(earlier),
        later: kind(later),
        mode: MergeMode::Deep,
    })
}

/// The recursive map rule shared by `Substitute` and `Deep`: keys the
/// later map carries recurse (through whatever mode is in force at the
/// CHILD's path, so an override applies where it is declared), keys only
/// the later map has are appended, and keys only the earlier map has
/// stand — which is "absent means no opinion", enforced by the loop's
/// shape rather than by a check.
///
/// Built earlier-first so the result keeps the earlier map's key order
/// with the later map's new keys appended, which is what
/// `merging_preserves_the_earlier_maps_key_order` pins.
fn merge_maps_recursively(
    earlier: &Value,
    later: &Value,
    path: &str,
    ctx: &Context<'_>,
    alloc: Alloc,
    depth: u32,
) -> Result<Value, MergeError> {
    let mut out = Value::map_in(alloc);

    for (key, earlier_value) in entries(earlier) {
        let key = key_text(key)?;
        let child = match later.get(key) {
            Some(later_value) => merge_value(
                earlier_value,
                later_value,
                &join(path, key),
                ctx,
                alloc,
                depth + 1,
            )?,
            None => clone_into(earlier_value, alloc)?,
        };
        out.set(key, child)?;
    }

    for (key, later_value) in entries(later) {
        let key = key_text(key)?;
        if !earlier.contains_key(key) {
            out.set(key, clone_into(later_value, alloc)?)?;
        }
    }

    Ok(out)
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
    path: &str,
    ctx: &Context<'_>,
    mode: MergeMode,
    alloc: Alloc,
    depth: u32,
) -> Result<Value, MergeError> {
    let mut result = clone_into(earlier, alloc)?;
    for item in items(later) {
        if !is_map(item) {
            return Err(MergeError::Kind {
                path: path.to_string(),
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

/// `Deep`'s list rule: extend with unique items.
///
/// With `mergelists` **off** (the default), unique non-map items are
/// appended and map items are appended unconditionally. With it **on**,
/// map elements merge by position, and only when at least one key
/// overlaps.
///
/// Uniqueness is structural equality, which is order-significant for a map
/// and byte-exact for a number. Note that this is why `Deep` cannot shrink
/// a list: there is no operation here that removes anything.
fn union_lists(
    earlier: &Value,
    later: &Value,
    path: &str,
    ctx: &Context<'_>,
    alloc: Alloc,
    depth: u32,
) -> Result<Value, MergeError> {
    let mut result: Vec<Value> = Vec::new();
    for item in items(earlier) {
        result.push(clone_into(item, alloc)?);
    }

    if ctx.options.mergelists {
        // Map elements of the later list, by the position they sat at --
        // the candidates for a positional merge.
        let mut later_maps: BTreeMap<usize, &Value> = items(later)
            .enumerate()
            .filter(|(_, item)| is_map(item))
            .collect();

        // Non-map items first, unique only, and before the positional
        // pass. The order is part of the contract: it is what a consumer
        // diffing two merged lists sees.
        for item in items(later) {
            if !is_map(item) && !contains(&result, item) {
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
                &join(path, &index.to_string()),
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
        for item in items(later) {
            if !is_map(item) && !contains(&result, item) {
                result.push(clone_into(item, alloc)?);
            }
        }
        for item in items(later) {
            if is_map(item) {
                result.push(clone_into(item, alloc)?);
            }
        }
    }

    let mut out = Value::list_in(alloc);
    for item in result {
        out.push(item)?;
    }
    Ok(out)
}

/// Whether `built` already holds a value equal to `item`.
fn contains(built: &[Value], item: &Value) -> bool {
    built.iter().any(|held| equal(held, item))
}

#[cfg(test)]
mod tests;
