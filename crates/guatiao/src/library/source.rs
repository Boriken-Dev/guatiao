// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Sources: libraries that come and go while the host runs.
//!
//! A **source** serves libraries that are not files a scan can probe — the
//! modules of an interpreter, the entries of a manifest — or a directory
//! the host wants kept in step with what is on disk. The host adds it once
//! and the registry keeps it for its life; what it serves are ordinary
//! libraries, opened, retired and closed like any other, each closed once
//! nothing holds it (no handle's lease on it is left) and its source
//! then told.
//!
//! **Nothing here runs on its own.** There is no thread and no file
//! watcher: the host calls [`Registry::refresh`] when it chooses — on a
//! timer, from its own watcher, from a menu — and gets back what changed.
//!
//! ```ignore
//! let mut registry = Registry::new("my-host", "1.0");
//! registry.add_source_file(Path::new("guatiao_source_py.dll"), &config)?;
//! registry.watch_dir(Path::new("plugins"), ScanRules::default())?;
//! let changed = registry.refresh();          // and again, whenever the host likes
//! ```

#![forbid(unsafe_code)]

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::sync::Arc;
use std::path::{Path, PathBuf};

use super::desc::SourceInfo;
use super::raw::{Host, Opened, Opening, Rejected, SourceCell, SourceFileError, open_source_file};
use super::registry::{Loading, Registry, Skipped};
use super::scan::{ScanRules, admit, candidate};
use crate::value::status::Status;
use crate::value::types::{Map, Value};

pub use super::serving::{Candidate, Source};

/// A library a refresh loaded or retired.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Served {
    /// The source's name: the name it was added under, a source file's
    /// path, or a watched directory.
    pub source: String,
    /// The candidate's id; for a watched directory, the file.
    pub id: String,
    /// The stamp it was listed under.
    pub stamp: String,
    /// The library key the registry files it under: what
    /// [`Registry::retire`] takes.
    pub key: String,
}

/// What a refresh could not do, and why. Never fatal: the rest went on.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Refused {
    /// The source's name.
    pub source: String,
    /// The candidate, or `None` when the source could not list at all.
    pub id: Option<String>,
    /// Why, in the source's or the registry's words.
    pub reason: String,
}

/// What one [`Registry::refresh`] changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Refreshed {
    /// Libraries loaded: new candidates, and new builds of changed ones.
    pub loaded: Vec<Served>,
    /// Libraries retired: vanished candidates, and the old builds of
    /// changed ones. Each closes once nothing holds it.
    pub retired: Vec<Served>,
    /// Candidates passed over, as a scan passes over a file: `(source, id,
    /// why)`.
    pub skipped: Vec<(String, String, Skipped)>,
    /// What could not be done.
    pub refused: Vec<Refused>,
}

impl Refreshed {
    /// Whether nothing changed and nothing was refused.
    pub fn is_empty(&self) -> bool {
        self.loaded.is_empty()
            && self.retired.is_empty()
            && self.skipped.is_empty()
            && self.refused.is_empty()
    }
}

/// Why a source could not be added.
#[derive(Debug)]
#[non_exhaustive]
pub enum SourceError {
    /// The loader refused to map the file.
    Open {
        /// The file.
        path: PathBuf,
        /// The loader's own message.
        reason: String,
    },
    /// The file mapped and exports no `guatiao_source`.
    NotASource {
        /// The file.
        path: PathBuf,
    },
    /// The source refused the configuration, in its own words.
    Refused {
        /// The source's name.
        name: String,
        /// The status it answered.
        status: Status,
        /// What it said.
        message: String,
    },
    /// Its table was refused where it entered: too small, a null slot, a
    /// slot outside its code, another envelope version.
    Malformed {
        /// The source's name.
        name: String,
        /// Which part, and what was wrong.
        reason: String,
    },
    /// A source or a watched directory by that name is already added.
    Duplicate {
        /// The name.
        name: String,
    },
}

impl core::fmt::Display for SourceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SourceError::Open { path, reason } => {
                write!(f, "{} could not be loaded: {reason}", path.display())
            }
            SourceError::NotASource { path } => {
                write!(
                    f,
                    "{} is not a source: it exports no `guatiao_source`",
                    path.display()
                )
            }
            SourceError::Refused { name, message, .. } => {
                write!(f, "the source {name} refused: {message}")
            }
            SourceError::Malformed { name, reason } => {
                write!(
                    f,
                    "the source {name} answered a table this build refuses: {reason}"
                )
            }
            SourceError::Duplicate { name } => write!(f, "a source named {name} is already added"),
        }
    }
}

impl core::error::Error for SourceError {}

impl SourceError {
    /// The status this error crosses a C boundary as.
    pub fn status(&self) -> Status {
        match self {
            SourceError::Open { .. } => Status::GUATIAO_ERR_NOT_FOUND,
            SourceError::NotASource { .. } => Status::GUATIAO_ERR_WRONG_KIND,
            SourceError::Refused { status, .. } => *status,
            SourceError::Malformed { .. } | SourceError::Duplicate { .. } => {
                Status::GUATIAO_ERR_BAD_VALUE
            }
        }
    }
}

fn rejected(name: &str, why: Rejected) -> SourceError {
    let reason = match why {
        Rejected::Malformed(why) => why.to_string(),
        Rejected::UnsupportedAbi { declared } => format!(
            "built for envelope ABI {declared}, and this host speaks {}",
            super::desc::ABI_VERSION
        ),
    };
    SourceError::Malformed {
        name: name.to_string(),
        reason,
    }
}

/// One source the registry holds, and what it opened.
#[derive(Debug)]
pub(crate) struct SourceState {
    cell: Arc<SourceCell>,
    /// The stamp each id is serving under now.
    serving: BTreeMap<String, String>,
    /// The stamp each id was last opened, or tried, under: not tried again
    /// until it changes.
    tried: BTreeMap<String, String>,
}

/// One directory the registry watches, and what it last saw there.
#[derive(Debug)]
pub(crate) struct WatchState {
    dir: PathBuf,
    rules: ScanRules,
    seen: BTreeMap<PathBuf, String>,
}

/// Where a library a source served is filed: unique per source and id.
fn served_path(source: &str, id: &str) -> PathBuf {
    PathBuf::from(format!("<{source}:{id}>"))
}

/// A file's length and modification time, as the stamp it is compared by.
fn file_stamp(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Some(format!("{}:{modified}", meta.len()))
}

impl Registry {
    fn has_source(&self, name: &str) -> bool {
        self.sources.iter().any(|s| s.cell.name == name)
            || self
                .watched
                .iter()
                .any(|w| w.dir.display().to_string() == name)
    }

    fn keep_source(&mut self, cell: Arc<SourceCell>) -> Result<(), SourceError> {
        if self.has_source(&cell.name) {
            return Err(SourceError::Duplicate {
                name: cell.name.clone(),
            });
        }
        self.sources.push(SourceState {
            cell,
            serving: BTreeMap::new(),
            tried: BTreeMap::new(),
        });
        Ok(())
    }

    /// Adds a source the host links, under `name`. Nothing is opened until
    /// [`refresh`](Registry::refresh).
    pub fn add_source(
        &mut self,
        name: &str,
        source: impl Source + 'static,
    ) -> Result<(), SourceError> {
        if self.has_source(name) {
            return Err(SourceError::Duplicate {
                name: name.to_string(),
            });
        }
        self.keep_source(SourceCell::rust(name, Box::new(source)))
    }

    /// Adds a source the host links that speaks C: `table` is read and
    /// checked here, and its slots and `ctx` must stay valid for this
    /// registry's life. Registering a table is trusting its producer, as
    /// registering an entry point is.
    pub fn add_source_table(&mut self, name: &str, table: &SourceInfo) -> Result<(), SourceError> {
        if self.has_source(name) {
            return Err(SourceError::Duplicate {
                name: name.to_string(),
            });
        }
        let cell = SourceCell::table(name, table).map_err(|why| rejected(name, why))?;
        self.keep_source(cell)
    }

    /// Maps a source file and adds the source its `guatiao_source` answers
    /// for `config`, under the file's path, which it answers.
    ///
    /// **The file is never unmapped**, whatever happens to the libraries it
    /// serves, and it is not a library: it cannot be retired. Mapping it
    /// runs its static initialisers, so name files you are willing to run.
    pub fn add_source_file(&mut self, path: &Path, config: &Map) -> Result<String, SourceError> {
        let name = path.display().to_string();
        if self.has_source(&name) {
            return Err(SourceError::Duplicate { name });
        }
        let host = self.host();
        let config = Value::from(config.clone());
        let cell = open_source_file(path, host, &config).map_err(|e| match e {
            SourceFileError::Open(reason) => SourceError::Open {
                path: path.to_path_buf(),
                reason,
            },
            SourceFileError::NotASource => SourceError::NotASource {
                path: path.to_path_buf(),
            },
            SourceFileError::Refused(status, message) => SourceError::Refused {
                name: name.clone(),
                status,
                message,
            },
            SourceFileError::Rejected(why) => rejected(&name, why),
        })?;
        self.keep_source(cell)?;
        Ok(name)
    }

    /// Watches a directory: each refresh loads a library file that
    /// appeared (by `rules`, as a scan would), retires one that vanished,
    /// and retires and loads again one whose length or modification time
    /// changed. Nothing is read until [`refresh`](Registry::refresh).
    ///
    /// A changed file is loaded from the same path, and the platform's
    /// loader may answer the image it already holds while the old build is
    /// still mapped: replace a library under a new name, or let the old
    /// one close first, to be sure of the new code.
    pub fn watch_dir(&mut self, dir: &Path, rules: ScanRules) -> Result<(), SourceError> {
        let name = dir.display().to_string();
        if self.has_source(&name) {
            return Err(SourceError::Duplicate { name });
        }
        self.watched.push(WatchState {
            dir: dir.to_path_buf(),
            rules,
            seen: BTreeMap::new(),
        });
        Ok(())
    }

    /// The names of the sources added, and of the directories watched.
    pub fn sources(&self) -> Vec<String> {
        self.sources
            .iter()
            .map(|s| s.cell.name.clone())
            .chain(self.watched.iter().map(|w| w.dir.display().to_string()))
            .collect()
    }

    /// Asks every source what it serves now, and makes the registry agree.
    ///
    /// Diffs by source, candidate and stamp: a new candidate is opened and
    /// loaded; a changed one is opened and, once it loads, its old build is
    /// retired (instances of the old keep working on their lease, new
    /// lookups find the new); a vanished one is retired; a library the host
    /// retired itself stays out until its stamp changes. Every refusal is
    /// reported, never fatal. Watched directories are refreshed the same
    /// way, file by file.
    pub fn refresh(&mut self) -> Refreshed {
        self.collect();
        let mut report = Refreshed::default();
        let host = self.host();
        for at in 0..self.sources.len() {
            self.refresh_source(at, host, &mut report);
        }
        for at in 0..self.watched.len() {
            self.refresh_dir(at, &mut report);
        }
        report
    }

    /// The library a source serves under `id`, retired, if it is still
    /// registered.
    fn retire_at(&mut self, path: &Path, source: &str, id: &str, stamp: &str) -> Option<Served> {
        let key = self
            .loaded()
            .iter()
            .find(|l| l.path == path)
            .map(|l| l.key.to_string())?;
        self.retire(&key).ok().map(|_| Served {
            source: source.to_string(),
            id: id.to_string(),
            stamp: stamp.to_string(),
            key,
        })
    }

    fn refresh_source(&mut self, at: usize, host: Host, report: &mut Refreshed) {
        let cell = Arc::clone(&self.sources[at].cell);
        let name = cell.name.clone();
        let refuse = |id: Option<&str>, reason: String| Refused {
            source: name.clone(),
            id: id.map(str::to_string),
            reason,
        };
        let (candidates, wrong) = match cell.list(host) {
            Ok(listed) => listed,
            Err(reason) => {
                report.refused.push(refuse(None, reason));
                return;
            }
        };
        report
            .refused
            .extend(wrong.into_iter().map(|reason| refuse(None, reason)));

        let listed: BTreeSet<&str> = candidates.iter().map(Candidate::id).collect();
        let gone: Vec<String> = self.sources[at]
            .tried
            .keys()
            .filter(|id| !listed.contains(id.as_str()))
            .cloned()
            .collect();
        for id in gone {
            self.sources[at].tried.remove(&id);
            if let Some(stamp) = self.sources[at].serving.remove(&id)
                && let Some(retired) = self.retire_at(&served_path(&name, &id), &name, &id, &stamp)
            {
                report.retired.push(retired);
            }
        }

        let mut seen = BTreeSet::new();
        for candidate in &candidates {
            let (id, stamp) = (candidate.id(), candidate.stamp());
            if !seen.insert(id) {
                report
                    .refused
                    .push(refuse(Some(id), "the source listed it twice".to_string()));
                continue;
            }
            if self.sources[at].tried.get(id).map(String::as_str) == Some(stamp) {
                continue;
            }
            self.sources[at]
                .tried
                .insert(id.to_string(), stamp.to_string());
            let opened = match cell.open(host, id, stamp) {
                Opening::Refused(reason) => {
                    report.refused.push(refuse(Some(id), reason));
                    continue;
                }
                Opening::Opened(opened) => opened,
            };
            if let Opened::Loaded(view, _) = &opened
                && view.id != id
            {
                let reason = format!(
                    "the source listed `{id}` and opened a library that calls itself `{}`",
                    view.id
                );
                if let Opened::Loaded(_, origin) = opened {
                    let _ = origin.close();
                }
                report.refused.push(refuse(Some(id), reason));
                continue;
            }
            let path = served_path(&name, id);
            // The old build leaves only once the new one has opened.
            if matches!(opened, Opened::Loaded(..))
                && let Some(old) = self.sources[at].serving.remove(id)
                && let Some(retired) = self.retire_at(&path, &name, id, &old)
            {
                report.retired.push(retired);
            }
            let outcome = match self.absorb(&path, &name, opened) {
                Ok(Loading::Loaded(one)) => Ok(Ok(one.key.to_string())),
                Ok(Loading::Skipped(why)) => Ok(Err(why)),
                Err(e) => Err(e.to_string()),
            };
            match outcome {
                Ok(Ok(key)) => {
                    self.sources[at]
                        .serving
                        .insert(id.to_string(), stamp.to_string());
                    report.loaded.push(Served {
                        source: name.clone(),
                        id: id.to_string(),
                        stamp: stamp.to_string(),
                        key,
                    });
                }
                Ok(Err(why)) => report.skipped.push((name.clone(), id.to_string(), why)),
                Err(reason) => report.refused.push(refuse(Some(id), reason)),
            }
        }
    }

    fn refresh_dir(&mut self, at: usize, report: &mut Refreshed) {
        let dir = self.watched[at].dir.clone();
        let source = dir.display().to_string();
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) => {
                report.refused.push(Refused {
                    source,
                    id: None,
                    reason: e.to_string(),
                });
                return;
            }
        };
        let now: BTreeMap<PathBuf, String> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter_map(candidate)
            .filter_map(|p| file_stamp(&p).map(|s| (p, s)))
            .collect();

        let gone: Vec<(PathBuf, String)> = self.watched[at]
            .seen
            .iter()
            .filter(|(path, _)| !now.contains_key(*path))
            .map(|(p, s)| (p.clone(), s.clone()))
            .collect();
        for (path, stamp) in gone {
            self.watched[at].seen.remove(&path);
            let id = path.display().to_string();
            if let Some(retired) = self.retire_at(&path, &source, &id, &stamp) {
                report.retired.push(retired);
            }
        }

        for (path, stamp) in now {
            let previous = self.watched[at].seen.insert(path.clone(), stamp.clone());
            if previous.as_deref() == Some(stamp.as_str()) {
                continue;
            }
            let id = path.display().to_string();
            if let Some(old) = previous
                && let Some(retired) = self.retire_at(&path, &source, &id, &old)
            {
                report.retired.push(retired);
            }
            let rules = self.watched[at].rules.clone();
            if let Err(why) = admit(&path, &mut |declared| {
                rules.check(declared).map_err(str::to_string)
            }) {
                report.skipped.push((source.clone(), id, why));
                continue;
            }
            let outcome = match self.load_from(&path, &source) {
                Ok(Loading::Loaded(one)) => Ok(Ok(one.key.to_string())),
                Ok(Loading::Skipped(why)) => Ok(Err(why)),
                Err(e) => Err(e.to_string()),
            };
            match outcome {
                Ok(Ok(key)) => report.loaded.push(Served {
                    source: source.clone(),
                    id,
                    stamp,
                    key,
                }),
                Ok(Err(why)) => report.skipped.push((source.clone(), id, why)),
                Err(reason) => report.refused.push(Refused {
                    source: source.clone(),
                    id: Some(id),
                    reason,
                }),
            }
        }
    }
}

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
