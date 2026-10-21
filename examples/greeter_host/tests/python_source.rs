// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The example host adding the guatiao-py bridge as a source and calling
//! greeters written in Python as the trait. The bridge is its own
//! workspace, built against a Python: `GUATIAO_PY_BRIDGE` names the build,
//! and without it this states a skip.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn the_host_calls_greeters_written_in_python_through_the_bridge() {
    let Ok(bridge) = std::env::var("GUATIAO_PY_BRIDGE") else {
        println!(
            "skipped: GUATIAO_PY_BRIDGE names no guatiao-py build (crates/guatiao-py, built \
             against a Python whose library is on the search path)"
        );
        return;
    };
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let slash = |p: PathBuf| {
        p.canonicalize()
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/")
            .trim_start_matches("//?/")
            .to_string()
    };
    let config = format!(
        r#"{{"dirs": ["{}"], "paths": ["{}", "{}"]}}"#,
        slash(repo.join("examples/python_provider")),
        slash(repo.join("bindings/python/src")),
        slash(repo.join("examples/greeter_kind/python")),
    );
    let exe = std::env::current_exe().expect("a test binary knows its own path");
    let deps = exe.parent().expect("in a directory").to_path_buf();

    let output = Command::new(env!("CARGO_BIN_EXE_greeter_host"))
        .arg(&deps)
        .env("GREETER_HOST_SOURCE", &bridge)
        .env("GREETER_HOST_SOURCE_CONFIG", &config)
        .output()
        .expect("the host runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "the host failed\n--- stdout\n{stdout}\n--- stderr\n{stderr}"
    );
    assert!(
        stdout.contains("served py_greeter as py_greeter"),
        "{stdout}"
    );
    assert!(
        stdout.contains("py_greeter_hello greets: hello, ana"),
        "{stdout}"
    );
    assert!(
        stdout.contains("py_greeter_hello shouts: HELLO, ANA"),
        "{stdout}"
    );
    assert!(
        stdout.contains(
            "py_greeter_shouter configured with ShoutSettings { prefix: \"hey\" } greets: hey ANA"
        ),
        "{stdout}"
    );
    assert!(
        stdout.contains("py_greeter_hello held a conversation of 2 turn(s)"),
        "{stdout}"
    );
}
