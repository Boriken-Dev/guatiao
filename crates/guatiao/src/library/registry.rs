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
//!
//! # What a provider is filed under is the host's choice
//!
//! A [`KeyTemplate`] renders one, `%id` by default. A host that wants two
//! builds of one provider at once passes `%id@%version` to
//! [`Registry::keyed_by`] and gets two entries where the default would
//! have refused the second as a duplicate.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::desc::{ABI_VERSION, HostInfo};
use super::key::{KeyError, KeyFields, KeyTemplate};
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
    /// Two **different** providers land on one key. Names both paths,
    /// because "duplicate provider" without the second one is a message
    /// that sends the reader looking through every library they have.
    ///
    /// Not the same provider twice — that is
    /// [`Skipped::ProviderAlreadyLoaded`] and is ordinary. This is two
    /// distinct implementations claiming one name, which only a host's own
    /// [`KeyTemplate`] can produce: `%id` gives every provider a distinct
    /// key by construction, and a template naming less than that does not.
    Duplicate {
        /// What they both render.
        key: String,
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
            LoadError::Duplicate { key, first, second } => write!(
                f,
                "both {} and {} offer `{key}`",
                first.display(),
                second.display()
            ),
        }
    }
}

impl std::error::Error for LoadError {}

/// Why a file was passed over rather than loaded or failed.
///
/// **Every one of these is an answer, not a failure.** A scan of a real
/// directory produces them constantly and a host that treated them as
/// errors would report a healthy installation as broken.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Skipped {
    /// It mapped, but exported no entry symbol. Not a library, and not an
    /// error: a directory of libraries may hold anything.
    NoEntrySymbol,
    /// Its entry point answered null, which is a library saying it has
    /// nothing for this host.
    DeclinedThisHost,
    /// This library is already loaded, and names where from.
    ///
    /// **The routine case, not a problem.** A host's search path and the
    /// directory beside its executable are commonly the same directory, so
    /// a plugin is offered two or three times on an ordinary run; a plugin
    /// staged into place by copy is offered under two paths. Reporting
    /// either as an error prints a page of noise over a healthy install,
    /// and refusing the second registration silently disables whatever the
    /// first one had not got to yet.
    ///
    /// Recognised by canonical **path** first, which costs no load, and by
    /// library **id** after, which catches the same library at two paths.
    AlreadyLoaded {
        /// Where it was loaded from the first time.
        from: PathBuf,
    },
    /// One provider inside a library that loaded: a provider with this id
    /// is already registered, so this one was passed over and **the rest
    /// of the library went on loading**.
    ///
    /// Two libraries offering one provider is ordinary — a re-export, a
    /// vendored copy — and they agree on its id, which is exactly what
    /// makes this detectable. The first one loaded wins, because it is the
    /// one already handed out.
    ProviderAlreadyLoaded {
        /// The provider's id.
        id: String,
        /// The library that had already registered it.
        from: PathBuf,
    },
    /// Its bytes could not be read, or could not be parsed as an object
    /// file at all.
    ///
    /// Neither a library nor a broken one — a text file with a library's
    /// extension is simply not a library. Reported rather than dropped,
    /// because somebody looking for a plugin that did not appear needs to
    /// see that the file was considered.
    NotExaminable,
}

/// What one [`Registry::load_file`] did.
///
/// Two outcomes rather than `Option`, because "nothing happened" has
/// several readings a caller acts on differently: a file that is not a
/// library at all, a library declining this host, and one this registry
/// already has.
#[derive(Debug)]
pub enum Loading<'a> {
    /// It loaded, and here is what it offered.
    Loaded(&'a Loaded),
    /// It did not, and this is why. Not a failure.
    Skipped(Skipped),
}

impl<'a> Loading<'a> {
    /// What it loaded, or `None` when it was skipped.
    pub fn loaded(self) -> Option<&'a Loaded> {
        match self {
            Loading::Loaded(one) => Some(one),
            Loading::Skipped(_) => None,
        }
    }

    /// Why it was passed over, or `None` when it loaded.
    pub fn skipped(&self) -> Option<&Skipped> {
        match self {
            Loading::Loaded(_) => None,
            Loading::Skipped(why) => Some(why),
        }
    }
}

/// One provider a loaded library offers.
#[derive(Debug)]
pub struct Provider {
    view: ProviderView,
    from: PathBuf,
    library: String,
    version: String,
    key: String,
}

impl Provider {
    /// Every kind it speaks. May be empty, which is a provider reached by
    /// name rather than by capability.
    pub fn kinds(&self) -> &[String] {
        &self.view.kinds
    }

    /// Whether it speaks this kind.
    ///
    /// The capability question, asked of one provider.
    /// [`Registry::providers`] is the same question asked of all of them.
    pub fn supports(&self, kind: &str) -> bool {
        self.view.supports(kind)
    }

    /// Its own identifier, unique across every provider loaded.
    pub fn id(&self) -> &str {
        &self.view.id
    }

    /// The id of the library offering it.
    pub fn library(&self) -> &str {
        &self.library
    }

    /// Its version: the one it declared, or its library's when it declared
    /// none. Declared semver, compared here as a string.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// What this registry filed it under, rendered from the host's
    /// [`KeyTemplate`]. The argument [`Registry::provider`] takes.
    pub fn key(&self) -> &str {
        &self.key
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

    /// Everything a [`KeyTemplate`] can name.
    fn fields(&self) -> KeyFields<'_> {
        KeyFields {
            id: &self.view.id,
            name: &self.view.display_name,
            library: &self.library,
            version: &self.version,
        }
    }
}

/// One library that was loaded, and what it offered.
#[derive(Debug)]
pub struct Loaded {
    /// The file it came from.
    pub path: PathBuf,
    /// What this registry filed it under, from the host's library
    /// template. Two libraries rendering one key are the same library as
    /// far as this host is concerned, and the second is skipped.
    pub key: String,
    /// The library's own identifier.
    pub id: String,
    /// Its version string, uninterpreted.
    pub version: String,
    /// How many providers it registered — which is not how many it
    /// offered, when one of them was already loaded from elsewhere.
    pub providers: usize,
    /// Providers it offered that were already registered. Empty on an
    /// ordinary load; non-empty when this library re-exports something a
    /// host already had.
    pub skipped: Vec<Skipped>,
    /// Whatever else the library declared, or `None`. Borrowed from its
    /// image, which is never unloaded.
    pub meta: Option<&'static crate::value::types::Map>,
}

/// The host's own table of what it has loaded.
#[derive(Debug)]
pub struct Registry {
    host: HostInfo,
    key: KeyTemplate,
    library_key: KeyTemplate,
    loaded: Vec<Loaded>,
    providers: Vec<Provider>,
    /// A rendered key to an index into `providers`.
    by_key: BTreeMap<String, usize>,
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
            key: KeyTemplate::default(),
            library_key: KeyTemplate::library("%id").expect("one field and nothing else"),
            loaded: Vec::new(),
            providers: Vec::new(),
            by_key: BTreeMap::new(),
        }
    }

    /// Names LIBRARIES with `template` rather than the default `%id`.
    ///
    /// This is the "how many versions of one library may I have" knob.
    /// `%id` means one, and a second is skipped as
    /// [`Skipped::AlreadyLoaded`]; `"%id@%version"` means as many as are
    /// on disk, because two builds then render two keys.
    ///
    /// With `%id` and a directory scan, **which** build survives is the
    /// scan's order: [`scan_dir`](super::scan::scan_dir) visits names in
    /// order and the first to claim a key keeps it, so a host wanting the
    /// newest of `libfoo-1.2.0` and `libfoo-1.10.0` scans descending. The
    /// loader sorts names and does not parse versions — ordering a version
    /// is a host's policy, with the semver library it already has.
    pub fn libraries_keyed_by(mut self, template: &str) -> Result<Registry, KeyError> {
        let key = KeyTemplate::library(template)?;
        let mut seen: BTreeMap<String, usize> = BTreeMap::new();
        for (at, one) in self.loaded.iter().enumerate() {
            let rendered = key.render(KeyFields::library(&one.id, &one.version));
            if seen.insert(rendered.clone(), at).is_some() {
                return Err(KeyError::Collides { key: rendered });
            }
        }
        for one in &mut self.loaded {
            one.key = key.render(KeyFields::library(&one.id, &one.version));
        }
        self.library_key = key;
        Ok(self)
    }

    /// The template this registry files libraries under.
    pub fn library_key_template(&self) -> &KeyTemplate {
        &self.library_key
    }

    /// Names PROVIDERS with `template` rather than the default `%id`.
    ///
    /// `"%id@%version"` is the one to reach for: it is what lets two
    /// builds of one provider sit in one registry. The whole field list is
    /// on [`KeyTemplate`].
    ///
    /// Callable at any point — anything already loaded is re-keyed, and a
    /// template that would give two of them the same key is refused with
    /// [`KeyError::Collides`] rather than losing one.
    pub fn keyed_by(mut self, template: &str) -> Result<Registry, KeyError> {
        let key = KeyTemplate::provider(template)?;
        let rendered: Vec<String> = self
            .providers
            .iter()
            .map(|p| key.render(p.fields()))
            .collect();

        let mut by_key = BTreeMap::new();
        for (at, one) in rendered.iter().enumerate() {
            if by_key.insert(one.clone(), at).is_some() {
                return Err(KeyError::Collides { key: one.clone() });
            }
        }

        for (provider, one) in self.providers.iter_mut().zip(rendered) {
            provider.key = one;
        }
        self.by_key = by_key;
        self.key = key;
        Ok(self)
    }

    /// The template this registry files providers under.
    pub fn key_template(&self) -> &KeyTemplate {
        &self.key
    }

    /// Where this library is already loaded from, by canonical path.
    ///
    /// Asked **before** opening anything, because it is the one dedup that
    /// costs no `dlopen` — and mapping a library is irreversible.
    fn loaded_from_path(&self, path: &Path) -> Option<&Loaded> {
        let want = path.canonicalize().ok()?;
        self.loaded
            .iter()
            .find(|l| l.path.canonicalize().ok().as_ref() == Some(&want))
    }

    /// Loads one file the caller named.
    ///
    /// [`Loading::Skipped`] is an **answer**, not a failure: the file is
    /// not a library, the library declined this host, or it is one this
    /// registry already has. That is why a caller can hand this every file
    /// in a directory without pre-filtering by name, and why a repeat is
    /// cheap rather than loud — a host's search path and the directory
    /// beside its executable are routinely the same place.
    ///
    /// **Mapping a library runs its static initialisers**, which may do
    /// anything, including abort the process. Name files you are willing
    /// to run.
    pub fn load_file(&mut self, path: &Path) -> Result<Loading<'_>, LoadError> {
        // By path first: the only dedup that can happen without loading.
        if let Some(already) = self.loaded_from_path(path) {
            let from = already.path.clone();
            return Ok(Loading::Skipped(Skipped::AlreadyLoaded { from }));
        }

        let opened = super::raw::open_library(path, &self.host).map_err(|e| LoadError::Open {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?;
        let Some(lib) = opened else {
            return Ok(Loading::Skipped(Skipped::NoEntrySymbol));
        };
        let LibraryView {
            id,
            version,
            meta,
            providers: views,
        } = lib;

        // By KEY after: the same library staged at two paths is the same
        // library, which is the case a path comparison cannot see and a
        // filename comparison only guesses at. Whether two versions of one
        // library are the same thing is the host's template's answer.
        let library_key = self.library_key.render(KeyFields::library(&id, &version));
        if let Some(already) = self.loaded.iter().find(|l| l.key == library_key) {
            let from = already.path.clone();
            return Ok(Loading::Skipped(Skipped::AlreadyLoaded { from }));
        }

        // A provider already registered is passed over and the rest of the
        // library goes on. Two libraries offering one provider is ordinary
        // — a re-export agrees on the id, which is what makes it visible —
        // and refusing the whole library over it would lose every provider
        // that is NOT a repeat.
        let mut skipped = Vec::new();
        let mut taken: Vec<(ProviderView, String, String)> = Vec::new();
        for view in views {
            if let Some(first) = self.providers.iter().find(|p| p.id() == view.id) {
                skipped.push(Skipped::ProviderAlreadyLoaded {
                    id: view.id.clone(),
                    from: first.from.clone(),
                });
                continue;
            }
            if let Some(first) = taken.iter().find(|(v, _, _)| v.id == view.id) {
                // Twice within ONE library, which is a library bug rather
                // than a re-export. Still a skip: the first one stands.
                skipped.push(Skipped::ProviderAlreadyLoaded {
                    id: first.0.id.clone(),
                    from: path.to_path_buf(),
                });
                continue;
            }

            // A provider's own version, or its library's when it declared
            // none.
            let at = view.version.clone().unwrap_or_else(|| version.clone());
            let key = self.key.render(KeyFields {
                id: &view.id,
                name: &view.display_name,
                library: &id,
                version: &at,
            });
            taken.push((view, at, key));
        }

        // Two DIFFERENT providers on one key is the host's template
        // failing, and it is refused before anything is recorded so the
        // registry is left exactly as it was.
        for (i, (_, _, key)) in taken.iter().enumerate() {
            let first = self
                .by_key
                .get(key)
                .map(|&at| self.providers[at].from.clone())
                .or_else(|| {
                    taken[..i]
                        .iter()
                        .any(|(_, _, earlier)| earlier == key)
                        .then(|| path.to_path_buf())
                });
            if let Some(first) = first {
                return Err(LoadError::Duplicate {
                    key: key.clone(),
                    first,
                    second: path.to_path_buf(),
                });
            }
        }

        let count = taken.len();
        for (view, at, key) in taken {
            self.by_key.insert(key.clone(), self.providers.len());
            self.providers.push(Provider {
                view,
                from: path.to_path_buf(),
                library: id.clone(),
                version: at,
                key,
            });
        }
        self.loaded.push(Loaded {
            path: path.to_path_buf(),
            key: library_key,
            id,
            version,
            providers: count,
            skipped,
            meta,
        });
        Ok(Loading::Loaded(self.loaded.last().expect("just pushed")))
    }

    /// Every provider that speaks one kind, in the order they were loaded.
    ///
    /// The capability question — "what can serve this?" — as opposed to
    /// [`provider`](Registry::provider), which is the identity one. A
    /// provider serving several kinds answers to each of them, which is
    /// why it is one provider rather than one per kind.
    pub fn providers(&self, kind: &str) -> impl Iterator<Item = &Provider> {
        self.providers.iter().filter(move |p| p.supports(kind))
    }

    /// One provider by the key this registry filed it under.
    ///
    /// Under the default template that is its id; under `%id@%version` it
    /// is `"hello@1.2.0"`. [`Provider::key`] is the other side of this,
    /// for a host that has a provider and wants the name it answers to.
    pub fn provider(&self, key: &str) -> Option<&Provider> {
        self.by_key.get(key).map(|&at| &self.providers[at])
    }

    /// Every provider with one id, whatever the key template made of it —
    /// which is every loaded version of it, in the order they loaded.
    ///
    /// Which of them is "newest" is the host's to decide, with the semver
    /// library it already has.
    pub fn providers_of(&self, id: &str) -> impl Iterator<Item = &Provider> {
        self.providers.iter().filter(move |p| p.id() == id)
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
