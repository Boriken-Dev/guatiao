// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Windows: which libraries a file imports that the narrow search leaves
//! missing, and mapping each from where a wider search finds it, so that
//! only those come from there.
//!
//! The narrow search (`LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR |
//! LOAD_LIBRARY_SEARCH_DEFAULT_DIRS`) looks beside the file, beside the
//! host's executable, in the system directory and in directories the
//! process added; a library already loaded answers its name. A file it
//! refuses with error 126 is read here (its import table, never mapped),
//! and each direct import none of those places holds is named. Mapping
//! such an import first, by its full path, makes it the one the file's
//! import binds to: a library already loaded answers its name. A library
//! mapped that way forwards exports to others (`python3.dll` to
//! `python3XX.dll`), which are bound only when the file is, under the
//! file's search, so those beside it are mapped first as well.

use std::path::{Path, PathBuf};

use libloading::os::windows::{
    LOAD_LIBRARY_AS_DATAFILE, LOAD_LIBRARY_AS_IMAGE_RESOURCE, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, Library,
};

/// The search every file is opened with first.
pub(super) const NARROW: u32 = LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
    fn GetWindowsDirectoryW(buffer: *mut u16, size: u32) -> u32;
    fn GetModuleFileNameW(module: *mut core::ffi::c_void, name: *mut u16, size: u32) -> u32;
}

/// A directory one of the two calls answers.
fn system_path(call: unsafe extern "system" fn(*mut u16, u32) -> u32) -> Option<PathBuf> {
    let mut buffer = vec![0u16; 1024];
    // SAFETY: a writable buffer of the length given.
    let len = unsafe { call(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    (len > 0 && len < buffer.len()).then(|| PathBuf::from(String::from_utf16_lossy(&buffer[..len])))
}

/// The libraries `file` names in its import table, each once, in order.
/// Delay-loaded imports are not there, and none for a file that does not
/// parse.
fn direct_imports(file: &Path) -> Vec<String> {
    use object::Object;
    let Ok(data) = std::fs::read(file) else {
        return Vec::new();
    };
    let Ok(parsed) = object::File::parse(&*data) else {
        return Vec::new();
    };
    let Ok(imports) = parsed.imports() else {
        return Vec::new();
    };
    let mut names: Vec<String> = Vec::new();
    for import in imports.into_iter().flatten() {
        let name = String::from_utf8_lossy(import.library()).into_owned();
        if plain(&name) && !names.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
            names.push(name);
        }
    }
    names
}

/// The libraries `file`'s exports forward to, by file name, each once.
fn forwarded(file: &Path) -> Vec<String> {
    use object::read::pe::{ExportTarget, ImageNtHeaders, PeFile};
    fn targets<Pe: ImageNtHeaders>(data: &[u8]) -> Vec<Vec<u8>> {
        let Ok(pe) = PeFile::<Pe>::parse(data) else {
            return Vec::new();
        };
        let Ok(Some(table)) = pe.export_table() else {
            return Vec::new();
        };
        let Ok(exports) = table.exports() else {
            return Vec::new();
        };
        exports
            .into_iter()
            .filter_map(|e| match e.target {
                ExportTarget::ForwardByName(library, _)
                | ExportTarget::ForwardByOrdinal(library, _) => Some(library.to_vec()),
                ExportTarget::Address(_) => None,
            })
            .collect()
    }
    let Ok(data) = std::fs::read(file) else {
        return Vec::new();
    };
    let found = match object::FileKind::parse(&*data) {
        Ok(object::FileKind::Pe32) => targets::<object::pe::ImageNtHeaders32>(&data),
        Ok(object::FileKind::Pe64) => targets::<object::pe::ImageNtHeaders64>(&data),
        _ => Vec::new(),
    };
    let mut names: Vec<String> = Vec::new();
    for library in found {
        let mut name = String::from_utf8_lossy(&library).into_owned();
        if !name.contains('.') {
            name.push_str(".dll");
        }
        if plain(&name) && !names.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
            names.push(name);
        }
    }
    names
}

/// Where the library loaded under `name` was mapped from, if one is.
fn loaded_from(name: &str) -> Option<PathBuf> {
    let handle = Library::open_already_loaded(name).ok()?.into_raw();
    let mut buffer = vec![0u16; 32 * 1024];
    // SAFETY: a module this handle holds loaded, and a buffer of the
    // length given.
    let len = unsafe {
        GetModuleFileNameW(
            handle as *mut core::ffi::c_void,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
        )
    } as usize;
    // SAFETY: the handle `into_raw` gave, given back once to be let go.
    drop(unsafe { Library::from_raw(handle) });
    (len > 0 && len < buffer.len()).then(|| PathBuf::from(String::from_utf16_lossy(&buffer[..len])))
}

/// Whether `name` is a file name and nothing more: a name a file records
/// with a directory or a drive in it would be looked for there instead of
/// where the search goes.
fn plain(name: &str) -> bool {
    !name.is_empty() && !name.contains(['\\', '/', ':'])
}

/// Whether a library of this name is already loaded, or is an API set the
/// system answers itself.
fn answered(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("api-ms-") || lower.starts_with("ext-ms-") {
        return true;
    }
    Library::open_already_loaded(name).is_ok()
}

/// Whether the loader finds `name` beside the host, in a directory the
/// process added or in the system directory, asked for it as data: mapped
/// as a resource, which runs nothing, and let go at once.
fn found_by_loader(name: &str) -> bool {
    // SAFETY: a data file's mapping runs no code of it.
    unsafe {
        Library::load_with_flags(
            name,
            LOAD_LIBRARY_AS_IMAGE_RESOURCE
                | LOAD_LIBRARY_AS_DATAFILE
                | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
        )
    }
    .is_ok()
}

/// What `file` imports directly that is not already loaded, not beside it
/// and not where the loader's default directories (the host's, those the
/// process added, the system's) hold it.
pub(super) fn missing(file: &Path) -> Vec<String> {
    let mut places: Vec<PathBuf> = Vec::new();
    places.extend(file.parent().map(Path::to_path_buf));
    places.extend(
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf)),
    );
    places.extend(system_path(GetSystemDirectoryW));
    direct_imports(file)
        .into_iter()
        .filter(|name| {
            !answered(name)
                && !places.iter().any(|dir| dir.join(name).is_file())
                && !found_by_loader(name)
        })
        .collect()
}

/// Where a missing import is looked for, in order: `dirs`, then, `wide`,
/// where Windows' standard search goes after the system directory: the
/// 16-bit system directory, the Windows directory, the current directory
/// and `PATH`.
pub(super) fn places(dirs: &[PathBuf], wide: bool) -> Vec<PathBuf> {
    let mut places = dirs.to_vec();
    if wide {
        if let Some(windows) = system_path(GetWindowsDirectoryW) {
            places.push(windows.join("System"));
            places.push(windows);
        }
        places.extend(std::env::current_dir().ok());
        if let Some(path) = std::env::var_os("PATH") {
            places.extend(std::env::split_paths(&path).filter(|p| p.is_absolute()));
        }
    }
    places
}

/// Why the missing imports could not be mapped first.
pub(super) enum Preload {
    /// None of the places holds these.
    Unfound(Vec<String>),
    /// The loader refused one, in its words.
    Refused(String),
}

/// Maps each of `missing` from the first of `places` holding it, by its
/// full path and under the narrow search, and what each forwards to from
/// beside it; nothing is mapped unless every one is found. What a library
/// `file` imports that is already loaded forwards to is mapped from beside
/// it too, as another thread may be mapping it now. The handles answered
/// keep them mapped; dropped, they are let go.
///
/// # Safety
///
/// Mapping runs a library's initialisers: the caller names files it is
/// willing to run, and these are what such a file imports.
pub(super) unsafe fn preload(
    file: &Path,
    missing: &[String],
    places: &[PathBuf],
    words: impl Fn(&libloading::Error) -> String,
) -> Result<Vec<Library>, Preload> {
    let mut files = Vec::new();
    for name in direct_imports(file) {
        let Some(loaded) = loaded_from(&name) else {
            continue;
        };
        let Some(dir) = loaded.parent() else {
            continue;
        };
        for target in forwarded(&loaded) {
            let beside = dir.join(&target);
            if !answered(&target) && beside.is_file() {
                files.push(beside);
            }
        }
    }
    let mut unfound = Vec::new();
    for name in missing {
        match places
            .iter()
            .map(|dir| dir.join(name))
            .find(|f| f.is_file())
        {
            Some(file) => files.push(file),
            None => unfound.push(name.clone()),
        }
    }
    if !unfound.is_empty() {
        return Err(Preload::Unfound(unfound));
    }
    let mut mapped = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();
    while let Some(file) = files.pop() {
        if seen.contains(&file) {
            continue;
        }
        seen.push(file.clone());
        // SAFETY: the caller's contract.
        match unsafe { Library::load_with_flags(&file, NARROW) } {
            Ok(library) => mapped.push(library),
            Err(e) => {
                return Err(Preload::Refused(format!(
                    "{}: {}",
                    file.display(),
                    words(&e)
                )));
            }
        }
        if let Some(dir) = file.parent() {
            for name in forwarded(&file) {
                let beside = dir.join(&name);
                if !answered(&name) && beside.is_file() {
                    files.push(beside);
                }
            }
        }
    }
    Ok(mapped)
}

#[cfg(test)]
mod tests {
    use super::plain;

    #[test]
    fn only_a_bare_file_name_is_looked_for() {
        assert!(plain("python3.dll"));
        assert!(plain("api-ms-win-crt-runtime-l1-1-0.dll"));
        for name in [
            "",
            r"C:\evil\x.dll",
            "..\\x.dll",
            "sub/x.dll",
            "D:x.dll",
            r"\\host\share\x.dll",
        ] {
            assert!(!plain(name), "{name}");
        }
    }
}
