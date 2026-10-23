// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The example host scanning a library written in C++
//! (`examples/cpp_greeter`, built with its CMake project) and calling its
//! greeters as the trait. `GUATIAO_CPP_LIBRARY` names the directory holding
//! that build, and without it this states a skip. The C++ library links
//! the guatiao library its CMake package found.

use std::path::PathBuf;
use std::process::Command;

fn cpp_library_dir() -> Option<String> {
    std::env::var("GUATIAO_CPP_LIBRARY").ok()
}

#[test]
fn the_host_calls_greeters_written_in_cpp_as_an_ordinary_library() {
    let Some(dir) = cpp_library_dir() else {
        println!(
            "skipped: GUATIAO_CPP_LIBRARY names no directory holding the C++ greeters built from \
             examples/cpp_greeter"
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
        "cpp_greeter_hello [greeter, counter] from cpp_greeter",
        "cpp_greeter_hello greets: hello, ana",
        "cpp_greeter_hello shouts: HELLO, ANA",
        "cpp_greeter_hello checked a configuration, added [name, greeting], usable: true",
        "cpp_greeter_hello greets from it: hello, world",
        "cpp_greeter_hello has been asked 1 time(s)",
        "cpp_greeter_shouter configured with ShoutSettings { prefix: \"hey\" } greets: hey ANA",
        "cpp_greeter_hello held a conversation of 2 turn(s)",
        "and was dropped with the conversation: true",
    ] {
        assert!(stdout.contains(line), "no {line:?} in\n{stdout}");
    }
}

/// A C++ library is unmapped once nothing it served is alive: its own
/// `unload` slot refuses while an instance lives, and the host's lease
/// refuses before even asking.
#[test]
fn a_cpp_library_is_unmapped_once_nothing_it_served_is_alive() {
    let Some(dir) = cpp_library_dir() else {
        println!("skipped: GUATIAO_CPP_LIBRARY names no C++ library");
        return;
    };
    let file = std::fs::read_dir(&dir)
        .expect("the C++ library's directory reads")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| stem.ends_with("cpp_greeter"))
                && path.extension().and_then(|e| e.to_str())
                    == Some(std::env::consts::DLL_EXTENSION)
        })
        .expect("a cpp_greeter library in GUATIAO_CPP_LIBRARY");
    let mut registry = guatiao::library::Registry::new("cpp-unload", "0.1");
    let key = registry
        .load_file(&file)
        .expect("the C++ library loads")
        .loaded()
        .expect("and is not skipped")
        .key
        .clone();
    let config = guatiao::Value::from({
        let mut m = guatiao::Map::new();
        m.set("prefix", "hey");
        m
    });
    let offer = registry
        .offer::<dyn greeter_kind::Greeter>("cpp_greeter_shouter")
        .expect("the shouter is a provider here")
        .expect("and speaks the greeter kind");
    let instance = offer.instantiate(&config).expect("an instance is built");
    assert_eq!(
        instance
            .greet("ana")
            .expect("it greets")
            .get("greeting")
            .and_then(guatiao::value::convert::TryAsRef::<str>::try_as_ref),
        Some("hey ANA")
    );
    let busy = registry
        .unload(&key)
        .expect_err("an instance holds the library");
    assert!(
        matches!(busy, guatiao::library::UnloadError::Busy { .. }),
        "{busy}"
    );
    drop(instance);
    drop(offer);
    registry
        .unload(&key)
        .expect("nothing it served is alive, and the library agrees");
}
