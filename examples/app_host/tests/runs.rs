// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The program starts, which on Windows means the loader found
//! `myapp.dll` beside it and bound the one symbol it imports.

#[test]
fn the_program_runs_beside_the_application_library() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_app_host"))
        .output()
        .expect("the program starts");
    assert!(
        out.status.success(),
        "exit {:?}: {} (on Windows `myapp.dll` must be built beside it: `cargo build --workspace`)",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    if cfg!(windows) {
        assert!(text.contains("myapp 1 from its library"), "{text}");
    } else {
        assert!(text.contains("guatiao from the crate"), "{text}");
    }
}
