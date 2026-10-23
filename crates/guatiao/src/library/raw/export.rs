// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A Rust [`Source`] exported as a source file: the `guatiao_source`
//! entry [`source!`](crate::source) writes, and the table's three slots
//! over the trait.

use alloc::string::String;
use alloc::vec::Vec;
use std::sync::{Mutex, PoisonError};

use super::*;
use crate::library::desc::{ABI_VERSION, SourceInfo};
use crate::library::kind::{LibraryParts, ProviderError, return_alloc, write_err};
use crate::library::serving::{Candidate, Source};
use crate::value::convert::TryAsRef;
use crate::value::error::ValueError;
use crate::value::types::{List, Text};

/// One added source: the trait object and what its opens answered.
struct Exported<S> {
    source: S,
    /// The stamp each id was last listed under: what `close` tells the
    /// source, since the C `open` names only the id.
    listed: Mutex<Vec<(String, String)>>,
    /// Each descriptor `open` answered, by address, until its `close`.
    open: Mutex<Vec<(usize, Kept)>>,
}

/// What one open answered, kept until the registry closes it.
struct Kept {
    id: String,
    stamp: String,
    /// Parts the source built and handed over, from `Box::into_raw`, freed
    /// at `close`; null for a borrowed descriptor, which stays the source's.
    parts: *mut LibraryParts,
}

// SAFETY: `parts` is the one owner of a `LibraryParts`, which is `Send`.
unsafe impl Send for Kept {}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The entry point [`source!`](crate::source) writes: builds the source
/// with `make` from the host and the configuration map, and answers its
/// table, kept for the life of the process.
///
/// # Safety
///
/// As `guatiao_source` in `guatiao.h`: `host` is null or a host
/// descriptor valid for the process, `config` is null or a well-formed
/// value, `out` and `err` are null or writable.
#[doc(hidden)]
pub unsafe fn source_entry<S: Source + 'static>(
    host: *const HostInfo<'static>,
    config: *const Value,
    out: *mut *const SourceInfo,
    err: *mut ProviderError,
    make: impl FnOnce(Host, &Map) -> Result<S, ProviderError>,
) -> Status {
    if !out.is_null() {
        // SAFETY: non-null and writable by the caller's contract.
        unsafe { out.write(core::ptr::null()) };
    }
    if !err.is_null() {
        // SAFETY: as above; the placeholder owns nothing.
        unsafe { err.write(ProviderError::none()) };
    }
    let started = crate::exports::guard_with(
        Err(ProviderError::new(
            Status::GUATIAO_ERR_INTERNAL,
            "the source panicked while starting",
        )),
        || -> Result<*const SourceInfo, ProviderError> {
            // SAFETY: the caller's contract.
            let host = unsafe { Host::from_raw(host) }
                .ok_or_else(|| ProviderError::new(Status::GUATIAO_ERR_NULL, "no host"))?;
            let empty = Map::new();
            // SAFETY: the caller's contract.
            let config: &Map = match unsafe { config.as_ref() } {
                None => &empty,
                Some(value) => TryAsRef::<Map>::try_as_ref(value).ok_or_else(|| {
                    ProviderError::new(
                        Status::GUATIAO_ERR_WRONG_KIND,
                        "the configuration is not a map",
                    )
                })?,
            };
            let source = make(host, config)?;
            // Both kept for the life of the process, as the entry's
            // contract promises the host.
            let state = Box::into_raw(Box::new(Exported {
                source,
                listed: Mutex::new(Vec::new()),
                open: Mutex::new(Vec::new()),
            }));
            let table = Box::into_raw(Box::new(SourceInfo {
                struct_size: size_of::<SourceInfo>() as u32,
                abi_version: ABI_VERSION,
                ctx: state.cast::<c_void>(),
                list: Some(list::<S>),
                open: Some(open::<S>),
                close: Some(close::<S>),
            }));
            Ok(table.cast_const())
        },
    );
    match started {
        Ok(table) => {
            if out.is_null() {
                return Status::GUATIAO_ERR_NULL;
            }
            // SAFETY: non-null and writable, as above.
            unsafe { out.write(table) };
            Status::GUATIAO_OK
        }
        // SAFETY: `err` is null or writable by the caller's contract.
        Err(e) => unsafe { write_err(err, e) },
    }
}

/// The state behind a slot's `ctx`.
///
/// # Safety
///
/// `ctx` is null or the one `source_entry::<S>` answered.
unsafe fn state<'a, S>(ctx: *mut c_void) -> Option<&'a Exported<S>> {
    // SAFETY: the caller's contract; leaked for the process.
    unsafe { ctx.cast::<Exported<S>>().as_ref() }
}

fn candidates(found: &[Candidate], alloc: Alloc) -> Result<List, ValueError> {
    let mut list = List::new_in(alloc);
    for c in found {
        let mut map = Map::new_in(alloc);
        map.set_in("id", Text::new_in(alloc, c.id())?, alloc)?;
        map.set_in("version", Text::new_in(alloc, c.version())?, alloc)?;
        let mut kinds = List::new_in(alloc);
        for kind in c.kinds() {
            kinds.try_push(Text::new_in(alloc, kind)?)?;
        }
        map.set_in("kinds", kinds, alloc)?;
        map.set_in("stamp", Text::new_in(alloc, c.stamp())?, alloc)?;
        list.try_push(map)?;
    }
    Ok(list)
}

unsafe extern "C" fn list<S: Source>(
    ctx: *mut c_void,
    host: *const HostInfo<'static>,
    out: *mut List,
) -> Status {
    crate::exports::guard(|| {
        // SAFETY: the `ctx` this source answered.
        let Some(state) = (unsafe { state::<S>(ctx) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        // SAFETY: the registry's host block, valid for the process.
        let Some(host) = (unsafe { Host::from_raw(host) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        if out.is_null() {
            return Status::GUATIAO_ERR_NULL;
        }
        // SAFETY: the registry's out slot, holding a carrier.
        let alloc = unsafe { return_alloc(out) };
        let found = match state.source.list(host) {
            Ok(found) => found,
            Err(e) => return e.status,
        };
        let list = match candidates(&found, alloc) {
            Ok(list) => list,
            Err(e) => return e.into(),
        };
        *lock(&state.listed) = found
            .iter()
            .map(|c| (c.id().to_string(), c.stamp().to_string()))
            .collect();
        // SAFETY: non-null; the carrier owns nothing and is written over.
        unsafe { out.write(list) };
        Status::GUATIAO_OK
    })
}

unsafe extern "C" fn open<S: Source>(
    ctx: *mut c_void,
    host: *const HostInfo<'static>,
    id: Str,
    out: *mut *const LibraryInfo<'static>,
    err: *mut ProviderError,
) -> Status {
    if !out.is_null() {
        // SAFETY: non-null and writable by the registry's contract.
        unsafe { out.write(core::ptr::null()) };
    }
    crate::exports::guard(|| {
        // SAFETY: the `ctx` this source answered.
        let Some(state) = (unsafe { state::<S>(ctx) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        // SAFETY: the registry's host block, valid for the process.
        let Some(host) = (unsafe { Host::from_raw(host) }) else {
            return Status::GUATIAO_ERR_NULL;
        };
        if out.is_null() {
            return Status::GUATIAO_ERR_NULL;
        }
        let Ok(id) = core::str::from_utf8(id.into()) else {
            let e = ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, "the id is not UTF-8");
            // SAFETY: `err` is null or writable by the registry's contract.
            return unsafe { write_err(err, e) };
        };
        let local = match state.source.open(host, id) {
            Ok(local) => local,
            // SAFETY: as above.
            Err(e) => return unsafe { write_err(err, e) },
        };
        let (info, parts) = match local {
            LocalLibrary::Owned(parts) => {
                // A raw owner from here on: moving the `Box` would retag it
                // and invalidate the descriptor's pointers into it.
                let parts = Box::into_raw(parts);
                // SAFETY: the box just released, live until `close`.
                let info: *const LibraryInfo<'_> = unsafe { (*parts).info() };
                (info.cast::<LibraryInfo<'static>>(), parts)
            }
            LocalLibrary::Borrowed(info) => (
                (info as *const LibraryInfo<'_>).cast::<LibraryInfo<'static>>(),
                core::ptr::null_mut(),
            ),
        };
        let stamp = lock(&state.listed)
            .iter()
            .find(|(listed, _)| listed == id)
            .map(|(_, stamp)| stamp.clone())
            .unwrap_or_default();
        lock(&state.open).push((
            info as usize,
            Kept {
                id: id.to_string(),
                stamp,
                parts,
            },
        ));
        // SAFETY: non-null and writable; the descriptor stays valid until
        // `close`, held here or by the source.
        unsafe { out.write(info) };
        Status::GUATIAO_OK
    })
}

unsafe extern "C" fn close<S: Source>(
    ctx: *mut c_void,
    id: Str,
    info: *const LibraryInfo<'static>,
) {
    crate::exports::guard_with((), || {
        // SAFETY: the `ctx` this source answered.
        let Some(state) = (unsafe { state::<S>(ctx) }) else {
            return;
        };
        let id = core::str::from_utf8(id.into()).unwrap_or("");
        let kept = {
            let mut open = lock(&state.open);
            open.iter()
                .position(|(at, kept)| *at == info as usize && kept.id == id)
                .or_else(|| open.iter().position(|(at, _)| *at == info as usize))
                .map(|i| open.remove(i).1)
        };
        let Some(kept) = kept else {
            return;
        };
        if !kept.parts.is_null() {
            // SAFETY: from `Box::into_raw` in `open`, freed once, here,
            // after the registry let the library go.
            drop(unsafe { Box::from_raw(kept.parts) });
        }
        state.source.close(&kept.id, &kept.stamp);
    });
}

#[cfg(all(test, feature = "load"))]
mod tests {
    use alloc::sync::Arc;

    use super::*;
    use crate::library::Registry;
    use crate::library::kind::ProviderParts;

    /// Counts its own drops: kept with each library's parts.
    struct Marker(Arc<Mutex<usize>>);

    impl Drop for Marker {
        fn drop(&mut self) {
            *lock(&self.0) += 1;
        }
    }

    /// A shelf whose ids come from the configuration, counting what the
    /// registry made of them.
    struct Shelf {
        ids: Vec<String>,
        stamp: Mutex<String>,
        refuse: Mutex<bool>,
        closed: Arc<Mutex<Vec<(String, String)>>>,
        dropped: Arc<Mutex<usize>>,
    }

    impl Source for Shelf {
        fn list(&self, _host: Host) -> Result<Vec<Candidate>, ProviderError> {
            let stamp = lock(&self.stamp).clone();
            if stamp == "panic" {
                panic!("asked to");
            }
            Ok(self
                .ids
                .iter()
                .map(|id| {
                    Candidate::new(id.as_str(), "1.0.0", vec!["thing".into()], stamp.as_str())
                })
                .collect())
        }

        fn open(&self, _host: Host, id: &str) -> Result<LocalLibrary<'static>, ProviderError> {
            if *lock(&self.refuse) {
                return Err(ProviderError::new(
                    Status::GUATIAO_ERR_BAD_VALUE,
                    "not today",
                ));
            }
            let provider = ProviderParts::builder(&format!("{id}_p"))
                .kinds(&["thing"])
                .keep(Marker(Arc::clone(&self.dropped)))
                .build();
            Ok(LibraryParts::new(id, "1.0.0", vec![provider]).into())
        }

        fn close(&self, id: &str, stamp: &str) {
            lock(&self.closed).push((id.to_string(), stamp.to_string()));
        }
    }

    type Made = (Arc<Mutex<Vec<(String, String)>>>, Arc<Mutex<usize>>);

    fn make(made: &Made) -> impl FnOnce(Host, &Map) -> Result<Shelf, ProviderError> {
        let made = (Arc::clone(&made.0), Arc::clone(&made.1));
        move |_host, config| {
            let ids = config
                .get("ids")
                .and_then(|v| TryAsRef::<List>::try_as_ref(v))
                .ok_or_else(|| ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, "no ids"))?
                .iter()
                .filter_map(|v| TryAsRef::<str>::try_as_ref(v).map(str::to_string))
                .collect();
            Ok(Shelf {
                ids,
                stamp: Mutex::new("1".to_string()),
                refuse: Mutex::new(false),
                closed: made.0,
                dropped: made.1,
            })
        }
    }

    fn start(
        registry: &mut Registry,
        config: &Value,
        made: &Made,
    ) -> Result<*const SourceInfo, ProviderError> {
        let host = registry.host();
        let mut out: *const SourceInfo = core::ptr::null();
        let mut err = ProviderError::none();
        // SAFETY: the registry's host block, a well-formed value and
        // writable out slots.
        let status = unsafe { source_entry(host.as_raw(), config, &mut out, &mut err, make(made)) };
        if status == Status::GUATIAO_OK {
            assert!(!out.is_null());
            Ok(out)
        } else {
            assert!(out.is_null());
            assert_eq!(err.status, status);
            Err(err)
        }
    }

    fn config(ids: &[&str]) -> Value {
        let mut list = List::new();
        for id in ids {
            list.push(*id);
        }
        let mut map = Map::new();
        map.set("ids", list);
        Value::from(map)
    }

    #[test]
    fn an_exported_source_serves_through_its_table_and_hears_each_close_once() {
        let made: Made = Default::default();
        let mut registry = Registry::new("export-tests", "1.0");
        let table = start(&mut registry, &config(&["a", "b"]), &made).expect("a table");
        // SAFETY: the table `source_entry` answered, kept for the process.
        let table = unsafe { &*table };
        registry.add_source_table("shelf", table).expect("added");
        let first = registry.refresh();
        assert!(first.refused.is_empty(), "{first:?}");
        assert_eq!(first.loaded.len(), 2);
        assert_eq!(registry.providers("thing").count(), 2);

        registry.retire("a").expect("served");
        registry.collect();
        assert_eq!(*lock(&made.0), [("a".to_string(), "1".to_string())]);
        assert_eq!(*lock(&made.1), 1, "a's parts were dropped");

        // A refused open keeps what serves, in the source's words.
        // SAFETY: the state behind the table's `ctx`.
        let shelf = unsafe { state::<Shelf>(table.ctx) }.expect("a state");
        *lock(&shelf.source.refuse) = true;
        *lock(&shelf.source.stamp) = "2".to_string();
        let second = registry.refresh();
        assert!(
            second.refused.iter().any(|r| r.reason == "not today"),
            "{second:?}"
        );
        assert_eq!(registry.providers("thing").count(), 1);

        // A panic while listing is a status, and the refresh goes on.
        *lock(&shelf.source.stamp) = "panic".to_string();
        let third = registry.refresh();
        assert_eq!(third.refused.len(), 1, "{third:?}");
    }

    #[test]
    fn a_source_refusing_its_configuration_answers_its_words() {
        let made: Made = Default::default();
        let mut registry = Registry::new("export-tests", "1.0");
        let err = start(&mut registry, &Value::from(Map::new()), &made).unwrap_err();
        assert_eq!(err.message(), "no ids");
        let err = start(&mut registry, &Value::from(5i64), &made).unwrap_err();
        assert_eq!(err.status, Status::GUATIAO_ERR_WRONG_KIND);
    }
}
