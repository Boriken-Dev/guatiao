// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a source is, apart from the registry that refreshes it: the
//! [`Source`] trait and the [`Candidate`] it lists. Built without `load`,
//! since a source file maps nothing itself; [`source!`](crate::source)
//! exports one.

#![forbid(unsafe_code)]

use super::kind::ProviderError;
use super::raw::{Host, LocalLibrary};
use crate::value::types::Map;

/// One library a source can open, described without running it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    id: String,
    version: String,
    kinds: Vec<String>,
    stamp: String,
}

impl Candidate {
    /// A candidate: the library's `id` (which the library it opens must
    /// carry), its version, the kinds its providers serve, and a `stamp`
    /// the source changes whenever the candidate does.
    pub fn new(
        id: impl Into<String>,
        version: impl Into<String>,
        kinds: Vec<String>,
        stamp: impl Into<String>,
    ) -> Candidate {
        Candidate {
            id: id.into(),
            version: version.into(),
            kinds,
            stamp: stamp.into(),
        }
    }

    /// The library's id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Its version, uninterpreted.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The kinds its providers serve, as the source declares them.
    pub fn kinds(&self) -> &[String] {
        &self.kinds
    }

    /// What changes when the candidate does. Compared, never read.
    pub fn stamp(&self) -> &str {
        &self.stamp
    }
}

/// Something a source wants its host to show that is neither a refusal
/// nor a skip: a setting it runs looser than its default, say. A refresh
/// reports a note once, the first time the source answers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    subject: String,
    text: String,
    detail: Map,
}

impl Note {
    /// A note about `subject` (what part of the source it concerns; empty
    /// for the source as a whole), in words for a person.
    pub fn new(subject: impl Into<String>, text: impl Into<String>) -> Note {
        Note {
            subject: subject.into(),
            text: text.into(),
            detail: Map::new(),
        }
    }

    /// The same note carrying `detail`: what a program reads instead of
    /// the words, keys of the source's choosing.
    pub fn with_detail(self, detail: Map) -> Note {
        Note { detail, ..self }
    }

    /// What part of the source it concerns; empty for the whole.
    pub fn subject(&self) -> &str {
        &self.subject
    }

    /// The words.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// What a program reads instead of the words; empty when the source
    /// gives none.
    pub fn detail(&self) -> &Map {
        &self.detail
    }
}

/// A source a Rust host links: the calls a source file's
/// `guatiao_source_info` table makes, as a trait.
///
/// The registry calls them one at a time, from whichever thread calls the
/// registry, and catches a panic in any of them.
pub trait Source: Send + Sync {
    /// Every library this source can open now. Runs no provider code.
    fn list(&self, host: Host) -> Result<Vec<Candidate>, ProviderError>;

    /// Opens the candidate listed as `id`. The library's id must be `id`.
    /// Owned parts are kept by the library's lease until it is closed; a
    /// borrowed descriptor is read here and not kept.
    fn open(&self, host: Host, id: &str) -> Result<LocalLibrary<'static>, ProviderError>;

    /// Called once for every library `open` answered, after the registry
    /// closed it: nothing it offered is held, and its parts are dropped.
    /// `stamp` is the one it was listed under when it was opened, which
    /// tells two builds of one id apart.
    fn close(&self, id: &str, stamp: &str) {
        let _ = (id, stamp);
    }

    /// What the source wants its host to see now, asked after every
    /// `list`, a failed one too. Answer the whole set each time: the
    /// registry reports a note the first time it appears, not again while
    /// it stays. A note is known by its subject, text and detail, so keep
    /// them stable (no counters or times): one that changes is told again.
    /// An error is reported as a refusal and leaves the notes told standing.
    /// None by default.
    fn notes(&self) -> Result<Vec<Note>, ProviderError> {
        Ok(Vec::new())
    }
}
