// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The source table, as `guatiao.h` declares `guatiao_source_info`, and the
//! entry point that answers it.

use alloc::collections::BTreeMap;
use core::ffi::c_void;
use core::panic::AssertUnwindSafe;
use std::panic::catch_unwind;
use std::path::PathBuf;
use std::sync::Mutex;

use guatiao::library::{ABI_VERSION, HostInfo, LibraryInfo, ProviderError, SourceInfo};
use guatiao::value::convert::TryAsRef;
use guatiao::{Alloc, List, Map, Status, Str, Text, Value};

use crate::manifest::{self, Found};

// What a scanner reads before mapping this file: a source, never a library.
guatiao::declares!("source=python");

/// What one added source holds: where its manifests are, and which
/// descriptor each open answered, by address, with the bridge's handle.
struct State {
    dirs: Vec<PathBuf>,
    manifests: Vec<PathBuf>,
    open: Mutex<BTreeMap<usize, i64>>,
}

fn paths_of(config: &Map, key: &str) -> Result<Vec<String>, String> {
    let Some(value) = config.get(key) else {
        return Ok(Vec::new());
    };
    let list: &List = TryAsRef::<List>::try_as_ref(value)
        .ok_or_else(|| format!("`{key}` is not a list of paths"))?;
    list.iter()
        .map(|v| {
            TryAsRef::<str>::try_as_ref(v)
                .map(str::to_string)
                .ok_or_else(|| format!("`{key}` holds something that is not a path"))
        })
        .collect()
}

/// Writes a refusal through `err` and answers its status.
///
/// # Safety
///
/// `err` is null or writable.
unsafe fn refuse(err: *mut ProviderError, status: Status, why: &str) -> Status {
    if !err.is_null() {
        // SAFETY: non-null and writable by the caller's contract, written
        // empty on entry.
        unsafe { err.write(ProviderError::new(status, why)) };
    }
    status
}

/// The source's entry point: reads `{"dirs": [..], "manifests": [..],
/// "paths": [..]}`, starts or joins the interpreter, and answers the table.
///
/// # Safety
///
/// As `guatiao_source` in `guatiao.h`: `config` is null or a well-formed
/// value, `out` and `err` are null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_source(
    host: *const HostInfo<'static>,
    config: *const Value,
    out: *mut *const SourceInfo,
    err: *mut ProviderError,
) -> Status {
    if !out.is_null() {
        // SAFETY: non-null and writable by the caller's contract.
        unsafe { out.write(core::ptr::null()) };
    }
    if !err.is_null() {
        // SAFETY: as above; the placeholder owns nothing.
        unsafe { err.write(ProviderError::none()) };
    }
    let answer = catch_unwind(AssertUnwindSafe(
        || -> Result<*const SourceInfo, (Status, String)> {
            let _ = host;
            let refused = |why: String| (Status::GUATIAO_ERR_BAD_VALUE, why);
            // SAFETY: null or a well-formed value by the caller's contract.
            let config: Map = match unsafe { config.as_ref() } {
                None => Map::new(),
                Some(value) => TryAsRef::<Map>::try_as_ref(value).cloned().ok_or_else(|| {
                    (
                        Status::GUATIAO_ERR_WRONG_KIND,
                        "the configuration is not a map".to_string(),
                    )
                })?,
            };
            let dirs = paths_of(&config, "dirs").map_err(refused)?;
            let manifests = paths_of(&config, "manifests").map_err(refused)?;
            let paths = paths_of(&config, "paths").map_err(refused)?;
            let own = own_path();
            os::share_python_symbols();
            crate::bridge::start(&paths, own.as_deref())
                .map_err(|why| (Status::GUATIAO_ERR_UNSUPPORTED, why))?;
            // Kept for the life of the process, as the table's contract asks.
            let state = Box::leak(Box::new(State {
                dirs: dirs.into_iter().map(PathBuf::from).collect(),
                manifests: manifests.into_iter().map(PathBuf::from).collect(),
                open: Mutex::new(BTreeMap::new()),
            }));
            let table = Box::leak(Box::new(SourceInfo {
                struct_size: size_of::<SourceInfo>() as u32,
                abi_version: ABI_VERSION,
                ctx: (state as *mut State).cast::<c_void>(),
                list: Some(list),
                open: Some(open),
                close: Some(close),
            }));
            Ok(table as *const SourceInfo)
        },
    ));
    match answer {
        Ok(Ok(table)) => {
            if !out.is_null() {
                // SAFETY: non-null and writable, as above.
                unsafe { out.write(table) };
            }
            Status::GUATIAO_OK
        }
        // SAFETY: `err` is null or writable by the caller's contract.
        Ok(Err((status, why))) => unsafe { refuse(err, status, &why) },
        Err(payload) => {
            core::mem::forget(payload);
            // SAFETY: as above.
            unsafe {
                refuse(
                    err,
                    Status::GUATIAO_ERR_INTERNAL,
                    "the source panicked while starting",
                )
            }
        }
    }
}

/// The state a slot's `ctx` names.
///
/// # Safety
///
/// `ctx` is the one `guatiao_source` answered, leaked for the process.
unsafe fn state<'a>(ctx: *mut c_void) -> &'a State {
    // SAFETY: the caller's contract.
    unsafe { &*ctx.cast::<State>() }
}

/// What a manifest says the library's version is, or its module's
/// literal `__version__`; a refusal reads as a broken manifest.
fn versioned(found: Found) -> (Found, String) {
    match found {
        Found::Library(m) => match &m.version {
            Some(v) => {
                let v = v.clone();
                (Found::Library(m), v)
            }
            None => match crate::bridge::literal_version(&m) {
                Ok(v) => (Found::Library(m), v),
                Err(why) => (
                    Found::Broken {
                        path: PathBuf::from(&m.id),
                        stamp: m.stamp.clone(),
                        why: format!("{}: {why}", m.id),
                    },
                    String::new(),
                ),
            },
        },
        broken => (broken, String::new()),
    }
}

fn candidate(found: &Found, version: &str, alloc: Alloc) -> Result<Map, guatiao::ValueError> {
    let mut map = Map::new_in(alloc);
    let (kinds, stamp): (&[String], &str) = match found {
        Found::Library(m) => (&m.kinds, &m.stamp),
        Found::Broken { stamp, .. } => (&[], stamp),
    };
    map.set_in("id", Text::new_in(alloc, &found.id())?, alloc)?;
    map.set_in("version", Text::new_in(alloc, version)?, alloc)?;
    let mut list = List::new_in(alloc);
    for kind in kinds {
        list.try_push(Text::new_in(alloc, kind)?)?;
    }
    map.set_in("kinds", list, alloc)?;
    map.set_in("stamp", Text::new_in(alloc, stamp)?, alloc)?;
    Ok(map)
}

unsafe extern "C" fn list(
    ctx: *mut c_void,
    _host: *const HostInfo<'static>,
    out: *mut List,
) -> Status {
    let built = catch_unwind(AssertUnwindSafe(|| -> Result<List, guatiao::ValueError> {
        // SAFETY: the `ctx` this source answered.
        let state = unsafe { state(ctx) };
        // SAFETY: the registry's out slot, holding a carrier.
        let alloc = unsafe { guatiao::library::kind::return_alloc(out) };
        let mut list = List::new_in(alloc);
        for found in manifest::find(&state.dirs, &state.manifests) {
            let (found, version) = versioned(found);
            list.try_push(candidate(&found, &version, alloc)?)?;
        }
        Ok(list)
    }));
    match built {
        Ok(Ok(list)) => {
            if out.is_null() {
                return Status::GUATIAO_ERR_NULL;
            }
            // SAFETY: the carrier owns nothing; the list is written over it.
            unsafe { out.write(list) };
            Status::GUATIAO_OK
        }
        Ok(Err(e)) => e.into(),
        Err(payload) => {
            core::mem::forget(payload);
            Status::GUATIAO_ERR_INTERNAL
        }
    }
}

unsafe extern "C" fn open(
    ctx: *mut c_void,
    _host: *const HostInfo<'static>,
    id: Str,
    out: *mut *const LibraryInfo<'static>,
    err: *mut ProviderError,
) -> Status {
    let answer = catch_unwind(AssertUnwindSafe(|| -> Result<usize, (Status, String)> {
        // SAFETY: the `ctx` this source answered.
        let state = unsafe { state(ctx) };
        let id = core::str::from_utf8(id.into()).map_err(|_| {
            (
                Status::GUATIAO_ERR_BAD_VALUE,
                "the id is not UTF-8".to_string(),
            )
        })?;
        let found = manifest::find(&state.dirs, &state.manifests)
            .into_iter()
            .find(|f| f.id() == id)
            .ok_or_else(|| {
                (
                    Status::GUATIAO_ERR_NOT_FOUND,
                    format!("no manifest describes `{id}` now"),
                )
            })?;
        let (manifest, version) = match versioned(found) {
            (Found::Library(m), version) => (m, version),
            (Found::Broken { why, .. }, _) => return Err((Status::GUATIAO_ERR_BAD_VALUE, why)),
        };
        let (info, handle) = crate::bridge::open(&manifest, &version)
            .map_err(|why| (Status::GUATIAO_ERR_BAD_VALUE, why))?;
        state
            .open
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(info, handle);
        Ok(info)
    }));
    match answer {
        Ok(Ok(info)) => {
            if out.is_null() {
                return Status::GUATIAO_ERR_NULL;
            }
            // SAFETY: non-null and writable by the registry's contract; the
            // descriptor lives while the Python library does, until `close`.
            unsafe { out.write(info as *const LibraryInfo<'static>) };
            Status::GUATIAO_OK
        }
        // SAFETY: `err` is null or writable by the registry's contract.
        Ok(Err((status, why))) => unsafe { refuse(err, status, &why) },
        Err(payload) => {
            core::mem::forget(payload);
            // SAFETY: as above.
            unsafe {
                refuse(
                    err,
                    Status::GUATIAO_ERR_INTERNAL,
                    "the source panicked while opening",
                )
            }
        }
    }
}

unsafe extern "C" fn close(ctx: *mut c_void, _id: Str, info: *const LibraryInfo<'static>) {
    let done = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the `ctx` this source answered.
        let state = unsafe { state(ctx) };
        let handle = state
            .open
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&(info as usize));
        if let Some(handle) = handle {
            crate::bridge::close(handle);
        }
    }));
    if let Err(payload) = done {
        core::mem::forget(payload);
    }
}

/// This file's own path, so the Python binding loads the guatiao functions
/// it exports rather than looking for another copy.
fn own_path() -> Option<String> {
    os::own_path((own_path as fn() -> Option<String>) as *const c_void)
}

/// Where libpython is mapped from: the object holding one of its functions.
#[cfg(unix)]
fn python_path() -> Option<String> {
    os::own_path(pyo3::ffi::Py_IsInitialized as *const c_void)
}

#[cfg(windows)]
mod os {
    use core::ffi::c_void;

    const FROM_ADDRESS: u32 = 0x4;
    const UNCHANGED_REFCOUNT: u32 = 0x2;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleExW(flags: u32, address: *const c_void, module: *mut *mut c_void) -> i32;
        fn GetModuleFileNameW(module: *mut c_void, name: *mut u16, size: u32) -> u32;
    }

    /// Windows resolves an extension module's imports against the
    /// `python3X.dll` it names, already loaded: nothing to share.
    pub(super) fn share_python_symbols() {}

    pub(super) fn own_path(address: *const c_void) -> Option<String> {
        let mut module = core::ptr::null_mut();
        // SAFETY: an address inside this module, and a writable handle slot;
        // the reference count is left as it is.
        if unsafe { GetModuleHandleExW(FROM_ADDRESS | UNCHANGED_REFCOUNT, address, &mut module) }
            == 0
        {
            return None;
        }
        let mut name = vec![0u16; 32 * 1024];
        // SAFETY: the module just found, and a buffer of the length given.
        let len =
            unsafe { GetModuleFileNameW(module, name.as_mut_ptr(), name.len() as u32) } as usize;
        (len > 0 && len < name.len()).then(|| String::from_utf16_lossy(&name[..len]))
    }
}

#[cfg(unix)]
mod os {
    use core::ffi::{CStr, c_char, c_int, c_void};

    #[repr(C)]
    struct DlInfo {
        fname: *const c_char,
        fbase: *mut c_void,
        sname: *const c_char,
        saddr: *mut c_void,
    }

    unsafe extern "C" {
        fn dladdr(address: *const c_void, info: *mut DlInfo) -> c_int;
    }

    pub(super) fn own_path(address: *const c_void) -> Option<String> {
        let mut info = DlInfo {
            fname: core::ptr::null(),
            fbase: core::ptr::null_mut(),
            sname: core::ptr::null(),
            saddr: core::ptr::null_mut(),
        };
        // SAFETY: an address inside this object, and a writable record.
        if unsafe { dladdr(address, &mut info) } == 0 || info.fname.is_null() {
            return None;
        }
        // SAFETY: the loader's own NUL-terminated name for the object.
        let name = unsafe { CStr::from_ptr(info.fname) };
        let path = std::path::PathBuf::from(name.to_string_lossy().into_owned());
        Some(path.canonicalize().unwrap_or(path).display().to_string())
    }

    /// Makes libpython's symbols global. A host maps this file, and with it
    /// libpython, with `RTLD_LOCAL`; an extension module (`_ctypes`, any
    /// compiled package) is built to find the interpreter's symbols in the
    /// global scope, and fails to import without this. Reopening with
    /// `RTLD_NOLOAD` loads nothing: it only widens the scope of the copy
    /// already mapped. A Python host's own symbols stay first.
    pub(super) fn share_python_symbols() {
        let Some(path) = super::python_path() else {
            return;
        };
        let Ok(path) = std::ffi::CString::new(path) else {
            return;
        };
        // SAFETY: a NUL-terminated path of an object already mapped; the
        // handle is left open, as libpython stays for the process.
        let _ = unsafe {
            libc::dlopen(
                path.as_ptr(),
                libc::RTLD_NOW | libc::RTLD_GLOBAL | libc::RTLD_NOLOAD,
            )
        };
    }
}

#[cfg(not(any(windows, unix)))]
mod os {
    pub(super) fn share_python_symbols() {}

    pub(super) fn own_path(_: *const core::ffi::c_void) -> Option<String> {
        None
    }
}
