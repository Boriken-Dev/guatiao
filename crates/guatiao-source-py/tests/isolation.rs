// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What each `isolation` lets into the interpreter this file starts.
//!
//! The interpreter is one per process and its environment is the
//! process's, so each case runs a copy of this binary from a host
//! directory of its own, with the files and variables the case plants,
//! as a child: `child <isolation> <home or -> <work dir>`. The child adds
//! the bridge, opens a probe library that writes what its interpreter
//! looks like (`sys.prefix`, where `sitecustomize` came from) beside
//! itself, and prints that.
//!
//! Its own `main` (`harness = false`). The bridge is this crate's cdylib,
//! which `cargo test` does not build: `cargo build` first, or this states
//! a skip.

extern crate alloc;

use alloc::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use guatiao::library::{Registry, SourceError};
use guatiao::value::convert::TryAsRef;
use guatiao::{List, Map};

/// The probe library: what its interpreter looks like, written beside it
/// when it is imported.
const PROBE: &str = r#"
import os
import sys

_site = sys.modules.get("sitecustomize")
_facts = {
    "prefix": os.path.normcase(os.path.realpath(sys.prefix)),
    "isolated": str(sys.flags.isolated),
    "sitecustomize": os.path.normcase(os.path.realpath(_site.__file__)) if _site else "-",
    "executable": sys.executable,
    "path": os.pathsep.join(sys.path),
}
import importlib.util

_shadow = importlib.util.find_spec("iso_shadow")
_facts["shadow"] = os.path.normcase(os.path.realpath(_shadow.origin)) if _shadow else "-"
with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "facts.txt"), "w", encoding="utf-8") as out:
    for key, value in _facts.items():
        out.write(f"{key}={value}\n")

from greeter_kind import Counter

from guatiao import provide

__version__ = "0.1.0"
__all__ = ["Probe"]


class Probe(provide.Provider, Counter):
    def count(self) -> int:
        return 1
"#;

const SITECUSTOMIZE: &str = "PLANTED = True\n";

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A library's manifest in `dir`, by the file name the source lists.
fn manifest_file(dir: &Path, stem: &str) -> PathBuf {
    dir.join(format!("{stem}.guatiao"))
}

fn library_name(base: &str) -> String {
    format!(
        "{}{base}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    )
}

fn bridge() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let debug = exe.parent()?.parent()?;
    let path = debug.join(library_name("guatiao_source_py"));
    path.is_file().then_some(path)
}

fn text_list(items: &[PathBuf]) -> List {
    let mut list = List::new();
    for item in items {
        list.push(item.to_string_lossy().as_ref());
    }
    list
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("child") {
        child(&args[2..]);
        return;
    }
    let Some(bridge) = bridge() else {
        println!("skipped: the bridge is not built beside this test; `cargo build` first");
        return;
    };
    let Some(python) = Python::find() else {
        println!("skipped: no Python found for the cases to plant files for");
        return;
    };
    println!("isolation: Python at {}", python.base.display());
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("isolation");
    let _ = std::fs::remove_dir_all(&root);
    let mut failures = Vec::new();
    let mut check = |name: &str, ok: bool, detail: String| {
        if !ok {
            failures.push(format!("{name}: {detail}"));
        }
    };

    // What the environment brings: only `inherit` takes it.
    // Before 3.14 CPython on Windows reads a `pyvenv.cfg` beside the
    // program whatever it is told, and `isolated` refuses instead.
    let unheld_venv = cfg!(windows) && python.minor < 14;
    for (isolation, runs) in [("inherit", true), ("shipped", false), ("isolated", false)] {
        let case = Case::new(&root, &format!("env-{isolation}"));
        let planted = case.plant_site("envsite");
        let facts = case.run(&bridge, isolation, None, &[("PYTHONPATH", &planted)]);
        check(
            &case.name,
            facts.started() && facts.ran_from(&planted) == runs,
            format!("sitecustomize on PYTHONPATH ran: expected {runs}; {facts:?}"),
        );
        // The host is told when the environment reaches the interpreter.
        let told = if runs {
            "isolation effective=inherit configured=inherit joined=false"
        } else {
            ""
        };
        check(
            &case.name,
            facts.get("notes") == told,
            format!("the refresh's notes: expected `{told}`; {facts:?}"),
        );
    }

    // A `pyvenv.cfg` beside the executable: the program shipped as a
    // virtual environment, `sys.prefix` its directory (3.14) or the one
    // above (`site`, before). `isolated` reads only its own `home`.
    // `inherit` elsewhere than Windows names the program `python3`, found
    // on `PATH`, and reads beside that instead.
    let inherit_reads_host = cfg!(windows);
    for (isolation, honoured) in [
        ("shipped", true),
        ("isolated", false),
        ("inherit", inherit_reads_host),
    ] {
        let case = Case::new(&root, &format!("venv-{isolation}"));
        std::fs::write(
            case.host.join("pyvenv.cfg"),
            format!("home = {}\n", python.executable_dir.display()),
        )
        .unwrap();
        let facts = case.run(&bridge, isolation, None, &[]);
        if unheld_venv && isolation == "isolated" {
            check(
                &case.name,
                facts.refused("cannot keep") && facts.refused("pyvenv.cfg"),
                format!("refused on 3.{}: {facts:?}", python.minor),
            );
            continue;
        }
        let venv = facts.prefix_is(&case.host) || facts.prefix_is(&case.dir);
        check(
            &case.name,
            facts.started() && venv == honoured,
            format!("pyvenv.cfg beside the executable honoured: expected {honoured}; {facts:?}"),
        );
        if isolation == "isolated" {
            check(
                &case.name,
                facts.get("isolated") == "1",
                format!("{facts:?}"),
            );
        }
    }

    if cfg!(windows) {
        // A `._pth` beside the executable: what an embedded Python ships.
        for (isolation, honoured) in [("shipped", true), ("isolated", false), ("inherit", true)] {
            let case = Case::new(&root, &format!("pth-{isolation}"));
            let planted = case.plant_site("pthsite");
            let mut pth = String::new();
            for dir in ["Lib", "DLLs"] {
                pth.push_str(&format!("{}\n", python.base.join(dir).display()));
            }
            pth.push_str(&format!("{}\nimport site\n", planted.display()));
            std::fs::write(case.host.join(format!("{HOST}._pth")), pth).unwrap();
            let facts = case.run(&bridge, isolation, None, &[]);
            check(
                &case.name,
                facts.started() && facts.ran_from(&planted) == honoured,
                format!("._pth beside the executable honoured: expected {honoured}; {facts:?}"),
            );
        }
    } else {
        // A bare program name is looked up on `PATH`: a virtual
        // environment whose `bin` is there decides `sys.prefix` under
        // `inherit`, and never otherwise.
        for (isolation, honoured) in [("inherit", true), ("shipped", false)] {
            let case = Case::new(&root, &format!("pathvenv-{isolation}"));
            let venv = case.dir.join("venv");
            let bin = venv.join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            std::fs::copy(&python.executable, bin.join("python3")).unwrap();
            std::fs::write(
                venv.join("pyvenv.cfg"),
                format!("home = {}\n", python.executable_dir.display()),
            )
            .unwrap();
            let path = std::env::join_paths(core::iter::once(bin.clone()).chain(
                std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
            ))
            .unwrap();
            let facts = case.run(&bridge, isolation, None, &[("PATH", Path::new(&path))]);
            check(
                &case.name,
                facts.started() && facts.prefix_is(&venv) == honoured,
                format!("a virtual environment on PATH honoured: expected {honoured}; {facts:?}"),
            );
        }
    }

    // `inherit` starts with any one of the variables CPython reads set,
    // where 3.9 on Windows found no standard library and ended the process.
    for (name, value) in [
        ("PYTHONUSERBASE", "userbase"),
        ("PYTHONDONTWRITEBYTECODE", "1"),
        ("PYTHONVERBOSE", "0"),
    ] {
        let case = Case::new(&root, &format!("inherit-{}", name.to_lowercase()));
        let value = if value == "userbase" {
            case.dir.join("userbase")
        } else {
            PathBuf::from(value)
        };
        let facts = case.run(&bridge, "inherit", None, &[(name, &value)]);
        check(
            &case.name,
            facts.started(),
            format!("inherit with {name} set started: {facts:?}"),
        );
    }

    // Python files beside the host's executable: shipped with the program,
    // so `shipped` may read them (Windows puts that directory on
    // `sys.path`); `isolated` never does.
    for isolation in ["shipped", "isolated"] {
        let case = Case::new(&root, &format!("exedir-{isolation}"));
        std::fs::write(case.host.join("sitecustomize.py"), SITECUSTOMIZE).unwrap();
        std::fs::write(case.host.join("iso_shadow.py"), "SHADOW = True\n").unwrap();
        let facts = case.run(&bridge, isolation, None, &[]);
        let ran = facts.ran_from(&case.host);
        let shadowed = facts.get("shadow") == normal(&case.host.join("iso_shadow.py"));
        let (expect_ran, expect_shadowed) = match isolation {
            "isolated" => (false, false),
            _ => (ran, cfg!(windows)),
        };
        check(
            &case.name,
            facts.started() && ran == expect_ran && shadowed == expect_shadowed,
            format!(
                "beside the executable: sitecustomize ran {ran}, a module found {shadowed}; {facts:?}"
            ),
        );
    }

    // `home` relative to the bridge file's own directory.
    let case = Case::new(&root, "isolated-relative-home");
    let relative = relative_to(bridge.parent().unwrap(), &python.base);
    let facts = case.run(&bridge, "isolated", Some(&relative), &[]);
    check(
        &case.name,
        facts.prefix_is(&python.base),
        format!(
            "a relative home {} is the bridge's: {facts:?}",
            relative.display()
        ),
    );

    // A start that failed is the answer to every later one.
    let case = Case::new(&root, "failed-start");
    let said = case.run_raw(&bridge, "failed-start", None, &[]);
    check(
        &case.name,
        said.contains("ok: failed-start"),
        format!("a failed start: {said}"),
    );

    // `home` names the installation an `isolated` interpreter reads.
    let case = Case::new(&root, "isolated-home");
    let facts = case.run(&bridge, "isolated", Some(&python.base), &[]);
    check(
        &case.name,
        facts.prefix_is(&python.base),
        format!("sys.prefix is the home given: {facts:?}"),
    );

    if cfg!(windows) {
        // With no `home`, the installation `import_dirs` mapped
        // `python3.dll` from is the interpreter's: `sys.prefix` is it.
        for isolation in ["shipped", "isolated"] {
            let case = Case::new(&root, &format!("import-dirs-home-{isolation}"));
            let facts = case.run(&bridge, isolation, None, &[]);
            check(
                &case.name,
                facts.prefix_is(&python.base),
                format!("sys.prefix is the installation in import_dirs: {facts:?}"),
            );
        }

        // `home` alone does not bring `python3.dll`: the registry maps the
        // bridge, and what it imports, before the bridge reads its
        // configuration. Under the default search the copy on `PATH` is
        // not reached either, and the refusal names `import_dirs`.
        let case = Case::new(&root, "home-without-import-dirs");
        let said = case.run_raw(&bridge, "home-alone", Some(&python.base), &[]);
        check(
            &case.name,
            said.contains("ok: home-alone"),
            format!("home without import_dirs: {said}"),
        );
    }

    // Refusals, before anything starts.
    let case = Case::new(&root, "refusals");
    let said = case.run_raw(&bridge, "refusals", None, &[]);
    check(
        &case.name,
        said.contains("ok: refusals"),
        format!("the refusals: {said}"),
    );

    if failures.is_empty() {
        let _ = std::fs::remove_dir_all(&root);
        println!("isolation: every case passed");
    } else {
        for failure in &failures {
            eprintln!("FAILED: {failure}");
        }
        std::process::exit(1);
    }
}

/// The copy of this binary each case runs as, without its suffix.
const HOST: &str = "iso_host";

/// The Python the bridge will load, and what the cases plant for it.
struct Python {
    /// Its installation: `sys.base_prefix`.
    base: PathBuf,
    /// Where its executable is (a `pyvenv.cfg`'s `home`).
    executable_dir: PathBuf,
    executable: PathBuf,
    /// Its minor version.
    minor: u32,
}

impl Python {
    /// Windows: the installation holding the first `python3.dll` on
    /// `PATH`, which the bridge imports. Elsewhere: what `PYO3_PYTHON`,
    /// else `python3`, says of itself.
    fn find() -> Option<Python> {
        if cfg!(windows) {
            let base = std::env::split_paths(&std::env::var_os("PATH")?)
                .find(|dir| dir.join("python3.dll").is_file() && dir.join("Lib").is_dir())?;
            // `python3X.dll` beside `python3.dll` names the minor.
            let minor = std::fs::read_dir(&base)
                .ok()?
                .filter_map(|e| e.ok()?.file_name().into_string().ok())
                .find_map(|n| {
                    n.strip_prefix("python3")?
                        .strip_suffix(".dll")?
                        .parse()
                        .ok()
                })?;
            return Some(Python {
                executable: base.join("python.exe"),
                executable_dir: base.clone(),
                base,
                minor,
            });
        }
        let python = std::env::var("PYO3_PYTHON").unwrap_or_else(|_| "python3".to_string());
        let out = Command::new(&python)
            .args([
                "-I",
                "-c",
                "import os,sys;print(sys.base_prefix);print(os.path.realpath(sys.executable));print(sys.version_info[1])",
            ])
            .output()
            .ok()?;
        let text = String::from_utf8(out.stdout).ok()?;
        let mut lines = text.lines();
        let base = PathBuf::from(lines.next()?);
        let executable = PathBuf::from(lines.next()?);
        let minor = lines.next()?.trim().parse().ok()?;
        Some(Python {
            base,
            executable_dir: executable.parent()?.to_path_buf(),
            executable,
            minor,
        })
    }
}

/// A directory per case: `host` (the binary's copy), `work` (the probe
/// library), and whatever the case plants.
struct Case {
    name: String,
    dir: PathBuf,
    host: PathBuf,
    work: PathBuf,
}

impl Case {
    fn new(root: &Path, name: &str) -> Case {
        let dir = root.join(name);
        let host = dir.join("host");
        let work = dir.join("work");
        std::fs::create_dir_all(&host).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(work.join("iso_probe.py"), PROBE).unwrap();
        std::fs::write(
            manifest_file(&work, "iso_probe"),
            "type = \"python\"\nmodule = \"iso_probe\"\nversion = \"0.1.0\"\nkinds = [\"counter\"]\npaths = [\".\"]\n",
        )
        .unwrap();
        Case {
            name: name.to_string(),
            dir,
            host,
            work,
        }
    }

    /// A directory holding a `sitecustomize.py`.
    fn plant_site(&self, name: &str) -> PathBuf {
        let at = self.dir.join(name);
        std::fs::create_dir_all(&at).unwrap();
        std::fs::write(at.join("sitecustomize.py"), SITECUSTOMIZE).unwrap();
        at
    }

    fn run_raw(
        &self,
        bridge: &Path,
        isolation: &str,
        home: Option<&Path>,
        env: &[(&str, &Path)],
    ) -> String {
        let exe = self
            .host
            .join(format!("{HOST}{}", std::env::consts::EXE_SUFFIX));
        std::fs::copy(std::env::current_exe().unwrap(), &exe).unwrap();
        let mut command = Command::new(&exe);
        command
            .arg("child")
            .arg(isolation)
            .arg(home.map_or_else(|| "-".into(), |h| h.as_os_str().to_owned()))
            .arg(&self.work)
            .arg(bridge)
            .current_dir(&self.dir)
            .env_remove("PYTHONPATH")
            .env_remove("PYTHONHOME");
        for (key, value) in env {
            command.env(key, value);
        }
        let out = command.output().expect("the host copy runs");
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    }

    fn run(
        &self,
        bridge: &Path,
        isolation: &str,
        home: Option<&Path>,
        env: &[(&str, &Path)],
    ) -> Facts {
        let said = self.run_raw(bridge, isolation, home, env);
        let mut facts = BTreeMap::new();
        for line in said.lines() {
            if let Some((key, value)) = line.strip_prefix("fact ").and_then(|l| l.split_once('=')) {
                facts.insert(key.to_string(), value.to_string());
            }
        }
        if facts.is_empty() {
            facts.insert("output".to_string(), said);
        }
        Facts(facts)
    }
}

#[derive(Debug)]
struct Facts(BTreeMap<String, String>);

impl Facts {
    fn get(&self, key: &str) -> &str {
        self.0.get(key).map_or("", String::as_str)
    }

    /// Whether the probe ran: the interpreter started and imported it.
    fn started(&self) -> bool {
        self.0.contains_key("prefix")
    }

    /// Whether adding the source was refused saying `says`.
    fn refused(&self, says: &str) -> bool {
        !self.started() && self.get("output").contains(says)
    }

    fn ran_from(&self, dir: &Path) -> bool {
        let planted = normal(&dir.join("sitecustomize.py"));
        self.get("sitecustomize") == planted
    }

    fn prefix_is(&self, dir: &Path) -> bool {
        self.get("prefix") == normal(dir)
    }
}

/// A path as the probe writes one: real, and lower-case on Windows.
fn normal(path: &Path) -> String {
    let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let text = real.display().to_string();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text).to_string();
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}

/// What every child's configuration starts with: on Windows, the Python
/// installation as `import_dirs`, where the registry finds the
/// `python3.dll` the bridge imports, and `python3XX.dll` it forwards to,
/// with the default import search and nothing on `PATH` reached.
fn base_config() -> Map {
    let mut config = Map::new();
    if cfg!(windows) {
        let python = Python::find().expect("the parent found one");
        config.set("import_dirs", text_list(&[python.base]));
    }
    config
}

/// The child: adds the bridge under one isolation and prints what the
/// probe wrote, or why it was refused.
fn child(args: &[String]) {
    let (isolation, home, work, bridge) = (
        args[0].as_str(),
        args[1].as_str(),
        Path::new(&args[2]),
        Path::new(&args[3]),
    );
    if isolation == "refusals" {
        refusals(bridge);
        return;
    }
    if isolation == "failed-start" {
        failed_start(bridge, work);
        return;
    }
    if isolation == "home-alone" {
        home_alone(bridge, Path::new(home));
        return;
    }
    let mut registry = Registry::new("isolation", "1.0");
    let mut config = base_config();
    config.set("isolation", isolation);
    if home != "-" {
        config.set("home", home);
    }
    config.set("manifests", text_list(&[manifest_file(work, "iso_probe")]));
    config.set(
        "paths",
        text_list(&[
            repo().join("bindings/python/guatiao/src"),
            repo().join("examples/greeter_kind/python"),
        ]),
    );
    if let Err(e) = registry.add_source_file(bridge, &config) {
        println!("refused: {e}");
        return;
    }
    let changed = registry.refresh();
    if !changed.refused.is_empty() {
        println!("refused: {:?}", changed.refused);
    }
    let notes: Vec<String> = changed
        .notes
        .iter()
        .map(|(_, note)| {
            let text = |key: &str| {
                note.detail()
                    .get(key)
                    .and_then(TryAsRef::<str>::try_as_ref)
                    .unwrap_or("")
                    .to_string()
            };
            let joined = note
                .detail()
                .get("joined")
                .and_then(TryAsRef::<bool>::try_as_ref)
                .copied()
                .unwrap_or_default();
            format!(
                "{} effective={} configured={} joined={joined}",
                note.subject(),
                text("effective"),
                text("configured")
            )
        })
        .collect();
    println!("fact notes={}", notes.join("; "));
    match std::fs::read_to_string(work.join("facts.txt")) {
        Ok(text) => {
            for line in text.lines() {
                println!("fact {line}");
            }
        }
        Err(e) => println!("no facts: {e}"),
    }
}

/// `to` as a path relative to `from`, both absolute on one volume.
fn relative_to(from: &Path, to: &Path) -> PathBuf {
    let from: Vec<_> = from.components().collect();
    let to: Vec<_> = to.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut out = PathBuf::new();
    for _ in common..from.len() {
        out.push("..");
    }
    for part in &to[common..] {
        out.push(part.as_os_str());
    }
    out
}

/// An interpreter that did not start, from a `home` holding no standard
/// library: refused, and every later source refused with its words, not
/// started again over what CPython left.
fn failed_start(bridge: &Path, work: &Path) {
    let empty = work.join("empty-home");
    std::fs::create_dir_all(&empty).unwrap();
    let mut config = base_config();
    config.set("isolation", "isolated");
    config.set("home", empty.display().to_string().as_str());
    let first = Registry::new("first", "1.0").add_source_file(bridge, &config);
    match first {
        Err(SourceError::Refused { message, .. }) if message.contains("did not start") => {}
        other => panic!("expected the start to fail, got {other:?}"),
    };
    let second = Registry::new("second", "1.0").add_source_file(bridge, &base_config());
    match second {
        Err(SourceError::Refused { message, .. })
            if message.contains("did not start earlier in this process") => {}
        other => panic!("expected the first failure again, got {other:?}"),
    }
    println!("ok: failed-start");
}

/// A configuration naming the installation as `home` and nothing in
/// `import_dirs`, the import search left at its default: refused before the
/// bridge runs, naming `python3.dll` and `import_dirs`.
fn home_alone(bridge: &Path, home: &Path) {
    let mut config = Map::new();
    config.set("isolation", "isolated");
    config.set("home", home.display().to_string().as_str());
    match Registry::new("home-alone", "1.0").add_source_file(bridge, &config) {
        Err(SourceError::Open { reason, .. })
            if reason.contains("python3.dll") && reason.contains("import_dirs") => {}
        other => panic!("expected the bridge's import to be missing, got {other:?}"),
    }
    println!("ok: home-alone");
}

/// Settings refused before any interpreter starts, and a second source
/// asking for other settings than the one interpreter started with.
fn refusals(bridge: &Path) {
    let refused = |config: &Map, says: &str| {
        let mut registry = Registry::new("refusals", "1.0");
        match registry.add_source_file(bridge, config) {
            Err(SourceError::Refused { message, .. }) if message.contains(says) => {}
            other => panic!("expected a refusal saying {says:?}, got {other:?}"),
        }
    };
    let with = |pairs: &[(&str, &str)]| {
        let mut config = base_config();
        config.set(
            "paths",
            text_list(&[
                repo().join("bindings/python/guatiao/src"),
                repo().join("examples/greeter_kind/python"),
            ]),
        );
        for (key, value) in pairs {
            config.set(key, *value);
        }
        config
    };
    refused(
        &with(&[("isolation", "sealed")]),
        "not `shipped`, `inherit` or `isolated`",
    );
    let here = std::env::current_dir().unwrap();
    let home = here.display().to_string();
    refused(
        &with(&[("isolation", "inherit"), ("home", &home)]),
        "is not taken under the isolation `inherit`",
    );
    refused(
        &with(&[("home", &here.join("nowhere").display().to_string())]),
        "names no directory",
    );
    let mut first = Registry::new("first", "1.0");
    first
        .add_source_file(bridge, &with(&[("isolation", "shipped")]))
        .unwrap_or_else(|e| panic!("the first source: {e}"));
    refused(
        &with(&[("isolation", "inherit")]),
        "already started with `shipped`",
    );
    println!("ok: refusals");
}
