// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Manifests: what a Python library is, read without importing it.
//!
//! ```toml
//! module = "py_greeter"                # the module, which is the library: its id
//! version = "1.0.0"                    # optional: else its literal `__version__`
//! kinds = ["greeter", "counter"]       # every kind its providers serve
//! paths = ["."]                        # put on sys.path, relative to this file
//! ```

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use guatiao::value::convert::TryAsRef;
use guatiao::{Alloc, List, Map};

/// The suffix a manifest's file name ends with.
pub(crate) const SUFFIX: &str = ".guatiao-py.toml";

/// The most `.py` files a stamp looks at under one library's paths, and
/// how deep it walks: a library is a package, not a disk.
const MOST_FILES: usize = 10_000;
const DEEPEST: usize = 8;

/// One library a manifest describes.
#[derive(Debug, Clone)]
pub(crate) struct Manifest {
    /// The module, and so the library's id.
    pub(crate) id: String,
    /// `None` when the manifest leaves it to the module's `__version__`.
    pub(crate) version: Option<String>,
    pub(crate) kinds: Vec<String>,
    /// Absolute, resolved against the manifest's directory.
    pub(crate) paths: Vec<String>,
    /// The manifest's content and the newest `.py` file under its paths.
    pub(crate) stamp: String,
}

/// What reading one manifest came to: a library, or why not.
#[derive(Debug, Clone)]
pub(crate) enum Found {
    Library(Manifest),
    /// Listed under the manifest's path, so opening it reports why.
    Broken {
        path: PathBuf,
        stamp: String,
        why: String,
    },
}

impl Found {
    pub(crate) fn id(&self) -> String {
        match self {
            Found::Library(m) => m.id.clone(),
            Found::Broken { path, .. } => path.display().to_string(),
        }
    }
}

/// Every manifest in `dirs` (and their immediate subdirectories) and in
/// `files`, each once, in path order.
pub(crate) fn find(dirs: &[PathBuf], files: &[PathBuf]) -> Vec<Found> {
    let mut paths: Vec<PathBuf> = files.to_vec();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if is_manifest(&path) {
                paths.push(path);
            } else if path.is_dir()
                && let Ok(inner) = std::fs::read_dir(&path)
            {
                paths.extend(
                    inner
                        .filter_map(Result::ok)
                        .map(|e| e.path())
                        .filter(|p| is_manifest(p)),
                );
            }
        }
    }
    let mut seen = Vec::new();
    paths.retain(|p| {
        let real = p.canonicalize().unwrap_or_else(|_| p.clone());
        if seen.contains(&real) {
            false
        } else {
            seen.push(real);
            true
        }
    });
    paths.sort();
    paths.iter().map(|p| read(p)).collect()
}

fn is_manifest(path: &Path) -> bool {
    path.is_file()
        && path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(SUFFIX))
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn modified(path: &Path) -> u128 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// The newest modification time of a `.py` file under `root`.
fn newest_py(root: &Path, depth: usize, budget: &mut usize, newest: &mut u128) {
    if depth > DEEPEST || *budget == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "__pycache__") {
                continue;
            }
            newest_py(&path, depth + 1, budget, newest);
        } else if path.extension().is_some_and(|e| e == "py") {
            *budget = budget.saturating_sub(1);
            *newest = (*newest).max(modified(&path));
            if *budget == 0 {
                return;
            }
        }
    }
}

fn text_list(map: &Map, key: &str) -> Result<Vec<String>, String> {
    match map.get(key) {
        None => Ok(Vec::new()),
        Some(value) => {
            let list: &List = TryAsRef::<List>::try_as_ref(value)
                .ok_or_else(|| format!("`{key}` is not a list"))?;
            list.iter()
                .map(|v| {
                    TryAsRef::<str>::try_as_ref(v)
                        .map(str::to_string)
                        .ok_or_else(|| format!("`{key}` holds something that is not text"))
                })
                .collect()
        }
    }
}

/// Reads one manifest. Imports nothing.
pub(crate) fn read(path: &Path) -> Found {
    let broken = |stamp: String, why: String| Found::Broken {
        path: path.to_path_buf(),
        stamp,
        why: format!("{}: {why}", path.display()),
    };
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => return broken(format!("{}", modified(path)), e.to_string()),
    };
    let file_stamp = format!("{:016x}", fnv1a(&bytes));
    let text = match core::str::from_utf8(&bytes) {
        Ok(text) => text,
        Err(_) => return broken(file_stamp, "not UTF-8".to_string()),
    };
    let value = match guatiao_serde::text::toml::from_str(
        text,
        Alloc::rust(),
        guatiao_serde::Presentation::new(),
    ) {
        Ok(value) => value,
        Err(e) => return broken(file_stamp, e.to_string()),
    };
    let Some(map) = TryAsRef::<Map>::try_as_ref(&value) else {
        return broken(file_stamp, "not a table".to_string());
    };
    let text_field = |key: &str| -> Result<String, String> {
        map.get(key)
            .and_then(TryAsRef::<str>::try_as_ref)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .ok_or_else(|| format!("`{key}` is missing or not text"))
    };
    let base = path.parent().unwrap_or(Path::new("."));
    let read = || -> Result<Manifest, String> {
        let id = text_field("module")?;
        let version = match map.get("version") {
            None => None,
            Some(v) => Some(
                TryAsRef::<str>::try_as_ref(v)
                    .ok_or_else(|| "`version` is not text".to_string())?
                    .to_string(),
            ),
        };
        let kinds = text_list(map, "kinds")?;
        let mut relative = text_list(map, "paths")?;
        if relative.is_empty() {
            relative.push(".".to_string());
        }
        let paths: Vec<String> = relative
            .iter()
            .map(|p| {
                // Absolute, and not canonical: Windows spells a canonical path
                // `\\?\C:\..`, which Python's own paths never match.
                let joined = base.join(p);
                std::path::absolute(&joined)
                    .unwrap_or(joined)
                    .display()
                    .to_string()
            })
            .collect();
        let mut budget = MOST_FILES;
        let mut newest = 0u128;
        for p in &paths {
            newest_py(Path::new(p), 0, &mut budget, &mut newest);
        }
        Ok(Manifest {
            id,
            version,
            kinds,
            paths,
            stamp: format!("{file_stamp}:{newest}"),
        })
    };
    match read() {
        Ok(manifest) => Found::Library(manifest),
        Err(why) => broken(file_stamp, why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("guatiao-py-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_manifest_is_read_and_its_stamp_follows_its_code() {
        let dir = scratch("read");
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(
            dir.join("lib").join("one.guatiao-py.toml"),
            "module = \"one\"\nversion = \"1.0.0\"\nkinds = [\"greeter\"]\n",
        )
        .unwrap();
        std::fs::write(dir.join("lib").join("one.py"), "x = 1\n").unwrap();
        std::fs::write(dir.join("broken.guatiao-py.toml"), "id = \n").unwrap();

        let found = find(core::slice::from_ref(&dir), &[]);
        assert_eq!(found.len(), 2);
        let Found::Library(one) = found.iter().find(|f| f.id() == "one").unwrap().clone() else {
            panic!("{found:?}");
        };
        assert_eq!(one.kinds, ["greeter"]);
        assert_eq!(one.paths.len(), 1, "`.` by default");
        assert!(
            found
                .iter()
                .any(|f| matches!(f, Found::Broken { why, .. } if why.contains("broken")))
        );

        std::thread::sleep(core::time::Duration::from_millis(20));
        std::fs::write(dir.join("lib").join("one.py"), "x = 2\n").unwrap();
        let Found::Library(again) = read(&dir.join("lib").join("one.guatiao-py.toml")) else {
            panic!("still a library");
        };
        assert_ne!(
            again.stamp, one.stamp,
            "an edited .py file changes the stamp"
        );
    }

    #[test]
    fn a_field_of_the_wrong_kind_is_named() {
        let dir = scratch("wrong");
        let path = dir.join("w.guatiao-py.toml");
        std::fs::write(&path, "module = \"w\"\nkinds = \"greeter\"\n").unwrap();
        let Found::Broken { why, .. } = read(&path) else {
            panic!("refused");
        };
        assert!(why.contains("`kinds` is not a list"), "{why}");
    }
}
