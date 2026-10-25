// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What the tests share: a source served in this process, libraries
//! written to a scratch directory, and the limits map.

#![allow(dead_code)]

extern crate alloc;

use alloc::sync::Arc;
use std::path::{Path, PathBuf};

use greeter_kind::Greeter;
use guatiao::library::{Candidate, Host, LocalLibrary, ProviderError, Registry, Source};
use guatiao::{List, Map};
use guatiao_bridge::BridgeSource;
use guatiao_libffi::Libffi;
use guatiao_source_rhai::RhaiEngine;

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

pub fn limits(pairs: &[(&str, u64)]) -> Map {
    let mut map = Map::new();
    for (k, v) in pairs {
        map.set(k, *v);
    }
    map
}

pub fn config(dir: &Path, limits: Option<Map>) -> Map {
    let mut config = Map::new();
    config.set("dirs", texts(&[&dir.display().to_string()]));
    config.set("kinds", texts(&[&kinds_file()]));
    if let Some(limits) = limits {
        config.set("limits", limits);
    }
    config
}

struct Held(Arc<BridgeSource<RhaiEngine>>);

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
        BridgeSource::<RhaiEngine>::pinned(config, "rhai-test")
            .expect("Rhai starts")
            .with_mechanism(Arc::new(Libffi)),
    );
    let mut registry = Registry::new("rhai-test-host", "1.0");
    registry.add_source("rhai", Held(source)).expect("added");
    registry
}

pub fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("guatiao-rhai-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// A library of one script: `code` is its file, `id` its name.
pub fn put(dir: &Path, id: &str, code: &str) {
    std::fs::write(
        dir.join(format!("{id}.guatiao-rhai.toml")),
        format!("module = \"{id}\"\nkinds = [\"greeter\", \"counter\"]\n"),
    )
    .expect("written");
    std::fs::write(dir.join(format!("{id}.rhai")), code).expect("written");
}

pub fn greeter(registry: &Registry, key: &str) -> Arc<dyn Greeter> {
    registry
        .offer::<dyn Greeter>(key)
        .unwrap_or_else(|| panic!("{key} is served"))
        .expect("as the trait")
        .shared()
        .expect("an instance")
}
