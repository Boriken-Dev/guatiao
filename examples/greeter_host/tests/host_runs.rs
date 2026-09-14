// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The example host, run as a program against a directory holding a real
//! library, and read by its output. An example nobody runs drifts; this
//! one is run by `cargo test`.

use std::path::PathBuf;
use std::process::Command;

use guatiao::library::SearchPath;

/// Where cargo put the dev-dependency cdylib: beside this test binary
/// (`deps/`) and one up (`target/debug`). Both are named, so the run also
/// shows a library reachable twice loading once.
fn places() -> Vec<PathBuf> {
    let exe = std::env::current_exe().expect("a test binary knows its own path");
    let deps = exe.parent().expect("in a directory").to_path_buf();
    let file = format!(
        "{}derived_greeter{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let mut out = Vec::new();
    for dir in [
        deps.clone(),
        deps.parent().map(PathBuf::from).unwrap_or_default(),
    ] {
        if dir.join(&file).is_file() {
            out.push(dir);
        }
    }
    assert!(!out.is_empty(), "{file} is not beside {}", deps.display());
    out
}

#[test]
fn the_host_scans_offers_and_builds_a_configured_greeter() {
    let places = places();
    let spec = places
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(&SearchPath::SEPARATOR.to_string());

    let output = Command::new(env!("CARGO_BIN_EXE_greeter_host"))
        .env("GREETER_HOST_PATH", &spec)
        .output()
        .expect("the host runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "the host failed\n--- stdout\n{stdout}\n--- stderr\n{stderr}"
    );

    // 1. It looked where it was told, and the report names the library.
    assert!(stdout.contains("derived_greeter"), "{stdout}");
    // 2. The unconfigured provider answered as the trait, through both
    // of its kinds.
    assert!(
        stdout.contains("derived_greeter_hello greets: hello, ana"),
        "{stdout}"
    );
    assert!(
        stdout.contains("derived_greeter_hello shouts: HELLO, ANA"),
        "{stdout}"
    );
    assert!(stdout.contains("has been asked 1 time(s)"), "{stdout}");
    // 3. The configured one was built from the host's own type and
    // answered with it.
    assert!(
        stdout.contains("\"prefix\""),
        "the schema was shown: {stdout}"
    );
    assert!(
        stdout.contains("derived_greeter_shouter configured with ShoutSettings { prefix: \"hey\" } greets: hey ANA"),
        "{stdout}"
    );
    // And a value that does not fit was refused by the provider, not by
    // a crash.
    assert!(
        stdout.contains("derived_greeter_shouter given nothing:"),
        "{stdout}"
    );
    // 4. A conversation crossed back as an object the host drove; the
    // transcript came through the host's own buffer; the listener the
    // host handed in was called from the library and dropped with the
    // conversation -- both destroys ran, across the boundary.
    assert!(
        stdout.contains("derived_greeter_hello held a conversation of 2 turn(s)"),
        "{stdout}"
    );
    assert!(stdout.contains("  good morning\n  how are you"), "{stdout}");
    assert!(
        stdout.contains(
            "the listener heard [\"good morning\", \"how are you\"] and was dropped with the conversation: true"
        ),
        "{stdout}"
    );
}

#[test]
fn a_place_that_does_not_exist_is_reported_and_the_rest_is_still_walked() {
    let mut places = places();
    places.insert(0, PathBuf::from("no-such-directory-for-greeter-host"));
    let spec = places
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(&SearchPath::SEPARATOR.to_string());
    let output = Command::new(env!("CARGO_BIN_EXE_greeter_host"))
        .env("GREETER_HOST_PATH", &spec)
        .output()
        .expect("the host runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(
        stdout.contains("unreadable: no-such-directory-for-greeter-host"),
        "{stdout}"
    );
    assert!(stdout.contains("hey ANA"), "{stdout}");
}
