// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Starting the interpreter under the configuration's `isolation`.
//!
//! The build links the stable ABI (`abi3`), which has no `PyConfig`, and
//! `PyConfig`'s layout changes between minors. So nothing here reads or
//! writes one of its fields: `shipped` and `isolated` start from CPython's
//! isolated configuration, made by `PyConfig_InitIsolatedConfig` in a
//! buffer larger than any minor's, with `home` and the program's name
//! given through `Py_SetPythonHome`/`Py_SetProgramName`, which that
//! configuration reads when it has none of its own (3.9 to 3.14); from
//! 3.14, `PyInitConfig_*` names each by its key. The functions are found
//! in the loaded libpython at run time. `inherit` is `Py_InitializeEx(0)`.

use core::ffi::{CStr, c_char, c_int, c_void};
use std::sync::Mutex;

use pyo3::Python;

/// How much of the environment the interpreter takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Isolation {
    /// What is shipped with the program, nothing from the environment.
    Shipped,
    /// The environment as well, as a `python` started there would.
    Inherit,
    /// Only the configuration's `home` and `paths`.
    Isolated,
}

impl Isolation {
    pub(crate) fn parse(text: &str) -> Option<Isolation> {
        match text {
            "shipped" => Some(Isolation::Shipped),
            "inherit" => Some(Isolation::Inherit),
            "isolated" => Some(Isolation::Isolated),
            _ => None,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Isolation::Shipped => "shipped",
            Isolation::Inherit => "inherit",
            Isolation::Isolated => "isolated",
        }
    }
}

/// What an interpreter this file starts is started with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Settings {
    pub(crate) isolation: Isolation,
    /// The Python installation, when the configuration names one.
    pub(crate) home: Option<String>,
}

/// Who started the one interpreter.
enum Started {
    /// The host, which is Python: its interpreter is used as it is.
    Host,
    /// This file, under these settings.
    Here(Settings),
}

static STARTED: Mutex<Option<Started>> = Mutex::new(None);

/// Why the interpreter could not be had as asked.
pub(crate) enum StartError {
    /// It already runs under other settings.
    Started(String),
    /// It did not start.
    Failed(String),
}

impl From<String> for StartError {
    fn from(why: String) -> StartError {
        StartError::Failed(why)
    }
}

impl From<&str> for StartError {
    fn from(why: &str) -> StartError {
        StartError::Failed(why.to_string())
    }
}

/// Starts the interpreter under `settings`, or joins the host's. A second
/// source asking for other settings than the interpreter started with is
/// refused: there is one interpreter for the program.
pub(crate) fn start(settings: &Settings) -> Result<(), StartError> {
    let mut started = STARTED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match &*started {
        Some(Started::Host) => return Ok(()),
        Some(Started::Here(first)) if first == settings => return Ok(()),
        Some(Started::Here(first)) => {
            return Err(StartError::Started(format!(
                "asks for `{}`{}, and the program's one interpreter already started with `{}`{}",
                settings.isolation.name(),
                home_words(&settings.home),
                first.isolation.name(),
                home_words(&first.home),
            )));
        }
        None => {}
    }
    // SAFETY: callable at any time, before initialisation too.
    if unsafe { pyo3::ffi::Py_IsInitialized() } != 0 {
        *started = Some(Started::Host);
        return Ok(());
    }
    if settings.isolation == Isolation::Isolated && cfg!(windows) {
        let unheld = files_isolated_cannot_hold();
        if !unheld.is_empty() {
            return Err(StartError::Failed(format!(
                "the isolation `isolated` cannot keep {} out on CPython 3.{} under Windows, which \
                 takes the program from the system before 3.14 and reads a `._pth` beside it \
                 whatever `home` says before 3.11: remove it, or run a later Python",
                unheld.join(" and "),
                minor().unwrap_or(0),
            )));
        }
    }
    match settings.isolation {
        Isolation::Inherit => Python::initialize(),
        Isolation::Shipped | Isolation::Isolated => {
            // CPython 3.9 and 3.10 on Windows find no standard library from
            // an isolated configuration with no `home`; they read the
            // program's `._pth` and `site` its `pyvenv.cfg` with one.
            let needs_home = settings.isolation == Isolation::Isolated
                || (cfg!(windows) && minor().is_some_and(|m| m < 11));
            let home = match &settings.home {
                Some(home) => Some(home.clone()),
                None if needs_home => Some(python_home().ok_or(
                    "no `home` was given and libpython's own directory could not be found",
                )?),
                None => None,
            };
            let (program, executable) = match (settings.isolation, &home) {
                (Isolation::Isolated, Some(home)) => {
                    let program = home_program(home);
                    (Some(program.clone()), Some(program))
                }
                _ => (host_program(), None),
            };
            initialize_isolated(home.as_deref(), program.as_deref(), executable.as_deref())?;
            // Joins what is now initialised; starts nothing.
            Python::initialize();
        }
    }
    *started = Some(Started::Here(settings.clone()));
    Ok(())
}

/// What an `isolated` interpreter would still read beside the program on
/// Windows, where CPython takes the executable from the system and a name
/// given for it changes nothing: before 3.14 `site` reads a `pyvenv.cfg`
/// there or one directory up, and before 3.11 the path code reads the
/// program's `._pth` whatever `home` says. Each such file that exists.
fn files_isolated_cannot_hold() -> Vec<String> {
    let (Some(minor), Ok(exe)) = (minor(), std::env::current_exe()) else {
        return Vec::new();
    };
    let Some(dir) = exe.parent() else {
        return Vec::new();
    };
    let mut candidates = Vec::new();
    if minor < 14 {
        candidates.push(dir.join("pyvenv.cfg"));
        if let Some(up) = dir.parent() {
            candidates.push(up.join("pyvenv.cfg"));
        }
    }
    if minor < 11 {
        candidates.push(exe.with_extension("_pth"));
    }
    candidates
        .into_iter()
        .filter(|p| p.is_file())
        .map(|p| p.display().to_string())
        .collect()
}

/// The loaded CPython's minor version, from `Py_GetVersion` (`3.14.0 ...`).
fn minor() -> Option<u32> {
    // SAFETY: callable before initialisation; a static NUL-terminated text.
    let version = unsafe { CStr::from_ptr(pyo3::ffi::Py_GetVersion()) };
    let version = version.to_str().ok()?;
    version.split(['.', ' ']).nth(1)?.parse().ok()
}

fn home_words(home: &Option<String>) -> String {
    home.as_ref()
        .map(|h| format!(" and home {h}"))
        .unwrap_or_default()
}

/// The program's name under `shipped`, its own executable: CPython reads
/// the `._pth` and `pyvenv.cfg` beside it. Windows finds the executable
/// itself; elsewhere a bare name is looked up on `PATH`, which would let
/// the environment choose the files read.
fn host_program() -> Option<String> {
    if cfg!(windows) {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    exe.to_str().map(str::to_string)
}

/// The program's name under `isolated`: the installation's own
/// executable, so that what CPython reads beside the program (`._pth`,
/// `pyvenv.cfg`, both at start and in `site`) is the installation's. It
/// need not exist.
fn home_program(home: &str) -> String {
    let home = std::path::Path::new(home);
    let exe = if cfg!(windows) {
        home.join("python.exe")
    } else {
        home.join("bin").join("python3")
    };
    exe.display().to_string()
}

type PyInitConfig = c_void;

/// CPython's `PyStatus`, the same from 3.8 on.
#[repr(C)]
struct PyStatus {
    kind: c_int,
    func: *const c_char,
    err_msg: *const c_char,
    exitcode: c_int,
}

/// Larger than `PyConfig` in any minor (about 400 bytes on 3.14).
#[repr(C, align(16))]
struct ConfigBuffer([u8; 8192]);

#[cfg(windows)]
type WChar = u16;
#[cfg(not(windows))]
type WChar = u32;

/// `text` as a NUL-terminated `wchar_t` string, kept for the process
/// whether or not this minor copies it: the cost is a path.
fn leaked_wide(text: &str) -> *const WChar {
    #[cfg(windows)]
    let mut wide: Vec<WChar> = text.encode_utf16().collect();
    #[cfg(not(windows))]
    let mut wide: Vec<WChar> = text.chars().map(u32::from).collect();
    wide.push(0);
    Vec::leak(wide).as_ptr()
}

/// Starts CPython from its isolated configuration, with `home` and the
/// program's name when there are any, and from 3.14 the executable CPython
/// reads files beside (before, it is the program's name elsewhere and the
/// system's answer on Windows). Leaves the calling thread without the GIL,
/// as `Py_InitializeEx` under pyo3 does.
fn initialize_isolated(
    home: Option<&str>,
    program: Option<&str>,
    executable: Option<&str>,
) -> Result<(), String> {
    if let (Some(create), Some(set), Some(init), Some(error), Some(free)) = (
        lookup(c"PyInitConfig_Create"),
        lookup(c"PyInitConfig_SetStr"),
        lookup(c"Py_InitializeFromInitConfig"),
        lookup(c"PyInitConfig_GetError"),
        lookup(c"PyInitConfig_Free"),
    ) {
        // SAFETY: CPython 3.14's functions, with these signatures
        // (`cpython/initconfig.h`).
        let (create, set, init, error, free) = unsafe {
            (
                core::mem::transmute::<*mut c_void, unsafe extern "C" fn() -> *mut PyInitConfig>(
                    create,
                ),
                core::mem::transmute::<
                    *mut c_void,
                    unsafe extern "C" fn(*mut PyInitConfig, *const c_char, *const c_char) -> c_int,
                >(set),
                core::mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut PyInitConfig) -> c_int>(
                    init,
                ),
                core::mem::transmute::<
                    *mut c_void,
                    unsafe extern "C" fn(*mut PyInitConfig, *mut *const c_char) -> c_int,
                >(error),
                core::mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut PyInitConfig)>(free),
            )
        };
        // SAFETY: creates a configuration this function frees below.
        let config = unsafe { create() };
        if config.is_null() {
            return Err("Python could not make its configuration".to_string());
        }
        let set_text = |key: &CStr, value: Option<&str>| -> Result<(), ()> {
            let Some(value) = value else { return Ok(()) };
            let value = alloc::ffi::CString::new(value).map_err(|_| ())?;
            // SAFETY: a live configuration and two NUL-terminated strings,
            // copied by the call.
            (unsafe { set(config, key.as_ptr(), value.as_ptr()) } == 0)
                .then_some(())
                .ok_or(())
        };
        let mut status = set_text(c"home", home)
            .and_then(|()| set_text(c"program_name", program))
            .and_then(|()| set_text(c"executable", executable))
            .map_err(|()| String::new());
        if status.is_ok() {
            // SAFETY: the configuration is complete; this thread then holds the GIL.
            if unsafe { init(config) } != 0 {
                status = Err(String::new());
            }
        }
        let status = status.map_err(|_| {
            let mut message: *const c_char = core::ptr::null();
            // SAFETY: the live configuration; the message it points at
            // lives as long as the configuration.
            if unsafe { error(config, &mut message) } == 1 && !message.is_null() {
                // SAFETY: NUL-terminated, as above.
                unsafe { CStr::from_ptr(message) }
                    .to_string_lossy()
                    .into_owned()
            } else {
                "Python did not start".to_string()
            }
        });
        // SAFETY: made above, freed once.
        unsafe { free(config) };
        status.map_err(|e| format!("Python did not start: {e}"))?;
    } else {
        let (Some(isolated), Some(init), Some(clear)) = (
            lookup(c"PyConfig_InitIsolatedConfig"),
            lookup(c"Py_InitializeFromConfig"),
            lookup(c"PyConfig_Clear"),
        ) else {
            return Err("this Python has no isolated configuration (3.9 or later)".to_string());
        };
        for (name, value) in [(c"Py_SetPythonHome", home), (c"Py_SetProgramName", program)] {
            let Some(value) = value else { continue };
            let Some(setter) = lookup(name) else {
                return Err(format!("this Python has no `{}`", name.to_string_lossy()));
            };
            // SAFETY: CPython's `void (*)(const wchar_t *)`, 3.9 to 3.14.
            let setter = unsafe {
                core::mem::transmute::<*mut c_void, unsafe extern "C" fn(*const WChar)>(setter)
            };
            // SAFETY: a NUL-terminated wide string kept for the process.
            unsafe { setter(leaked_wide(value)) };
        }
        // SAFETY: CPython's own functions, with these signatures
        // (`cpython/initconfig.h`, `pylifecycle.h`).
        let (isolated, init, clear) = unsafe {
            (
                core::mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut c_void)>(isolated),
                core::mem::transmute::<*mut c_void, unsafe extern "C" fn(*const c_void) -> PyStatus>(
                    init,
                ),
                core::mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut c_void)>(clear),
            )
        };
        let mut buffer = Box::new(ConfigBuffer([0; 8192]));
        let config = buffer.0.as_mut_ptr().cast::<c_void>();
        // SAFETY: an aligned buffer larger than this minor's `PyConfig`,
        // initialised, used and cleared here.
        let status = unsafe {
            isolated(config);
            let status = init(config);
            clear(config);
            status
        };
        if status.kind != 0 {
            let message = if status.err_msg.is_null() {
                "Python did not start".to_string()
            } else {
                // SAFETY: CPython's static NUL-terminated message.
                unsafe { CStr::from_ptr(status.err_msg) }
                    .to_string_lossy()
                    .into_owned()
            };
            return Err(format!("Python did not start: {message}"));
        }
    }
    // SAFETY: the interpreter started on this thread, which holds the GIL.
    unsafe { pyo3::ffi::PyEval_SaveThread() };
    Ok(())
}

/// A function of the loaded libpython, by name: the versioned library,
/// which exports what the stable ABI's `python3.dll` does not forward.
fn lookup(name: &CStr) -> Option<*mut c_void> {
    let found = os::lookup(name);
    (!found.is_null()).then_some(found)
}

/// Where an `isolated` interpreter's standard library is, when the
/// configuration names no `home`: the directory of the libpython this
/// file runs, or above its `lib` directory.
fn python_home() -> Option<String> {
    let library = std::path::PathBuf::from(os::python_file()?);
    let dir = library.parent()?;
    let lib = dir
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with("lib"));
    let home = if cfg!(windows) || !lib {
        dir
    } else {
        dir.parent()?
    };
    home.to_str().map(str::to_string)
}

#[cfg(windows)]
mod os {
    use core::ffi::{CStr, c_char, c_void};

    const FROM_ADDRESS: u32 = 0x4;
    const UNCHANGED_REFCOUNT: u32 = 0x2;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
        fn GetModuleHandleExW(flags: u32, address: *const c_void, module: *mut *mut c_void) -> i32;
        fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
        fn GetModuleFileNameW(module: *mut c_void, name: *mut u16, size: u32) -> u32;
    }

    /// `python3XX.dll`: what `python3.dll`'s `Py_IsInitialized` forwards
    /// to lives there.
    fn versioned() -> *mut c_void {
        let name: Vec<u16> = "python3.dll\0".encode_utf16().collect();
        // SAFETY: a NUL-terminated name; the stable ABI's library is
        // loaded, as this file imports it.
        let stable = unsafe { GetModuleHandleW(name.as_ptr()) };
        if stable.is_null() {
            return core::ptr::null_mut();
        }
        // SAFETY: a loaded module and a NUL-terminated name; a forwarded
        // export answers the address it forwards to.
        let address = unsafe { GetProcAddress(stable, c"Py_IsInitialized".as_ptr()) };
        if address.is_null() {
            return core::ptr::null_mut();
        }
        let mut module = core::ptr::null_mut();
        // SAFETY: an address inside a loaded module, a writable slot; the
        // reference count is left as it is.
        if unsafe { GetModuleHandleExW(FROM_ADDRESS | UNCHANGED_REFCOUNT, address, &mut module) }
            == 0
        {
            return core::ptr::null_mut();
        }
        module
    }

    pub(super) fn lookup(name: &CStr) -> *mut c_void {
        let module = versioned();
        if module.is_null() {
            return module;
        }
        // SAFETY: a loaded module and a NUL-terminated name.
        unsafe { GetProcAddress(module, name.as_ptr()) }
    }

    pub(super) fn python_file() -> Option<String> {
        let module = versioned();
        if module.is_null() {
            return None;
        }
        let mut name = vec![0u16; 32 * 1024];
        // SAFETY: a loaded module and a buffer of the length given.
        let len =
            unsafe { GetModuleFileNameW(module, name.as_mut_ptr(), name.len() as u32) } as usize;
        (len > 0 && len < name.len()).then(|| String::from_utf16_lossy(&name[..len]))
    }
}

#[cfg(unix)]
mod os {
    use core::ffi::{CStr, c_void};

    pub(super) fn lookup(name: &CStr) -> *mut c_void {
        // SAFETY: looks a name up in what the process has loaded;
        // libpython's symbols are global (`exports`' `share_python_symbols`).
        unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) }
    }

    pub(super) fn python_file() -> Option<String> {
        crate::exports::python_path()
    }
}

#[cfg(not(any(windows, unix)))]
mod os {
    use core::ffi::{CStr, c_void};

    pub(super) fn lookup(_: &CStr) -> *mut c_void {
        core::ptr::null_mut()
    }

    pub(super) fn python_file() -> Option<String> {
        None
    }
}
