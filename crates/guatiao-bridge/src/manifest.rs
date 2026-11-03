// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Manifests: what a library is, read without running it.
//!
//! A manifest is a file named `<name>.guatiao`, optionally followed by a
//! major and a minor, `<name>.guatiao.<major>` or
//! `<name>.guatiao.<major>.<minor>` (ASCII digits, like a versioned shared
//! object), beside or above the library's code, its `type` naming the
//! source that serves it. A `#!toml`, `#!json` or `#!yaml` first line names
//! the format and is removed; a manifest with no such line is TOML. JSON and
//! YAML are read when this crate is built with its `json` and `yaml`
//! features. The name never chooses the format, also when a file is named
//! explicitly (`find`'s `files`), which is read whatever it is called.
//!
//! When the manifest states a `version`, its major (and its minor, when the
//! name has one) must be the name's numbers, else it is broken, naming both;
//! with no `version`, or no numbers in the name, nothing is compared.
//!
//! ```toml
//! type = "python"              # the source that serves it: a format's `serves`
//! module = "greeters"          # the id key a format names: the library's id
//! version = "1.0.0"            # optional: else the engine reads it from the code
//! kinds = ["greeter"]          # every kind its providers serve
//! paths = ["."]                # where its code is, relative to this file
//! ```
//!
//! A source passes over a manifest of another type guatiao's sources serve
//! ([`TYPES`]) without a word, and lists one with no `type`, or a type
//! none of them serves, as broken, so a refresh reports it.
//!
//! Its stamp changes with the manifest's content and with the newest
//! modification of a code file under its paths, so a refresh notices an
//! edit to either.

use std::path::{Path, PathBuf};

use guatiao::Map;
use guatiao::value::convert::TryAsRef;

use crate::config::text_list;

/// The numbers a manifest's file name carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NameNumbers {
    major: Option<u64>,
    minor: Option<u64>,
}

/// Whether `file_name` is `<name>.guatiao[.<major>[.<minor>]]`, and the
/// numbers it carries. `<name>` is not empty; a number is ASCII digits.
fn name_numbers(file_name: &str) -> Option<NameNumbers> {
    let number = |part: &str| -> Option<u64> {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    let parts: Vec<&str> = file_name.split('.').collect();
    let n = parts.len();
    let stem_len = |guatiao_at: usize| {
        parts[..guatiao_at]
            .iter()
            .map(|p| p.len() + 1)
            .sum::<usize>()
    };
    let found = if n >= 4 && parts[n - 3] == "guatiao" {
        let (major, minor) = (number(parts[n - 2])?, number(parts[n - 1])?);
        Some((
            n - 3,
            NameNumbers {
                major: Some(major),
                minor: Some(minor),
            },
        ))
    } else if n >= 3 && parts[n - 2] == "guatiao" && number(parts[n - 1]).is_some() {
        let major = number(parts[n - 1])?;
        Some((
            n - 2,
            NameNumbers {
                major: Some(major),
                minor: None,
            },
        ))
    } else if n >= 2 && parts[n - 1] == "guatiao" {
        Some((
            n - 1,
            NameNumbers {
                major: None,
                minor: None,
            },
        ))
    } else {
        None
    };
    let (at, numbers) = found?;
    (stem_len(at) > 1).then_some(numbers)
}

/// The `type`s guatiao's own sources serve: each the name its
/// `guatiao::source!` declares. A `remote` manifest describes a peer, not
/// a library.
pub const TYPES: &[&str] = &["python", "js", "lua", "wasm", "rhai", "remote"];

/// The most code files a stamp looks at under one library's paths, and
/// how deep it walks: a library is a package, not a disk.
const MOST_FILES: usize = 10_000;
const DEEPEST: usize = 8;

/// Which manifests are one engine's and what its code looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManifestFormat {
    /// The `type` its manifests name (`python`): the name its source's
    /// `guatiao::source!` declares.
    pub serves: &'static str,
    /// The key holding the library's id (`module`).
    pub id_key: &'static str,
    /// Extensions of the code files a stamp follows, without the dot.
    /// None: the stamp is the manifest's content alone, and no directory
    /// is walked.
    pub code_extensions: &'static [&'static str],
    /// Directory names a stamp's walk skips (`__pycache__`).
    pub skip_dirs: &'static [&'static str],
}

impl ManifestFormat {
    fn is_manifest(&self, path: &Path) -> bool {
        path.is_file()
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| name_numbers(n).is_some())
    }

    /// The types a manifest may name: guatiao's, and this engine's own.
    fn known(&self) -> String {
        let mut known: Vec<&str> = TYPES.to_vec();
        if !known.contains(&self.serves) {
            known.push(self.serves);
        }
        known.join(", ")
    }

    fn is_code(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| self.code_extensions.contains(&e))
    }

    fn skips(&self, dir: &Path) -> bool {
        dir.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| self.skip_dirs.contains(&n))
    }
}

/// One library a manifest describes.
#[derive(Debug, Clone)]
pub struct Manifest {
    path: PathBuf,
    id: String,
    version: Option<String>,
    kinds: Vec<String>,
    paths: Vec<String>,
    stamp: String,
    table: Map,
}

impl Manifest {
    /// The manifest file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The library's id: the value of the format's id key.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The version it states, or `None` when it leaves it to the code.
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    /// Every kind its providers serve.
    pub fn kinds(&self) -> &[String] {
        &self.kinds
    }

    /// Where its code is: absolute, resolved against the manifest's
    /// directory, and not canonical (Windows spells a canonical path
    /// `\\?\C:\..`, which an engine's own paths never match).
    pub fn paths(&self) -> &[String] {
        &self.paths
    }

    /// What changes when the manifest or its code does.
    pub fn stamp(&self) -> &str {
        &self.stamp
    }

    /// The whole manifest, for the keys only an engine reads.
    pub fn table(&self) -> &Map {
        &self.table
    }
}

/// What reading one manifest came to.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Found {
    /// A library.
    Library(Manifest),
    /// Not one: listed under the manifest's path, so opening it reports why.
    Broken {
        /// The manifest.
        path: PathBuf,
        /// Its content's stamp, so a fix is noticed.
        stamp: String,
        /// Why, naming the file.
        why: String,
    },
}

impl Found {
    /// The id it is listed under: a library's id, or a broken manifest's
    /// path.
    pub fn id(&self) -> String {
        match self {
            Found::Library(m) => m.id.clone(),
            Found::Broken { path, .. } => path.display().to_string(),
        }
    }

    /// Its stamp.
    pub fn stamp(&self) -> &str {
        match self {
            Found::Library(m) => &m.stamp,
            Found::Broken { stamp, .. } => stamp,
        }
    }
}

/// Every manifest of `format`'s type in `dirs` (and their immediate
/// subdirectories) and in `files`, each once, in path order; a manifest
/// with no type, or a type no source serves, is in it as broken.
pub fn find(format: &ManifestFormat, dirs: &[PathBuf], files: &[PathBuf]) -> Vec<Found> {
    let mut paths: Vec<PathBuf> = files.to_vec();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if format.is_manifest(&path) {
                paths.push(path);
            } else if path.is_dir()
                && let Ok(inner) = std::fs::read_dir(&path)
            {
                paths.extend(
                    inner
                        .filter_map(Result::ok)
                        .map(|e| e.path())
                        .filter(|p| format.is_manifest(p)),
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
    paths.iter().filter_map(|p| read(format, p)).collect()
}

/// A stamp of `bytes`, which changes when they do: FNV-1a (64-bit) as 16
/// hex digits. Not a digest: it notices an edit, it proves nothing.
pub fn content_stamp(bytes: &[u8]) -> String {
    let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    format!("{hash:016x}")
}

fn modified(path: &Path) -> u128 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// The newest modification time of a code file under `root`.
fn newest_code(
    format: &ManifestFormat,
    root: &Path,
    depth: usize,
    budget: &mut usize,
    newest: &mut u128,
) {
    if depth > DEEPEST || *budget == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            if format.skips(&path) {
                continue;
            }
            newest_code(format, &path, depth + 1, budget, newest);
        } else if format.is_code(&path) {
            *budget = budget.saturating_sub(1);
            *newest = (*newest).max(modified(&path));
            if *budget == 0 {
                return;
            }
        }
    }
}

/// Whether the `version` a manifest states is the major (and the minor)
/// its file name carries, if it carries any.
fn agrees_with_name(path: &Path, version: &str) -> Result<(), String> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let Some(NameNumbers { major, minor }) = name_numbers(name) else {
        return Ok(());
    };
    let Some(major) = major else {
        return Ok(());
    };
    let mut numbers = version.split(|c: char| !c.is_ascii_digit());
    let stated = |part: Option<&str>| part.and_then(|p| p.parse::<u64>().ok());
    let (stated_major, stated_minor) = (stated(numbers.next()), stated(numbers.next()));
    let wanted = match minor {
        Some(minor) => format!("{major}.{minor}"),
        None => major.to_string(),
    };
    let agrees =
        stated_major == Some(major) && minor.is_none_or(|minor| stated_minor == Some(minor));
    if agrees {
        Ok(())
    } else {
        Err(format!(
            "`version` {version} does not agree with the {wanted} in the file name {name}"
        ))
    }
}

/// Reads one manifest, or `None` when its `type` is another source's.
/// Runs nothing.
pub fn read(format: &ManifestFormat, path: &Path) -> Option<Found> {
    let broken = |stamp: String, why: String| {
        Some(Found::Broken {
            path: path.to_path_buf(),
            stamp,
            why: format!("{}: {why}", path.display()),
        })
    };
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => return broken(format!("{}", modified(path)), e.to_string()),
    };
    let file_stamp = content_stamp(&bytes);
    let value = match crate::document::parse(&bytes) {
        Ok(value) => value,
        Err(e) => return broken(file_stamp, e.to_string()),
    };
    let Some(map) = TryAsRef::<Map>::try_as_ref(&value) else {
        return broken(file_stamp, "not a table".to_string());
    };
    match map.get("type").and_then(TryAsRef::<str>::try_as_ref) {
        Some(named) if named == format.serves => {}
        Some(named) if TYPES.contains(&named) => return None,
        Some(_) => {
            return broken(
                file_stamp,
                format!("`type` names none of the sources: {}", format.known()),
            );
        }
        None => {
            return broken(
                file_stamp,
                format!(
                    "`type` is missing or not text: it names the source that serves the library ({})",
                    format.known()
                ),
            );
        }
    }
    let text_field = |key: &str| -> Result<String, String> {
        map.get(key)
            .and_then(TryAsRef::<str>::try_as_ref)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .ok_or_else(|| format!("`{key}` is missing or not text"))
    };
    let base = path.parent().unwrap_or(Path::new("."));
    let read = || -> Result<Manifest, String> {
        let id = text_field(format.id_key)?;
        let version = match map.get("version") {
            None => None,
            Some(v) => Some(
                TryAsRef::<str>::try_as_ref(v)
                    .ok_or_else(|| "`version` is not text".to_string())?
                    .to_string(),
            ),
        };
        if let Some(version) = &version {
            agrees_with_name(path, version)?;
        }
        let kinds = text_list(map, "kinds").map_err(|e| e.to_string())?;
        let mut relative = text_list(map, "paths").map_err(|e| e.to_string())?;
        if relative.is_empty() {
            relative.push(".".to_string());
        }
        let paths: Vec<String> = relative
            .iter()
            .map(|p| {
                let joined = base.join(p);
                std::path::absolute(&joined)
                    .unwrap_or(joined)
                    .display()
                    .to_string()
            })
            .collect();
        let mut budget = MOST_FILES;
        let mut newest = 0u128;
        if !format.code_extensions.is_empty() {
            for p in &paths {
                newest_code(format, Path::new(p), 0, &mut budget, &mut newest);
            }
        }
        Ok(Manifest {
            path: path.to_path_buf(),
            id,
            version,
            kinds,
            paths,
            stamp: format!("{file_stamp}:{newest}"),
            table: map.clone(),
        })
    };
    match read() {
        Ok(manifest) => Some(Found::Library(manifest)),
        Err(why) => broken(file_stamp, why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PY: ManifestFormat = ManifestFormat {
        serves: "python",
        id_key: "module",
        code_extensions: &["py"],
        skip_dirs: &["__pycache__"],
    };

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("guatiao-bridge-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn broken_why(found: &[Found], file: &str) -> String {
        found
            .iter()
            .find_map(|f| match f {
                Found::Broken { path, why, .. } if path.ends_with(file) => Some(why.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{file} is not listed as broken: {found:?}"))
    }

    #[test]
    fn a_manifest_is_read_and_its_stamp_follows_its_code() {
        let dir = scratch("read");
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(
            dir.join("lib").join("one.guatiao"),
            "type = \"python\"\nmodule = \"one\"\nversion = \"1.0.0\"\nkinds = [\"greeter\"]\n",
        )
        .unwrap();
        std::fs::write(dir.join("lib").join("one.py"), "x = 1\n").unwrap();
        std::fs::write(dir.join("broken.guatiao"), "id = \n").unwrap();

        let found = find(&PY, core::slice::from_ref(&dir), &[]);
        assert_eq!(found.len(), 2);
        let Found::Library(one) = found.iter().find(|f| f.id() == "one").unwrap().clone() else {
            panic!("{found:?}");
        };
        assert_eq!(one.kinds(), ["greeter"]);
        assert_eq!(one.paths().len(), 1, "`.` by default");
        assert!(
            found
                .iter()
                .any(|f| matches!(f, Found::Broken { why, .. } if why.contains("broken")))
        );

        std::thread::sleep(core::time::Duration::from_millis(20));
        std::fs::write(dir.join("lib").join("one.py"), "x = 2\n").unwrap();
        let Some(Found::Library(again)) = read(&PY, &dir.join("lib").join("one.guatiao")) else {
            panic!("still a library");
        };
        assert_ne!(
            again.stamp(),
            one.stamp(),
            "an edited code file changes the stamp"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_field_of_the_wrong_kind_is_named() {
        let dir = scratch("wrong");
        let path = dir.join("w.guatiao");
        std::fs::write(
            &path,
            "type = \"python\"\nmodule = \"w\"\nkinds = \"greeter\"\n",
        )
        .unwrap();
        let Some(Found::Broken { why, .. }) = read(&PY, &path) else {
            panic!("refused");
        };
        assert!(why.contains("`kinds` is not a list"), "{why}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn another_sources_manifest_is_passed_over_and_a_missing_or_unknown_type_is_named() {
        let dir = scratch("types");
        let write = |name: &str, text: &str| std::fs::write(dir.join(name), text).unwrap();
        write("mine.guatiao", "type = \"python\"\nmodule = \"mine\"\n");
        write("theirs.guatiao", "type = \"lua\"\nmodule = \"theirs\"\n");
        write("untyped.guatiao", "module = \"untyped\"\n");
        write("numbered.guatiao", "type = 3\nmodule = \"numbered\"\n");
        write("typo.guatiao", "type = \"pyhton\"\nmodule = \"typo\"\n");

        let found = find(&PY, core::slice::from_ref(&dir), &[]);
        assert_eq!(found.len(), 4, "{found:?}");
        assert!(found.iter().any(|f| f.id() == "mine"));
        assert!(!found.iter().any(|f| f.id().contains("theirs")));
        for file in ["untyped.guatiao", "numbered.guatiao"] {
            let why = broken_why(&found, file);
            assert!(why.contains("`type` is missing or not text"), "{why}");
            assert!(why.contains("python, js, lua, wasm, rhai"), "{why}");
        }
        let why = broken_why(&found, "typo.guatiao");
        assert!(why.contains("`type` names none of the sources"), "{why}");

        assert!(read(&PY, &dir.join("theirs.guatiao")).is_none());
        let named = find(&PY, &[], &[dir.join("theirs.guatiao")]);
        assert!(
            named.is_empty(),
            "a file named in `manifests` too: {named:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_engine_of_its_own_type_reads_its_manifests_and_names_its_type() {
        const TOY: ManifestFormat = ManifestFormat {
            serves: "toy",
            id_key: "module",
            code_extensions: &[],
            skip_dirs: &[],
        };
        let dir = scratch("own");
        std::fs::write(dir.join("t.guatiao"), "type = \"toy\"\nmodule = \"t\"\n").unwrap();
        std::fs::write(dir.join("u.guatiao"), "module = \"u\"\n").unwrap();
        let found = find(&TOY, core::slice::from_ref(&dir), &[]);
        assert!(
            found
                .iter()
                .any(|f| matches!(f, Found::Library(m) if m.id() == "t"))
        );
        let why = broken_why(&found, "u.guatiao");
        assert!(
            why.contains("python, js, lua, wasm, rhai, remote, toy"),
            "{why}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    const TOML: &str = "type = \"python\"\nmodule = \"m\"\nkinds = [\"greeter\", \"tally\"]\n";
    #[cfg(feature = "json")]
    const JSON: &str =
        "{\"type\": \"python\", \"module\": \"m\", \"kinds\": [\"greeter\", \"tally\"]}\n";
    #[cfg(feature = "yaml")]
    const YAML: &str = "type: python\nmodule: m\nkinds:\n  - greeter\n  - tally\n";

    #[test]
    fn a_manifest_names_its_format_on_its_first_line_or_is_toml() {
        let dir = scratch("formats");
        #[allow(unused_mut)]
        let mut cases = vec![
            ("bare", TOML.to_string()),
            ("toml", format!("#!toml\n{TOML}")),
            ("bom", format!("\u{feff}#!toml\n{TOML}")),
        ];
        #[cfg(feature = "json")]
        {
            cases.push(("json", format!("#!json\n{JSON}")));
            cases.push(("bomjson", format!("\u{feff}#!json\n{JSON}")));
        }
        #[cfg(feature = "yaml")]
        {
            cases.push(("yaml", format!("#!yaml\n{YAML}")));
            cases.push(("shebang", format!("#!/usr/bin/env yaml\n{YAML}")));
            cases.push(("upper", format!("#!.YAML\n{YAML}")));
        }
        for (name, text) in &cases {
            let path = dir.join(format!("{name}.guatiao"));
            std::fs::write(&path, text).unwrap();
            let Some(Found::Library(m)) = read(&PY, &path) else {
                panic!("{name} is not a library");
            };
            assert_eq!(m.id(), "m", "{name}");
            assert_eq!(m.kinds(), ["greeter", "tally"], "{name}");
        }
        let found = find(&PY, core::slice::from_ref(&dir), &[]);
        assert_eq!(found.len(), cases.len(), "{found:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn named(id: &str, version: Option<&str>) -> String {
        let version = version.map_or(String::new(), |v| format!("version = \"{v}\"\n"));
        format!("type = \"python\"\nmodule = \"{id}\"\n{version}")
    }

    #[test]
    fn a_name_is_guatiao_optionally_followed_by_a_major_and_a_minor() {
        let dir = scratch("names");
        let accepted = [
            "a.guatiao",
            "a.guatiao.2",
            "a.guatiao.2.1",
            "a.guatiao.10.0",
            "a.b.guatiao.3",
            "a.guatiao.02",
        ];
        let rejected = [
            "a.guatiao.bak",
            "a.guatiao.2.1.3",
            "a.guatiao.cfg",
            "a.guatiao.toml",
            "a.guatiao.2.x",
            "a.guatiao.x.2",
            "a.guatiao.2.",
            "a.guatiao..2",
            "a.guatiao.\u{0663}",
            "a.guatiao.-1",
            "a.guatiao.2.1.bak",
            "guatiao",
            ".guatiao",
            "a.guatiao2",
            "a.GUATIAO",
        ];
        for name in accepted.iter().chain(&rejected) {
            std::fs::write(dir.join(name), named(name, None)).unwrap();
        }
        let found: Vec<String> = find(&PY, core::slice::from_ref(&dir), &[])
            .iter()
            .map(|f| match f {
                Found::Library(m) => m.path().file_name().unwrap().to_str().unwrap().to_string(),
                Found::Broken { path, .. } => {
                    path.file_name().unwrap().to_str().unwrap().to_string()
                }
            })
            .collect();
        for name in accepted {
            assert!(
                found.iter().any(|f| f == name),
                "{name} is found: {found:?}"
            );
        }
        for name in rejected {
            assert!(!found.iter().any(|f| f == name), "{name} is not: {found:?}");
        }
        assert_eq!(found.len(), accepted.len(), "{found:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stated_version_must_match_the_numbers_in_the_name() {
        let dir = scratch("numbers");
        let write = |name: &str, version: Option<&str>| {
            let path = dir.join(name);
            std::fs::write(&path, named("m", version)).unwrap();
            path
        };
        let is_library = |path: &Path| matches!(read(&PY, path), Some(Found::Library(_)));
        // The major alone, and the major and minor.
        assert!(is_library(&write("a.guatiao.2", Some("2.7.1"))));
        assert!(is_library(&write("b.guatiao.2.7", Some("2.7.1"))));
        assert!(is_library(&write("c.guatiao.2.7", Some("2.7.0-alpha.1"))));
        assert!(is_library(&write("d.guatiao.02", Some("2.0.0"))));
        // No number in the name, or no version stated: nothing to compare.
        assert!(is_library(&write("e.guatiao", Some("9.9.9"))));
        assert!(is_library(&write("f.guatiao.3", None)));
        assert!(is_library(&write("g.guatiao.3.1", None)));
        // A mismatch is broken and names both.
        let path = write("h.guatiao.2", Some("3.0.0"));
        let Some(Found::Broken { why, .. }) = read(&PY, &path) else {
            panic!("a major that differs is broken");
        };
        assert!(
            why.contains("h.guatiao.2") && why.contains("3.0.0"),
            "{why}"
        );
        assert!(why.contains("`2`") || why.contains(" 2"), "{why}");
        let path = write("i.guatiao.2.7", Some("2.8.0"));
        let Some(Found::Broken { why, .. }) = read(&PY, &path) else {
            panic!("a minor that differs is broken");
        };
        assert!(
            why.contains("i.guatiao.2.7") && why.contains("2.8.0"),
            "{why}"
        );
        let path = write("j.guatiao.2.7", Some("2"));
        assert!(matches!(read(&PY, &path), Some(Found::Broken { .. })));
        let path = write("k.guatiao.2", Some("two"));
        assert!(matches!(read(&PY, &path), Some(Found::Broken { .. })));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unknown_format_line_is_broken_and_names_the_file() {
        let dir = scratch("unknown");
        let path = dir.join("odd.guatiao");
        std::fs::write(&path, format!("#!ini\n{TOML}")).unwrap();
        let Some(Found::Broken { why, .. }) = read(&PY, &path) else {
            panic!("not broken");
        };
        assert!(why.contains("odd.guatiao"), "{why}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn broken_text(name: &str, text: &str) -> String {
        let dir = scratch(name);
        let path = dir.join(format!("{name}.guatiao"));
        std::fs::write(&path, text).unwrap();
        let Some(Found::Broken { why, .. }) = read(&PY, &path) else {
            panic!("{name} is not broken");
        };
        let _ = std::fs::remove_dir_all(&dir);
        assert!(why.contains(&format!("{name}.guatiao")), "{why}");
        why
    }

    #[test]
    fn a_toml_error_names_the_file_and_the_position_once_and_quotes_nothing() {
        // The first line is removed and still counted: the mistake is on line 3.
        let why = broken_text("toml", "#!toml\ntype = \"python\"\nsecret-value 7\n");
        assert!(why.contains("at line 3, column"), "{why}");
        assert!(!why.contains("line 2"), "{why}");
        assert!(!why.contains("secret-value"), "{why}");
    }

    #[cfg(feature = "json")]
    #[test]
    fn a_json_error_names_the_file_and_the_position_once_and_quotes_nothing() {
        let why = broken_text(
            "json",
            "#!json\n{\"type\": \"python\",\n \"secret-value\" 7}\n",
        );
        assert!(why.contains("at line 3, column"), "{why}");
        assert!(!why.contains("line 2"), "{why}");
        assert!(!why.contains("secret-value"), "{why}");
        assert!(!why.contains("column 0"), "{why}");

        // A document that ends where a value is wanted has no column 0.
        let why = broken_text("eof", "#!json\n{\"type\": \"python\",");
        assert!(!why.contains("column 0"), "{why}");
    }

    #[cfg(feature = "yaml")]
    #[test]
    fn a_yaml_error_names_the_position_and_quotes_nothing() {
        let why = broken_text("yaml", "#!yaml\ntype: python\nsecret-value: [1\n");
        assert!(why.contains("line 3") || why.contains("line 4"), "{why}");
        assert!(!why.contains("secret-value"), "{why}");
    }

    #[cfg(not(feature = "yaml"))]
    #[test]
    fn a_format_that_is_not_built_in_is_broken_and_says_so() {
        let why = broken_text("yaml", "#!yaml\ntype: python\nmodule: m\n");
        assert!(why.contains("not built in"), "{why}");
    }

    #[cfg(not(feature = "json"))]
    #[test]
    fn json_that_is_not_built_in_is_broken_and_says_so() {
        let why = broken_text("json", "#!json\n{}\n");
        assert!(why.contains("not built in"), "{why}");
    }

    #[cfg(feature = "json")]
    #[test]
    fn a_file_named_explicitly_is_read_whatever_it_is_called_and_the_name_never_picks_the_format() {
        let dir = scratch("explicit");
        // The extension says one format and the content another.
        let toml_named_json = dir.join("x.guatiao.json");
        std::fs::write(&toml_named_json, TOML).unwrap();
        let json_named_toml = dir.join("y.guatiao.toml");
        std::fs::write(&json_named_toml, format!("#!json\n{JSON}")).unwrap();
        let odd = dir.join("z.manifest");
        std::fs::write(&odd, TOML).unwrap();
        let found = find(&PY, &[], &[toml_named_json, json_named_toml, odd]);
        assert_eq!(found.len(), 3, "{found:?}");
        assert!(
            found.iter().all(|f| matches!(f, Found::Library(_))),
            "{found:?}"
        );
        // Found by a scan of the directory, none of the three is a manifest.
        assert!(find(&PY, core::slice::from_ref(&dir), &[]).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
