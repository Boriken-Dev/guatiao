// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The host's side: peers' providers as a guatiao source.
//!
//! [`RemoteSource`] lists what a peer's catalog lists and opens each
//! library as ordinary providers whose tables call over the link, built at
//! run time from the kinds the catalog describes. A refresh that finds the
//! link gone connects again (a child process is started again), and every
//! library of the old link is listed under a new stamp, so the registry
//! retires the old and opens the new.
//!
//! One peer the configuration names serves its libraries under their own
//! ids. Peers described by manifests are named, and serve their libraries
//! and providers as `<peer>/<id>`: identical devices serve the same
//! library, and a source lists each id once. They are listed at once, each
//! connecting on a thread of its own; a peer whose manifest changed keeps
//! its link while the registry holds a library it opened.

#![forbid(unsafe_code)]

use alloc::collections::BTreeMap;
use alloc::sync::{Arc, Weak};
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use guatiao::library::kind::{
    Answer, Arg, KindDescription, LibraryParts, ProviderParts, Serve, ServedTable,
};
use guatiao::library::{Candidate, Host, LocalLibrary, ProviderError, Source};
use guatiao::{Map, Status};
use guatiao_bridge::Kinds;
use guatiao_bridge::manifest::{self, Found};
use guatiao_libffi::Libffi;

use crate::config;
use crate::link::{Inner, Link, LinkOptions, Nothing, Proxies, Setup};
use crate::lock;
use crate::protocol::Target;
use crate::protocol::catalog::{Catalog, LibraryEntry};
use crate::transport::Connect;

#[cfg(all(test, feature = "tcp"))]
mod tests;

/// Described peers listed at once by one refresh.
const LISTED_AT_ONCE: usize = 16;

/// One link and what its catalog listed.
struct Current {
    link: Arc<Link>,
    generation: u64,
    libraries: BTreeMap<String, LibraryEntry>,
}

/// One peer: where it is, its link, and what its catalog last listed.
struct Peer {
    connect: Box<dyn Connect>,
    options: LinkOptions,
    proxies: Proxies,
    current: Mutex<Option<Current>>,
    /// Bumped by `disconnect`: a link that connected meanwhile is ended,
    /// not kept.
    disconnects: AtomicU64,
    /// What the last listing that succeeded answered: a peer that cannot
    /// be listed now, whether it could not be reached or its new link
    /// failed to list, keeps what it served. `None` until one succeeds.
    listed: Mutex<Option<Vec<Candidate>>>,
    /// How many of each library the registry has open, by the id the peer
    /// gives and the stamp it was listed under.
    opened: Mutex<BTreeMap<(String, String), usize>>,
    /// Counts links across every peer of a source, so no stamp repeats
    /// within it.
    generations: Arc<AtomicU64>,
    /// The settings of its TLS policy looser than the modern one.
    #[cfg(feature = "tls")]
    weakened: Vec<crate::transport::tls::Weakening>,
}

/// Peers' providers, served to a host's registry: one peer, or every peer
/// a directory of manifests describes. A clone is the same source: keep
/// one to reach its links after adding the other.
#[derive(Clone)]
pub struct RemoteSource {
    inner: Arc<Peers>,
}

enum Peers {
    /// One peer, its libraries under their own ids.
    One(Peer),
    /// Named peers, each a manifest.
    Described(Described),
}

/// The peers manifests describe, as the last refresh found them.
struct Described {
    dirs: Vec<PathBuf>,
    files: Vec<PathBuf>,
    /// The source's configuration: every peer's, unless its manifest
    /// names a key.
    defaults: Map,
    generations: Arc<AtomicU64>,
    state: Mutex<Listing>,
}

/// What the last listing found.
#[derive(Default)]
struct Listing {
    /// Each peer by name, and the stamp of the description it was made
    /// from: a changed one is a new peer.
    named: BTreeMap<String, (Arc<Peer>, String)>,
    /// Peers a changed description replaced, by name: each keeps its link
    /// while the registry holds a library it opened.
    leaving: Vec<(String, Arc<Peer>)>,
    /// Candidates listed only to be reported, by id (a manifest's path):
    /// a manifest describing no peer, a peer that cannot be reached. Their
    /// open answers this.
    refusals: BTreeMap<String, (Status, String)>,
}

impl Listing {
    /// Takes out the replaced peers the registry holds nothing of, for
    /// their links to end.
    fn let_go(&mut self) -> Vec<Arc<Peer>> {
        let (kept, gone) = core::mem::take(&mut self.leaving)
            .into_iter()
            .partition(|(_, peer)| peer.holds());
        self.leaving = kept;
        gone.into_iter().map(|(_, peer)| peer).collect()
    }

    /// Lists `id` only to report `why` when it is opened.
    fn refuse(
        &mut self,
        listed: &mut Vec<Candidate>,
        id: String,
        stamp: &str,
        status: Status,
        why: String,
    ) {
        listed.push(Candidate::new(id.as_str(), "", Vec::new(), stamp));
        self.refusals.insert(id, (status, why));
    }
}

impl core::fmt::Debug for RemoteSource {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match &*self.inner {
            Peers::One(peer) => f
                .debug_struct("RemoteSource")
                .field("peer", &peer.connect.describe())
                .field("options", &peer.options)
                .finish(),
            Peers::Described(d) => f
                .debug_struct("RemoteSource")
                .field("dirs", &d.dirs)
                .field("manifests", &d.files)
                .finish(),
        }
    }
}

/// Every table of one provider calls through here.
struct Tables {
    link: Weak<Inner>,
    /// The peer's table id, by kind.
    ids: BTreeMap<String, u64>,
}

impl Serve for Tables {
    fn call(
        &self,
        _ctx: usize,
        kind: &KindDescription,
        slot: usize,
        args: Vec<Arg<'_>>,
    ) -> Result<Answer, ProviderError> {
        let link = self.link.upgrade().ok_or_else(|| {
            ProviderError::new(
                Status::GUATIAO_ERR_GONE,
                "the link this provider came through has ended; refresh to reconnect",
            )
        })?;
        let id = self.ids.get(kind.name()).copied().ok_or_else(|| {
            ProviderError::new(Status::GUATIAO_ERR_NOT_FOUND, "no table of this kind")
        })?;
        link.call(Target::Table(id), kind, slot, args)
    }
}

/// The stamp a library is listed under: the peer's, and the link's
/// generation, so a reconnect lists everything anew.
fn listed_stamp(stamp: &str, generation: u64) -> String {
    format!("{stamp}@{generation}")
}

fn unlisted_stamp(stamp: &str) -> Option<(&str, u64)> {
    let (theirs, generation) = stamp.rsplit_once('@')?;
    Some((theirs, generation.parse().ok()?))
}

/// What a link's catalog lists, as candidates.
fn candidates(current: &Current) -> Vec<Candidate> {
    current
        .libraries
        .values()
        .map(|l| {
            let mut kinds: Vec<String> = l
                .providers
                .iter()
                .flat_map(|p| p.tables.iter().map(|t| t.kind.clone()))
                .collect();
            kinds.sort();
            kinds.dedup();
            Candidate::new(
                l.id.as_str(),
                l.version.as_str(),
                kinds,
                listed_stamp(&l.stamp, current.generation),
            )
        })
        .collect()
}

/// `id` under a peer's name, or as it is for an unnamed peer.
fn named(peer: Option<&str>, id: &str) -> String {
    match peer {
        Some(peer) => format!("{peer}/{id}"),
        None => id.to_string(),
    }
}

impl Peer {
    fn new(connect: Box<dyn Connect>, options: LinkOptions, generations: Arc<AtomicU64>) -> Peer {
        Peer {
            connect,
            options,
            proxies: Proxies::new(),
            current: Mutex::new(None),
            disconnects: AtomicU64::new(0),
            listed: Mutex::new(None),
            opened: Mutex::new(BTreeMap::new()),
            generations,
            #[cfg(feature = "tls")]
            weakened: Vec::new(),
        }
    }

    /// The same peer, `map` being the configuration it was made from.
    fn configured(self, map: &Map) -> Peer {
        let _ = map;
        Peer {
            #[cfg(feature = "tls")]
            weakened: config::weakened_in(map),
            ..self
        }
    }

    /// The link, connecting first unless one is alive. The connection and
    /// its handshake run outside the lock, so `disconnect` never waits for
    /// them.
    fn link(&self) -> Result<Arc<Link>, ProviderError> {
        if let Some(c) = lock(&self.current).as_ref()
            && c.link.alive()
        {
            return Ok(Arc::clone(&c.link));
        }
        let disconnects = self.disconnects.load(Ordering::Acquire);
        let conn = self.connect.connect().map_err(|e| {
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                // A trust check refused the peer: the status of every trust failure.
                return ProviderError::new(
                    Status::GUATIAO_ERR_BAD_VALUE,
                    &format!("{}: {e}", self.connect.describe()),
                );
            }
            ProviderError::new(
                Status::GUATIAO_ERR_GONE,
                &format!("{} could not be reached: {e}", self.connect.describe()),
            )
        })?;
        let link = Link::client(
            conn,
            &self.options,
            Setup {
                handler: Arc::new(Nothing),
                proxies: self.proxies.clone(),
                kinds: Arc::new(Kinds::new()),
                name: "guatiao-source-remote".to_string(),
            },
        )?;
        let link = Arc::new(link);
        let mut current = lock(&self.current);
        if self.disconnects.load(Ordering::Acquire) != disconnects {
            drop(current);
            link.end();
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_GONE,
                "the source was disconnected while it connected",
            ));
        }
        if let Some(c) = current.as_ref()
            && c.link.alive()
        {
            // Another thread connected meanwhile: one link is kept.
            let theirs = Arc::clone(&c.link);
            drop(current);
            link.end();
            return Ok(theirs);
        }
        let generation = self.generations.fetch_add(1, Ordering::Relaxed) + 1;
        *current = Some(Current {
            link: Arc::clone(&link),
            generation,
            libraries: BTreeMap::new(),
        });
        Ok(link)
    }

    fn list(&self) -> Result<Vec<Candidate>, ProviderError> {
        let link = self.link()?;
        let catalog = Catalog::from_value(&link.list()?)
            .map_err(|why| ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, &why))?;
        let mut kinds = Kinds::new();
        for k in &catalog.kinds {
            kinds
                .add_value(k)
                .map_err(|e| ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, &e.to_string()))?;
        }
        link.set_kinds(Arc::new(kinds));
        let mut current = lock(&self.current);
        let Some(c) = current.as_mut().filter(|c| Arc::ptr_eq(&c.link, &link)) else {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_GONE,
                "the link changed while it was listed",
            ));
        };
        c.libraries = catalog
            .libraries
            .iter()
            .map(|l| (l.id.clone(), l.clone()))
            .collect();
        let listed = candidates(c);
        *lock(&self.listed) = Some(listed.clone());
        Ok(listed)
    }

    /// What the last listing that succeeded answered, under the stamps it
    /// answered.
    fn listed_before(&self) -> Vec<Candidate> {
        lock(&self.listed).clone().unwrap_or_default()
    }

    /// Whether a listing of it ever succeeded.
    fn ever_listed(&self) -> bool {
        lock(&self.listed).is_some()
    }

    /// Whether the registry holds a library this peer opened.
    fn holds(&self) -> bool {
        !lock(&self.opened).is_empty()
    }

    /// Whether its link's last listing listed `id`.
    fn lists(&self, id: &str) -> bool {
        lock(&self.current)
            .as_ref()
            .is_some_and(|c| c.libraries.contains_key(id))
    }

    /// Opens the library the peer calls `id`, named as `peer` names it.
    fn open(&self, id: &str, peer: Option<&str>) -> Result<LocalLibrary<'static>, ProviderError> {
        let (link, entry, generation) = {
            let current = lock(&self.current);
            let c = current.as_ref().ok_or_else(|| {
                ProviderError::new(Status::GUATIAO_ERR_GONE, "not connected: refresh first")
            })?;
            let entry = c.libraries.get(id).cloned().ok_or_else(|| {
                ProviderError::new(Status::GUATIAO_ERR_NOT_FOUND, "the peer no longer lists it")
            })?;
            (Arc::clone(&c.link), entry, c.generation)
        };
        let library = self.build(&link, &entry, peer)?;
        let key = (entry.id.clone(), listed_stamp(&entry.stamp, generation));
        *lock(&self.opened).entry(key).or_insert(0) += 1;
        Ok(library)
    }

    /// The library `entry` lists, its tables calling over `link`.
    fn build(
        &self,
        link: &Arc<Link>,
        entry: &LibraryEntry,
        peer: Option<&str>,
    ) -> Result<LocalLibrary<'static>, ProviderError> {
        link.open(&entry.id, &entry.stamp)?;
        let kinds = link.kinds();
        let mut providers = Vec::new();
        for p in &entry.providers {
            let serve: Arc<dyn Serve> = Arc::new(Tables {
                link: Arc::downgrade(link.inner()),
                ids: p.tables.iter().map(|t| (t.kind.clone(), t.table)).collect(),
            });
            let mut builder = ProviderParts::builder(&named(peer, &p.id)).name(&p.name);
            if let Some(meta) = &p.meta {
                builder = builder.meta(meta.clone());
            }
            let mut tables = Vec::new();
            for t in &p.tables {
                let kind: Arc<KindDescription> =
                    Arc::clone(kinds.get(&t.kind).ok_or_else(|| {
                        ProviderError::new(
                            Status::GUATIAO_ERR_NOT_FOUND,
                            &format!("the peer serves `{}` without describing it", t.kind),
                        )
                    })?);
                if kind.object() {
                    continue;
                }
                let table = ServedTable::new(
                    kind,
                    |s| t.slots.iter().any(|n| n == s.name()),
                    Arc::clone(&serve),
                    &Libffi,
                )
                .map_err(|e| {
                    ProviderError::new(
                        Status::GUATIAO_ERR_BAD_VALUE,
                        &format!("`{}` cannot be served as `{}`: {e}", p.id, t.kind),
                    )
                })?;
                builder = builder.kinds(&[t.kind.as_str()]).table(table.kind_table());
                tables.push(table);
            }
            let document = kinds
                .document(
                    p.tables
                        .iter()
                        .map(|t| t.kind.as_str())
                        .filter(|k| kinds.get(k).is_some_and(|d| !d.object())),
                )
                .map_err(|e| ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, &e.to_string()))?;
            // Any non-null ctx: a provider's tables share one, and these
            // find their peer table by kind.
            builder = builder
                .ctx(core::ptr::without_provenance_mut(1))
                .kind_descriptions(document);
            for table in tables {
                builder = builder.keep(table);
            }
            providers.push(builder.build());
        }
        Ok(LibraryParts::new(&named(peer, &entry.id), &entry.version, providers).into())
    }

    fn close(&self, id: &str, stamp: &str) {
        {
            let mut opened = lock(&self.opened);
            let key = (id.to_string(), stamp.to_string());
            if let Some(n) = opened.get_mut(&key) {
                *n -= 1;
                if *n == 0 {
                    opened.remove(&key);
                }
            }
        }
        let Some((theirs, generation)) = unlisted_stamp(stamp) else {
            return;
        };
        let current = lock(&self.current);
        if let Some(c) = current.as_ref()
            && c.generation == generation
            && c.link.alive()
        {
            c.link.close(id, theirs);
        }
    }

    fn disconnect(&self) {
        let ended = {
            let mut current = lock(&self.current);
            self.disconnects.fetch_add(1, Ordering::AcqRel);
            current.take()
        };
        if let Some(c) = ended {
            c.link.end();
        }
    }
}

/// What listing one peer answered.
type Listed = Result<Vec<Candidate>, ProviderError>;

/// Lists every peer, at most [`LISTED_AT_ONCE`] at a time, each answer in
/// its peer's place.
fn list_each(peers: &[Arc<Peer>]) -> Vec<Listed> {
    if peers.len() < 2 {
        return peers.iter().map(|p| p.list()).collect();
    }
    let next = AtomicUsize::new(0);
    let answers: Vec<Mutex<Option<Listed>>> = peers.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|s| {
        for _ in 0..peers.len().min(LISTED_AT_ONCE) {
            let work = || {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(peer) = peers.get(i) else {
                        break;
                    };
                    let answer = peer.list();
                    *lock(&answers[i]) = Some(answer);
                }
            };
            // A thread that will not start leaves its share to the others,
            // or to the loop below.
            if std::thread::Builder::new().spawn_scoped(s, work).is_err() {
                break;
            }
        }
    });
    answers
        .into_iter()
        .zip(peers)
        .map(|(answer, peer)| {
            answer
                .into_inner()
                .unwrap_or_else(PoisonError::into_inner)
                .unwrap_or_else(|| peer.list())
        })
        .collect()
}

impl Described {
    /// Every peer the manifests describe now, listed; a manifest that
    /// describes none, and a peer that cannot be listed, as a candidate
    /// whose open reports it.
    fn list(&self) -> Vec<Candidate> {
        let found = manifest::find(&config::PEER_MANIFESTS, &self.dirs, &self.files);
        let (mut old, leaving) = {
            let state = lock(&self.state);
            (state.named.clone(), state.leaving.clone())
        };
        let mut now = Listing {
            leaving,
            ..Listing::default()
        };
        let mut listed = Vec::new();
        let mut described: Vec<(String, String, Arc<Peer>, String)> = Vec::new();
        for one in found {
            let m = match one {
                Found::Broken { path, stamp, why } => {
                    let id = path.display().to_string();
                    now.refuse(&mut listed, id, &stamp, Status::GUATIAO_ERR_BAD_VALUE, why);
                    continue;
                }
                Found::Library(m) => m,
                _ => continue,
            };
            let at = m.path().display().to_string();
            let name = m.id();
            let stamp_now = || {
                format!(
                    "{}:{}",
                    m.stamp(),
                    config::key_stamp(&self.defaults, m.table(), m.path())
                )
            };
            let mut stamp = stamp_now();
            if name.contains('/') {
                let why = format!(
                    "{at}: `name` holds a `/`, which parts a peer's name from its libraries' ids"
                );
                now.refuse(&mut listed, at, &stamp, Status::GUATIAO_ERR_BAD_VALUE, why);
                continue;
            }
            if let Some((first, ..)) = described.iter().find(|(_, n, ..)| n == name) {
                let why = format!("{at}: a peer named `{name}` is already described by {first}");
                now.refuse(&mut listed, at, &stamp, Status::GUATIAO_ERR_BAD_VALUE, why);
                continue;
            }
            let peer = match old.remove(name) {
                Some((peer, was)) if was == stamp => peer,
                was => {
                    if let Some((peer, _)) = was {
                        // It keeps its link while the registry holds a
                        // library it opened.
                        now.leaving.push((name.to_string(), peer));
                    }
                    match config::peer_of(&self.defaults, m.table(), m.path()) {
                        Ok((connect, options, merged)) => {
                            // Making it may have made its identity file:
                            // stamped as it is now, the next refresh finds
                            // the same peer.
                            stamp = stamp_now();
                            Arc::new(
                                Peer::new(connect, options, Arc::clone(&self.generations))
                                    .configured(&merged),
                            )
                        }
                        Err(e) => {
                            let why = format!("{at}: {}", e.message());
                            now.refuse(&mut listed, at, &stamp, e.status, why);
                            continue;
                        }
                    }
                }
            };
            now.named
                .insert(name.to_string(), (Arc::clone(&peer), stamp.clone()));
            described.push((at, name.to_string(), peer, stamp));
        }
        for (_, (peer, _)) in old {
            peer.disconnect();
        }
        let peers: Vec<Arc<Peer>> = described.iter().map(|(_, _, p, _)| Arc::clone(p)).collect();
        let answers = list_each(&peers);
        for ((at, name, peer, stamp), answer) in described.into_iter().zip(answers) {
            let (served, failed) = match answer {
                Ok(served) => (served, None),
                Err(e) => {
                    let mut before = peer.listed_before();
                    if !peer.ever_listed() {
                        // A new peer never listed: the one it replaces
                        // stays listed as it was.
                        let mut leaving = now.leaving.iter().rev();
                        if let Some((_, was)) = leaving.find(|(n, p)| *n == name && p.holds()) {
                            before = was.listed_before();
                        }
                    }
                    (before, Some(e))
                }
            };
            listed.extend(served.into_iter().map(|c| {
                Candidate::new(
                    named(Some(&name), c.id()),
                    c.version(),
                    c.kinds().to_vec(),
                    c.stamp(),
                )
            }));
            if let Some(e) = failed {
                // Once per outage: an error's words vary from one try to
                // the next.
                let stamp = format!("unlisted:{stamp}");
                let why = format!("{at}: the peer `{name}`: {}", e.message());
                now.refuse(&mut listed, at, &stamp, e.status, why);
            }
        }
        let gone = now.let_go();
        *lock(&self.state) = now;
        gone.iter().for_each(|p| p.disconnect());
        listed
    }

    fn open(&self, id: &str) -> Result<LocalLibrary<'static>, ProviderError> {
        let peer = {
            let state = lock(&self.state);
            if let Some((status, why)) = state.refusals.get(id) {
                return Err(ProviderError::new(*status, why));
            }
            id.split_once('/')
                .and_then(|(name, library)| {
                    let (peer, _) = state.named.get(name)?;
                    // What a replaced peer still lists, while its successor
                    // cannot be listed, it opens.
                    let peer = if peer.lists(library) {
                        peer
                    } else {
                        state
                            .leaving
                            .iter()
                            .rev()
                            .find(|(n, p)| n == name && p.lists(library))
                            .map_or(peer, |(_, p)| p)
                    };
                    Some((Arc::clone(peer), name.to_string(), library.to_string()))
                })
                .ok_or_else(|| {
                    ProviderError::new(Status::GUATIAO_ERR_NOT_FOUND, "no peer lists it")
                })?
        };
        let (peer, name, library) = peer;
        peer.open(&library, Some(&name))
    }

    fn close(&self, id: &str, stamp: &str) {
        let Some((name, library)) = id.split_once('/') else {
            return;
        };
        let peers: Vec<Arc<Peer>> = {
            let state = lock(&self.state);
            let leaving = state.leaving.iter().filter(|(n, _)| n == name);
            let named = state.named.get(name).map(|(peer, _)| peer);
            named
                .into_iter()
                .chain(leaving.map(|(_, p)| p))
                .cloned()
                .collect()
        };
        // Closing writes to the link: not under the state's lock.
        peers.iter().for_each(|p| p.close(library, stamp));
        let gone = lock(&self.state).let_go();
        gone.iter().for_each(|p| p.disconnect());
    }

    fn peers(&self) -> Vec<Arc<Peer>> {
        let state = lock(&self.state);
        state
            .named
            .values()
            .map(|(peer, _)| peer)
            .chain(state.leaving.iter().map(|(_, peer)| peer))
            .cloned()
            .collect()
    }
}

impl RemoteSource {
    /// A source of the providers the peer `connect` reaches serves. Nothing
    /// connects until the first refresh.
    pub fn new(connect: Box<dyn Connect>, options: LinkOptions) -> RemoteSource {
        RemoteSource {
            inner: Arc::new(Peers::One(Peer::new(
                connect,
                options,
                Arc::new(AtomicU64::new(0)),
            ))),
        }
    }

    /// The source a configuration describes, as [`config`]
    /// reads it: one peer by `connect` or `child`, or the peers the
    /// manifests in `dirs` and `manifests` describe, read at every refresh.
    /// Nothing connects until the first refresh.
    pub fn from_config(config: &Map) -> Result<RemoteSource, ProviderError> {
        if !config.contains_key("dirs") && !config.contains_key("manifests") {
            let (connect, options) = config::client_of(config)?;
            return Ok(RemoteSource {
                inner: Arc::new(Peers::One(
                    Peer::new(connect, options, Arc::new(AtomicU64::new(0))).configured(config),
                )),
            });
        }
        if config.contains_key("connect") || config.contains_key("child") {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_BAD_VALUE,
                "the configuration names one peer (`connect` or `child`) and manifests of peers (`dirs`, `manifests`): one or the other",
            ));
        }
        config::check_defaults(config)?;
        Ok(RemoteSource {
            inner: Arc::new(Peers::Described(Described {
                dirs: config::paths(config, "dirs")?,
                files: config::paths(config, "manifests")?,
                defaults: config.clone(),
                generations: Arc::new(AtomicU64::new(0)),
                state: Mutex::new(Listing::default()),
            })),
        })
    }

    /// Connects now, unless connected: the link a refresh would use. A
    /// source of described peers has a link for each, and answers
    /// `GUATIAO_ERR_NOT_FOUND`.
    pub fn link(&self) -> Result<Arc<Link>, ProviderError> {
        match &*self.inner {
            Peers::One(peer) => peer.link(),
            Peers::Described(_) => Err(ProviderError::new(
                Status::GUATIAO_ERR_NOT_FOUND,
                "this source reaches every peer its manifests describe, each by a link of its own",
            )),
        }
    }

    /// Handles on the peers' objects that are still out: for described
    /// peers, those of the peers the manifests still describe and of
    /// replaced peers whose libraries the registry still holds.
    pub fn objects_out(&self) -> usize {
        match &*self.inner {
            Peers::One(peer) => peer.proxies.live(),
            Peers::Described(d) => d.peers().iter().map(|p| p.proxies.live()).sum(),
        }
    }

    /// Every setting looser than the modern policy (TLS 1.3 only) in the
    /// TLS policy a configuration or a peer manifest names, by peer: its
    /// name, or empty for the one peer of a `connect` or `child`
    /// configuration. What a host shows or logs, as
    /// `guatiao-remote-server` prints its own. Described peers are those
    /// the last refresh found. Empty when every peer is held to the modern
    /// policy, and for a source made by [`RemoteSource::new`], whose
    /// `TlsConfig::weakened` its host holds already.
    #[cfg(feature = "tls")]
    pub fn weakened(&self) -> Vec<(String, crate::transport::tls::Weakening)> {
        let each = |name: &str, peer: &Peer| {
            peer.weakened
                .iter()
                .map(|w| (name.to_string(), w.clone()))
                .collect::<Vec<_>>()
        };
        match &*self.inner {
            Peers::One(peer) => each("", peer),
            Peers::Described(d) => lock(&d.state)
                .named
                .iter()
                .flat_map(|(name, (peer, _))| each(name, peer))
                .collect(),
        }
    }

    /// Drops every link: calls in flight answer `GONE`; a refresh connects
    /// again.
    pub fn disconnect(&self) {
        match &*self.inner {
            Peers::One(peer) => peer.disconnect(),
            Peers::Described(d) => d.peers().iter().for_each(|p| p.disconnect()),
        }
    }
}

impl Source for RemoteSource {
    fn list(&self, _host: Host) -> Result<Vec<Candidate>, ProviderError> {
        match &*self.inner {
            Peers::One(peer) => peer.list(),
            Peers::Described(d) => Ok(d.list()),
        }
    }

    fn open(&self, _host: Host, id: &str) -> Result<LocalLibrary<'static>, ProviderError> {
        match &*self.inner {
            Peers::One(peer) => peer.open(id, None),
            Peers::Described(d) => d.open(id),
        }
    }

    fn close(&self, id: &str, stamp: &str) {
        match &*self.inner {
            Peers::One(peer) => peer.close(id, stamp),
            Peers::Described(d) => d.close(id, stamp),
        }
    }
}
