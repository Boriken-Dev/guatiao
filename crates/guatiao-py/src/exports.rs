// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The `extern "C"` surface: the source's entry point, the descriptor a
//! Python library built, and finding this file's own path per OS.

use core::ffi::c_void;

use guatiao::Map;
use guatiao::library::{Host, LibraryInfo, LocalLibrary, ProviderError};
use guatiao_bridge::{BridgeError, BridgeSource};

use crate::engine::Python;

/// Reads `{"dirs": [..], "manifests": [..], "paths": [..]}` and starts or
/// joins the interpreter.
fn start(_host: Host, config: &Map) -> Result<BridgeSource<Python>, ProviderError> {
    os::share_python_symbols();
    Ok(BridgeSource::shared(config)?)
}

guatiao::source!(serves = "python", make = start);

/// The library descriptor at `address`: what `guatiao.provide` built with
/// ctypes for the library `bridge.py` opened.
pub(crate) fn descriptor(address: usize) -> Result<LocalLibrary<'static>, BridgeError> {
    if address == 0 || !address.is_multiple_of(align_of::<LibraryInfo<'static>>()) {
        return Err(BridgeError::engine(
            "the Python library answered no descriptor",
        ));
    }
    // SAFETY: the address of the `LibraryInfo` the Python binding built for
    // the library just opened; its `Library` keeps the structure and every
    // callback it names alive until `close_library`, which the bridge calls
    // only after the registry let the library go.
    let info = unsafe { &*core::ptr::with_exposed_provenance::<LibraryInfo<'static>>(address) };
    Ok(LocalLibrary::Borrowed(info))
}

/// This file's own path, so the Python binding loads the guatiao functions
/// it exports rather than looking for another copy.
pub(crate) fn own_path() -> Option<String> {
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
        let Ok(path) = alloc::ffi::CString::new(path) else {
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
