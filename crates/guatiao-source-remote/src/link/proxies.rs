// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Objects the peer holds, as handles this side calls: one table per
//! object kind, every slot a call over the link that gave the object.
//!
//! The tables outlive every handle built from them: a handle is plain
//! memory that names its table, and may be held by code that knows
//! nothing of links. So they are shared by every link of a source or a
//! server, and when their owner goes while a handle is still out, they
//! are kept for the life of the process rather than freed under it.

use alloc::collections::BTreeMap;
use alloc::sync::{Arc, Weak};
use std::sync::{Mutex, PoisonError};

use guatiao::Status;
use guatiao::library::ProviderError;
use guatiao::library::kind::{
    Answer, Arg, KindDescription, ObjectRaw, RawObject, Serve, ServedTable,
};
use guatiao_libffi::Libffi;

use super::Inner;
use crate::lock;
use crate::protocol::Target;

/// One handle on a peer's object.
struct Record {
    link: Weak<Inner>,
    remote: u64,
    /// A lent object's handle: dropping it ends nothing on the peer.
    lent: bool,
}

#[derive(Default)]
struct State {
    /// By kind name and floor hash.
    tables: BTreeMap<(String, u32), Arc<ServedTable>>,
    records: BTreeMap<usize, Record>,
    next: usize,
}

pub(crate) struct ProxiesInner {
    state: Mutex<State>,
}

impl Drop for ProxiesInner {
    fn drop(&mut self) {
        let state = self.state.get_mut().unwrap_or_else(PoisonError::into_inner);
        if !state.records.is_empty() {
            // A handle still names these tables: keep them.
            for (_, table) in core::mem::take(&mut state.tables) {
                core::mem::forget(table);
            }
        }
    }
}

/// Handles on the peer's objects, for every link that shares them.
#[derive(Clone)]
pub(crate) struct Proxies {
    inner: Arc<ProxiesInner>,
}

impl core::fmt::Debug for Proxies {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Proxies")
            .field("live", &lock(&self.inner.state).records.len())
            .finish()
    }
}

impl Default for Proxies {
    fn default() -> Proxies {
        Proxies::new()
    }
}

struct ProxyServe {
    proxies: Weak<ProxiesInner>,
}

fn gone(why: &str) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_GONE, why)
}

impl Serve for ProxyServe {
    fn call(
        &self,
        ctx: usize,
        kind: &KindDescription,
        slot: usize,
        args: Vec<Arg<'_>>,
    ) -> Result<Answer, ProviderError> {
        let proxies = self
            .proxies
            .upgrade()
            .ok_or_else(|| gone("the link this object came through has ended"))?;
        let (link, remote) = {
            let state = lock(&proxies.state);
            let record = state
                .records
                .get(&ctx)
                .ok_or_else(|| gone("the object has been let go"))?;
            (record.link.upgrade(), record.remote)
        };
        let link = link.ok_or_else(|| gone("the link this object came through has ended"))?;
        link.call(Target::Object(remote), kind, slot, args)
    }

    fn destroy(&self, ctx: usize) {
        let Some(proxies) = self.proxies.upgrade() else {
            return;
        };
        let record = lock(&proxies.state).records.remove(&ctx);
        if let Some(record) = record
            && !record.lent
            && let Some(link) = record.link.upgrade()
        {
            link.release(record.remote);
        }
    }
}

impl Proxies {
    /// None yet.
    pub(crate) fn new() -> Proxies {
        Proxies {
            inner: Arc::new(ProxiesInner {
                state: Mutex::new(State::default()),
            }),
        }
    }

    /// How many handles are out.
    pub(crate) fn live(&self) -> usize {
        lock(&self.inner.state).records.len()
    }

    fn table(&self, kind: &Arc<KindDescription>) -> Result<Arc<ServedTable>, ProviderError> {
        let key = (kind.name().to_string(), kind.floor_hash());
        let mut state = lock(&self.inner.state);
        if let Some(t) = state.tables.get(&key) {
            return Ok(Arc::clone(t));
        }
        let serve: Arc<dyn Serve> = Arc::new(ProxyServe {
            proxies: Arc::downgrade(&self.inner),
        });
        let table = Arc::new(
            ServedTable::new(Arc::clone(kind), |_| true, serve, &Libffi).map_err(|e| {
                ProviderError::new(
                    Status::GUATIAO_ERR_UNSUPPORTED,
                    &format!("no table for `{}`: {e}", kind.name()),
                )
            })?,
        );
        state.tables.insert(key, Arc::clone(&table));
        Ok(table)
    }

    /// A handle on object `remote` of kind `kind`, which `link`'s peer
    /// holds. Dropping it releases the object, unless `lent`.
    pub(crate) fn object(
        &self,
        kind: &Arc<KindDescription>,
        link: &Arc<Inner>,
        remote: u64,
        lent: bool,
    ) -> Result<RawObject, ProviderError> {
        let table = self.table(kind)?;
        let ctx = {
            let mut state = lock(&self.inner.state);
            state.next += 1;
            let ctx = state.next;
            state.records.insert(
                ctx,
                Record {
                    link: Arc::downgrade(link),
                    remote,
                    lent,
                },
            );
            ctx
        };
        table.object(ctx).ok_or_else(|| {
            lock(&self.inner.state).records.remove(&ctx);
            ProviderError::new(
                Status::GUATIAO_ERR_WRONG_KIND,
                &format!("`{}` is not an object kind", kind.name()),
            )
        })
    }

    /// The peer's id for `raw` when it is a handle of this set on an
    /// object `link`'s peer holds: the object can go back as itself.
    pub(crate) fn remote_of(&self, raw: &ObjectRaw, link: &Arc<Inner>) -> Option<u64> {
        let state = lock(&self.inner.state);
        let ours = state
            .tables
            .values()
            .any(|t| core::ptr::eq(t.as_ptr(), raw.table));
        if !ours {
            return None;
        }
        let record = state.records.get(&raw.ctx.addr())?;
        let same = record.link.upgrade().is_some_and(|l| Arc::ptr_eq(&l, link));
        same.then_some(record.remote)
    }

    /// Lets a handle go without releasing its object: the object went
    /// back to its holder as itself.
    pub(crate) fn forget(&self, raw: RawObject) {
        let raw = raw.into_raw();
        lock(&self.inner.state).records.remove(&raw.ctx.addr());
    }
}
