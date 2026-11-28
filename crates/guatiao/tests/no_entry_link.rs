// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Two crates that call `providers!` link into one executable once each
//! has `no-entry` on. Without it the entry symbols collide where
//! the linker rejects duplicates (MSVC), which the control checks.

#![cfg(feature = "provider")]

use std::path::{Path, PathBuf};
use std::process::Command;

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Builds the executable of a scratch workspace with `features` on for
/// both provider crates; true when it linked.
fn builds(name: &str, features: &str) -> bool {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&root);
    let guatiao = Path::new(env!("CARGO_MANIFEST_DIR"))
        .to_str()
        .unwrap()
        .replace('\\', "/");
    let kind = format!("{guatiao}/../../examples/greeter_kind");
    for n in ["pa", "pb"] {
        write(
            &root.join(n).join("Cargo.toml"),
            &format!(
                "[package]\nname = \"{n}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
                 [lib]\ncrate-type = [\"cdylib\", \"rlib\"]\n\
                 [dependencies]\nguatiao = {{ path = \"{guatiao}\", features = [\"provider\"] }}\n\
                 greeter_kind = {{ path = \"{kind}\" }}\n\
                 [features]\nno-entry = []\n"
            ),
        );
        write(
            &root.join(n).join("src/lib.rs"),
            "use greeter_kind::Counter;\n\
             #[derive(Default, guatiao::Provider)]\n#[provider(Counter)]\npub struct C;\n\
             impl Counter for C { fn count(&self) -> i64 { 0 } }\n\
             #[cfg(feature = \"no-entry\")]\nguatiao::providers!(no_entry; C);\n\
             #[cfg(not(feature = \"no-entry\"))]\nguatiao::providers!(C);\n",
        );
    }
    write(
        &root.join("Cargo.toml"),
        &format!(
            "[package]\nname = \"bin\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
             [workspace]\nmembers = [\"pa\", \"pb\"]\n\
             [profile.dev]\ncodegen-units = 1\n\
             [dependencies]\ngreeter_kind = {{ path = \"{kind}\" }}\n\
             pa = {{ path = \"pa\"{features} }}\npb = {{ path = \"pb\"{features} }}\n"
        ),
    );
    write(
        &root.join("src/main.rs"),
        "use greeter_kind::Counter;\nfn main() { println!(\"{}\", pa::C.count() + pb::C.count()); }\n",
    );
    Command::new(env!("CARGO"))
        .args(["build", "--offline", "--quiet"])
        .current_dir(&root)
        .env("CARGO_TARGET_DIR", root.join("target"))
        .status()
        .unwrap()
        .success()
}

#[test]
fn two_provider_crates_link_with_no_entry() {
    assert!(builds("no_entry_on", ", features = [\"no-entry\"]"));
}

#[test]
#[cfg(target_env = "msvc")]
fn the_same_crates_collide_without_the_feature() {
    assert!(!builds("no_entry_off", ""));
}
