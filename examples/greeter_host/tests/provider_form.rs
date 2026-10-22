// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A provider's form, read back by a host from the library on disk and
//! checked against the provider's configuration schema: from Rust, and from
//! a C program that links the two shared libraries and nothing else.
//!
//! `derived_greeter_shouter` declares `#[provider(Greeter, config, form =
//! Shouter)]`: two sections, a placeholder, a unit and a field shown only
//! when another holds `true`.

use std::path::PathBuf;
use std::process::Command;

use greeter_kind::Greeter;
use guatiao::library::Registry;
use guatiao::schema::SchemaRef;
use guatiao::{Map, TryAsRef, Value};
use guatiao_intake::{FormRef, PROVIDER_FORM_KEY, check, is_visible};

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

/// The example library, which cargo builds as a dev-dependency.
fn derived_greeter() -> PathBuf {
    shared_library("derived_greeter")
        .expect("the dev-dependency's cdylib is beside the test binary")
}

/// `{"exclaim": <exclaim>}`, the values `is_visible` judges by.
fn exclaiming(exclaim: bool) -> Value {
    let mut values = Map::new();
    values.set("exclaim", exclaim);
    values.into()
}

#[test]
fn a_provider_s_form_is_read_back_from_its_meta_and_checked() {
    let mut registry = Registry::new("form-host", "1.0");
    registry
        .load_file(&derived_greeter())
        .expect("the example library loads");
    let offer = registry
        .offer::<dyn Greeter>("derived_greeter_shouter")
        .expect("filed under its id")
        .expect("a valid table");

    let form = offer
        .meta()
        .and_then(|meta| meta.get(PROVIDER_FORM_KEY))
        .and_then(FormRef::new)
        .expect("the descriptor carries a form under `form`");
    let schema = offer
        .config_schema()
        .and_then(SchemaRef::new)
        .expect("a provider built from a configuration declares its schema");
    check(schema, form).expect("the form fits the schema it is shown beside");

    let sections: Vec<(&str, &str)> = form.sections().map(|s| (s.id(), s.label())).collect();
    assert_eq!(sections, [("words", "Words"), ("volume", "Volume")]);
    assert_eq!(form.hints("prefix").placeholder(), "hey");
    let marks = form.hints("marks");
    assert_eq!(marks.unit(), "marks");
    let when = marks.visible_when().expect("`marks` waits on a condition");
    assert_eq!(when.field(), "exclaim");
    assert_eq!(when.equals(), Some(&Value::from(true)));

    assert!(!is_visible(schema, form, "marks", &exclaiming(false)).expect("declared"));
    assert!(is_visible(schema, form, "marks", &exclaiming(true)).expect("declared"));

    // The settings the form shows are the ones the provider is built from.
    let mut config = Map::new();
    config.set("prefix", "hey");
    config.set("exclaim", true);
    config.set("marks", 2);
    let shouter = offer
        .instantiate(&config.into())
        .expect("the configuration fits");
    let answer = shouter.greet("ana").expect("an answer");
    let greeting: &str = answer
        .get("greeting")
        .and_then(TryAsRef::<str>::try_as_ref)
        .expect("a greeting");
    assert_eq!(greeting, "hey ANA!!");

    let hello = registry
        .offer::<dyn Greeter>("derived_greeter_hello")
        .expect("filed")
        .expect("valid");
    assert!(hello.meta().is_none(), "no form, no meta");
}

/// The C compilers worth trying, in order.
fn find_compiler() -> Option<PathBuf> {
    for name in ["clang", r"C:\Program Files\LLVM\bin\clang.exe", "cc", "gcc"] {
        let path = PathBuf::from(name);
        if Command::new(&path)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
        {
            return Some(path);
        }
    }
    None
}

#[test]
fn a_c_host_reads_a_provider_s_form_and_checks_it() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source = manifest.join("tests/c/provider_form.c");
    let Some(cc) = find_compiler() else {
        println!(
            "skipped: no C compiler found (tried clang, the default LLVM install path, cc and gcc)."
        );
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
    let dir = core.parent().expect("in a directory").to_path_buf();
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(if cfg!(windows) {
        "greeter_host_provider_form.exe"
    } else {
        "greeter_host_provider_form"
    });

    let crates = manifest.join("../../crates");
    let mut compile = Command::new(&cc);
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
            .arg(format!("-L{}", dir.display()))
            .arg(format!(
                "-L{}",
                intake.parent().expect("in a directory").display()
            ))
            .arg("-lguatiao_intake")
            .arg("-lguatiao")
            .arg(format!("-Wl,-rpath,{}", dir.display()));
    }
    let compiled = compile.output().expect("the compiler runs");
    assert!(
        compiled.status.success(),
        "the C host did not compile or link.\nstderr:\n{}",
        String::from_utf8_lossy(&compiled.stderr)
    );

    // On Windows the two libraries are found beside the program's working
    // directory or on PATH; both live in `dir`.
    let path = std::env::join_paths(core::iter::once(dir.clone()).chain(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    )))
    .expect("a joinable PATH");
    let run = Command::new(&out)
        .arg(derived_greeter())
        .current_dir(&dir)
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
