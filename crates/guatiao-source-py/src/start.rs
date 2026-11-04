// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Starting the interpreter under the configuration's `isolation`.
//!
//! The build links the stable ABI (`abi3`), which has no `PyConfig`, and
//! `PyConfig`'s layout changes between minors. So nothing here reads or
//! writes one of its fields. From 3.14, `PyInitConfig_*` names each setting
//! by its key. Before, `shipped` and `isolated` start from CPython's
//! isolated configuration, made by `PyConfig_InitIsolatedConfig` in a
//! buffer larger than any minor's, with `home`, the program's name and the
//! module search path given through `Py_SetPythonHome`, `Py_SetProgramName`
//! and `Py_SetPath`, which that configuration reads when it has none of its
//! own; and `inherit` is `Py_InitializeEx(0)`. The functions are found in
//! the loaded libpython at run time.

use core::ffi::{CStr, c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use pyo3::Python;
use pyo3::types::PyAnyMethods;

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
#[derive(Clone, Debug)]
pub(crate) struct Settings {
    pub(crate) isolation: Isolation,
    /// Whether the configuration named the isolation, or left the default.
    pub(crate) asked: bool,
    /// The Python installation, when the configuration names one.
    pub(crate) home: Option<String>,
}

impl Settings {
    fn same(&self, other: &Settings) -> bool {
        self.isolation == other.isolation && self.home == other.home
    }
}

/// Who started the one interpreter, or why it is not running.
enum Started {
    /// It was running before this file was asked: the host is Python, or
    /// another copy of this file started it.
    Running,
    /// This file, under these settings.
    Here(Settings),
    /// This file tried and CPython refused, in these words. What it left
    /// behind cannot be started again.
    Failed(String),
}

static STARTED: Mutex<Option<Started>> = Mutex::new(None);

/// How the interpreter was had.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Got {
    /// This file started it under the settings asked.
    Started,
    /// It was running before this file was asked, and is used as it is;
    /// `environment` when it does not ignore its environment.
    Joined { environment: bool },
}

/// Why the interpreter could not be had as asked.
pub(crate) enum StartError {
    /// It runs, under other settings than these.
    Started(String),
    /// It did not start, or cannot here.
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

/// Starts the interpreter under `settings`, or joins one already running.
/// A second source asking for other settings than the interpreter started
/// with is refused: there is one interpreter for the program. A start that
/// failed answers every later one.
pub(crate) fn start(settings: &Settings) -> Result<Got, StartError> {
    let mut started = STARTED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match &*started {
        Some(Started::Failed(why)) => {
            return Err(StartError::Failed(format!(
                "Python did not start earlier in this process: {why}"
            )));
        }
        Some(Started::Running) => return joined(settings),
        Some(Started::Here(first)) if first.same(settings) => return Ok(Got::Started),
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
        *started = Some(Started::Running);
        return joined(settings);
    }
    let minor = minor().ok_or("the version of the loaded Python cannot be read")?;
    if minor < 9 {
        return Err(StartError::Failed(format!(
            "this file needs CPython 3.9 or later, and the one loaded is 3.{minor}"
        )));
    }
    refuse_what_cannot_be_kept_out(settings, minor)?;
    let plan = plan(settings, minor)?;
    match initialize(&plan) {
        Ok(()) => {
            // Joins what is now initialised; starts nothing.
            Python::initialize();
            if let (Some(_), Some(home)) = (&plan.search, &plan.home) {
                name_the_prefix(home);
            }
            *started = Some(Started::Here(settings.clone()));
            Ok(Got::Started)
        }
        Err(why) => {
            *started = Some(Started::Failed(why.clone()));
            Err(StartError::Failed(format!("Python did not start: {why}")))
        }
    }
}

/// Joins an interpreter running before this file was asked, which is used
/// as it is: refused when the configuration asked for `isolated`, or named
/// `shipped`, and its flags do not say so.
fn joined(settings: &Settings) -> Result<Got, StartError> {
    let read = |flag: &str| {
        Python::attach(|py| -> pyo3::PyResult<i64> {
            py.import("sys")?.getattr("flags")?.getattr(flag)?.extract()
        })
        .map_err(|e| StartError::Failed(format!("the running interpreter's flags: {e}")))
    };
    let got = Got::Joined {
        environment: read("ignore_environment")? == 0,
    };
    let (flag, needs) = match settings.isolation {
        Isolation::Isolated => ("isolated", true),
        Isolation::Shipped if settings.asked => ("ignore_environment", true),
        _ => return Ok(got),
    };
    let set = read(flag)?;
    if (set != 0) == needs {
        return Ok(got);
    }
    Err(StartError::Started(format!(
        "asks for `{}`, and the interpreter already running in this process does not hold to it \
         (sys.flags.{flag} is {set}); it is used as it is under `inherit`",
        settings.isolation.name(),
    )))
}

/// Before 3.14 a module search path given with `Py_SetPath` leaves
/// `sys.prefix` and its kin empty: they name `home`, which the standard
/// library on that path is.
fn name_the_prefix(home: &str) {
    let _ = Python::attach(|py| -> pyo3::PyResult<()> {
        let sys = py.import("sys")?;
        for name in ["prefix", "exec_prefix", "base_prefix", "base_exec_prefix"] {
            if sys.getattr(name)?.extract::<String>()?.is_empty() {
                sys.setattr(name, home)?;
            }
        }
        Ok(())
    });
}

/// Refuses, before CPython is touched, what this Python would read whatever
/// it is told. On Windows before 3.14 CPython takes the executable from the
/// system and `site` reads a `pyvenv.cfg` beside it or one directory up;
/// on macOS `site` takes `sys.prefix` from `__PYVENV_LAUNCHER__`.
fn refuse_what_cannot_be_kept_out(settings: &Settings, minor: u32) -> Result<(), StartError> {
    if settings.isolation == Isolation::Inherit {
        return Ok(());
    }
    if cfg!(target_os = "macos") && std::env::var_os("__PYVENV_LAUNCHER__").is_some() {
        return Err(StartError::Failed(format!(
            "the isolation `{}` cannot keep `__PYVENV_LAUNCHER__` out on macOS, where `site` \
             takes `sys.prefix` from it: unset it, or use `inherit`",
            settings.isolation.name(),
        )));
    }
    if settings.isolation == Isolation::Isolated && cfg!(windows) && minor < 14 {
        let unheld = venv_files_beside_program();
        if !unheld.is_empty() {
            return Err(StartError::Failed(format!(
                "the isolation `isolated` cannot keep {} out on CPython 3.{minor} under Windows, \
                 where `site` reads it beside the program before 3.14: remove it, or run a later \
                 Python",
                unheld.join(" and "),
            )));
        }
    }
    Ok(())
}

/// Each `pyvenv.cfg` beside the program or one directory up.
fn venv_files_beside_program() -> Vec<String> {
    let Ok(exe) = std::env::current_exe() else {
        return Vec::new();
    };
    let Some(dir) = exe.parent() else {
        return Vec::new();
    };
    [Some(dir), dir.parent()]
        .into_iter()
        .flatten()
        .map(|d| d.join("pyvenv.cfg"))
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

/// What `initialize` is given.
struct Plan {
    /// `inherit`: the environment's configuration.
    inherit: bool,
    minor: u32,
    home: Option<String>,
    program: Option<String>,
    /// The executable CPython reads files beside (3.14 and later).
    executable: Option<String>,
    /// The whole module search path, when it is the configuration's: the
    /// standard library of `home`, with no `site` (before 3.14 `site` finds
    /// nothing to add, `sys.prefix` being empty until it is named).
    search: Option<Vec<String>>,
}

fn plan(settings: &Settings, minor: u32) -> Result<Plan, StartError> {
    // CPython 3.9 and 3.10 on Windows find no standard library with no
    // `home` from the isolated configuration, nor from the environment's
    // with any `PYTHON*` variable set; they read the program's `._pth`
    // and `site` its `pyvenv.cfg` with one.
    let old_windows = cfg!(windows) && minor < 11;
    let libpython_home = || {
        python_home().ok_or("no `home` was given and libpython's own directory could not be found")
    };
    let home = match (&settings.home, settings.isolation) {
        (Some(home), _) => Some(home.clone()),
        (None, Isolation::Isolated) => Some(libpython_home()?),
        (None, Isolation::Shipped) if old_windows => Some(libpython_home()?),
        (None, Isolation::Inherit) if old_windows && std::env::var_os("PYTHONHOME").is_none() => {
            Some(libpython_home()?)
        }
        (None, _) => None,
    };
    Ok(match (settings.isolation, &home) {
        (Isolation::Isolated, Some(home)) => {
            let program = home_program(home);
            Plan {
                inherit: false,
                minor,
                home: Some(home.clone()),
                program: Some(program.clone()),
                executable: Some(program),
                search: Some(standard_library(Path::new(home), minor)),
            }
        }
        (isolation, _) => Plan {
            inherit: isolation == Isolation::Inherit,
            minor,
            home,
            program: if isolation == Isolation::Inherit {
                None
            } else {
                host_program()
            },
            executable: None,
            search: None,
        },
    })
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
    let home = Path::new(home);
    let exe = if cfg!(windows) {
        home.join("python.exe")
    } else {
        home.join("bin").join("python3")
    };
    exe.display().to_string()
}

/// The standard library of the installation at `home`, as CPython lays it
/// out: the zip, `DLLs` and `Lib` on Windows; elsewhere the zip,
/// `lib/python3.X` and its `lib-dynload`, under `lib64` where that holds it.
fn standard_library(home: &Path, minor: u32) -> Vec<String> {
    let paths: Vec<PathBuf> = if cfg!(windows) {
        vec![
            home.join(format!("python3{minor}.zip")),
            home.join("DLLs"),
            home.join("Lib"),
        ]
    } else {
        let version = format!("python3.{minor}");
        let lib = ["lib64", "lib"]
            .into_iter()
            .find(|l| home.join(l).join(&version).is_dir())
            .unwrap_or("lib");
        vec![
            home.join("lib").join(format!("python3{minor}.zip")),
            home.join(lib).join(&version),
            home.join(lib).join(&version).join("lib-dynload"),
        ]
    };
    paths.iter().map(|p| p.display().to_string()).collect()
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

/// The separator of a module search path.
const DELIMITER: &str = if cfg!(windows) { ";" } else { ":" };

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

/// A function of the loaded libpython, transmuted to the signature its
/// header gives it.
///
/// # Safety
///
/// `F` is that signature.
unsafe fn function<F: Copy>(name: &CStr) -> Option<F> {
    let found = lookup(name)?;
    // SAFETY: the caller's contract; a function pointer is one word.
    Some(unsafe { core::mem::transmute_copy::<*mut c_void, F>(&found) })
}

/// Starts CPython as `plan` says, and answers CPython's words when it does
/// not. Leaves the calling thread without the GIL, as `Py_InitializeEx`
/// under pyo3 does.
fn initialize(plan: &Plan) -> Result<(), String> {
    if plan.minor >= 14 {
        initialize_3_14(plan)?;
    } else if plan.inherit {
        if let Some(home) = &plan.home {
            set_global(c"Py_SetPythonHome", home)?;
        }
        // SAFETY: not yet initialised; CPython ends the process where it
        // cannot start, which before 3.14 nothing here can ask otherwise.
        unsafe { pyo3::ffi::Py_InitializeEx(0) };
    } else {
        initialize_isolated(plan)?;
    }
    // SAFETY: the interpreter started on this thread, which holds the GIL.
    unsafe { pyo3::ffi::PyEval_SaveThread() };
    Ok(())
}

/// Hands one of the process-wide settings the isolated configuration reads
/// before 3.14.
fn set_global(name: &CStr, value: &str) -> Result<(), String> {
    // SAFETY: CPython's `void (*)(const wchar_t *)`, 3.9 to 3.14.
    let setter = unsafe { function::<unsafe extern "C" fn(*const WChar)>(name) }
        .ok_or_else(|| format!("this Python has no `{}`", name.to_string_lossy()))?;
    // SAFETY: a NUL-terminated wide string kept for the process.
    unsafe { setter(leaked_wide(value)) };
    Ok(())
}

/// 3.14 and later: every setting named by its key, the configuration
/// starting from the isolated one (`inherit` turns the environment back on).
fn initialize_3_14(plan: &Plan) -> Result<(), String> {
    type Create = unsafe extern "C" fn() -> *mut PyInitConfig;
    type SetStr = unsafe extern "C" fn(*mut PyInitConfig, *const c_char, *const c_char) -> c_int;
    type SetInt = unsafe extern "C" fn(*mut PyInitConfig, *const c_char, i64) -> c_int;
    type SetStrList = unsafe extern "C" fn(
        *mut PyInitConfig,
        *const c_char,
        usize,
        *const *const c_char,
    ) -> c_int;
    type Init = unsafe extern "C" fn(*mut PyInitConfig) -> c_int;
    type GetError = unsafe extern "C" fn(*mut PyInitConfig, *mut *const c_char) -> c_int;
    type Free = unsafe extern "C" fn(*mut PyInitConfig);
    // SAFETY: CPython 3.14's functions, with these signatures
    // (`cpython/initconfig.h`).
    let found = unsafe {
        (
            function::<Create>(c"PyInitConfig_Create"),
            function::<SetStr>(c"PyInitConfig_SetStr"),
            function::<SetInt>(c"PyInitConfig_SetInt"),
            function::<SetStrList>(c"PyInitConfig_SetStrList"),
            function::<Init>(c"Py_InitializeFromInitConfig"),
            function::<GetError>(c"PyInitConfig_GetError"),
            function::<Free>(c"PyInitConfig_Free"),
        )
    };
    let (
        Some(create),
        Some(set_str),
        Some(set_int),
        Some(set_list),
        Some(init),
        Some(error),
        Some(free),
    ) = found
    else {
        return Err("this Python has no `PyInitConfig` functions".to_string());
    };
    // SAFETY: creates a configuration this function frees below.
    let config = unsafe { create() };
    if config.is_null() {
        return Err("Python could not make its configuration".to_string());
    }
    let text = |key: &CStr, value: Option<&str>| -> Result<(), ()> {
        let Some(value) = value else { return Ok(()) };
        let value = alloc::ffi::CString::new(value).map_err(|_| ())?;
        // SAFETY: a live configuration and two NUL-terminated strings,
        // copied by the call.
        (unsafe { set_str(config, key.as_ptr(), value.as_ptr()) } == 0)
            .then_some(())
            .ok_or(())
    };
    let int = |key: &CStr, value: i64| -> Result<(), ()> {
        // SAFETY: a live configuration and a NUL-terminated key.
        (unsafe { set_int(config, key.as_ptr(), value) } == 0)
            .then_some(())
            .ok_or(())
    };
    let list = |key: &CStr, values: &[String]| -> Result<(), ()> {
        let owned: Vec<alloc::ffi::CString> = values
            .iter()
            .map(|v| alloc::ffi::CString::new(v.as_str()))
            .collect::<Result<_, _>>()
            .map_err(|_| ())?;
        let pointers: Vec<*const c_char> = owned.iter().map(|v| v.as_ptr()).collect();
        // SAFETY: a live configuration, a NUL-terminated key and that many
        // NUL-terminated strings, copied by the call.
        (unsafe { set_list(config, key.as_ptr(), pointers.len(), pointers.as_ptr()) } == 0)
            .then_some(())
            .ok_or(())
    };
    let mut status = Ok(());
    if plan.inherit {
        // The environment's configuration, without its signal handlers, as
        // `Py_InitializeEx(0)` starts it.
        status = status
            .and_then(|()| int(c"isolated", 0))
            .and_then(|()| int(c"use_environment", 1))
            .and_then(|()| int(c"user_site_directory", 1))
            .and_then(|()| int(c"safe_path", 0))
            .and_then(|()| int(c"pathconfig_warnings", 1));
    }
    status = status
        .and_then(|()| text(c"home", plan.home.as_deref()))
        .and_then(|()| text(c"program_name", plan.program.as_deref()))
        .and_then(|()| text(c"executable", plan.executable.as_deref()));
    if let Some(search) = &plan.search {
        // Exactly that path: no `site`, which would add the installation's
        // site-packages and run a `sitecustomize` found there.
        status = status
            .and_then(|()| list(c"module_search_paths", search))
            .and_then(|()| int(c"site_import", 0));
    }
    if status.is_ok() {
        // SAFETY: the configuration is complete; this thread then holds the GIL.
        if unsafe { init(config) } != 0 {
            status = Err(());
        }
    }
    let status = status.map_err(|()| {
        let mut message: *const c_char = core::ptr::null();
        // SAFETY: the live configuration; the message it points at lives
        // as long as the configuration.
        if unsafe { error(config, &mut message) } == 1 && !message.is_null() {
            // SAFETY: NUL-terminated, as above.
            unsafe { CStr::from_ptr(message) }
                .to_string_lossy()
                .into_owned()
        } else {
            "CPython gave no reason".to_string()
        }
    });
    // SAFETY: made above, freed once.
    unsafe { free(config) };
    status
}

/// Before 3.14: the isolated configuration, with `home`, the program's name
/// and the module search path through the process-wide setters it reads.
fn initialize_isolated(plan: &Plan) -> Result<(), String> {
    type InitIsolated = unsafe extern "C" fn(*mut c_void);
    type Init = unsafe extern "C" fn(*const c_void) -> PyStatus;
    type Clear = unsafe extern "C" fn(*mut c_void);
    // SAFETY: CPython's own functions, with these signatures
    // (`cpython/initconfig.h`, `pylifecycle.h`).
    let found = unsafe {
        (
            function::<InitIsolated>(c"PyConfig_InitIsolatedConfig"),
            function::<Init>(c"Py_InitializeFromConfig"),
            function::<Clear>(c"PyConfig_Clear"),
        )
    };
    let (Some(isolated), Some(init), Some(clear)) = found else {
        return Err("this Python has no isolated configuration".to_string());
    };
    if let Some(home) = &plan.home {
        set_global(c"Py_SetPythonHome", home)?;
    }
    if let Some(program) = &plan.program {
        set_global(c"Py_SetProgramName", program)?;
    }
    if let Some(search) = &plan.search {
        set_global(c"Py_SetPath", &search.join(DELIMITER))?;
    }
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
    if status.kind == 0 {
        return Ok(());
    }
    Err(if status.err_msg.is_null() {
        "CPython gave no reason".to_string()
    } else {
        // SAFETY: CPython's static NUL-terminated message.
        unsafe { CStr::from_ptr(status.err_msg) }
            .to_string_lossy()
            .into_owned()
    })
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
