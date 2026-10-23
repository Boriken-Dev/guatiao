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

/// A source a Rust host links: the three calls a source file's
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
}
