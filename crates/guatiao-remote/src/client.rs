// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The host's side: a peer's providers as a guatiao source.
//!
//! [`RemoteSource`] lists what the peer's catalog lists and opens each
//! library as ordinary providers whose tables call over the link, built at
//! run time from the kinds the catalog describes. A refresh that finds the
//! link gone connects again (a child process is started again), and every
//! library of the old link is listed under a new stamp, so the registry
//! retires the old and opens the new.

#![forbid(unsafe_code)]

use alloc::collections::BTreeMap;
use alloc::sync::{Arc, Weak};
use std::sync::{Mutex, PoisonError};

use guatiao::Status;
use guatiao::library::kind::{
    Answer, Arg, KindDescription, LibraryParts, ProviderParts, Serve, ServedTable,
};
use guatiao::library::{Candidate, Host, LocalLibrary, ProviderError, Source};
use guatiao_bridge::Kinds;
use guatiao_libffi::Libffi;

use crate::link::{Inner, Link, LinkOptions, Nothing, Proxies, Setup};
use crate::protocol::Target;
use crate::protocol::catalog::{Catalog, LibraryEntry};
use crate::transport::Connect;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One link and what its catalog listed.
struct Current {
    link: Arc<Link>,
    generation: u64,
    libraries: BTreeMap<String, LibraryEntry>,
}

/// A peer's providers, served to a host's registry. A clone is the same
/// source: keep one to reach the link after adding the other.
#[derive(Clone)]
pub struct RemoteSource {
    inner: Arc<SourceInner>,
}

struct SourceInner {
    connect: Box<dyn Connect>,
    options: LinkOptions,
    proxies: Proxies,
    current: Mutex<Option<Current>>,
    generation: Mutex<u64>,
}

impl core::fmt::Debug for RemoteSource {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RemoteSource")
            .field("peer", &self.inner.connect.describe())
            .field("options", &self.inner.options)
            .finish()
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

impl RemoteSource {
    /// A source of the providers the peer `connect` reaches serves. Nothing
    /// connects until the first refresh.
    pub fn new(connect: Box<dyn Connect>, options: LinkOptions) -> RemoteSource {
        RemoteSource {
            inner: Arc::new(SourceInner {
                connect,
                options,
                proxies: Proxies::new(),
                current: Mutex::new(None),
                generation: Mutex::new(0),
            }),
        }
    }

    /// Connects now, unless connected: the link a refresh would use.
    pub fn link(&self) -> Result<Arc<Link>, ProviderError> {
        let this = &self.inner;
        let mut current = lock(&this.current);
        if let Some(c) = current.as_ref()
            && c.link.alive()
        {
            return Ok(Arc::clone(&c.link));
        }
        let conn = this.connect.connect().map_err(|e| {
            ProviderError::new(
                Status::GUATIAO_ERR_GONE,
                &format!("{} could not be reached: {e}", this.connect.describe()),
            )
        })?;
        let link = Link::client(
            conn,
            &this.options,
            Setup {
                handler: Arc::new(Nothing),
                proxies: this.proxies.clone(),
                kinds: Arc::new(Kinds::new()),
                name: "guatiao-remote".to_string(),
            },
        )?;
        let generation = {
            let mut g = lock(&this.generation);
            *g += 1;
            *g
        };
        let link = Arc::new(link);
        *current = Some(Current {
            link: Arc::clone(&link),
            generation,
            libraries: BTreeMap::new(),
        });
        Ok(link)
    }

    /// Handles on the peer's objects that are still out.
    pub fn objects_out(&self) -> usize {
        self.inner.proxies.live()
    }

    /// Drops the link: calls in flight answer `GONE`; a refresh connects
    /// again.
    pub fn disconnect(&self) {
        if let Some(c) = lock(&self.inner.current).take() {
            c.link.end();
        }
    }
}

impl Source for RemoteSource {
    fn list(&self, _host: Host) -> Result<Vec<Candidate>, ProviderError> {
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
        let mut current = lock(&self.inner.current);
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
        Ok(catalog
            .libraries
            .iter()
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
                    listed_stamp(&l.stamp, c.generation),
                )
            })
            .collect())
    }

    fn open(&self, _host: Host, id: &str) -> Result<LocalLibrary<'static>, ProviderError> {
        let (link, entry) = {
            let current = lock(&self.inner.current);
            let c = current.as_ref().ok_or_else(|| {
                ProviderError::new(Status::GUATIAO_ERR_GONE, "not connected: refresh first")
            })?;
            let entry = c.libraries.get(id).cloned().ok_or_else(|| {
                ProviderError::new(Status::GUATIAO_ERR_NOT_FOUND, "the peer no longer lists it")
            })?;
            (Arc::clone(&c.link), entry)
        };
        link.open(&entry.id, &entry.stamp)?;
        let kinds = link.kinds();
        let mut providers = Vec::new();
        for p in &entry.providers {
            let serve: Arc<dyn Serve> = Arc::new(Tables {
                link: Arc::downgrade(link.inner()),
                ids: p.tables.iter().map(|t| (t.kind.clone(), t.table)).collect(),
            });
            let mut builder = ProviderParts::builder(&p.id).name(&p.name);
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
            // Any non-null ctx: a provider's tables share one, and these
            // find their peer table by kind.
            builder = builder.ctx(core::ptr::without_provenance_mut(1));
            for table in tables {
                builder = builder.keep(table);
            }
            providers.push(builder.build());
        }
        Ok(LibraryParts::new(&entry.id, &entry.version, providers).into())
    }

    fn close(&self, id: &str, stamp: &str) {
        let Some((theirs, generation)) = unlisted_stamp(stamp) else {
            return;
        };
        let current = lock(&self.inner.current);
        if let Some(c) = current.as_ref()
            && c.generation == generation
            && c.link.alive()
        {
            c.link.close(id, theirs);
        }
    }
}
