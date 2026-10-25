// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What the tests share: building the example guests, a source in this
//! process, and the kinds' traits.

#![allow(dead_code)]

extern crate alloc;

use alloc::sync::Arc;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use greeter_kind::{Greeter, Listener};
use guatiao::library::{Candidate, Host, LocalLibrary, ProviderError, Registry, Source};
use guatiao::value::convert::TryAsRef;
use guatiao::{List, Map};
use guatiao_bridge::BridgeSource;
use guatiao_libffi::Libffi;
use guatiao_wasm::WasmEngine;

pub fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn kinds_file() -> String {
    repo()
        .join("examples/greeter_kind/include/greeter_kind.kinds.json")
        .display()
        .to_string()
}

pub fn texts(items: &[&str]) -> List {
    let mut list = List::new();
    for item in items {
        list.push(*item);
    }
    list
}

pub fn config(dirs: &[&Path], limits: Option<Map>) -> Map {
    let dirs: Vec<String> = dirs.iter().map(|d| d.display().to_string()).collect();
    let dirs: Vec<&str> = dirs.iter().map(String::as_str).collect();
    let mut config = Map::new();
    config.set("dirs", texts(&dirs));
    config.set("kinds", texts(&[&kinds_file()]));
    if let Some(limits) = limits {
        config.set("limits", limits);
    }
    config
}

pub fn limits(pairs: &[(&str, u64)]) -> Map {
    let mut map = Map::new();
    for (k, v) in pairs {
        map.set(k, *v);
    }
    map
}

/// The source a registry holds, shared with the test.
pub struct Held(pub Arc<BridgeSource<WasmEngine>>);

impl Source for Held {
    fn list(&self, host: Host) -> Result<Vec<Candidate>, ProviderError> {
        self.0.list(host)
    }
    fn open(&self, host: Host, id: &str) -> Result<LocalLibrary<'static>, ProviderError> {
        self.0.open(host, id)
    }
    fn close(&self, id: &str, stamp: &str) {
        self.0.close(id, stamp);
    }
}

pub fn serving(config: &Map) -> Registry {
    let source = Arc::new(
        BridgeSource::<WasmEngine>::shared(config)
            .expect("wasmtime starts")
            .with_mechanism(Arc::new(Libffi)),
    );
    let mut registry = Registry::new("wasm-test-host", "1.0");
    registry.add_source("wasm", Held(source)).expect("added");
    registry
}

/// The role this process plays when a test runs itself again as a child.
pub fn child_role() -> Option<String> {
    std::env::var("GUATIAO_WASM_CHILD").ok()
}

/// Runs the test `name` again in a child process as `role`: what can take
/// the whole process down (a native stack overflow aborts it) is watched
/// from outside. Answers whether the child succeeded, and its output.
pub fn in_child(name: &str, role: &str) -> (bool, String) {
    let out = Command::new(std::env::current_exe().expect("this test binary"))
        .args([name, "--exact", "--nocapture", "--test-threads=1"])
        .env("GUATIAO_WASM_CHILD", role)
        .output()
        .expect("the child runs");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

pub fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("guatiao-wasm-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

pub fn greeter(registry: &Registry, key: &str) -> Arc<dyn Greeter> {
    registry
        .offer::<dyn Greeter>(key)
        .unwrap_or_else(|| panic!("{key} is served"))
        .expect("as the trait")
        .shared()
        .expect("an instance")
}

pub fn greeting(answer: &Map) -> &str {
    answer
        .get("greeting")
        .and_then(TryAsRef::<str>::try_as_ref)
        .unwrap_or("")
}

/// A listener the host implements.
pub struct Heard(pub Arc<Mutex<Vec<String>>>);

impl Listener for Heard {
    fn heard(&mut self, what: &str) {
        self.0.lock().unwrap().push(what.to_string());
    }
}

impl Drop for Heard {
    fn drop(&mut self) {
        self.0.lock().unwrap().push("<destroyed>".to_string());
    }
}

/// Whether a guest that cannot be built fails the test (CI) rather than
/// skipping it.
pub fn require_all() -> bool {
    std::env::var_os("GUATIAO_REQUIRE_ALL").is_some_and(|v| v == "1")
}

fn skipped(why: String) -> Option<PathBuf> {
    assert!(!require_all(), "GUATIAO_REQUIRE_ALL is set: {why}");
    println!("skipped: {why}");
    None
}

/// Builds the Rust example into `target_dir`, with `word` as its
/// greeting when given; answers the module.
pub fn build_rust_guest(target_dir: &Path, word: Option<&str>) -> Option<PathBuf> {
    let example = repo().join("examples/wasm_greeter_rust");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut build = Command::new(cargo);
    build
        .args(["build", "--release", "--target", "wasm32-unknown-unknown"])
        .arg("--manifest-path")
        .arg(example.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(target_dir)
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("WASM_GREETER_WORD");
    if let Some(word) = word {
        build.env("WASM_GREETER_WORD", word);
    }
    let out = match build.output() {
        Ok(out) => out,
        Err(e) => return skipped(format!("cargo did not run: {e}")),
    };
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        if stderr.contains("may not be installed") || stderr.contains("can't find crate for `core`")
        {
            return skipped(
                "the wasm32-unknown-unknown target is not installed (rustup target add wasm32-unknown-unknown)"
                    .to_string(),
            );
        }
        panic!("the Rust guest did not build:\n{stderr}");
    }
    Some(target_dir.join("wasm32-unknown-unknown/release/wasm_greeter_rust.wasm"))
}

/// Builds the Go example to `out`, with `word` as its greeting when given.
pub fn build_go_guest(out: &Path, word: Option<&str>) -> Option<PathBuf> {
    let example = repo().join("examples/wasm_greeter_go");
    let partial = out.with_extension(format!("{}.part", std::process::id()));
    let mut build = Command::new("go");
    build
        .args(["build", "-buildmode=c-shared"])
        .env("GOOS", "wasip1")
        .env("GOARCH", "wasm")
        .current_dir(&example);
    if let Some(word) = word {
        build.arg(format!("-ldflags=-X main.word={word}"));
    }
    build.arg("-o").arg(&partial).arg(".");
    let built = match build.output() {
        Ok(out) => out,
        Err(e) => {
            return skipped(format!(
                "no Go toolchain ({e}): Go 1.24 or later builds the Go guest"
            ));
        }
    };
    assert!(
        built.status.success(),
        "the Go guest did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    std::fs::rename(&partial, out).expect("the module moved into place");
    Some(out.to_path_buf())
}

/// The Rust example, built in its own directory, which its manifest names.
pub fn rust_example() -> Option<PathBuf> {
    static BUILT: OnceLock<Option<PathBuf>> = OnceLock::new();
    BUILT
        .get_or_init(|| {
            let example = repo().join("examples/wasm_greeter_rust");
            build_rust_guest(&example.join("target"), None).map(|_| example)
        })
        .clone()
}

/// The Go example, built beside its manifest.
pub fn go_example() -> Option<PathBuf> {
    static BUILT: OnceLock<Option<PathBuf>> = OnceLock::new();
    BUILT
        .get_or_init(|| {
            let example = repo().join("examples/wasm_greeter_go");
            build_go_guest(&example.join("wasm_greeter_go.wasm"), None).map(|_| example)
        })
        .clone()
}

fn library_name(base: &str) -> String {
    format!(
        "{}{base}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    )
}

/// This crate's own file, beside the test binary's directory.
pub fn source_file() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let debug = exe.parent()?.parent()?;
    let path = debug.join(library_name("guatiao_wasm"));
    path.is_file().then_some(path)
}
