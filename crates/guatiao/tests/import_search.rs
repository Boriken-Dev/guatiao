// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Where the loader looks for what a library file imports.
//!
//! `tests/c_consumer/import_plugin.c` imports `import_dep.c`. Each case
//! places the files, starts a copy of this binary in a host directory of
//! its own with the current directory and `PATH` the case chooses, and has
//! it load the plugin as a library and add it as a source file under one
//! [`ImportSearch`]. What the child prints says whether the file mapped. A
//! process per case, because a library a process loaded answers every
//! later import of its name, and `PATH` and the current directory belong
//! to the whole process.
//!
//! Its own `main` (`harness = false`): `child <library|path>
//! <load|source|unload> <file> [preload=<file>] [dir=<dir>] [added=<dir>]
//! [released=<name>]` is the child, `dir` a source file's `import_dirs`,
//! `added` a directory the host adds with `AddDllDirectory`, `released` a
//! library `unload` must have let go; anything else runs every case.

use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "support/c_compiler.rs"]
mod c_compiler;

use guatiao::library::{ImportSearch, LoadError, Loading, Registry, Skipped, SourceError};
use guatiao::{List, Map};

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
    let mut failures = config_cases();
    let Some(cc) = c_compiler::find() else {
        println!("skipped: no C compiler found (as c_library.rs)");
        return;
    };
    let fixtures = Fixtures::build(&cc);
    failures.extend(if cfg!(windows) {
        windows_cases(&fixtures)
    } else {
        unix_cases(&fixtures)
    });
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

/// `import_dirs` that is not a list of text is refused before the file is
/// opened, and is not the source's to see.
fn config_cases() -> Vec<String> {
    let mut failures = Vec::new();
    let mut registry = Registry::new("import_search", "1.0");
    for (value, says) in [
        (guatiao::Value::from("dir"), "is not a list of directories"),
        (
            guatiao::Value::from({
                let mut list = List::new();
                list.push(3_i64);
                list
            }),
            "holds something that is not text",
        ),
    ] {
        let mut config = Map::new();
        config.set("import_dirs", value);
        match registry.add_source_file(Path::new("no-such-source"), &config) {
            Err(e @ SourceError::Config { .. })
                if e.to_string().contains(says)
                    && e.status() == guatiao::Status::GUATIAO_ERR_BAD_VALUE => {}
            other => failures.push(format!("import_dirs refused saying {says:?}: {other:?}")),
        }
    }
    failures
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
    let mut config = Map::new();
    let mut released = None;
    for extra in &args[3..] {
        if let Some(preload) = extra.strip_prefix("preload=") {
            // SAFETY: the test's own fixture, whose initialisers do nothing.
            let library = unsafe { libloading::Library::new(preload) }.expect("the preload maps");
            core::mem::forget(library);
        } else if let Some(dir) = extra.strip_prefix("dir=") {
            let mut dirs = List::new();
            dirs.push(dir);
            config.set("import_dirs", dirs);
        } else if let Some(dir) = extra.strip_prefix("added=") {
            add_dll_directory(Path::new(dir));
        } else if let Some(name) = extra.strip_prefix("released=") {
            released = Some(name.to_string());
        } else {
            panic!("no argument {extra}");
        }
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
        "source" => match registry.add_source_file(file, &config) {
            Err(SourceError::Refused { .. }) => "mapped".to_string(),
            Err(SourceError::Open { reason, .. }) => format!("refused: {}", one_line(&reason)),
            other => format!("unexpected: {other:?}"),
        },
        "unload" => match registry
            .load_file(file)
            .map(|l| l.loaded().map(|l| l.key.to_string()))
        {
            Ok(Some(key)) => match registry.unload(&key) {
                Ok(()) => match released {
                    Some(name) if still_loaded(&name) => {
                        format!("refused: {name} is still mapped after the unload")
                    }
                    _ => "mapped".to_string(),
                },
                Err(e) => format!("unexpected: {e}"),
            },
            Err(LoadError::Open { reason, .. }) => format!("refused: {}", one_line(&reason)),
            other => format!("unexpected: {other:?}"),
        },
        other => panic!("no action {other}"),
    };
    println!("{answer}");
}

/// Adds `dir` to the process's DLL search, as a host may.
#[cfg(windows)]
fn add_dll_directory(dir: &Path) {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AddDllDirectory(dir: *const u16) -> *mut core::ffi::c_void;
    }
    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: a NUL-terminated absolute path.
    let cookie = unsafe { AddDllDirectory(wide.as_ptr()) };
    assert!(
        !cookie.is_null(),
        "AddDllDirectory refused {}",
        dir.display()
    );
}

#[cfg(not(windows))]
fn add_dll_directory(_: &Path) {
    panic!("AddDllDirectory is Windows'");
}

/// Whether a library of this name is still loaded in this process.
#[cfg(windows)]
fn still_loaded(name: &str) -> bool {
    libloading::os::windows::Library::open_already_loaded(name).is_ok()
}

#[cfg(not(windows))]
fn still_loaded(_: &str) -> bool {
    false
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
    plugin: PathBuf,
    /// Windows: the dependency again, its initialiser refusing.
    refusing_dep: Option<PathBuf>,
    /// Windows: a second dependency, and a plugin importing both.
    second_dep: Option<PathBuf>,
    plugin_two: Option<PathBuf>,
    /// Windows: the plugin as a library that loads and may be unloaded.
    plugin_unloads: Option<PathBuf>,
    /// Windows, where the linker writes forwarders: a library forwarding
    /// the dependency's export, and a plugin importing it from there.
    forwarding: Option<(PathBuf, PathBuf)>,
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

        let try_compile = |out: &Path, source: &str, extra: &[&str]| -> Result<(), String> {
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
            if done.status.success() {
                Ok(())
            } else {
                Err(String::from_utf8_lossy(&done.stderr).into_owned())
            }
        };
        let compile = |out: &Path, source: &str, extra: &[&str]| {
            if let Err(why) = try_compile(out, source, extra) {
                panic!("{source} did not build:\n{why}");
            }
        };
        // clang names the import library after the DLL, `.lib`; a MinGW
        // compiler writes none and links the DLL itself.
        let link_to = |dll: &Path| -> String {
            let implib = dll.with_extension("lib");
            let link = if implib.is_file() {
                implib
            } else {
                dll.to_path_buf()
            };
            link.to_str().unwrap().to_string()
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
        let plugin = build.join(dll("import_plugin"));
        if !cfg!(windows) {
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
            return Fixtures {
                root,
                dep,
                plugin,
                refusing_dep: None,
                second_dep: None,
                plugin_two: None,
                plugin_unloads: None,
                forwarding: None,
                origin_plugin: Some(out),
            };
        }
        compile(&plugin, "import_plugin.c", &[&link_to(&dep)]);
        let dir = build.join("refusing");
        std::fs::create_dir_all(&dir).unwrap();
        let refusing_dep = dir.join(dll("import_dep"));
        compile(&refusing_dep, "import_dep.c", &["-DIMPORT_DEP_REFUSES"]);
        let second_dep = build.join(dll("import_dep2"));
        compile(&second_dep, "import_dep.c", &["-DIMPORT_DEP_SECOND"]);
        let dir = build.join("two");
        std::fs::create_dir_all(&dir).unwrap();
        let plugin_two = dir.join(dll("import_plugin"));
        compile(
            &plugin_two,
            "import_plugin.c",
            &["-DIMPORT_PLUGIN_TWO", &link_to(&dep), &link_to(&second_dep)],
        );
        let dir = build.join("unloads");
        std::fs::create_dir_all(&dir).unwrap();
        let plugin_unloads = dir.join(dll("import_plugin"));
        compile(
            &plugin_unloads,
            "import_plugin.c",
            &["-DIMPORT_PLUGIN_LIBRARY", &link_to(&dep)],
        );
        let forwarding_dep = build.join(dll("import_fwd"));
        let dir = build.join("forwarded");
        std::fs::create_dir_all(&dir).unwrap();
        let plugin_forwarded = dir.join(dll("import_plugin"));
        let forwarding = try_compile(&forwarding_dep, "import_dep.c", &["-DIMPORT_DEP_FORWARDS"])
            .and_then(|()| {
                try_compile(
                    &plugin_forwarded,
                    "import_plugin.c",
                    &[&link_to(&forwarding_dep)],
                )
            });
        let forwarding = match forwarding {
            Ok(()) => Some((forwarding_dep, plugin_forwarded)),
            Err(why) => {
                println!("skipped: the forwarding cases; this linker wrote no forwarder:\n{why}");
                None
            }
        };
        Fixtures {
            root,
            dep,
            plugin,
            refusing_dep: Some(refusing_dep),
            second_dep: Some(second_dep),
            plugin_two: Some(plugin_two),
            plugin_unloads: Some(plugin_unloads),
            forwarding,
            origin_plugin: None,
        }
    }
}

/// One placement of the files, and the answers expected under it. Each
/// file is copied under its own name.
#[derive(Clone, Copy)]
struct Case<'a> {
    name: &'static str,
    plugin: &'a Path,
    /// Beside the plugin, beside the host, in the current directory, in a
    /// directory on `PATH`, in a directory the source's `import_dirs`
    /// names (added only when it holds something).
    beside: &'a [&'a Path],
    host_dir: &'a [&'a Path],
    cwd: &'a [&'a Path],
    on_path: &'a [&'a Path],
    dirs: &'a [&'a Path],
    /// `dirs` given relative to the plugin's directory, and placed there.
    relative_dirs: bool,
    /// In a directory the host adds with `AddDllDirectory`.
    added: &'a [&'a Path],
    /// A library `unload` must have let go.
    released: Option<&'a str>,
    /// A copy the host loads itself before the plugin, from elsewhere.
    preload: Option<&'a Path>,
}

impl Case<'_> {
    /// Places the files and runs the child: `Ok(None)` when it mapped,
    /// `Ok(Some(reason))` when it was refused.
    fn run(&self, root: &Path, search: &str, action: &str) -> Result<Option<String>, String> {
        let dir = root.join(format!("{}-{search}-{action}", self.name));
        assert!(!dir.exists(), "two cases are named {}", self.name);
        let place = |sub: &str, files: &[&Path]| -> PathBuf {
            let at = dir.join(sub);
            std::fs::create_dir_all(&at).unwrap();
            for file in files {
                std::fs::copy(file, at.join(file.file_name().unwrap())).unwrap();
            }
            at
        };
        let plugin_dir = place("plugin", self.beside);
        let plugin = plugin_dir.join(dll("import_plugin"));
        std::fs::copy(self.plugin, &plugin).unwrap();
        let host_dir = place("host", self.host_dir);
        let cwd = place("cwd", self.cwd);
        let on_path = place("path", self.on_path);
        let dirs = if self.relative_dirs {
            place("plugin/deps", self.dirs)
        } else {
            place("dirs", self.dirs)
        };
        let added = place("added", self.added);

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
            command.arg(format!("preload={}", preload.display()));
        }
        if self.relative_dirs {
            command.arg("dir=deps");
        } else if !self.dirs.is_empty() {
            command.arg(format!("dir={}", dirs.display()));
        }
        if !self.added.is_empty() {
            command.arg(format!("added={}", added.display()));
        }
        if let Some(name) = self.released {
            command.arg(format!("released={name}"));
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

    /// Checks the answer under one search, for each action; a refusal's
    /// reason must contain `says`.
    fn expect_saying_for(
        &self,
        root: &Path,
        search: &str,
        actions: &[&str],
        mapped: bool,
        says: &str,
        failures: &mut Vec<String>,
    ) {
        for action in actions {
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

    fn expect_saying(
        &self,
        root: &Path,
        search: &str,
        mapped: bool,
        says: &str,
        failures: &mut Vec<String>,
    ) {
        self.expect_saying_for(root, search, &["load", "source"], mapped, says, failures);
    }

    fn expect(&self, root: &Path, search: &str, mapped: bool, failures: &mut Vec<String>) {
        self.expect_saying(root, search, mapped, "", failures);
    }
}

fn windows_cases(f: &Fixtures) -> Vec<String> {
    let mut failures = Vec::new();
    let dep: &Path = &f.dep;
    let refusing: &Path = f.refusing_dep.as_deref().expect("built on Windows");
    let second: &Path = f.second_dep.as_deref().expect("built on Windows");
    let case = |name| Case {
        name,
        plugin: &f.plugin,
        beside: &[],
        host_dir: &[],
        cwd: &[],
        on_path: &[],
        dirs: &[],
        relative_dirs: false,
        added: &[],
        released: None,
        preload: None,
    };

    // A plugin shipped in a directory of its own beside what it imports.
    let beside = Case {
        beside: &[dep],
        ..case("beside")
    };
    beside.expect(&f.root, "library", true, &mut failures);
    beside.expect(&f.root, "path", true, &mut failures);

    // What it imports is only in the current directory, or only on PATH:
    // found only when the host names `path`.
    let cwd = Case {
        cwd: &[dep],
        ..case("cwd")
    };
    cwd.expect(&f.root, "library", false, &mut failures);
    cwd.expect(&f.root, "path", true, &mut failures);
    let on_path = Case {
        on_path: &[dep],
        ..case("on_path")
    };
    on_path.expect(&f.root, "library", false, &mut failures);
    on_path.expect(&f.root, "path", true, &mut failures);

    // Beside the host's executable, or already loaded by the host.
    let host_dir = Case {
        host_dir: &[dep],
        ..case("host_dir")
    };
    host_dir.expect(&f.root, "library", true, &mut failures);
    let preloaded = Case {
        preload: Some(dep),
        ..case("preloaded")
    };
    preloaded.expect(&f.root, "library", true, &mut failures);

    // A copy beside the plugin wins over copies in the current directory
    // and on PATH, under either search.
    let beside_wins = Case {
        beside: &[dep],
        cwd: &[refusing],
        on_path: &[refusing],
        ..case("beside_wins")
    };
    beside_wins.expect(&f.root, "library", true, &mut failures);
    beside_wins.expect(&f.root, "path", true, &mut failures);

    // Only what is missing comes from the wider search: one import beside
    // the host, the other only on PATH, a copy of the first that refuses
    // to start in the current directory and on PATH. Windows' standard
    // search would take the first from the current directory.
    let only_the_missing = Case {
        plugin: f.plugin_two.as_deref().expect("built on Windows"),
        host_dir: &[dep],
        on_path: &[second, refusing],
        cwd: &[refusing],
        ..case("only_the_missing")
    };
    only_the_missing.expect(&f.root, "path", true, &mut failures);
    only_the_missing.expect_saying(&f.root, "library", false, "import_dep2.dll", &mut failures);

    // A source file's `import_dirs`: looked in for a missing import under
    // either search, before anything `path` adds.
    let in_dirs = Case {
        dirs: &[dep],
        on_path: &[refusing],
        ..case("in_dirs")
    };
    for search in ["library", "path"] {
        in_dirs.expect_saying_for(&f.root, search, &["source"], true, "", &mut failures);
    }
    // A library loaded from there that forwards to another beside it, as
    // `python3.dll` does to `python3XX.dll`.
    if let Some((forwarding, plugin)) = &f.forwarding {
        let forwarded = Case {
            plugin,
            dirs: &[forwarding, dep],
            ..case("forwarded")
        };
        forwarded.expect_saying_for(&f.root, "library", &["source"], true, "", &mut failures);
        let not_given = Case {
            plugin,
            cwd: &[forwarding, dep],
            ..case("forwarded_not_given")
        };
        not_given.expect_saying_for(
            &f.root,
            "library",
            &["source"],
            false,
            "import_fwd.dll",
            &mut failures,
        );
    }

    // A directory the host added holds one import, PATH the other and a
    // copy of the first that refuses to start: only the second comes from
    // PATH, and `import_dirs` holding that copy does not win either.
    let added = Case {
        plugin: f.plugin_two.as_deref().expect("built on Windows"),
        added: &[dep],
        on_path: &[second, refusing],
        ..case("added")
    };
    added.expect_saying_for(
        &f.root,
        "path",
        &["load", "source"],
        true,
        "",
        &mut failures,
    );
    let added_dirs = Case {
        plugin: f.plugin_two.as_deref().expect("built on Windows"),
        added: &[dep],
        dirs: &[second, refusing],
        ..case("added_dirs")
    };
    added_dirs.expect_saying_for(&f.root, "library", &["source"], true, "", &mut failures);

    // `import_dirs` relative to the source file's own directory.
    let relative = Case {
        dirs: &[dep],
        relative_dirs: true,
        ..case("relative_dirs")
    };
    relative.expect_saying_for(&f.root, "library", &["source"], true, "", &mut failures);

    // What a file was missing and was taken from PATH goes with the file.
    let released = Case {
        plugin: f.plugin_unloads.as_deref().expect("built on Windows"),
        on_path: &[dep],
        released: Some("import_dep.dll"),
        ..case("released")
    };
    released.expect_saying_for(&f.root, "path", &["unload"], true, "", &mut failures);

    // Nowhere at all: refused, the reason naming the loader's own words,
    // the import and, under the default, where it did not look.
    let nowhere = case("nowhere");
    nowhere.expect_saying(&f.root, "library", false, "PATH", &mut failures);
    nowhere.expect_saying(&f.root, "path", false, "os error 126", &mut failures);
    let nowhere_named = case("nowhere_named");
    nowhere_named.expect_saying(&f.root, "library", false, "import_dep.dll", &mut failures);
    nowhere_named.expect_saying(&f.root, "path", false, "import_dep.dll", &mut failures);
    failures
}

fn unix_cases(f: &Fixtures) -> Vec<String> {
    let mut failures = Vec::new();
    let origin = f.origin_plugin.as_deref().expect("built on unix");
    let dep: &Path = &f.dep;
    // Its own directory in its run path (`$ORIGIN`, `@loader_path`): found
    // there, either way.
    let with_origin = Case {
        name: "origin",
        plugin: origin,
        beside: &[dep],
        host_dir: &[],
        cwd: &[],
        on_path: &[],
        dirs: &[],
        relative_dirs: false,
        added: &[],
        released: None,
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
        beside: &[],
        cwd: &[dep],
        ..without
    };
    in_cwd.expect(&f.root, "library", false, &mut failures);
    in_cwd.expect(&f.root, "path", false, &mut failures);
    let on_library_path = Case {
        name: "loader_variable",
        cwd: &[],
        on_path: &[dep],
        ..in_cwd
    };
    on_library_path.expect(&f.root, "library", true, &mut failures);
    on_library_path.expect(&f.root, "path", true, &mut failures);
    // A source file's `import_dirs` changes nothing here either.
    let in_dirs = Case {
        name: "in_dirs",
        on_path: &[],
        dirs: &[dep],
        ..on_library_path
    };
    in_dirs.expect_saying_for(&f.root, "library", &["source"], false, "", &mut failures);
    failures
}
