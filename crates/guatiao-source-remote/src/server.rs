// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The device's side: a registry's providers, served over links.
//!
//! A [`Server`] owns a [`Registry`] and the descriptions of the kinds it
//! serves: those it is given, and those each provider carries of its own.
//! A link lists the catalog, opens a library (which holds its providers
//! for that link until it closes the library or the link ends), and calls
//! their tables through the descriptions. A provider of a kind with no
//! description is not served.

#![forbid(unsafe_code)]

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use guatiao::library::kind::{
    Answer, Arg, Caller, DescribedRemote, KindDescription, SlotDescription,
};
use guatiao::library::{Provider, ProviderError, Refreshed, Registry};
use guatiao::value::convert::{ToValue, TryAsRef};
use guatiao::{Alloc, List, Map, Status, Value};
use guatiao_bridge::Kinds;
use guatiao_libffi::Libffi;

use crate::link::{Handler, Inner, Link, LinkError, LinkOptions, Proxies, Setup, Table};
use crate::protocol::catalog::{Catalog, LibraryEntry, ProviderEntry, TableEntry};
use crate::transport::Connection;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The kinds a server describes to its links: each as it was given, for
/// the catalog, and read.
#[derive(Debug, Default, Clone)]
pub struct Described {
    kinds: Kinds,
    values: Vec<Value>,
}

impl Described {
    /// None yet.
    pub fn new() -> Described {
        Described::default()
    }

    /// Adds every kind a value describes (a document or one kind). The
    /// catalog lists each kind on its own, once.
    pub fn add_value(&mut self, value: &Value) -> Result<usize, ProviderError> {
        let n = self.kinds.add_value(value).map_err(ProviderError::from)?;
        let kinds = TryAsRef::<Map>::try_as_ref(value)
            .filter(|m| m.get("format").is_some())
            .and_then(|m| m.get("kinds"))
            .and_then(TryAsRef::<List>::try_as_ref);
        let one = [value.clone()];
        let each: Vec<Value> = match kinds {
            Some(list) => list.iter().cloned().collect(),
            None => one.to_vec(),
        };
        for kind in each {
            let name = TryAsRef::<Map>::try_as_ref(&kind)
                .and_then(|m| m.get("name"))
                .and_then(TryAsRef::<str>::try_as_ref)
                .map(str::to_string);
            let known = self.values.iter().any(|v| {
                TryAsRef::<Map>::try_as_ref(v)
                    .and_then(|m| m.get("name"))
                    .and_then(TryAsRef::<str>::try_as_ref)
                    == name.as_deref()
            });
            if !known {
                self.values.push(kind);
            }
        }
        Ok(n)
    }

    /// Adds every kind JSON text describes: a `<crate>.kinds.json`
    /// document, or `<Trait>Vtable::description()`.
    pub fn add_json(&mut self, json: &str) -> Result<usize, ProviderError> {
        let value = guatiao_serde::text::json::from_str(
            json,
            Alloc::rust(),
            guatiao_serde::Presentation::new(),
        )
        .map_err(|e| {
            ProviderError::new(
                Status::GUATIAO_ERR_BAD_VALUE,
                &format!("a kind description is not JSON: {e}"),
            )
        })?;
        self.add_value(&value)
    }

    /// Adds every kind a description document on disk holds.
    pub fn load(&mut self, path: &Path) -> Result<usize, ProviderError> {
        let text = std::fs::read_to_string(path).map_err(|e| {
            ProviderError::new(
                Status::GUATIAO_ERR_NOT_FOUND,
                &format!("{}: {e}", path.display()),
            )
        })?;
        self.add_json(&text)
    }

    /// The kinds, read.
    pub fn kinds(&self) -> &Kinds {
        &self.kinds
    }
}

/// A library's tables held for a link, by id.
type HeldTables = Vec<(u64, Arc<dyn Table>)>;

/// Whether two slots are called alike: their docs aside.
fn same_call(a: &SlotDescription, b: &SlotDescription) -> bool {
    a.name() == b.name()
        && a.required() == b.required()
        && a.fallible() == b.fallible()
        && a.args() == b.args()
        && a.answer() == b.answer()
}

/// Whether a table built for one description is called rightly through
/// the other: the same name, floor and slots as far as the shorter goes.
/// Docs do not count, and a kind grown by slots past its floor agrees.
fn agree(a: &KindDescription, b: &KindDescription) -> bool {
    a.name() == b.name()
        && a.object() == b.object()
        && a.floor_hash() == b.floor_hash()
        && a.slots()
            .iter()
            .zip(b.slots())
            .all(|(x, y)| same_call(x, y))
}

/// The kinds a server serves: those it was given, then those providers'
/// own documents add as each is first served. A name keeps the
/// description it became known by for the server's life.
struct Known {
    kinds: Arc<Kinds>,
    /// Each kind as the catalog lists it, in the order it became known.
    values: Vec<Value>,
}

impl Known {
    fn new(described: Described) -> Known {
        Known {
            kinds: Arc::new(described.kinds),
            values: described.values,
        }
    }

    /// Adds the kinds `own` describes that are not known yet. False, adding
    /// nothing, when one disagrees with a kind known by that name.
    fn learn(&mut self, own: &[KindDescription]) -> bool {
        let mut fresh: Vec<(&KindDescription, Value)> = Vec::new();
        for kind in own {
            let known = self.kinds.get(kind.name()).map(|k| &**k);
            let seen = fresh
                .iter()
                .map(|(k, _)| *k)
                .find(|k| k.name() == kind.name());
            match known.or(seen) {
                Some(known) if agree(known, kind) => {}
                Some(_) => return false,
                None => match kind.to_value() {
                    Ok(value) => fresh.push((kind, value)),
                    Err(_) => return false,
                },
            }
        }
        if fresh.is_empty() {
            return true;
        }
        let mut kinds = (*self.kinds).clone();
        for (kind, value) in fresh {
            if kinds.add(kind.clone()).is_err() {
                return false;
            }
            self.values.push(value);
        }
        self.kinds = Arc::new(kinds);
        true
    }
}

/// One provider table, held open for a link.
struct Held {
    remote: DescribedRemote,
}

impl Table for Held {
    fn kind(&self) -> &Arc<KindDescription> {
        self.remote.kind()
    }

    fn call(&self, slot: usize, args: Vec<Arg<'_>>) -> Result<Answer, ProviderError> {
        self.remote.call(slot, args)
    }
}

/// What one link has opened.
#[derive(Default)]
struct Opened {
    /// By library id and stamp: the table ids it holds.
    libraries: BTreeMap<(String, String), Vec<u64>>,
    tables: BTreeMap<u64, Arc<dyn Table>>,
}

/// A table's place in the catalog.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct TableKey {
    library: String,
    stamp: String,
    provider: String,
    kind: String,
}

struct ServerInner {
    registry: Mutex<Registry>,
    known: Mutex<Known>,
    callers: Mutex<BTreeMap<String, Arc<Caller>>>,
    proxies: Proxies,
    /// Per library key, how many times it has loaded while served: part
    /// of its stamp.
    generations: Mutex<BTreeMap<String, u64>>,
    ids: Mutex<(BTreeMap<TableKey, u64>, u64)>,
    links: Mutex<BTreeMap<usize, Opened>>,
    /// Every link running, kept until it ends.
    live: Mutex<BTreeMap<usize, Arc<Link>>>,
}

/// A registry's providers, served over links.
#[derive(Clone)]
pub struct Server {
    inner: Arc<ServerInner>,
    options: LinkOptions,
}

impl core::fmt::Debug for Server {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Server")
            .field("options", &self.options)
            .field("links", &lock(&self.inner.links).len())
            .finish()
    }
}

fn link_key(link: &Arc<Inner>) -> usize {
    Arc::as_ptr(link).addr()
}

impl ServerInner {
    fn caller(&self, kind: &Arc<KindDescription>) -> Result<Arc<Caller>, ProviderError> {
        let mut callers = lock(&self.callers);
        if let Some(c) = callers.get(kind.name()) {
            return Ok(Arc::clone(c));
        }
        let c =
            Arc::new(Caller::new(Arc::clone(kind), &Libffi).map_err(|e| {
                ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, &e.to_string())
            })?);
        callers.insert(kind.name().to_string(), Arc::clone(&c));
        Ok(c)
    }

    fn table_id(&self, key: TableKey) -> u64 {
        let mut ids = lock(&self.ids);
        if let Some(id) = ids.0.get(&key) {
            return *id;
        }
        ids.1 += 1;
        let id = ids.1;
        ids.0.insert(key, id);
        id
    }

    fn stamp(&self, key: &str, version: &str) -> String {
        let generation = lock(&self.generations).get(key).copied().unwrap_or(0);
        format!("{key}#{version}#{generation}")
    }

    /// The kinds `p` is served through, after learning those its own
    /// document adds; `None` leaves it out. A document that cannot be read
    /// counts as none.
    fn kinds_for(&self, p: &Provider) -> Option<Arc<Kinds>> {
        let own = p.kind_descriptions().unwrap_or_default();
        let mut known = lock(&self.known);
        known.learn(&own).then(|| Arc::clone(&known.kinds))
    }

    /// Every kind known now.
    fn kinds(&self) -> Arc<Kinds> {
        Arc::clone(&lock(&self.known).kinds)
    }

    /// Gives `link` every kind known now. Under the lock, so a link's
    /// kinds only grow, whichever of its requests finishes last.
    fn give_kinds(&self, link: &Arc<Inner>) {
        let known = lock(&self.known);
        link.set_kinds(Arc::clone(&known.kinds));
    }

    fn catalog(&self) -> Result<Catalog, ProviderError> {
        let registry = lock(&self.registry);
        // Every provider's document is read first, so a provider describing
        // nothing is served through another's, whichever comes first.
        let served: Vec<bool> = registry
            .all()
            .iter()
            .map(|p| self.kinds_for(p).is_some())
            .collect();
        let kinds = self.kinds();
        let mut libraries = Vec::new();
        for loaded in registry.loaded() {
            let key: &str = &loaded.key;
            let stamp = self.stamp(key, &loaded.version);
            let mut providers = Vec::new();
            let here = registry.all().iter().zip(&served);
            for (p, _) in here.filter(|(p, served)| **served && p.library() == key) {
                let mut tables = Vec::new();
                for kind_name in p.kinds() {
                    let Some(kind) = kinds.get(kind_name) else {
                        continue;
                    };
                    if kind.object() {
                        continue;
                    }
                    let Ok(remote) = DescribedRemote::new(p.view(), self.caller(kind)?) else {
                        continue;
                    };
                    let slots = kind
                        .slots()
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| remote.implements(*i))
                        .map(|(_, s)| s.name().to_string())
                        .collect();
                    tables.push(TableEntry {
                        kind: kind_name.clone(),
                        table: self.table_id(TableKey {
                            library: loaded.id.clone(),
                            stamp: stamp.clone(),
                            provider: p.id().to_string(),
                            kind: kind_name.clone(),
                        }),
                        slots,
                    });
                }
                if tables.is_empty() {
                    continue;
                }
                providers.push(ProviderEntry {
                    id: p.id().to_string(),
                    name: p.display_name().to_string(),
                    meta: p.meta().cloned(),
                    tables,
                });
            }
            if providers.is_empty() {
                continue;
            }
            libraries.push(LibraryEntry {
                id: loaded.id.clone(),
                version: loaded.version.clone(),
                stamp,
                providers,
            });
        }
        Ok(Catalog {
            kinds: lock(&self.known).values.clone(),
            libraries,
        })
    }

    fn hold(&self, library: &str, stamp: &str) -> Result<HeldTables, ProviderError> {
        let registry = lock(&self.registry);
        let loaded = registry
            .loaded()
            .iter()
            .find(|l| l.id == library && self.stamp(&l.key, &l.version) == stamp)
            .ok_or_else(|| {
                ProviderError::new(
                    Status::GUATIAO_ERR_NOT_FOUND,
                    &format!("`{library}` is not served under that stamp: refresh"),
                )
            })?;
        let key: &str = &loaded.key;
        let mut held: Vec<(u64, Arc<dyn Table>)> = Vec::new();
        for p in registry.all().iter().filter(|p| p.library() == key) {
            let Some(kinds) = self.kinds_for(p) else {
                continue;
            };
            for kind_name in p.kinds() {
                let Some(kind) = kinds.get(kind_name) else {
                    continue;
                };
                if kind.object() {
                    continue;
                }
                let Ok(remote) = DescribedRemote::new(p.view(), self.caller(kind)?) else {
                    continue;
                };
                let id = self.table_id(TableKey {
                    library: library.to_string(),
                    stamp: stamp.to_string(),
                    provider: p.id().to_string(),
                    kind: kind_name.clone(),
                });
                held.push((id, Arc::new(Held { remote })));
            }
        }
        Ok(held)
    }
}

impl Handler for ServerInner {
    fn list(&self, link: &Arc<Inner>) -> Result<Value, ProviderError> {
        let catalog = self.catalog()?;
        // The link reads the objects of every kind its catalog lists.
        self.give_kinds(link);
        Ok(catalog.to_value())
    }

    fn open(&self, link: &Arc<Inner>, library: &str, stamp: &str) -> Result<(), ProviderError> {
        let held = self.hold(library, stamp)?;
        self.give_kinds(link);
        let mut links = lock(&self.links);
        let opened = links.entry(link_key(link)).or_default();
        let key = (library.to_string(), stamp.to_string());
        if opened.libraries.contains_key(&key) {
            return Ok(());
        }
        let ids = held.iter().map(|(id, _)| *id).collect();
        for (id, table) in held {
            opened.tables.insert(id, table);
        }
        opened.libraries.insert(key, ids);
        Ok(())
    }

    fn close(&self, link: &Arc<Inner>, library: &str, stamp: &str) {
        let dropped = {
            let mut links = lock(&self.links);
            let Some(opened) = links.get_mut(&link_key(link)) else {
                return;
            };
            let ids = opened
                .libraries
                .remove(&(library.to_string(), stamp.to_string()))
                .unwrap_or_default();
            ids.into_iter()
                .filter_map(|id| opened.tables.remove(&id))
                .collect::<Vec<_>>()
        };
        drop(dropped);
    }

    fn table(&self, link: &Arc<Inner>, id: u64) -> Option<Arc<dyn Table>> {
        lock(&self.links)
            .get(&link_key(link))
            .and_then(|o| o.tables.get(&id))
            .cloned()
    }

    fn ended(&self, link: &Arc<Inner>) {
        let opened = lock(&self.links).remove(&link_key(link));
        drop(opened);
        let live = lock(&self.live).remove(&link_key(link));
        drop(live);
    }
}

/// Lets a link's handler be the server's own, shared state.
struct Handle(Arc<ServerInner>);

impl Handler for Handle {
    fn list(&self, link: &Arc<Inner>) -> Result<Value, ProviderError> {
        self.0.list(link)
    }
    fn open(&self, link: &Arc<Inner>, library: &str, stamp: &str) -> Result<(), ProviderError> {
        self.0.open(link, library, stamp)
    }
    fn close(&self, link: &Arc<Inner>, library: &str, stamp: &str) {
        self.0.close(link, library, stamp);
    }
    fn table(&self, link: &Arc<Inner>, id: u64) -> Option<Arc<dyn Table>> {
        self.0.table(link, id)
    }
    fn ended(&self, link: &Arc<Inner>) {
        self.0.ended(link);
    }
}

impl Server {
    /// Serves `registry`'s providers to links set up with `options`, each
    /// kind through the description `described` gives of it, else through
    /// the one the first provider serving it carries. A provider whose own
    /// description of a kind is called otherwise than the one served (docs
    /// and slots past the shorter aside) is left out, as is one of a kind
    /// described by neither.
    pub fn new(registry: Registry, described: Described, options: LinkOptions) -> Server {
        Server {
            inner: Arc::new(ServerInner {
                registry: Mutex::new(registry),
                known: Mutex::new(Known::new(described)),
                callers: Mutex::new(BTreeMap::new()),
                proxies: Proxies::new(),
                generations: Mutex::new(BTreeMap::new()),
                ids: Mutex::new((BTreeMap::new(), 0)),
                links: Mutex::new(BTreeMap::new()),
                live: Mutex::new(BTreeMap::new()),
            }),
            options,
        }
    }

    /// The options its links are set up with.
    pub fn options(&self) -> &LinkOptions {
        &self.options
    }

    /// Sets up a link over `conn` and serves it on its own threads. The
    /// server keeps the link until either side ends it.
    pub fn serve(&self, conn: Connection) -> Result<Arc<Link>, LinkError> {
        let link = Link::server(
            conn,
            &self.options,
            Setup {
                handler: Arc::new(Handle(Arc::clone(&self.inner))),
                proxies: self.inner.proxies.clone(),
                kinds: self.inner.kinds(),
                name: "guatiao-source-remote server".to_string(),
            },
        )?;
        let link = Arc::new(link);
        let key = link_key(link.inner());
        lock(&self.inner.live).insert(key, Arc::clone(&link));
        if !link.alive() {
            // It ended before it was kept: nothing will remove it now.
            lock(&self.inner.live).remove(&key);
        }
        Ok(link)
    }

    /// Ends every link: their calls in flight answer `GONE`.
    pub fn end_links(&self) {
        let live = core::mem::take(&mut *lock(&self.inner.live));
        for link in live.values() {
            link.end();
        }
    }

    /// How many links are running.
    pub fn links(&self) -> usize {
        lock(&self.inner.live).len()
    }

    /// Refreshes the registry's sources. A library that loads again is
    /// served under a new stamp, so a host refreshing next opens it anew.
    pub fn refresh(&self) -> Refreshed {
        let refreshed = lock(&self.inner.registry).refresh();
        let mut generations = lock(&self.inner.generations);
        for served in &refreshed.loaded {
            *generations.entry(served.key.clone()).or_insert(0) += 1;
        }
        refreshed
    }

    /// The registry, locked: load, register or retire through it.
    pub fn registry(&self) -> MutexGuard<'_, Registry> {
        lock(&self.inner.registry)
    }

    /// How many links have libraries open.
    pub fn links_holding(&self) -> usize {
        lock(&self.inner.links)
            .values()
            .filter(|o| !o.libraries.is_empty())
            .count()
    }

    /// Handles this server's providers hold on its peers' objects.
    pub fn objects_out(&self) -> usize {
        self.inner.proxies.live()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document() -> serde_json::Value {
        let text = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../examples/greeter_kind/include/greeter_kind.kinds.json"),
        )
        .unwrap();
        serde_json::from_str(&text).unwrap()
    }

    fn read(document: &serde_json::Value) -> Vec<KindDescription> {
        KindDescription::document_json(&document.to_string()).unwrap()
    }

    /// The example kinds, the greeter's doc replaced by `doc`.
    fn kinds(doc: &str) -> Vec<KindDescription> {
        let mut document = document();
        document["kinds"][0]["doc"] = doc.into();
        read(&document)
    }

    fn greeter(known: &Known) -> Option<String> {
        known
            .kinds
            .get("greeter")
            .and_then(|k| k.doc().map(str::to_string))
    }

    #[test]
    fn a_provider_calling_a_kind_otherwise_is_left_out() {
        let mut known = Known::new(Described::new());
        assert!(known.learn(&kinds("Greets by name.")));
        assert_eq!(known.values.len(), 5, "every kind listed once");
        assert!(known.learn(&kinds("Greets by name.")), "the same again");
        assert_eq!(known.values.len(), 5);

        // Called otherwise: `greet` takes bytes.
        let mut other = document();
        other["kinds"][0]["slots"][0]["args"][0]["type"] = "bytes".into();
        assert!(!known.learn(&read(&other)));
        assert_eq!(known.values.len(), 5);
    }

    #[test]
    fn docs_and_slots_past_the_shorter_do_not_count() {
        let mut known = Known::new(Described::new());
        assert!(known.learn(&kinds("Greets by name.")));
        assert!(known.learn(&kinds("Greets otherwise.")), "a doc differs");
        let mut shorter = document();
        let slots = shorter["kinds"][0]["slots"].as_array_mut().unwrap();
        let last = slots.pop().unwrap();
        assert_eq!(last["required"], false, "a slot past the floor");
        assert!(known.learn(&read(&shorter)), "a slot fewer");
        assert_eq!(greeter(&known).as_deref(), Some("Greets by name."));
    }

    #[test]
    fn a_document_naming_a_kind_twice_lists_it_once() {
        let mut twice = document();
        let first = twice["kinds"][0].clone();
        twice["kinds"].as_array_mut().unwrap().push(first);
        let mut known = Known::new(Described::new());
        assert!(known.learn(&read(&twice)));
        assert_eq!(known.values.len(), 5);
    }

    #[test]
    fn a_given_description_is_the_one_served() {
        let mut described = Described::new();
        for kind in &kinds("Greets as given.") {
            described.add_value(&kind.to_value().unwrap()).unwrap();
        }
        let mut known = Known::new(described);
        assert!(known.learn(&kinds("Greets by name.")));
        assert_eq!(greeter(&known).as_deref(), Some("Greets as given."));
        assert_eq!(known.values.len(), 5, "nothing added over the given");
        let mut other = document();
        other["kinds"][0]["slots"][0]["args"][0]["type"] = "bytes".into();
        assert!(!known.learn(&read(&other)), "called otherwise than given");
    }
}
