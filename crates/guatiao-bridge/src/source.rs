// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The glue from an [`Engine`] to guatiao's [`Source`]: listing reads
//! manifests and runs nothing, opening runs the engine, closing lets the
//! library go.

use alloc::sync::Arc;
use std::sync::{Mutex, PoisonError};

use guatiao::library::kind::Mechanism;
use guatiao::library::{Candidate, Host, LocalLibrary, ProviderError, Source};

use crate::config::Config;
use crate::engine::{Engine, Opened, ThreadModel};
use crate::error::BridgeError;
use crate::kinds::Kinds;
use crate::manifest::{self, Found, Manifest};
use crate::thread::Runner;

/// A source of the libraries an engine serves: what a language crate
/// exports with `guatiao::source!`, or a Rust host adds with
/// `Registry::add_source`.
///
/// Each library is a manifest ([`ManifestFormat`](crate::ManifestFormat)
/// says how they are named) in the configuration's `dirs` or
/// `manifests`. A refresh lists them without running any code (a version
/// the manifest leaves out is read by [`Engine::version_of`]), opens a new
/// or changed one through [`Engine::open`], and closes one the registry
/// let go through [`Engine::close`].
///
/// Providers an engine declares ([`Opened::Described`]) are served through
/// tables built from their kinds' descriptions, which needs a
/// [`Mechanism`] ([`with_mechanism`](Self::with_mechanism)); without one,
/// such a library is refused.
pub struct BridgeSource<E: Engine> {
    runner: Runner<E>,
    config: Config,
    kinds: Arc<Kinds>,
    mechanism: Option<Arc<dyn Mechanism>>,
    /// The stamp each id was last listed under: what the registry names an
    /// open by when it closes it.
    listed: Mutex<Vec<(String, String)>>,
    /// Every library opened and not yet closed: its id, the stamp it was
    /// listed under, and the engine's handle.
    open: Mutex<Vec<(String, String, E::Library)>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl<E: Engine> core::fmt::Debug for BridgeSource<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BridgeSource")
            .field("threads", &self.runner.threads())
            .field("kinds", &self.kinds.names().collect::<Vec<_>>())
            .finish()
    }
}

fn kinds_of(config: &Config) -> Result<Kinds, BridgeError> {
    let mut kinds = Kinds::new();
    for path in config.kinds() {
        kinds.load(path)?;
    }
    Ok(kinds)
}

impl<E: Engine> BridgeSource<E> {
    /// Starts an engine every caller shares, on this thread: one that is
    /// [`ThreadModel::Any`] or [`ThreadModel::Locked`].
    pub fn shared(config: &guatiao::Map) -> Result<BridgeSource<E>, BridgeError>
    where
        E: Send + Sync,
    {
        if E::THREADS == ThreadModel::Pinned {
            return Err(BridgeError::Unavailable(
                "this engine is pinned to a thread of its own: start it with `pinned`".to_string(),
            ));
        }
        let config = Config::from_map(config)?;
        let kinds = kinds_of(&config)?;
        let engine = E::start(&config)?;
        Ok(BridgeSource {
            runner: Runner::shared(engine),
            config,
            kinds: Arc::new(kinds),
            mechanism: None,
            listed: Mutex::new(Vec::new()),
            open: Mutex::new(Vec::new()),
        })
    }

    /// Starts the engine on a thread of its own, named `thread`, which
    /// owns it for the life of this source: every call is queued to it.
    /// The thread's stack is the one [`Engine::stack_size`] asks for.
    pub fn pinned(config: &guatiao::Map, thread: &str) -> Result<BridgeSource<E>, BridgeError> {
        let config = Config::from_map(config)?;
        let kinds = kinds_of(&config)?;
        let on_thread = config.clone();
        let runner = Runner::pinned(thread, E::stack_size(&config), move || E::start(&on_thread))?;
        Ok(BridgeSource {
            runner,
            config,
            kinds: Arc::new(kinds),
            mechanism: None,
            listed: Mutex::new(Vec::new()),
            open: Mutex::new(Vec::new()),
        })
    }

    /// Serves the providers an engine declares through tables `mechanism`
    /// builds (`guatiao_libffi::Libffi`, for one).
    #[must_use]
    pub fn with_mechanism(mut self, mechanism: Arc<dyn Mechanism>) -> BridgeSource<E> {
        self.mechanism = Some(mechanism);
        self
    }

    /// Its configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The kinds it was given descriptions of.
    pub fn kinds(&self) -> &Kinds {
        &self.kinds
    }

    /// Where its engine's calls run.
    pub fn threads(&self) -> ThreadModel {
        self.runner.threads()
    }

    /// Runs `f` with the engine, where its thread model says: what a
    /// language crate reaches its own engine through.
    pub fn run<T: Send + 'static>(
        &self,
        f: impl FnOnce(&E) -> T + Send + 'static,
    ) -> Result<T, BridgeError> {
        self.runner.run(f)
    }

    fn manifests(&self) -> Vec<Found> {
        manifest::find(&E::MANIFESTS, self.config.dirs(), self.config.manifests())
    }

    /// The version a library is opened at: the manifest's, or the
    /// engine's reading of the code, refused in words that name the
    /// library.
    fn version(&self, manifest: &Manifest) -> Result<String, BridgeError> {
        if let Some(v) = manifest.version() {
            return Ok(v.to_string());
        }
        let reading = manifest.clone();
        self.runner
            .run(move |e| e.version_of(&reading))
            .and_then(|r| r)
            .map_err(|e| BridgeError::Manifest {
                path: manifest.path().to_path_buf(),
                why: format!("{}: {e}", manifest.id()),
            })
    }

    /// Records an open under the stamp `id` was last listed under, or the
    /// manifest's own when it was never listed.
    fn opened(&self, id: &str, stamp: &str, library: E::Library) {
        let stamp = lock(&self.listed)
            .iter()
            .find(|(listed, _)| listed == id)
            .map_or_else(|| stamp.to_string(), |(_, s)| s.clone());
        lock(&self.open).push((id.to_string(), stamp, library));
    }

    fn described(
        &self,
        manifest: &Manifest,
        version: &str,
        library: E::Library,
        providers: Vec<crate::engine::Provided>,
    ) -> Result<LocalLibrary<'static>, BridgeError> {
        let Some(mechanism) = &self.mechanism else {
            return Err(BridgeError::Unavailable(
                "serving described providers needs a mechanism: give the source one with \
                 `with_mechanism`"
                    .to_string(),
            ));
        };
        crate::described::library(
            self.runner.clone(),
            &self.kinds,
            &**mechanism,
            manifest.id(),
            version,
            library,
            providers,
        )
        .map(LocalLibrary::from)
    }
}

impl<E: Engine> Source for BridgeSource<E> {
    fn list(&self, _host: Host) -> Result<Vec<Candidate>, ProviderError> {
        let candidates: Vec<Candidate> = self
            .manifests()
            .into_iter()
            .map(|found| {
                // A library whose version cannot be read is listed under
                // its id, with nothing it serves, so opening it says why.
                let (version, kinds) = match &found {
                    Found::Library(m) => match self.version(m) {
                        Ok(version) => (version, m.kinds().to_vec()),
                        Err(_) => (String::new(), Vec::new()),
                    },
                    Found::Broken { .. } => (String::new(), Vec::new()),
                };
                Candidate::new(found.id(), version, kinds, found.stamp())
            })
            .collect();
        *lock(&self.listed) = candidates
            .iter()
            .map(|c| (c.id().to_string(), c.stamp().to_string()))
            .collect();
        Ok(candidates)
    }

    fn open(&self, _host: Host, id: &str) -> Result<LocalLibrary<'static>, ProviderError> {
        let found = self
            .manifests()
            .into_iter()
            .find(|f| f.id() == id)
            .ok_or_else(|| BridgeError::NotListed { id: id.to_string() })?;
        let manifest = match found {
            Found::Library(m) => m,
            Found::Broken { path, why, .. } => {
                return Err(BridgeError::Manifest { path, why }.into());
            }
        };
        let version = self.version(&manifest)?;
        let (opening, at) = (manifest.clone(), version.clone());
        let opened = self.runner.run(move |e| e.open(&opening, &at))??;
        match opened {
            Opened::Built {
                library,
                descriptor,
            } => {
                self.opened(id, manifest.stamp(), library);
                Ok(descriptor)
            }
            Opened::Described { library, providers } => {
                match self.described(&manifest, &version, library.clone(), providers) {
                    Ok(parts) => {
                        self.opened(id, manifest.stamp(), library);
                        Ok(parts)
                    }
                    Err(e) => {
                        // Opened and refused here: the engine hears of it now.
                        let _ = self.runner.run(move |e| e.close(library));
                        Err(e.into())
                    }
                }
            }
        }
    }

    fn close(&self, id: &str, stamp: &str) {
        let library = {
            let mut open = lock(&self.open);
            open.iter()
                .position(|(i, s, _)| i == id && s == stamp)
                .map(|at| open.remove(at).2)
        };
        if let Some(library) = library {
            let _ = self.runner.run(move |e| e.close(library));
        }
    }
}
