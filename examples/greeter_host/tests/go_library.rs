// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The example host scanning a library written in Go, built with
//! `go build -buildmode=c-shared` (`examples/go_provider/cshared`), and
//! calling its greeters as the trait. `GUATIAO_GO_LIBRARY` names the
//! directory holding that build, and without it this states a skip. The
//! Go library finds its own guatiao through `GUATIAO_LIBRARY`, set here
//! to the workspace's build.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn the_host_calls_greeters_written_in_go_as_an_ordinary_library() {
    let Ok(dir) = std::env::var("GUATIAO_GO_LIBRARY") else {
        println!(
            "skipped: GUATIAO_GO_LIBRARY names no directory holding the Go greeters built with \
             `go build -buildmode=c-shared ./go_provider/cshared` in examples/"
        );
        return;
    };
    let exe = std::env::current_exe().expect("a test binary knows its own path");
    let workspace: PathBuf = exe
        .parent()
        .and_then(|deps| deps.parent())
        .expect("target/debug/deps")
        .to_path_buf();

    let output = Command::new(env!("CARGO_BIN_EXE_greeter_host"))
        .env("GREETER_HOST_PATH", &dir)
        .env("GUATIAO_LIBRARY", &workspace)
        .output()
        .expect("the host runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "the host failed\n--- stdout\n{stdout}\n--- stderr\n{stderr}"
    );
    for line in [
        "go_greeter_hello [greeter, counter] from github.com/Boriken-Dev/guatiao/examples/go_provider/go_greeter",
        "go_greeter_hello greets: hello, ana",
        "go_greeter_hello shouts: HELLO, ANA",
        "go_greeter_hello checked a configuration, added [name, greeting], usable: true",
        "go_greeter_hello has been asked 1 time(s)",
        "go_greeter_shouter configured with ShoutSettings { prefix: \"hey\" } greets: hey ANA",
        "go_greeter_hello held a conversation of 2 turn(s)",
        "and was dropped with the conversation: true",
    ] {
        assert!(stdout.contains(line), "no {line:?} in\n{stdout}");
    }
}

/// A Go library holds a Go runtime, which cannot be unloaded: its own
/// `unload` slot always refuses, and the host leaves it mapped. Loaded in
/// this process, so `GUATIAO_LIBRARY` must already name the workspace's
/// build for the Go runtime to read when it starts.
#[test]
fn a_go_library_refuses_to_be_unmapped() {
    let (Ok(dir), Ok(_)) = (
        std::env::var("GUATIAO_GO_LIBRARY"),
        std::env::var("GUATIAO_LIBRARY"),
    ) else {
        println!(
            "skipped: GUATIAO_GO_LIBRARY names no Go library, or GUATIAO_LIBRARY is unset for \
             the Go runtime it starts"
        );
        return;
    };
    let file = PathBuf::from(dir).join(format!(
        "{}go_greeter{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    let mut registry = guatiao::library::Registry::new("go-unload", "0.1");
    let key = registry
        .load_file(&file)
        .expect("the Go library loads")
        .loaded()
        .expect("and is not skipped")
        .key
        .clone();
    let refused = registry
        .unload(&key)
        .expect_err("a Go runtime stays mapped");
    assert!(
        matches!(
            refused,
            guatiao::library::UnloadError::Refused { status, .. } if status == guatiao::Status::GUATIAO_ERR_BUSY
        ),
        "{refused}"
    );
}
