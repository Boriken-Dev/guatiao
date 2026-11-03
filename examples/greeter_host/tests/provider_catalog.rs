// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A settings screen's question, asked of a scanned directory: every
//! provider, with its kinds, id, name, version, its library's digest,
//! whether it can run here and why not, its configuration schema and the
//! form it is shown with (its own, or the one the schema implies), and
//! **none of them built**. From Rust, and from a C program that links the
//! two shared libraries and nothing else.
//!
//! The directory holds `derived_greeter` and `catalog_library.c` (in
//! `crates/guatiao/tests/c_consumer`), built here. Every `create` of the
//! latter refuses saying whether it was the first it was asked, so asking
//! for one instance after the listing proves the listing asked for none.

use std::path::{Path, PathBuf};
use std::process::Command;

use greeter_kind::Counter;
use guatiao::library::{InstantiateError, Registry, scan_dir};
use guatiao::schema::SchemaRef;
use guatiao::{Alloc, Map, Value};
use guatiao_intake::{FormRef, check, provider_form};
use sha2::Digest as _;

#[path = "../../../crates/guatiao/tests/support/c_compiler.rs"]
mod c_compiler;

/// Where cargo put a shared library: beside this test binary (`deps/`) or
/// one up (`target/debug`).
fn shared_library(stem: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let deps = exe.parent()?;
    let name = format!(
        "{}{stem}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    [deps.to_path_buf(), deps.join("..")]
        .into_iter()
        .map(|dir| dir.join(&name))
        .find(|path| path.is_file())
}

/// A directory of its own holding `derived_greeter` and the C catalog
/// library, or `None` without a C compiler.
fn plugins(name: &str) -> Option<PathBuf> {
    let cc = c_compiler::find()?;
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    // Left over from an earlier run: its files may still be mapped by
    // nothing now, and a fresh directory is the point.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a fresh directory");

    let greeter = shared_library("derived_greeter")
        .expect("the dev-dependency's cdylib is beside the test binary");
    std::fs::copy(&greeter, dir.join(greeter.file_name().expect("a file")))
        .expect("the greeter copies");

    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo = manifest.join("../..");
    let library = dir.join(format!(
        "{}catalog_library{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    let mut compile = c_compiler::command(&cc);
    compile
        .args(["-shared", "-std=c11", "-Wall", "-Wextra", "-Werror"])
        .arg("-I")
        .arg(repo.join("crates/guatiao/include"))
        .arg("-I")
        .arg(repo.join("examples/greeter_kind/include"))
        .arg("-o")
        .arg(&library)
        .arg(repo.join("crates/guatiao/tests/c_consumer/catalog_library.c"));
    if !cfg!(windows) {
        compile.arg("-fPIC");
    }
    let compiled = compile.output().expect("the compiler runs");
    assert!(
        compiled.status.success(),
        "catalog_library.c did not compile.\nstderr:\n{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    Some(dir)
}

/// `sha256:` and the hex of the file's bytes, as a host would compute it.
fn sha256_of(path: &Path) -> String {
    let sum = sha2::Sha256::digest(std::fs::read(path).expect("the file reads"));
    let hex: String = sum.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

/// What a settings screen shows of one provider.
#[derive(Debug)]
struct Row {
    kinds: Vec<String>,
    name: String,
    version: String,
    digest: String,
    unavailable: Option<String>,
    schema: Option<Value>,
    form: Option<Value>,
}

#[test]
fn a_rust_host_lists_every_provider_of_a_scanned_directory_and_builds_none() {
    let Some(dir) = plugins("provider_catalog_rust") else {
        println!(
            "skipped: no C compiler found (tried clang, the default LLVM install path, cc and gcc)."
        );
        return;
    };
    let mut registry = Registry::new("catalog-host", "1.0");
    let report = scan_dir(&mut registry, &dir).expect("the directory reads");
    assert_eq!(report.loaded.len(), 2, "{report:?}");
    assert!(report.failed.is_empty(), "{report:?}");

    let mut rows = std::collections::HashMap::new();
    for provider in registry.all() {
        let library = registry
            .library_of(provider)
            .expect("every provider has its library");
        let form = provider_form(provider.config_schema(), provider.meta(), Alloc::rust())
            .expect("a form or none");
        rows.insert(
            provider.id().to_string(),
            Row {
                kinds: provider.kinds().to_vec(),
                name: provider.display_name().to_string(),
                version: provider.version().to_string(),
                digest: library
                    .digest()
                    .expect("the file reads")
                    .expect("a library from a file has one"),
                unavailable: provider.available().err().map(str::to_string),
                schema: provider.config_schema().cloned(),
                form,
            },
        );
    }
    let mut ids: Vec<&str> = rows.keys().map(String::as_str).collect();
    ids.sort_unstable();
    assert_eq!(
        ids,
        [
            "catalog_configured",
            "catalog_implied",
            "catalog_plain",
            "catalog_unavailable",
            "derived_greeter_hello",
            "derived_greeter_shouter",
        ]
    );

    let c_digest = sha256_of(&dir.join(format!(
        "{}catalog_library{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    )));
    let greeter_digest = sha256_of(&dir.join(format!(
        "{}derived_greeter{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    )));

    // Its own form, under `meta["form"]`, at a version of its own.
    let configured = &rows["catalog_configured"];
    assert_eq!(configured.kinds, ["counter"]);
    assert_eq!(configured.name, "Configured, with a form");
    assert_eq!(configured.version, "2.0");
    assert_eq!(configured.digest, c_digest);
    assert_eq!(configured.unavailable, None);
    let schema = SchemaRef::new(configured.schema.as_ref().expect("a schema")).expect("a map");
    let form = FormRef::new(configured.form.as_ref().expect("a form")).expect("a map");
    check(schema, form).expect("the declared form fits its schema");
    let sections: Vec<(&str, &str)> = form.sections().map(|s| (s.id(), s.label())).collect();
    assert_eq!(sections, [("net", "Network")]);
    assert_eq!(form.hints("port").placeholder(), "8080");

    // No form: the one its schema implies, a section by id and the secret
    // masked.
    let implied = &rows["catalog_implied"];
    assert_eq!(implied.version, "1.0", "its library's");
    let schema = SchemaRef::new(implied.schema.as_ref().expect("a schema")).expect("a map");
    let form = FormRef::new(implied.form.as_ref().expect("an implied form")).expect("a map");
    check(schema, form).expect("what a schema implies fits it");
    let sections: Vec<&str> = form.sections().map(|s| s.id()).collect();
    assert_eq!(sections, ["account"]);
    assert_eq!(form.hints("token").widget(), "password");

    // Nothing to configure, and one that cannot run here, saying why.
    let unavailable = &rows["catalog_unavailable"];
    assert_eq!(unavailable.unavailable.as_deref(), Some("no device here"));
    assert!(unavailable.schema.is_none() && unavailable.form.is_none());
    let plain = &rows["catalog_plain"];
    assert_eq!(plain.unavailable, None);
    assert!(plain.schema.is_none() && plain.form.is_none());

    // The Rust library: the derive's form, and a provider of three kinds.
    let shouter = &rows["derived_greeter_shouter"];
    assert_eq!(shouter.digest, greeter_digest);
    let form = FormRef::new(shouter.form.as_ref().expect("a form")).expect("a map");
    let sections: Vec<&str> = form.sections().map(|s| s.id()).collect();
    assert_eq!(sections, ["words", "volume"]);
    let hello = &rows["derived_greeter_hello"];
    assert_eq!(hello.kinds, ["greeter", "counter", "wide"]);
    assert!(hello.form.is_none());

    // Now one instance: the refusal says it was the first create asked.
    let mut config = Map::new();
    config.set("host", "localhost");
    config.set("port", 8080);
    let refused = registry
        .provider("catalog_configured")
        .expect("filed under its id")
        .instantiate::<dyn Counter>(&config.into());
    match refused {
        Err(InstantiateError::Refused(e)) => assert_eq!(
            e.message(),
            "built nothing: the first create this library was asked",
            "listing every provider asked for an instance"
        ),
        other => panic!("the library refuses every instance: {other:?}"),
    }
}

#[test]
fn a_library_s_digest_is_of_the_file_it_was_mapped_from() {
    let Some(dir) = plugins("provider_catalog_digest") else {
        println!("skipped: no C compiler found.");
        return;
    };
    let file = dir.join(format!(
        "{}catalog_library{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    let mapped = sha256_of(&file);
    let mut registry = Registry::new("catalog-host", "1.0");
    registry.load_file(&file).expect("it loads");

    // Another file at that path now: renaming a mapped file works on every
    // platform, where overwriting one does not.
    let moved = dir.join("moved_away.bin");
    std::fs::rename(&file, &moved).expect("a mapped file renames");
    std::fs::write(&file, b"not the library").expect("a new file at the old path");

    let library = &registry.loaded()[0];
    assert_eq!(
        library.digest().expect("reads").expect("from a file"),
        mapped,
        "the digest is of what was mapped, not of what the path names now"
    );
    assert_ne!(sha256_of(&file), mapped);

    // A library from no file has none.
    let mut linked = Registry::new("catalog-host", "1.0");
    linked
        .register_local(
            "nothing",
            |_host| -> Option<guatiao::library::kind::LibraryParts> {
                Some(guatiao::library::kind::LibraryParts::new(
                    "nothing",
                    "1.0",
                    vec![],
                ))
            },
        )
        .expect("registers");
    assert_eq!(linked.loaded()[0].digest().expect("no I/O"), None);
}

#[test]
fn a_c_host_lists_every_provider_and_its_form_and_builds_none() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source = manifest.join("tests/c/provider_catalog.c");
    let Some(cc) = c_compiler::find() else {
        println!("skipped: no C compiler found.");
        return;
    };
    let (Some(core), Some(intake)) = (shared_library("guatiao"), shared_library("guatiao_intake"))
    else {
        println!(
            "skipped: guatiao and guatiao_intake are not beside this test binary; \
             run `cargo build --workspace --all-features` first."
        );
        return;
    };
    let plugins = plugins("provider_catalog_c").expect("a compiler was found above");
    let lib_dir = core.parent().expect("in a directory").to_path_buf();
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(if cfg!(windows) {
        "greeter_host_provider_catalog.exe"
    } else {
        "greeter_host_provider_catalog"
    });

    let crates = manifest.join("../../crates");
    let mut compile = c_compiler::command(&cc);
    compile
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror"])
        .arg("-I")
        .arg(crates.join("guatiao/include"))
        .arg("-I")
        .arg(crates.join("guatiao-intake/include"))
        .arg("-o")
        .arg(&out)
        .arg(&source);
    if cfg!(windows) {
        compile
            .arg(core.with_extension("dll.lib"))
            .arg(intake.with_extension("dll.lib"));
    } else {
        compile
            .arg(format!("-L{}", lib_dir.display()))
            .arg(format!(
                "-L{}",
                intake.parent().expect("in a directory").display()
            ))
            .arg("-lguatiao_intake")
            .arg("-lguatiao")
            .arg(format!("-Wl,-rpath,{}", lib_dir.display()));
    }
    let compiled = compile.output().expect("the compiler runs");
    assert!(
        compiled.status.success(),
        "the C host did not compile or link.\nstderr:\n{}",
        String::from_utf8_lossy(&compiled.stderr)
    );

    let path = std::env::join_paths(core::iter::once(lib_dir.clone()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .expect("a joinable PATH");
    let run = Command::new(&out)
        .arg(&plugins)
        .current_dir(&lib_dir)
        .env("PATH", path)
        .output()
        .expect("the C host was just built");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success() && stdout.contains("all checks passed"),
        "the C host failed its own checks.\nstdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    println!("{}", stdout.trim());
}
