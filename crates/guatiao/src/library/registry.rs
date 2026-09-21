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
use std::sync::Arc;

use super::key::{KeyError, KeyFields, KeyTemplate};
use super::kind::{Kind, KindMismatch, Offer, Remote};
use super::raw::{
    EntryFn, Host, HostBlock, LibraryView, Opened, Origin, ProviderView, Rejected, Shared,
    Snapshot, SnapshotEntry,
};
use crate::value::alloc::Alloc;
use crate::value::types::Text;

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
    /// It is a library, and it speaks an envelope version this host does
    /// not. Names the version it declared, so the remedy — rebuild one
    /// side — is visible.
    UnsupportedAbi {
        /// The `abi_version` the library declared.
        declared: u32,
    },
    /// It is a library, and a scan rule kept it out **before it was
    /// mapped**, by what it declared. Names the rule, so a person looking
    /// for it sees which one.
    Filtered {
        /// The rule, as written.
        by: String,
    },
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

/// Why nothing can serve a kind.
///
/// Two answers rather than one, because a person acts on them
/// differently: install something, versus fix what you have.
#[derive(Debug)]
pub enum WhyNot<'a> {
    /// No loaded provider claims this kind at all.
    NothingClaimsIt,
    /// Providers claim it, and each said why it cannot run here.
    NoneAvailable(Vec<(&'a Provider, &'a str)>),
}

/// One provider a loaded library offers.
#[derive(Debug)]
pub struct Provider {
    view: ProviderView,
    from: PathBuf,
    /// Copied out of the offering library's descriptor, so retiring that
    /// library leaves nothing here pointing into its image.
    library: String,
    version: String,
    /// COMPUTED here rather than borrowed, so it is owned. A [`Text`] and
    /// not a `String`, because it is a string this crate hands to C: it is
    /// already a `guatiao_string` and needs no conversion at the boundary.
    key: Text,
    /// What this host ranked it, resolved when it loaded and again
    /// whenever the ranking changes. Absent means 0.
    priority: i32,
}

// `Send` and `Sync` for this type are declared in `raw.rs`; see there.

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

    /// What this host ranked it. Zero unless somebody said otherwise.
    ///
    /// **Host policy, not a property of the provider** — which is why it
    /// is here and not in the descriptor. A library does not know how a
    /// person ranks it against the others they installed, and two machines
    /// with the same libraries can rank them differently.
    pub fn priority(&self) -> i32 {
        self.priority
    }

    /// Whether it can actually run here, and why not when it cannot.
    ///
    /// **Asked every time, never cached** — see
    /// [`ProviderInfo::available`](super::desc::ProviderInfo::available).
    /// A provider that declares no slot is available.
    ///
    /// What "available" means is between this host and that library. All
    /// this does is carry the answer, and the reason when there is one.
    pub fn available(&self) -> Result<(), &'static str> {
        self.view.available()
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
    pub fn config_schema(&self) -> Option<&crate::value::types::Value> {
        self.view.config.as_ref()
    }

    /// Whatever else this provider declared, or `None`.
    ///
    /// A key this host does not recognise is skipped, the same rule the
    /// value model has for a tag it does not know.
    pub fn meta(&self) -> Option<&crate::value::types::Map> {
        self.view.meta.as_ref()
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

    /// The function table this provider speaks `kind` through, and the
    /// size it was compiled at: a per-kind table first, then
    /// [`vtable`](Provider::vtable) when it claims the kind. `None` for a
    /// kind it does not serve, or serves as a pure label.
    pub fn table_for(&self, kind: &str) -> Option<(*const std::ffi::c_void, usize)> {
        self.view.table_for(kind)
    }

    /// This provider's per-kind table for `K`, validated. The typed path:
    /// only a table in the descriptor's `tables` qualifies.
    pub fn as_kind<K: ?Sized + Kind>(&self) -> Result<Remote<K>, KindMismatch> {
        Remote::from_view(&self.view)
    }

    /// An instance of this provider built from `config`, as `K`. See
    /// [`Offer::instantiate`].
    pub fn instantiate<K: ?Sized + Kind>(
        &self,
        config: &crate::value::types::Value,
    ) -> Result<super::kind::Instance<K>, super::kind::ProviderError> {
        super::kind::Instance::build(&self.view, config)
    }

    /// The same, as an offer carrying what a chooser needs to show.
    pub fn offer<K: ?Sized + Kind>(&self) -> Result<Offer<K>, KindMismatch> {
        let remote = self.as_kind::<K>()?;
        Ok(Offer::new(
            remote,
            self.view.clone(),
            Some(self.key().to_string()),
            Some(self.library.clone()),
            self.version.clone(),
            self.priority,
        ))
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
    pub key: Text,
    /// The library's own identifier. Copied out of its descriptor.
    pub id: String,
    /// Its version string, uninterpreted. Copied out of its descriptor.
    pub version: String,
    /// How many providers it registered — which is not how many it
    /// offered, when one of them was already loaded from elsewhere.
    pub providers: usize,
    /// Providers it offered that were already registered. Empty on an
    /// ordinary load; non-empty when this library re-exports something a
    /// host already had.
    pub skipped: Vec<Skipped>,
    /// Whatever else the library declared, or `None`. A copy in this
    /// process's own heap.
    pub meta: Option<crate::value::types::Map>,
    /// `path` resolved once at load time, for the dedup that runs before
    /// anything is opened. Falls back to `path` when it cannot be resolved.
    canonical: PathBuf,
    /// The mapping, when this registry made one. Dropping it is what
    /// unmaps the library, which only `unload` does.
    origin: Origin,
    /// Its say in being unmapped, or `None` for a library that declares
    /// none. See [`LibraryInfo::unload`](super::desc::LibraryInfo::unload).
    unload: Option<unsafe extern "C" fn() -> crate::value::status::Status>,
}

impl Loaded {
    /// Whether there is a mapping to close at all, or this is code the
    /// host links.
    pub(crate) fn is_mapped(&self) -> bool {
        self.origin.is_mapped()
    }

    /// Its say in being unmapped, for `unload` to ask.
    pub(crate) fn unload_slot(
        &self,
    ) -> Option<unsafe extern "C" fn() -> crate::value::status::Status> {
        self.unload
    }
}

/// What one library taking its leave came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retired {
    /// The library key it answered to, which may now be loaded again.
    pub key: String,
    /// Its own identifier.
    pub id: String,
    /// Its version string, uninterpreted.
    pub version: String,
    /// How many providers left the registry with it.
    pub providers: usize,
}

/// Why a library could not be retired or unloaded.
#[derive(Debug)]
#[non_exhaustive]
pub enum UnloadError {
    /// No library answers to that key.
    NotFound {
        /// The key that was asked for.
        key: String,
    },
    /// It is code in the host's own binary — `register_local`,
    /// `register_entry` — so there is no mapping to unmap. Retiring it
    /// works; unloading it cannot.
    Linked {
        /// The library key.
        key: String,
    },
    /// The library has no `unload` slot, so it promises nothing about
    /// what it handed out. It is left as it was;
    /// [`unload_unchecked`](Registry::unload_unchecked) is the host
    /// insisting.
    NotSupported {
        /// The library key.
        key: String,
    },
    /// The library's own `unload` slot refused, and the library is left
    /// exactly as it was: registered and mapped.
    Refused {
        /// The library key.
        key: String,
        /// What it answered.
        status: crate::value::status::Status,
    },
    /// The loader could not close the mapping. The library is retired
    /// either way, and the handle is given up.
    Close {
        /// The library key.
        key: String,
        /// The loader's own message.
        reason: String,
    },
}

impl std::fmt::Display for UnloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnloadError::NotFound { key } => write!(f, "no library answers to `{key}`"),
            UnloadError::Linked { key } => write!(
                f,
                "`{key}` is linked into this binary and cannot be unmapped"
            ),
            UnloadError::NotSupported { key } => write!(
                f,
                "`{key}` has no unload slot, so it does not support being unloaded"
            ),
            UnloadError::Refused { key, status } => {
                write!(f, "`{key}` refused to be unloaded ({status:?})")
            }
            UnloadError::Close { key, reason } => write!(
                f,
                "`{key}` was retired and its mapping could not be closed: {reason}"
            ),
        }
    }
}

impl std::error::Error for UnloadError {}

/// The host's own table of what it has loaded.
///
/// # What a library sees of it
///
/// A library is handed a [`Host`]: a pointer to a block this registry
/// leaks on first use and never frees, carrying the host's name, its
/// allocator and a services table. The table reads a **snapshot** this
/// registry publishes after every change — loading, re-keying, ranking —
/// so a lookup from a library never borrows the registry itself, and no
/// lock is held while a library's entry point runs. A lookup made from an
/// entry point sees every library registered before it, not the one being
/// loaded. Once this registry is dropped the block answers
/// `GUATIAO_ERR_GONE`.
#[derive(Debug)]
pub struct Registry {
    /// The host's own name and version, OWNED. The block a library keeps
    /// carries its own copies.
    host_id: Box<str>,
    host_version: Box<str>,
    alloc: Option<Alloc>,
    key: KeyTemplate,
    library_key: KeyTemplate,
    loaded: Vec<Loaded>,
    providers: Vec<Provider>,
    /// A rendered key to an index into `providers`.
    by_key: BTreeMap<String, usize>,
    /// Provider id to the rank this host gave it.
    ///
    /// Kept beside the registry rather than on the descriptor because it
    /// is the HOST's opinion. Absent means 0, so an unranked provider
    /// sorts below any raised one and alongside every other unranked one.
    priorities: BTreeMap<String, i32>,
    /// What the services read. This is the strong handle; the block holds
    /// a weak one, so dropping the registry is what makes it `GONE`.
    shared: Arc<Shared>,
    /// The block libraries keep, leaked on first use.
    block: Option<&'static HostBlock>,
}

// `Send` and `Sync` for this type are declared in `raw.rs`, the one file
// here allowed to say `unsafe`; the reasoning is beside them.

impl Registry {
    /// A registry that introduces its host as `id`/`version`, offering
    /// libraries no allocator of its own.
    pub fn new(id: &str, version: &str) -> Registry {
        Registry::with_alloc(id, version, None)
    }

    /// The same, offering libraries the host's allocator so a tree they
    /// build for it is built in the host's arena.
    pub fn with_alloc(id: &str, version: &str, alloc: Option<Alloc>) -> Registry {
        Registry {
            host_id: id.into(),
            host_version: version.into(),
            alloc,
            key: KeyTemplate::default(),
            library_key: KeyTemplate::library("%id").expect("one field and nothing else"),
            loaded: Vec::new(),
            providers: Vec::new(),
            by_key: BTreeMap::new(),
            priorities: BTreeMap::new(),
            shared: Arc::new(Shared::default()),
            block: None,
        }
    }

    /// How this host introduces itself to a library: a handle to a block
    /// that outlives this registry.
    ///
    /// Leaked on the first call and reused after. A host driving a library
    /// itself hands it this; the loader hands it to every entry point.
    pub fn host(&mut self) -> Host {
        let block = match self.block {
            Some(block) => block,
            None => {
                let block = super::raw::leak_host_block(
                    &self.host_id,
                    &self.host_version,
                    self.alloc,
                    Arc::downgrade(&self.shared),
                );
                self.block = Some(block);
                block
            }
        };
        block.host()
    }

    /// Republishes what the services read. Called after every change.
    fn publish(&self) {
        let ranked = self.ranked(self.providers.iter());
        let mut snapshot = Snapshot::default();
        for provider in ranked {
            snapshot
                .by_key
                .insert(provider.key().to_string(), snapshot.entries.len());
            snapshot.entries.push(SnapshotEntry {
                kinds: provider.view.kinds.clone(),
                raw: provider.view.raw,
            });
        }
        // A poisoned lock means a reader panicked while holding it; the
        // snapshot it was reading is intact, and this replaces it.
        let mut slot = self
            .shared
            .snapshot
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *slot = snapshot;
    }

    /// Ranks every provider with this id, now and whenever one loads.
    ///
    /// **This is how a host chooses between two implementations of one
    /// kind**, and it is a host's call rather than a library's: a library
    /// does not know what else is installed, and two machines with the
    /// same set can rank them differently. A frontend reads its own
    /// configuration and calls this; nothing here reads a file.
    ///
    /// Takes effect immediately for what is already loaded, and is
    /// remembered for anything that loads later — so the order in which a
    /// host ranks and scans does not change the answer.
    pub fn set_priority(&mut self, id: &str, priority: i32) {
        self.priorities.insert(id.to_string(), priority);
        for provider in &mut self.providers {
            if provider.view.id == *id {
                provider.priority = priority;
            }
        }
        self.publish();
    }

    /// What this host ranked that id. Zero unless it said otherwise.
    pub fn priority(&self, id: &str) -> i32 {
        self.priorities.get(id).copied().unwrap_or(0)
    }

    /// Every provider serving one kind, best first.
    ///
    /// **`(priority DESC, key ASC)`**. The key tiebreak is deliberate:
    /// load order follows directory iteration, which no filesystem
    /// promises to keep stable across runs or machines, so "whichever
    /// loaded first" is not a rule anyone can document or reproduce. A key
    /// is arbitrary but **deterministic and inspectable**, and a host that
    /// wants a different winner says so with a priority rather than
    /// depending on an order nothing guarantees.
    ///
    /// Collected rather than lazy, because sorting needs every candidate
    /// first. A registry holds tens of providers, not millions.
    fn ranked<'a>(&'a self, of: impl Iterator<Item = &'a Provider>) -> Vec<&'a Provider> {
        let mut found: Vec<&Provider> = of.collect();
        found.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.key().cmp(b.key()))
        });
        found
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
    pub fn libraries_keyed_by(&mut self, template: &str) -> Result<(), KeyError> {
        let key = KeyTemplate::library(template)?;
        let mut seen: BTreeMap<String, usize> = BTreeMap::new();
        for (at, one) in self.loaded.iter().enumerate() {
            let rendered = key.render(KeyFields::library(&one.id, &one.version));
            if seen.insert(rendered.clone(), at).is_some() {
                return Err(KeyError::Collides { key: rendered });
            }
        }
        for one in &mut self.loaded {
            one.key = Text::new(&key.render(KeyFields::library(&one.id, &one.version)));
        }
        self.library_key = key;
        Ok(())
    }

    /// The template this registry files libraries under.
    pub fn library_key_template(&self) -> &KeyTemplate {
        &self.library_key
    }

    /// The id this host introduces itself by.
    pub fn host_id(&self) -> &str {
        &self.host_id
    }

    /// The version this host introduces itself by.
    pub fn host_version(&self) -> &str {
        &self.host_version
    }

    /// Names PROVIDERS with `template` rather than the default `%id`.
    ///
    /// `"%id@%version"` is the one to reach for: it is what lets two
    /// builds of one provider sit in one registry. The whole field list is
    /// on [`KeyTemplate`].
    ///
    /// Callable at any point — anything already loaded is re-keyed, and a
    /// template that would give two of them the same key is refused with
    /// [`KeyError::Collides`] rather than losing one. **A refusal changes
    /// nothing**: every key is rendered and checked before any is written.
    pub fn keyed_by(&mut self, template: &str) -> Result<(), KeyError> {
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
            provider.key = Text::new(&one);
        }
        self.by_key = by_key;
        self.key = key;
        self.publish();
        Ok(())
    }

    /// The template this registry files providers under.
    pub fn key_template(&self) -> &KeyTemplate {
        &self.key
    }

    /// A path as the dedup compares it: resolved, or itself when it
    /// cannot be.
    fn canonical(path: &Path) -> PathBuf {
        path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
    }

    /// Where this library is already loaded from, by canonical path.
    ///
    /// Asked **before** opening anything, because it is the one dedup that
    /// costs no `dlopen` — and mapping a library is irreversible.
    fn loaded_from_path(&self, path: &Path) -> Option<&Loaded> {
        let want = Registry::canonical(path);
        self.loaded.iter().find(|l| l.canonical == want)
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
    ///
    /// A library's entry point may ask its [`Host`] what is loaded: it
    /// sees every library registered before it, and not itself. A
    /// provider that needs a peer offered by a library loaded later looks
    /// it up from a vtable call, not from `describe`.
    pub fn load_file(&mut self, path: &Path) -> Result<Loading<'_>, LoadError> {
        // By path first: the only dedup that can happen without loading.
        if let Some(already) = self.loaded_from_path(path) {
            let from = already.path.clone();
            return Ok(Loading::Skipped(Skipped::AlreadyLoaded { from }));
        }

        // The entry point runs with nothing of this registry borrowed or
        // locked: the host it reads is the leaked block, and what the
        // block answers is the last published snapshot.
        let host = self.host();
        let opened = super::raw::open_library(path, host).map_err(|e| LoadError::Open {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?;
        self.absorb(path, opened)
    }

    /// Registers a library the host **links** rather than loads: its
    /// providers are compiled into the host, and `describe` is what its
    /// entry point would have called.
    ///
    /// The same absorb as [`load_file`](Registry::load_file) — the same
    /// keys, the same dedup, the same refusals, the same `Loaded` record
    /// — over a descriptor that came from this process instead of a
    /// file. `name` stands in for the path in every report (`<name>`),
    /// and a second registration under the same name is
    /// [`Skipped::AlreadyLoaded`] like a second load of one file.
    ///
    /// `describe` is what `guatiao::local_providers!` writes as
    /// `library`, or a hand-written library's own `describe`; it sees
    /// this registry's [`Host`] exactly as a loaded library's entry would.
    /// A `None` is the library declining this host.
    pub fn register_local(
        &mut self,
        name: &str,
        describe: fn(Host) -> Option<&'static super::desc::LibraryInfo>,
    ) -> Result<Loading<'_>, LoadError> {
        let path = PathBuf::from(format!("<{name}>"));
        if let Some(already) = self.loaded_from_path(&path) {
            let from = already.path.clone();
            return Ok(Loading::Skipped(Skipped::AlreadyLoaded { from }));
        }
        let host = self.host();
        let opened = super::raw::open_local(describe(host));
        self.absorb(&path, opened)
    }

    /// [`register_local`](Registry::register_local) for a library that
    /// speaks C: `entry` is what its `guatiao_library_entry` would be --
    /// a library written in C and linked into a Rust host, or anything a
    /// host written in C links (`guatiao_registry_register_entry`). The
    /// same absorb, keys, dedup, refusals and `<name>` path.
    ///
    /// **Handing over an entry point is choosing to run it**, as naming a
    /// file is for [`load_file`](Registry::load_file): `entry` is called
    /// with this registry's host block and is expected to answer null or
    /// a descriptor well-formed for its own `struct_size` that lives for
    /// the process -- exactly what a loaded library's entry point promises,
    /// and read under the same guards.
    pub fn register_entry(&mut self, name: &str, entry: EntryFn) -> Result<Loading<'_>, LoadError> {
        let path = PathBuf::from(format!("<{name}>"));
        if let Some(already) = self.loaded_from_path(&path) {
            let from = already.path.clone();
            return Ok(Loading::Skipped(Skipped::AlreadyLoaded { from }));
        }
        let host = self.host();
        let opened = super::raw::open_entry(entry, host);
        self.absorb(&path, opened)
    }

    /// Records what opening `path` came to. Everything after the `dlopen`,
    /// so it is testable without one.
    pub(crate) fn absorb(&mut self, path: &Path, opened: Opened) -> Result<Loading<'_>, LoadError> {
        let (lib, origin) = match opened {
            Opened::NoEntrySymbol => return Ok(Loading::Skipped(Skipped::NoEntrySymbol)),
            Opened::Declined => return Ok(Loading::Skipped(Skipped::DeclinedThisHost)),
            Opened::Rejected(Rejected::UnsupportedAbi { declared }) => {
                return Ok(Loading::Skipped(Skipped::UnsupportedAbi { declared }));
            }
            Opened::Rejected(Rejected::Malformed) => {
                return Err(LoadError::Malformed {
                    path: path.to_path_buf(),
                });
            }
            Opened::Loaded(view, origin) => (view, origin),
        };
        let LibraryView {
            id,
            version,
            meta,
            unload,
            providers: views,
        } = lib;

        // By KEY after: the same library staged at two paths is the same
        // library, which is the case a path comparison cannot see and a
        // filename comparison only guesses at. Whether two versions of one
        // library are the same thing is the host's template's answer.
        let library_key = self.library_key.render(KeyFields::library(&id, &version));
        if let Some(already) = self.loaded.iter().find(|l| *l.key == *library_key) {
            let from = already.path.clone();
            return Ok(Loading::Skipped(Skipped::AlreadyLoaded { from }));
        }

        // **The rendered key is what decides whether a provider is already
        // here.** Under `%id` a second build of one provider renders the
        // key the first did and is passed over; under `%id@%version` it
        // renders its own and is kept. Same key and same id is the ordinary
        // repeat — a re-export, a vendored copy, a second build — and the
        // rest of the library goes on loading. Same key and a DIFFERENT id
        // is the host's template failing, refused before anything is
        // recorded so the registry is left exactly as it was.
        let mut skipped = Vec::new();
        let mut taken: Vec<(ProviderView, String, String)> = Vec::new();
        for view in views {
            // A provider's own version, or its library's when it declared
            // none.
            let at = view.version.clone().unwrap_or_else(|| version.clone());
            let key = self.key.render(KeyFields {
                id: &view.id,
                name: &view.display_name,
                library: &id,
                version: &at,
            });

            // Whoever holds that key already: registered earlier, or
            // earlier in this same library.
            let holder: Option<(String, PathBuf)> = self
                .by_key
                .get(&key)
                .map(|&i| {
                    (
                        self.providers[i].view.id.clone(),
                        self.providers[i].from.clone(),
                    )
                })
                .or_else(|| {
                    taken
                        .iter()
                        .find(|(_, _, earlier)| *earlier == key)
                        .map(|(v, _, _)| (v.id.clone(), path.to_path_buf()))
                });
            match holder {
                Some((held_by, from)) if held_by == view.id => {
                    skipped.push(Skipped::ProviderAlreadyLoaded {
                        id: view.id.clone(),
                        from,
                    });
                }
                Some((_, first)) => {
                    return Err(LoadError::Duplicate {
                        key,
                        first,
                        second: path.to_path_buf(),
                    });
                }
                None => taken.push((view, at, key)),
            }
        }

        let count = taken.len();
        for (view, at, key) in taken {
            // What this host already said about that id, if anything —
            // read BEFORE the view moves into the provider.
            let priority = self.priorities.get(&view.id).copied().unwrap_or(0);
            self.by_key.insert(key.clone(), self.providers.len());
            self.providers.push(Provider {
                view,
                from: path.to_path_buf(),
                library: id.clone(),
                version: at,
                key: Text::new(&key),
                priority,
            });
        }
        self.loaded.push(Loaded {
            path: path.to_path_buf(),
            key: Text::new(&library_key),
            id,
            version,
            providers: count,
            skipped,
            meta,
            canonical: Registry::canonical(path),
            origin,
            unload,
        });
        self.publish();
        Ok(Loading::Loaded(self.loaded.last().expect("just pushed")))
    }

    /// Takes one library out of this registry, **leaving it mapped**.
    ///
    /// `key` is the library key, from the template
    /// [`libraries_keyed_by`](Registry::libraries_keyed_by) sets (`%id` by
    /// default). Its providers leave the registry and the snapshot a
    /// library reads, its [`Loaded`] record goes, and the key may be
    /// loaded again — a retired library is not
    /// [`Skipped::AlreadyLoaded`]. Nothing new can be obtained from it;
    /// what a host already took — a [`Remote`], an [`Offer`], a vtable
    /// pointer, a descriptor another library fetched through this host's
    /// services — keeps working, because the mapping stays.
    ///
    /// Safe for that reason: retiring dangles nothing.
    /// [`unload`](Registry::unload) is the one that unmaps, and it is
    /// `unsafe`; it is written in `raw.rs`, the file here allowed to say
    /// so.
    pub fn retire(&mut self, key: &str) -> Result<Retired, UnloadError> {
        let (retired, origin) = self.take_library(key)?;
        origin.keep();
        Ok(retired)
    }

    /// The record one library key answers to.
    ///
    /// For `unload`, which lives in `raw.rs` because this module carries
    /// `#![forbid(unsafe_code)]`.
    pub(crate) fn library(&self, key: &str) -> Option<&Loaded> {
        self.loaded.iter().find(|l| *l.key == *key)
    }

    /// Removes a library and hands back its record and its mapping.
    /// Whoever calls decides what becomes of the handle.
    pub(crate) fn take_library(&mut self, key: &str) -> Result<(Retired, Origin), UnloadError> {
        let at = self
            .loaded
            .iter()
            .position(|l| *l.key == *key)
            .ok_or_else(|| UnloadError::NotFound {
                key: key.to_string(),
            })?;
        let one = self.loaded.remove(at);

        // By path, which is what `absorb` recorded on every provider it
        // took from this library and is unique per registered library.
        let before = self.providers.len();
        self.providers.retain(|p| p.from != one.path);
        let providers = before - self.providers.len();

        // `by_key` is by index, so every index after a removal is wrong.
        self.by_key = self
            .providers
            .iter()
            .enumerate()
            .map(|(at, p)| (p.key().to_string(), at))
            .collect();
        self.publish();

        Ok((
            Retired {
                key: key.to_string(),
                id: one.id,
                version: one.version,
                providers,
            },
            one.origin,
        ))
    }

    /// Every provider that speaks one kind, best first: `(priority DESC,
    /// key ASC)`.
    ///
    /// The capability question — "what can serve this?" — as opposed to
    /// [`provider`](Registry::provider), which is the identity one. A
    /// provider serving several kinds answers to each of them, which is
    /// why it is one provider rather than one per kind.
    pub fn providers(&self, kind: &str) -> impl Iterator<Item = &Provider> {
        self.ranked(self.providers.iter().filter(move |p| p.supports(kind)))
            .into_iter()
    }

    /// Every provider of every kind, best first: `(priority DESC, key
    /// ASC)`. [`all`](Registry::all) is the same set in load order.
    pub fn all_ranked(&self) -> impl Iterator<Item = &Provider> {
        self.ranked(self.providers.iter()).into_iter()
    }

    /// Every provider serving `K` with a valid table, as the trait, best
    /// first — **unavailable ones included**. The consumer asks each and
    /// chooses; nothing here picks.
    ///
    /// A provider claiming `K` whose table fails validation is not an
    /// offer, because it cannot be called; [`mismatches`](Registry::mismatches)
    /// reports it.
    pub fn offers<K: ?Sized + Kind>(&self) -> impl Iterator<Item = Offer<K>> + '_ {
        self.providers(K::NAME).filter_map(|p| p.offer::<K>().ok())
    }

    /// Providers claiming `K` whose per-kind table failed validation,
    /// each with why, so a host can report them rather than lose them.
    ///
    /// A provider claiming `K` with no per-kind table — a library written
    /// before per-kind tables existed, or a pure label — is neither an
    /// offer nor a mismatch: the typed path never saw a table to judge.
    pub fn mismatches<K: ?Sized + Kind>(
        &self,
    ) -> impl Iterator<Item = (&Provider, KindMismatch)> + '_ {
        self.providers(K::NAME)
            .filter_map(|p| p.as_kind::<K>().err().map(|why| (p, why)))
            .filter(|(_, why)| *why != KindMismatch::NoTable)
    }

    /// One provider by key, as `K`: `None` when nothing answers to the
    /// key, `Some(Err)` when it does and its table does not validate.
    pub fn offer<K: ?Sized + Kind>(&self, key: &str) -> Option<Result<Offer<K>, KindMismatch>> {
        self.provider(key).map(Provider::offer::<K>)
    }

    /// One provider by the key this registry filed it under.
    ///
    /// Under the default template that is its id; under `%id@%version` it
    /// is `"hello@1.2.0"`. [`Provider::key`] is the other side of this,
    /// for a host that has a provider and wants the name it answers to.
    pub fn provider(&self, key: &str) -> Option<&Provider> {
        self.by_key.get(key).map(|&at| &self.providers[at])
    }

    /// Every provider that serves one kind **and can actually run here**.
    ///
    /// [`providers`](Registry::providers) is who CLAIMS the kind; this is
    /// who can serve it now. Each is asked at the moment of the question,
    /// so a provider whose optional dependency arrived or went away since
    /// the last call answers differently — which is the point.
    pub fn available(&self, kind: &str) -> impl Iterator<Item = &Provider> {
        self.providers(kind).filter(|p| p.available().is_ok())
    }

    /// The best provider serving one kind that can actually run here, or
    /// `None`.
    ///
    /// The question a host usually has. It is
    /// [`available`](Registry::available) taking the head, which is
    /// `(priority DESC, key ASC)` — and a host that wants to see what it
    /// passed over, to say "ssh -> openssh (also: putty)", iterates
    /// instead.
    pub fn best(&self, kind: &str) -> Option<&Provider> {
        self.available(kind).next()
    }

    /// Why nothing can serve `kind`, or `None` when something can.
    ///
    /// **The two answers are kept apart because the remedies differ.**
    /// "Nothing claims it" means install something; "everything that
    /// claims it is unavailable here" means fix what you already have, and
    /// carries each provider's own words about why. Collapsing them into
    /// one "unsupported" leaves a person with no idea which way to go.
    pub fn why_not(&self, kind: &str) -> Option<WhyNot<'_>> {
        let mut refused = Vec::new();
        for provider in self.providers(kind) {
            match provider.available() {
                // Something can serve it, so there is nothing to explain.
                Ok(()) => return None,
                Err(reason) => refused.push((provider, reason)),
            }
        }
        if refused.is_empty() {
            Some(WhyNot::NothingClaimsIt)
        } else {
            Some(WhyNot::NoneAvailable(refused))
        }
    }

    /// Every provider with one id, whatever the key template made of it —
    /// which is every loaded version of it, best first: `(priority DESC,
    /// key ASC)`.
    ///
    /// More than one only under a template that renders versions apart
    /// (`%id@%version`); under `%id` the second build was skipped at load.
    /// Which of them is "newest" is the host's to decide, with the semver
    /// library it already has.
    pub fn providers_of(&self, id: &str) -> impl Iterator<Item = &Provider> {
        self.ranked(self.providers.iter().filter(move |p| p.id() == id))
            .into_iter()
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::c_void;

    fn provider(id: &str, version: Option<&str>) -> ProviderView {
        ProviderView {
            kinds: vec!["thing".to_string()],
            id: id.to_string(),
            display_name: String::new(),
            config: None,
            vtable: std::ptr::null(),
            vtable_size: 0,
            ctx: std::ptr::null_mut::<c_void>(),
            meta: None,
            version: version.map(str::to_string),
            available: None,
            raw: std::ptr::null(),
            tables: Vec::new(),
            create: None,
            destroy: None,
        }
    }

    fn library(id: &str, version: &str, providers: Vec<ProviderView>) -> Opened {
        Opened::Loaded(
            LibraryView {
                id: id.to_string(),
                version: version.to_string(),
                meta: None,
                unload: None,
                providers,
            },
            Origin::Linked,
        )
    }

    fn registry() -> Registry {
        Registry::new("test-host", "1.0")
    }

    /// Each way a file can come to nothing is reported as itself.
    #[test]
    fn every_outcome_of_opening_a_file_is_reported_as_itself() {
        let mut r = registry();
        let p = Path::new("a.so");

        assert_eq!(
            r.absorb(p, Opened::NoEntrySymbol).unwrap().skipped(),
            Some(&Skipped::NoEntrySymbol)
        );
        assert_eq!(
            r.absorb(p, Opened::Declined).unwrap().skipped(),
            Some(&Skipped::DeclinedThisHost)
        );
        assert_eq!(
            r.absorb(
                p,
                Opened::Rejected(Rejected::UnsupportedAbi { declared: 7 })
            )
            .unwrap()
            .skipped(),
            Some(&Skipped::UnsupportedAbi { declared: 7 })
        );
        assert!(matches!(
            r.absorb(p, Opened::Rejected(Rejected::Malformed)),
            Err(LoadError::Malformed { .. })
        ));
        assert!(r.loaded().is_empty(), "nothing was recorded");
    }

    /// Under `%id`, a second build of one provider renders the key the
    /// first did and is skipped; under `%id@%version` it is kept.
    #[test]
    fn the_key_template_decides_whether_two_builds_of_a_provider_coexist() {
        let mut r = registry();
        r.libraries_keyed_by("%id@%version").unwrap();
        r.absorb(
            Path::new("one.so"),
            library("lib", "1.0.0", vec![provider("lib_p", None)]),
        )
        .unwrap();

        let second = r
            .absorb(
                Path::new("two.so"),
                library("lib", "2.0.0", vec![provider("lib_p", None)]),
            )
            .unwrap();
        let loaded = second.loaded().expect("the library loads");
        assert_eq!(loaded.providers, 0);
        assert!(matches!(
            loaded.skipped.as_slice(),
            [Skipped::ProviderAlreadyLoaded { id, .. }] if id == "lib_p"
        ));
        assert_eq!(r.providers_of("lib_p").count(), 1);

        let mut apart = registry();
        apart.libraries_keyed_by("%id@%version").unwrap();
        apart.keyed_by("%id@%version").unwrap();
        apart
            .absorb(
                Path::new("one.so"),
                library("lib", "1.0.0", vec![provider("lib_p", None)]),
            )
            .unwrap();
        let loaded = apart
            .absorb(
                Path::new("two.so"),
                library("lib", "2.0.0", vec![provider("lib_p", None)]),
            )
            .unwrap()
            .loaded()
            .expect("the second build loads")
            .providers;
        assert_eq!(loaded, 1);
        let versions: Vec<&str> = apart.providers_of("lib_p").map(Provider::version).collect();
        assert_eq!(
            versions,
            ["1.0.0", "2.0.0"],
            "every loaded version, key order"
        );
        assert!(apart.provider("lib_p@2.0.0").is_some());
    }

    /// Two DIFFERENT providers on one key is the template failing, and is
    /// refused before anything is recorded.
    #[test]
    fn two_different_providers_on_one_key_are_refused_and_nothing_is_recorded() {
        let mut r = registry();
        r.keyed_by("%library").unwrap();
        let err = r
            .absorb(
                Path::new("one.so"),
                library(
                    "lib",
                    "1.0.0",
                    vec![provider("lib_a", None), provider("lib_b", None)],
                ),
            )
            .unwrap_err();
        assert!(matches!(err, LoadError::Duplicate { key, .. } if key == "lib"));
        assert!(r.loaded().is_empty() && r.all().is_empty());
    }

    /// A provider offered twice within one library is a skip, not a
    /// refusal: the first stands.
    #[test]
    fn a_provider_repeated_inside_one_library_is_skipped() {
        let mut r = registry();
        let loaded = r
            .absorb(
                Path::new("one.so"),
                library(
                    "lib",
                    "1.0.0",
                    vec![provider("lib_p", None), provider("lib_p", None)],
                ),
            )
            .unwrap();
        let loaded = loaded.loaded().unwrap();
        assert_eq!((loaded.providers, loaded.skipped.len()), (1, 1));
    }
}
