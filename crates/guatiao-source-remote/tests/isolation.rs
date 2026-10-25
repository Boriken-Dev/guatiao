// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A provider written in Python, served by the guatiao-source-py source inside a
//! server in a child process, kills that process mid-call: the host's
//! call answers `GONE`, the host carries on, and a refresh starts the
//! child again and serves the provider anew.
//!
//! Needs a guatiao-source-py build (`GUATIAO_PY_BRIDGE`) and, on Windows, its
//! Python on `PATH`; skips, saying so, without them.

mod common;

use std::path::{Path, PathBuf};

use greeter_kind::Greeter;
use guatiao::Status;
use guatiao::library::Registry;
use guatiao::value::convert::TryAsRef;
use guatiao_source_remote::transport::child::ChildCommand;
use guatiao_source_remote::{LinkOptions, RemoteSource, Trust};

const MODULE: &str = r#"
import os

from greeter_kind import Greeter
from guatiao import provide

__version__ = "1.0.0"
__all__ = ["Fragile"]


class Fragile(provide.Provider, Greeter):
    def greet(self, name):
        if name == "crash":
            # The whole server process goes, as a crash in native code would.
            os.abort()
        return {"greeting": f"hello, {name}", "pid": str(os.getpid())}
"#;

fn here() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn a_crashing_python_provider_leaves_the_host_running_and_a_refresh_restarts_it() {
    let Some(bridge) = std::env::var_os("GUATIAO_PY_BRIDGE").map(PathBuf::from) else {
        println!("skipped: GUATIAO_PY_BRIDGE names no guatiao-source-py build");
        return;
    };
    let Some(core) = common::procs::library("guatiao") else {
        println!("skipped: the guatiao library is not built beside this test");
        return;
    };
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("remote_isolation");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("py_fragile.py"), MODULE).unwrap();
    std::fs::write(
        dir.join("py_fragile.guatiao.toml"),
        "type = \"python\"\nmodule = \"py_fragile\"\nkinds = [\"greeter\"]\npaths = [\".\"]\n",
    )
    .unwrap();

    let python_path = std::env::join_paths([
        here().join("../../bindings/python/guatiao/src"),
        here().join("../../examples/greeter_kind/python"),
    ])
    .unwrap();
    let config = format!(
        "{}={{\"dirs\": [{:?}]}}",
        bridge.display(),
        dir.display().to_string()
    );
    let child = ChildCommand::new(env!("CARGO_BIN_EXE_guatiao-remote-server"))
        .args(["--stdio", "--kinds"])
        .arg(here().join("../../examples/greeter_kind/include/greeter_kind.kinds.json"))
        .arg("--source")
        .arg(config)
        .env("PYTHONPATH", python_path)
        .env("GUATIAO_LIBRARY", core.parent().unwrap_or(Path::new(".")));
    let source = RemoteSource::new(Box::new(child), LinkOptions::new(Trust::Local));
    let mut registry = Registry::new("host", "1.0");
    registry.add_source("python", source).unwrap();
    let refreshed = registry.refresh();
    assert!(refreshed.refused.is_empty(), "{refreshed:?}");
    assert_eq!(refreshed.loaded.len(), 1, "{refreshed:?}");

    let pid = |registry: &Registry| {
        let greeter = registry
            .offer::<dyn Greeter>("py_fragile_fragile")
            .expect("served")
            .unwrap()
            .shared()
            .unwrap();
        let answer = greeter.greet("ana").unwrap();
        assert_eq!(
            answer.get("greeting").and_then(TryAsRef::<str>::try_as_ref),
            Some("hello, ana")
        );
        answer
            .get("pid")
            .and_then(TryAsRef::<str>::try_as_ref)
            .unwrap()
            .to_string()
    };
    let first = pid(&registry);

    let greeter = registry
        .offer::<dyn Greeter>("py_fragile_fragile")
        .unwrap()
        .unwrap()
        .shared()
        .unwrap();
    let e = greeter.greet("crash").unwrap_err();
    assert_eq!(e.status, Status::GUATIAO_ERR_GONE, "{}", e.message());
    // The host is still here, and its stale provider says so.
    assert_eq!(
        greeter.greet("bo").unwrap_err().status,
        Status::GUATIAO_ERR_GONE
    );
    drop(greeter);

    let refreshed = registry.refresh();
    assert!(refreshed.refused.is_empty(), "{refreshed:?}");
    assert_eq!(refreshed.loaded.len(), 1, "{refreshed:?}");
    let second = pid(&registry);
    assert_ne!(first, second, "a new child serves it");
    let _ = registry.collect();
}
