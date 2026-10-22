// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The example host scanning a library written in Fortran
//! (`examples/fortran_greeter`, built with its CMake project) and calling
//! its greeters as the trait. `GUATIAO_FORTRAN_LIBRARY` names the
//! directory holding that build, and without it this states a skip. The
//! Fortran library finds its own guatiao through `GUATIAO_LIBRARY`, set
//! here to the workspace's build.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn the_host_calls_greeters_written_in_fortran_as_an_ordinary_library() {
    let Ok(dir) = std::env::var("GUATIAO_FORTRAN_LIBRARY") else {
        println!(
            "skipped: GUATIAO_FORTRAN_LIBRARY names no directory holding the Fortran greeters \
             built from examples/fortran_greeter"
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
        "fortran_greeter_hello [greeter, counter] from fortran_greeter",
        "fortran_greeter_hello greets: hello, ana",
        "fortran_greeter_hello shouts: HELLO, ANA",
        "fortran_greeter_hello checked a configuration, added [name, greeting], usable: true",
        "fortran_greeter_hello has been asked 1 time(s)",
        "fortran_greeter_shouter configured with ShoutSettings { prefix: \"hey\" } greets: hey ANA",
        "fortran_greeter_hello held a conversation of 2 turn(s)",
        "and was dropped with the conversation: true",
    ] {
        assert!(stdout.contains(line), "no {line:?} in\n{stdout}");
    }
}

/// A Fortran library holds a Fortran runtime the host cannot see: its own
/// `unload` slot always refuses, and the host leaves it mapped. Loaded in
/// this process, so `GUATIAO_LIBRARY` must already name the workspace's
/// build for the library to open.
#[test]
fn a_fortran_library_refuses_to_be_unmapped() {
    let (Ok(dir), Ok(_)) = (
        std::env::var("GUATIAO_FORTRAN_LIBRARY"),
        std::env::var("GUATIAO_LIBRARY"),
    ) else {
        println!(
            "skipped: GUATIAO_FORTRAN_LIBRARY names no Fortran library, or GUATIAO_LIBRARY is \
             unset for the guatiao it opens"
        );
        return;
    };
    let file = std::fs::read_dir(&dir)
        .expect("the Fortran library's directory reads")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| stem.ends_with("fortran_greeter"))
                && path.extension().and_then(|e| e.to_str())
                    == Some(std::env::consts::DLL_EXTENSION)
        })
        .expect("a fortran_greeter library in GUATIAO_FORTRAN_LIBRARY");
    let mut registry = guatiao::library::Registry::new("fortran-unload", "0.1");
    let key = registry
        .load_file(&file)
        .expect("the Fortran library loads")
        .loaded()
        .expect("and is not skipped")
        .key
        .clone();
    let refused = registry
        .unload(&key)
        .expect_err("a Fortran library stays mapped");
    assert!(
        matches!(
            refused,
            guatiao::library::UnloadError::Refused { status, .. } if status == guatiao::Status::GUATIAO_ERR_BUSY
        ),
        "{refused}"
    );
}
