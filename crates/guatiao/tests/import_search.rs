// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Where the loader looks for what a library file imports.
//!
//! `tests/c_consumer/import_plugin.c` imports `import_dep.c`. Each case
//! places the two files, starts a copy of this binary in a host directory
//! of its own with the current directory and `PATH` the case chooses, and
//! has it load the plugin as a library and add it as a source file under
//! one [`ImportSearch`]. What the child prints says whether the file
//! mapped. A process per case, because a library a process loaded answers
//! every later import of its name, and `PATH` and the current directory
//! belong to the whole process.
//!
//! Its own `main` (`harness = false`): `child <library|path> <load|source>
//! <file> [preload]` is the child; anything else runs every case.

use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "support/c_compiler.rs"]
mod c_compiler;

use guatiao::Map;
use guatiao::library::{ImportSearch, LoadError, Loading, Registry, Skipped, SourceError};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("child") {
        child(&args[2..]);
        return;
    }
    // The platforms whose loader rules these cases state.
    if !cfg!(any(
        windows,
        target_os = "linux",
        target_os = "freebsd",
        target_os = "macos"
    )) {
        println!("skipped: the import search cases cover Windows, Linux, FreeBSD and macOS");
        return;
    }
    let Some(cc) = c_compiler::find() else {
        println!("skipped: no C compiler found (as c_library.rs)");
        return;
    };
    let fixtures = Fixtures::build(&cc);
    let failures = if cfg!(windows) {
        windows_cases(&fixtures)
    } else {
        unix_cases(&fixtures)
    };
    if failures.is_empty() {
        // Each case copied this binary: the copies go once nothing failed.
        let _ = std::fs::remove_dir_all(&fixtures.root);
        println!("import_search: every case passed");
    } else {
        for failure in &failures {
            eprintln!("FAILED: {failure}");
        }
        std::process::exit(1);
    }
}

/// The child: one load or one source, under one search, printed as
/// `mapped` or `refused: <reason>`.
fn child(args: &[String]) {
    let search = match args[0].as_str() {
        "library" => ImportSearch::Library,
        "path" => ImportSearch::Path,
        other => panic!("no search {other}"),
    };
    let file = Path::new(&args[2]);
    if let Some(preload) = args.get(3) {
        // SAFETY: the test's own fixture, whose initialisers do nothing.
        let library = unsafe { libloading::Library::new(preload) }.expect("the preload maps");
        core::mem::forget(library);
    }
    let mut registry = Registry::new("import_search", "1.0");
    registry.set_import_search(search);
    assert_eq!(registry.import_search(), search);
    let answer = match args[1].as_str() {
        "load" => match registry.load_file(file) {
            Ok(Loading::Skipped(Skipped::DeclinedThisHost)) => "mapped".to_string(),
            Err(LoadError::Open { reason, .. }) => format!("refused: {}", one_line(&reason)),
            other => format!("unexpected: {other:?}"),
        },
        "source" => match registry.add_source_file(file, &Map::new()) {
            Err(SourceError::Refused { .. }) => "mapped".to_string(),
            Err(SourceError::Open { reason, .. }) => format!("refused: {}", one_line(&reason)),
            other => format!("unexpected: {other:?}"),
        },
        other => panic!("no action {other}"),
    };
    println!("{answer}");
}

/// A reason on one line: dyld's run over several, and the parent reads
/// the child's last.
fn one_line(reason: &str) -> String {
    reason.lines().map(str::trim).collect::<Vec<_>>().join(" ")
}

fn dll(stem: &str) -> String {
    format!(
        "{}{stem}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    )
}

/// The built files, in a directory no case loads from.
struct Fixtures {
    root: PathBuf,
    dep: PathBuf,
    refusing_dep: Option<PathBuf>,
    plugin: PathBuf,
    /// Unix: the plugin whose run path is its own directory (`$ORIGIN`,
    /// `@loader_path` on macOS).
    origin_plugin: Option<PathBuf>,
}

impl Fixtures {
    fn build(cc: &Path) -> Fixtures {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let sources = manifest.join("tests/c_consumer");
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("import_search");
        let _ = std::fs::remove_dir_all(&root);
        let build = root.join("build");
        std::fs::create_dir_all(&build).unwrap();

        let compile = |out: &Path, source: &str, extra: &[&str]| {
            let mut command = c_compiler::command(cc);
            command.args(["-shared", "-std=c11", "-Wall", "-Wextra", "-Werror"]);
            if !cfg!(windows) {
                command.arg("-fPIC");
            }
            command
                .arg("-I")
                .arg(manifest.join("include"))
                .arg("-o")
                .arg(out)
                .arg(sources.join(source))
                .args(extra);
            let done = command.output().expect("the compiler runs");
            assert!(
                done.status.success(),
                "{source} did not build:\n{}",
                String::from_utf8_lossy(&done.stderr)
            );
        };

        let dep = build.join(dll("import_dep"));
        // A Mach-O library is imported by the name it records for itself:
        // `@rpath/...` leaves where to look to the importer, as an ELF
        // SONAME does, rather than the path it was built at.
        let install_name = ["-Wl,-install_name,@rpath/libimport_dep.dylib"];
        let dep_args: &[&str] = if cfg!(target_os = "macos") {
            &install_name
        } else {
            &[]
        };
        compile(&dep, "import_dep.c", dep_args);
        let refusing_dep = cfg!(windows).then(|| {
            let dir = build.join("refusing");
            std::fs::create_dir_all(&dir).unwrap();
            let out = dir.join(dll("import_dep"));
            compile(&out, "import_dep.c", &["-DIMPORT_DEP_REFUSES"]);
            out
        });
        let plugin = build.join(dll("import_plugin"));
        let origin_plugin = if cfg!(windows) {
            // clang names the import library after the DLL, `.lib`; a MinGW
            // compiler writes none and links the DLL itself.
            let implib = dep.with_extension("lib");
            let link = if implib.is_file() { &implib } else { &dep };
            compile(&plugin, "import_plugin.c", &[link.to_str().unwrap()]);
            None
        } else {
            let link = format!("-L{}", build.display());
            compile(&plugin, "import_plugin.c", &[&link, "-limport_dep"]);
            let dir = build.join("origin");
            std::fs::create_dir_all(&dir).unwrap();
            let out = dir.join(dll("import_plugin"));
            let own_directory = if cfg!(target_os = "macos") {
                "-Wl,-rpath,@loader_path"
            } else {
                "-Wl,-rpath,$ORIGIN"
            };
            compile(
                &out,
                "import_plugin.c",
                &[&link, "-limport_dep", own_directory],
            );
            Some(out)
        };
        Fixtures {
            root,
            dep,
            refusing_dep,
            plugin,
            origin_plugin,
        }
    }
}

/// One placement of the files, and the answers expected under it.
struct Case<'a> {
    name: &'static str,
    plugin: &'a Path,
    /// Copies of the dependency: beside the plugin, beside the host, in
    /// the current directory, in a directory on `PATH`.
    beside: Option<&'a Path>,
    host_dir: Option<&'a Path>,
    cwd: Option<&'a Path>,
    on_path: Option<&'a Path>,
    /// A copy the host loads itself before the plugin, from elsewhere.
    preload: Option<&'a Path>,
}

impl Case<'_> {
    /// Places the files and runs the child: `Ok(None)` when it mapped,
    /// `Ok(Some(reason))` when it was refused.
    fn run(&self, root: &Path, search: &str, action: &str) -> Result<Option<String>, String> {
        let dir = root.join(format!("{}-{search}-{action}", self.name));
        assert!(!dir.exists(), "two cases are named {}", self.name);
        let place = |sub: &str, file: Option<&Path>, name: &str| -> PathBuf {
            let at = dir.join(sub);
            std::fs::create_dir_all(&at).unwrap();
            if let Some(file) = file {
                std::fs::copy(file, at.join(name)).unwrap();
            }
            at
        };
        let dep_name = dll("import_dep");
        let plugin_dir = place("plugin", self.beside, &dep_name);
        let plugin = plugin_dir.join(dll("import_plugin"));
        std::fs::copy(self.plugin, &plugin).unwrap();
        let host_dir = place("host", self.host_dir, &dep_name);
        let cwd = place("cwd", self.cwd, &dep_name);
        let on_path = place("path", self.on_path, &dep_name);

        let host = host_dir.join(format!(
            "import_search_host{}",
            std::env::consts::EXE_SUFFIX
        ));
        std::fs::copy(std::env::current_exe().unwrap(), &host).unwrap();
        // Windows looks on PATH, dyld on DYLD_LIBRARY_PATH, an ELF loader
        // on LD_LIBRARY_PATH.
        let variable = if cfg!(windows) {
            "PATH"
        } else if cfg!(target_os = "macos") {
            "DYLD_LIBRARY_PATH"
        } else {
            "LD_LIBRARY_PATH"
        };
        let mut paths = vec![on_path];
        paths.extend(std::env::split_paths(
            &std::env::var_os(variable).unwrap_or_default(),
        ));
        let mut command = Command::new(&host);
        command
            .args(["child", search, action])
            .arg(&plugin)
            .current_dir(&cwd)
            .env(variable, std::env::join_paths(paths).unwrap());
        if let Some(preload) = self.preload {
            command.arg(preload);
        }
        let out = command.output().expect("the host copy runs");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let line = stdout.lines().last().unwrap_or("").to_string();
        if !out.status.success() {
            return Err(format!(
                "the child failed: {line}\n{}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        if line == "mapped" {
            Ok(None)
        } else if let Some(reason) = line.strip_prefix("refused: ") {
            println!("  {}-{search}-{action}: {line}", self.name);
            Ok(Some(reason.to_string()))
        } else {
            Err(line)
        }
    }

    /// Checks the answer under one search, for both actions; a refusal's
    /// reason must contain `says`.
    fn expect_saying(
        &self,
        root: &Path,
        search: &str,
        mapped: bool,
        says: &str,
        failures: &mut Vec<String>,
    ) {
        for action in ["load", "source"] {
            let what = format!("{} under {search} ({action})", self.name);
            match self.run(root, search, action) {
                Ok(None) if mapped => {}
                Ok(Some(reason)) if !mapped => {
                    if !reason.contains(says) {
                        failures.push(format!(
                            "{what}: the reason does not say {says:?}: {reason}"
                        ));
                    }
                }
                Ok(answer) => {
                    failures.push(format!("{what}: {answer:?}, expected mapped {mapped}"))
                }
                Err(why) => failures.push(format!("{what}: {why}")),
            }
        }
    }

    fn expect(&self, root: &Path, search: &str, mapped: bool, failures: &mut Vec<String>) {
        self.expect_saying(root, search, mapped, "", failures);
    }
}

fn windows_cases(f: &Fixtures) -> Vec<String> {
    let mut failures = Vec::new();
    let refusing = f.refusing_dep.as_deref();
    let case = |name| Case {
        name,
        plugin: &f.plugin,
        beside: None,
        host_dir: None,
        cwd: None,
        on_path: None,
        preload: None,
    };

    // A plugin shipped in a directory of its own beside what it imports.
    let beside = Case {
        beside: Some(&f.dep),
        ..case("beside")
    };
    beside.expect(&f.root, "library", true, &mut failures);
    beside.expect(&f.root, "path", true, &mut failures);

    // What it imports is only in the current directory, or only on PATH:
    // found only when the host names `path`.
    let cwd = Case {
        cwd: Some(&f.dep),
        ..case("cwd")
    };
    cwd.expect(&f.root, "library", false, &mut failures);
    cwd.expect(&f.root, "path", true, &mut failures);
    let on_path = Case {
        on_path: Some(&f.dep),
        ..case("on_path")
    };
    on_path.expect(&f.root, "library", false, &mut failures);
    on_path.expect(&f.root, "path", true, &mut failures);

    // Beside the host's executable, or already loaded by the host.
    let host_dir = Case {
        host_dir: Some(&f.dep),
        ..case("host_dir")
    };
    host_dir.expect(&f.root, "library", true, &mut failures);
    let preloaded = Case {
        preload: Some(&f.dep),
        ..case("preloaded")
    };
    preloaded.expect(&f.root, "library", true, &mut failures);

    // A copy beside the plugin wins over copies in the current directory
    // and on PATH, under either search.
    let beside_wins = Case {
        beside: Some(&f.dep),
        cwd: refusing,
        on_path: refusing,
        ..case("beside_wins")
    };
    beside_wins.expect(&f.root, "library", true, &mut failures);
    beside_wins.expect(&f.root, "path", true, &mut failures);

    // Nowhere at all: refused, the reason naming the loader's own words
    // and, under the default, where it did not look.
    let nowhere = case("nowhere");
    nowhere.expect_saying(&f.root, "library", false, "PATH", &mut failures);
    nowhere.expect_saying(&f.root, "path", false, "os error 126", &mut failures);
    failures
}

fn unix_cases(f: &Fixtures) -> Vec<String> {
    let mut failures = Vec::new();
    let origin = f.origin_plugin.as_deref().expect("built on unix");
    // Its own directory in its run path (`$ORIGIN`, `@loader_path`): found
    // there, either way.
    let with_origin = Case {
        name: "origin",
        plugin: origin,
        beside: Some(&f.dep),
        host_dir: None,
        cwd: None,
        on_path: None,
        preload: None,
    };
    with_origin.expect(&f.root, "library", true, &mut failures);
    with_origin.expect(&f.root, "path", true, &mut failures);
    // No run path: the dynamic loader does not look beside the library,
    // and the setting changes nothing.
    let without = Case {
        name: "no_runpath",
        plugin: &f.plugin,
        ..with_origin
    };
    without.expect(&f.root, "library", false, &mut failures);
    without.expect(&f.root, "path", false, &mut failures);
    // Nor in the current directory; the loader's variable is searched.
    let in_cwd = Case {
        name: "cwd",
        beside: None,
        cwd: Some(&f.dep),
        ..without
    };
    in_cwd.expect(&f.root, "library", false, &mut failures);
    in_cwd.expect(&f.root, "path", false, &mut failures);
    let on_library_path = Case {
        name: "loader_variable",
        cwd: None,
        on_path: Some(&f.dep),
        ..in_cwd
    };
    on_library_path.expect(&f.root, "library", true, &mut failures);
    on_library_path.expect(&f.root, "path", true, &mut failures);
    failures
}
