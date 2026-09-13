// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What the host holds: the libraries it has loaded and the providers
//! they offer.
//!
//! # The registry is a VALUE the host owns
//!
//! Not a process-global table, and not something a library writes into.
//! A library is asked what it offers and answers; the host records the
//! answer. Two artifacts in one process that each link this crate hold
//! two registries and neither can be surprised by the other, which is the
//! whole reason this crate holds no static state.
//!
//! It also removes the failure a registration-callback design has: a
//! library that registers into a copy of a table the host never reads,
//! where the symptom is a lookup answering "not found" for something that
//! was definitely registered.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::desc::{ABI_VERSION, HostInfo};
use super::raw::{LibraryView, ProviderView};
use crate::value::alloc::Alloc;
use crate::value::types::MaybeNull;
use crate::value::types::Str;

/// Why a file could not be loaded.
#[derive(Debug)]
#[non_exhaustive]
pub enum LoadError {
    /// The operating system refused to map it, or it is not a library at
    /// all. Carries what the loader said.
    Open {
        /// The file.
        path: PathBuf,
        /// The loader's own message, which names the real cause far
        /// better than any wording here could.
        reason: String,
    },
    /// It mapped and exported the entry symbol, but what came back was
    /// not a descriptor this build can read: too small, or holding text
    /// that is not UTF-8.
    Malformed {
        /// The file.
        path: PathBuf,
    },
    /// Two libraries offer the same `(kind, id)`. Names both, because
    /// "duplicate provider" without the second path is a message that
    /// sends the reader looking through every library they have.
    Duplicate {
        /// What they both claim.
        kind: String,
        /// What they both claim.
        id: String,
        /// The one already loaded.
        first: PathBuf,
        /// The one that arrived second and was refused.
        second: PathBuf,
    },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Open { path, reason } => {
                write!(f, "{} could not be loaded: {reason}", path.display())
            }
            LoadError::Malformed { path } => write!(
                f,
                "{} answered the entry symbol with a descriptor this build cannot read",
                path.display()
            ),
            LoadError::Duplicate {
                kind,
                id,
                first,
                second,
            } => write!(
                f,
                "both {} and {} offer {kind}/{id}",
                first.display(),
                second.display()
            ),
        }
    }
}

impl std::error::Error for LoadError {}

/// Why a file was passed over rather than loaded or failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Skipped {
    /// It mapped, but exported no entry symbol. Not a library, and not an
    /// error: a directory of libraries may hold anything.
    NoEntrySymbol,
    /// Its entry point answered null, which is a library saying it has
    /// nothing for this host.
    DeclinedThisHost,
}

/// One provider a loaded library offers.
#[derive(Debug)]
pub struct Provider {
    view: ProviderView,
    from: PathBuf,
}

impl Provider {
    /// What sort of thing it is.
    pub fn kind(&self) -> &str {
        &self.view.kind
    }

    /// Its identifier within that kind.
    pub fn id(&self) -> &str {
        &self.view.id
    }

    /// A name to show a person. Empty when the library offered none.
    pub fn display_name(&self) -> &str {
        &self.view.display_name
    }

    /// The library it came from.
    pub fn from(&self) -> &Path {
        &self.from
    }

    /// The schema for its configuration, as an ordinary value, or `None`
    /// when it takes none.
    ///
    /// Read it with [`crate::schema::read::SchemaRef`] and check a
    /// configuration against it with [`crate::schema::validate_map`] —
    /// the provider declared it; nothing here interprets it.
    pub fn config_schema(&self) -> Option<&'static crate::value::types::Value> {
        self.view.config
    }

    /// Whatever else this provider declared, or `None`.
    ///
    /// A key this host does not recognise is skipped, the same rule the
    /// value model has for a tag it does not know.
    pub fn meta(&self) -> Option<&'static crate::value::types::Map> {
        self.view.meta
    }

    /// The function table and the size the library compiled it at.
    ///
    /// **Deliberately raw, and deliberately not generic.** Whoever
    /// defines a `kind` defines what its table looks like, so only they
    /// can say where its floor is — the size at which an older library is
    /// still usable. A helper here could only compare against
    /// `size_of::<T>()`, which refuses every library built before the
    /// host's newest slot existed, and it would have to build a `&T`,
    /// which asserts the whole struct is readable when a shorter one is
    /// exactly what a `struct_size` guard is for.
    ///
    /// Read it the way [`crate::value::Alloc::from_raw`] reads an
    /// allocator: check the declared size against your own frozen floor,
    /// project each field through the raw pointer, and guard every field
    /// you appended later.
    pub fn vtable(&self) -> (*const std::ffi::c_void, usize) {
        (self.view.vtable, self.view.vtable_size)
    }

    /// The whole descriptor this provider was read from.
    ///
    /// [`ProviderView::vtable_as`] is on it, because casting a table to a
    /// type is the one thing this module cannot do: it carries
    /// `#![forbid(unsafe_code)]`, which is worth more than the
    /// convenience of a shorter call.
    pub fn view(&self) -> &ProviderView {
        &self.view
    }

    /// The context pointer to hand back to every call through the table.
    pub fn ctx(&self) -> *mut std::ffi::c_void {
        self.view.ctx
    }
}

/// One library that was loaded, and what it offered.
#[derive(Debug)]
pub struct Loaded {
    /// The file it came from.
    pub path: PathBuf,
    /// The library's own identifier.
    pub id: String,
    /// Its version string, uninterpreted.
    pub version: String,
    /// How many providers it registered.
    pub providers: usize,
    /// Whatever else the library declared, or `None`. Borrowed from its
    /// image, which is never unloaded.
    pub meta: Option<&'static crate::value::types::Map>,
}

/// The host's own table of what it has loaded.
#[derive(Debug)]
pub struct Registry {
    host: HostInfo,
    loaded: Vec<Loaded>,
    providers: Vec<Provider>,
    by_key: BTreeMap<(String, String), usize>,
}

impl Registry {
    /// A registry that introduces its host as `id`/`version`, offering
    /// libraries no allocator of its own.
    pub fn new(id: &'static str, version: &'static str) -> Registry {
        Registry::with_alloc(id, version, None)
    }

    /// The same, offering libraries the host's allocator so a tree they
    /// build for it is built in the host's arena.
    pub fn with_alloc(id: &'static str, version: &'static str, alloc: Option<Alloc>) -> Registry {
        Registry {
            host: HostInfo {
                struct_size: size_of::<HostInfo>() as u32,
                abi_version: ABI_VERSION,
                host_id: Str::borrowed(id),
                host_version: Str::borrowed(version),
                alloc: alloc.map_or(std::ptr::null(), |a| a.as_raw()),
                meta: MaybeNull::null(),
            },
            loaded: Vec::new(),
            providers: Vec::new(),
            by_key: BTreeMap::new(),
        }
    }

    /// Loads one file the caller named.
    ///
    /// `Ok(None)` is "that is not a library for us" — no entry symbol, or
    /// the library declined this host — which is an answer rather than a
    /// failure and is why a caller can hand this every library in a
    /// directory without pre-filtering by name.
    ///
    /// **Mapping a library runs its static initialisers**, which may do
    /// anything, including abort the process. Name files you are willing
    /// to run.
    pub fn load_file(&mut self, path: &Path) -> Result<Option<&Loaded>, LoadError> {
        let opened = super::raw::open_library(path, &self.host).map_err(|e| LoadError::Open {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?;
        let Some(lib) = opened else {
            return Ok(None);
        };
        let LibraryView {
            id,
            version,
            meta,
            providers: views,
        } = lib;

        // Refuse the whole library before recording any of it, so a
        // duplicate leaves the registry exactly as it was rather than
        // half-populated.
        for view in &views {
            let key = (view.kind.clone(), view.id.clone());
            if let Some(&first) = self.by_key.get(&key) {
                return Err(LoadError::Duplicate {
                    kind: view.kind.clone(),
                    id: view.id.clone(),
                    first: self.providers[first].from.clone(),
                    second: path.to_path_buf(),
                });
            }
        }

        let count = views.len();
        for view in views {
            let key = (view.kind.clone(), view.id.clone());
            self.by_key.insert(key, self.providers.len());
            self.providers.push(Provider {
                view,
                from: path.to_path_buf(),
            });
        }
        self.loaded.push(Loaded {
            path: path.to_path_buf(),
            id,
            version,
            providers: count,
            meta,
        });
        Ok(self.loaded.last())
    }

    /// Every provider of one kind, in the order they were loaded.
    pub fn providers(&self, kind: &str) -> impl Iterator<Item = &Provider> {
        self.providers.iter().filter(move |p| p.kind() == kind)
    }

    /// One provider by kind and id.
    pub fn provider(&self, kind: &str, id: &str) -> Option<&Provider> {
        self.by_key
            .get(&(kind.to_string(), id.to_string()))
            .map(|&i| &self.providers[i])
    }

    /// Every library loaded so far.
    pub fn loaded(&self) -> &[Loaded] {
        &self.loaded
    }

    /// Every provider, of every kind.
    pub fn all(&self) -> &[Provider] {
        &self.providers
    }
}
