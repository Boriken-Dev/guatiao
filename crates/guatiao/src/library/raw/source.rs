// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A source's table, read where it enters and called; a source file
//! opened; and what a library a source served answers when the registry
//! closes it.

use alloc::sync::Arc;

use super::check::{Bounds, Refusal};
use super::*;
use crate::library::desc::SourceInfo;
use crate::library::kind::ProviderError;
use crate::library::source::{Candidate, Source};
use crate::value::types::List;

/// The slots of a source table, read out and checked.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Table {
    ctx: *mut c_void,
    list: unsafe extern "C" fn(*mut c_void, *const HostInfo<'static>, *mut List) -> Status,
    open: unsafe extern "C" fn(
        *mut c_void,
        *const HostInfo<'static>,
        Str,
        *mut *const LibraryInfo<'static>,
        *mut ProviderError,
    ) -> Status,
    close: Option<unsafe extern "C" fn(*mut c_void, Str, *const LibraryInfo<'static>)>,
}

/// One source a registry holds: a source a Rust host linked, or a table
/// it read.
pub(crate) enum SourceImpl {
    Rust(Box<dyn Source>),
    Table(Table),
}

/// A source, named, and shared by the registry and every library it
/// served while one is open.
pub(crate) struct SourceCell {
    pub(crate) name: String,
    imp: SourceImpl,
    /// The file it came from, kept mapped: never closed, since a library it
    /// served may outlive the registry.
    #[cfg(feature = "load")]
    _mapping: Option<core::mem::ManuallyDrop<libloading::Library>>,
}

impl core::fmt::Debug for SourceCell {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SourceCell")
            .field("name", &self.name)
            .finish()
    }
}

// SAFETY: a source's slots are the source's contract to call from any
// thread, one at a time per source, which the registry holding it by
// `&mut` gives; its `ctx` is only passed back to them.
unsafe impl Send for SourceCell {}
// SAFETY: as above; a shared `SourceCell` only reads its fields.
unsafe impl Sync for SourceCell {}

/// What opening one candidate came to.
pub(crate) enum Opening {
    /// The registry's answer for any library: loaded, declined, rejected.
    Opened(Opened),
    /// The source refused, in its own words.
    Refused(String),
}

/// The library a source served: what closing it tells the source.
#[derive(Debug)]
pub(crate) struct Served {
    source: Arc<SourceCell>,
    pub(crate) id: String,
    pub(crate) stamp: String,
    /// The descriptor a table's `open` answered, by address.
    info: usize,
    pub(crate) parts: Option<Box<crate::library::kind::LibraryParts>>,
}

impl Served {
    /// Tells the source its library is closed.
    pub(crate) fn close(self) {
        self.source.close(&self.id, &self.stamp, self.info);
    }
}

impl SourceCell {
    /// A source a Rust host links.
    pub(crate) fn rust(name: &str, source: Box<dyn Source>) -> Arc<SourceCell> {
        Arc::new(SourceCell {
            name: name.to_string(),
            imp: SourceImpl::Rust(source),
            #[cfg(feature = "load")]
            _mapping: None,
        })
    }

    /// A table a host links, read under the checks every descriptor gets.
    pub(crate) fn table(name: &str, raw: *const SourceInfo) -> Result<Arc<SourceCell>, Rejected> {
        // SAFETY: handing over a table is the host's trust in its producer,
        // as registering an entry point is; what can be checked is.
        let table = unsafe { read_source(raw, Bounds::ANY) }?;
        Ok(Arc::new(SourceCell {
            name: name.to_string(),
            imp: SourceImpl::Table(table),
            #[cfg(feature = "load")]
            _mapping: None,
        }))
    }

    /// Every candidate the source lists, and a sentence for each entry of
    /// its list that is not one.
    pub(crate) fn list(&self, host: Host) -> Result<(Vec<Candidate>, Vec<String>), String> {
        let list = match &self.imp {
            SourceImpl::Rust(source) => {
                return match catch_unwind(AssertUnwindSafe(|| source.list(host))) {
                    Ok(Ok(found)) => Ok((found, Vec::new())),
                    Ok(Err(e)) => Err(e.message().to_string()),
                    Err(payload) => {
                        core::mem::forget(payload);
                        Err("the source panicked while listing".to_string())
                    }
                };
            }
            SourceImpl::Table(table) => {
                let mut out = <List as crate::library::kind::Carrier>::carrier(Alloc::rust());
                let status = crate::exports::guard_with(Status::GUATIAO_ERR_INTERNAL, || {
                    // SAFETY: the slot the table declared, called as its
                    // contract says: its `ctx`, the host block, an out slot
                    // holding a carrier.
                    unsafe { (table.list)(table.ctx, host.as_raw(), &mut out) }
                });
                if status != Status::GUATIAO_OK {
                    // A list it answered while refusing is still a list it
                    // handed over: freed here, through the allocators it records.
                    drop(out);
                    return Err(format!(
                        "the source could not list its libraries ({status:?})"
                    ));
                }
                out
            }
        };
        Ok(read_candidates(&list))
    }

    /// Opens the candidate `id`, listed under `stamp`.
    pub(crate) fn open(self: &Arc<Self>, host: Host, id: &str, stamp: &str) -> Opening {
        let served = |info: usize, parts| {
            Origin::Served(Served {
                source: Arc::clone(self),
                id: id.to_string(),
                stamp: stamp.to_string(),
                info,
                parts,
            })
        };
        match &self.imp {
            SourceImpl::Rust(source) => {
                let local = match catch_unwind(AssertUnwindSafe(|| source.open(host, id))) {
                    Ok(Ok(local)) => local,
                    Ok(Err(e)) => return Opening::Refused(e.message().to_string()),
                    Err(payload) => {
                        core::mem::forget(payload);
                        return Opening::Refused("the source panicked while opening".to_string());
                    }
                };
                match open_local(Some(local)) {
                    Opened::Loaded(view, mut origin) => {
                        let parts = origin.take_parts();
                        Opening::Opened(Opened::Loaded(view, served(0, parts)))
                    }
                    other => {
                        // Opened and not kept: the source hears of it at once.
                        self.close(id, stamp, 0);
                        Opening::Opened(other)
                    }
                }
            }
            SourceImpl::Table(table) => {
                let mut out: *const LibraryInfo<'static> = core::ptr::null();
                let mut err = ProviderError::none();
                let status = crate::exports::guard_with(Status::GUATIAO_ERR_INTERNAL, || {
                    // SAFETY: the slot the table declared, called as its
                    // contract says, with writable out slots.
                    unsafe {
                        (table.open)(table.ctx, host.as_raw(), Str::new(id), &mut out, &mut err)
                    }
                });
                if status != Status::GUATIAO_OK {
                    let e = crate::library::kind::take_err(err, status);
                    let words = e.message();
                    return Opening::Refused(if words.is_empty() {
                        format!("the source refused to open it ({status:?})")
                    } else {
                        words.to_string()
                    });
                }
                drop(err);
                if out.is_null() {
                    return Opening::Opened(Opened::Declined);
                }
                // SAFETY: non-null, from the source's `open`, and valid until
                // its `close`; a source serves descriptors from wherever it
                // builds them, so no image bounds them.
                match unsafe { read_library(out, Bounds::ANY) } {
                    Ok(view) => Opening::Opened(Opened::Loaded(view, served(out as usize, None))),
                    Err(why) => {
                        self.close(id, stamp, out as usize);
                        Opening::Opened(Opened::Rejected(why))
                    }
                }
            }
        }
    }

    /// Tells the source one open of `id` is closed. A panic stops here.
    fn close(&self, id: &str, stamp: &str, info: usize) {
        match &self.imp {
            SourceImpl::Rust(source) => {
                if let Err(payload) = catch_unwind(AssertUnwindSafe(|| source.close(id, stamp))) {
                    core::mem::forget(payload);
                }
            }
            SourceImpl::Table(table) => {
                if let Some(close) = table.close {
                    crate::exports::guard_with((), || {
                        // SAFETY: the slot the table declared, with the
                        // descriptor its own `open` answered for `id`.
                        unsafe {
                            close(table.ctx, Str::new(id), info as *const LibraryInfo<'static>)
                        }
                    });
                }
            }
        }
    }
}

/// The candidates in a list a source answered: a map each, with text
/// `id`, `version` and `stamp` and a list of text `kinds`. An entry that is
/// not one is a sentence in the second list, and the rest still count.
fn read_candidates(list: &List) -> (Vec<Candidate>, Vec<String>) {
    use crate::value::convert::TryAsRef;
    let mut found = Vec::new();
    let mut wrong = Vec::new();
    for (at, entry) in list.iter().enumerate() {
        let Some(map) = TryAsRef::<Map>::try_as_ref(entry) else {
            wrong.push(format!("entry {at} of the source's list is not a map"));
            continue;
        };
        let text = |key: &str| map.get(key).and_then(|v| TryAsRef::<str>::try_as_ref(v));
        let Some(id) = text("id").filter(|id| !id.is_empty()) else {
            wrong.push(format!("entry {at} of the source's list has no text `id`"));
            continue;
        };
        let kinds = map
            .get("kinds")
            .and_then(|v| TryAsRef::<List>::try_as_ref(v))
            .map(|l| {
                l.iter()
                    .filter_map(|k| TryAsRef::<str>::try_as_ref(k).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        found.push(Candidate::new(
            id,
            text("version").unwrap_or_default(),
            kinds,
            text("stamp").unwrap_or_default(),
        ));
    }
    (found, wrong)
}

/// Reads a source table under the checks every descriptor gets and, for
/// a mapped source, its image's bounds for the slots.
///
/// # Safety
///
/// `raw` came from a source's entry point or a host's registration, and
/// its producer's contract is that it is valid while it is read.
unsafe fn read_source(raw: *const SourceInfo, bounds: Bounds) -> Result<Table, Rejected> {
    const WHAT: &str = "the source table";
    if raw.is_null() {
        return Err(Refusal::new(WHAT, "is null").into());
    }
    // The table itself may live anywhere the source built it; its slots
    // are what an image bounds.
    Bounds::ANY
        .data(raw as usize, size_of::<u32>(), align_of::<SourceInfo>())
        .map_err(|why| Refusal::new(WHAT, why))?;
    // SAFETY: the leading word passed the checks.
    let declared = unsafe { core::ptr::addr_of!((*raw).struct_size).read() } as usize;
    if declared < SourceInfo::floor() {
        return Err(Refusal::short(WHAT, declared, SourceInfo::floor()).into());
    }
    Bounds::ANY
        .data(raw as usize, declared, align_of::<SourceInfo>())
        .map_err(|why| Refusal::new(WHAT, why))?;
    // SAFETY: every field read below lies within the floor, now readable.
    let info = unsafe { raw.read() };
    if info.abi_version != crate::library::desc::ABI_VERSION {
        return Err(Rejected::UnsupportedAbi {
            declared: info.abi_version,
        });
    }
    let code =
        |addr: usize, what: &'static str| bounds.code(addr).map_err(|why| Refusal::new(what, why));
    let Some(list) = info.list else {
        return Err(Refusal::new("the source's `list` slot", "is null").into());
    };
    let Some(open) = info.open else {
        return Err(Refusal::new("the source's `open` slot", "is null").into());
    };
    code(list as usize, "the source's `list` slot")?;
    code(open as usize, "the source's `open` slot")?;
    if let Some(close) = info.close {
        code(close as usize, "the source's `close` slot")?;
    }
    Ok(Table {
        ctx: info.ctx,
        list,
        open,
        close: info.close,
    })
}

/// Why a source file could not be added.
#[cfg(feature = "load")]
pub(crate) enum SourceFileError {
    /// The loader refused to map it.
    Open(String),
    /// It mapped and exports no `guatiao_source`.
    NotASource,
    /// Its entry point refused the configuration.
    Refused(Status, String),
    /// Its table was refused where it entered.
    Rejected(Rejected),
}

/// Maps a source file, asks its entry point for the table configured by
/// `config`, and reads it: its slots must lie in the file's code.
///
/// Safe to call for the reason [`open_library`] is: naming a file is
/// choosing to run it, which is the contract on
/// [`Registry::add_source_file`](crate::library::Registry::add_source_file).
#[cfg(feature = "load")]
pub(crate) fn open_source_file(
    path: &std::path::Path,
    host: Host,
    config: &Value,
    search: crate::library::registry::ImportSearch,
) -> Result<Arc<SourceCell>, SourceFileError> {
    // SAFETY: naming a file is choosing to run it.
    let library = unsafe { super::image::map(path, search) }.map_err(SourceFileError::Open)?;
    // SAFETY: the symbol is absent, which is an answer, or has the
    // signature this crate defines for it.
    let entry = match unsafe { library.get::<SourceFn>(SOURCE_SYMBOL) } {
        Ok(entry) => *entry,
        Err(_) => {
            // Its initialisers ran; whatever they started may address it.
            core::mem::forget(library);
            return Err(SourceFileError::NotASource);
        }
    };
    let image = super::mapped::image(entry as usize);
    let mut out: *const SourceInfo = core::ptr::null();
    let mut err = ProviderError::none();
    let status = crate::exports::guard_with(Status::GUATIAO_ERR_INTERNAL, || {
        // SAFETY: the entry point's contract: the host block, a well-formed
        // value, writable out slots.
        unsafe { entry(host.as_raw(), config, &mut out, &mut err) }
    });
    if status != Status::GUATIAO_OK {
        core::mem::forget(library);
        let e = crate::library::kind::take_err(err, status);
        return Err(SourceFileError::Refused(status, e.message().to_string()));
    }
    drop(err);
    // SAFETY: from the entry point, valid for the registry's life by its
    // contract; the slots are checked against the image they came from.
    let table = match unsafe { read_source(out, Bounds::of(image.as_ref())) } {
        Ok(table) => table,
        Err(why) => {
            core::mem::forget(library);
            return Err(SourceFileError::Rejected(why));
        }
    };
    Ok(Arc::new(SourceCell {
        name: path.display().to_string(),
        imp: SourceImpl::Table(table),
        _mapping: Some(core::mem::ManuallyDrop::new(library)),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe extern "C" fn list_nothing(
        _: *mut c_void,
        _: *const HostInfo<'static>,
        _: *mut List,
    ) -> Status {
        Status::GUATIAO_OK
    }

    unsafe extern "C" fn open_nothing(
        _: *mut c_void,
        _: *const HostInfo<'static>,
        _: Str,
        _: *mut *const LibraryInfo<'static>,
        _: *mut ProviderError,
    ) -> Status {
        Status::GUATIAO_ERR_NOT_FOUND
    }

    fn a_table(size: usize) -> SourceInfo {
        SourceInfo {
            struct_size: size as u32,
            abi_version: crate::library::ABI_VERSION,
            ctx: core::ptr::null_mut(),
            list: Some(list_nothing),
            open: Some(open_nothing),
            close: None,
        }
    }

    #[test]
    fn a_table_is_refused_below_its_floor_without_its_slots_or_from_another_abi() {
        let short = a_table(SourceInfo::floor() - 1);
        assert!(SourceCell::table("s", &short).is_err());
        let mut unlisted = a_table(size_of::<SourceInfo>());
        unlisted.list = None;
        assert!(SourceCell::table("s", &unlisted).is_err());
        let mut other = a_table(size_of::<SourceInfo>());
        other.abi_version = 7;
        assert!(matches!(
            SourceCell::table("s", &other),
            Err(Rejected::UnsupportedAbi { declared: 7 })
        ));
        assert!(SourceCell::table("s", core::ptr::null()).is_err());
        assert!(SourceCell::table("s", &a_table(size_of::<SourceInfo>())).is_ok());
    }

    #[test]
    fn an_entry_that_is_not_a_candidate_is_named_and_the_rest_still_count() {
        let mut list = List::new();
        let mut good = Map::new();
        good.set("id", "a");
        good.set("stamp", "1");
        list.push(good);
        list.push(Value::from(5i64));
        let mut nameless = Map::new();
        nameless.set("stamp", "2");
        list.push(nameless);
        let (found, wrong) = read_candidates(&list);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id(), "a");
        assert_eq!(wrong.len(), 2, "{wrong:?}");
    }
}
