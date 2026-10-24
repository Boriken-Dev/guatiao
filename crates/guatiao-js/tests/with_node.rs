// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The example greeter is one file for two engines: this runs it under
//! the Node binding, through the example host that binding's tests use,
//! and checks what it answers there is what `source.rs` checks of it here.
//!
//! Needs Node 22.18 or later, the Node binding built (`npm run build` in
//! `bindings/node/guatiao`), `npm ci` in `examples`, and `GUATIAO_LIBRARY`
//! naming a build of the guatiao library; without them it says it skips.

use std::path::PathBuf;
use std::process::Command;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn the_greeter_runs_under_node_too() {
    let Some(library) = std::env::var_os("GUATIAO_LIBRARY") else {
        println!("skipped: GUATIAO_LIBRARY names no guatiao build");
        return;
    };
    if !repo().join("bindings/node/guatiao/dist/index.js").is_file()
        || !repo().join("examples/node_modules/guatiao").exists()
    {
        println!("skipped: the Node binding is not built, or `npm ci` was not run in examples");
        return;
    }
    let ran = match Command::new("node")
        .arg(repo().join("examples/node_host/host.ts"))
        .arg("--node-library")
        .arg(repo().join("examples/js_provider/quickjs_greeter/greeters.js"))
        .env("GUATIAO_LIBRARY", library)
        .output()
    {
        Ok(ran) => ran,
        Err(e) => {
            println!("skipped: no node to run: {e}");
            return;
        }
    };
    let stdout = String::from_utf8_lossy(&ran.stdout);
    if !ran.status.success()
        && String::from_utf8_lossy(&ran.stderr).contains("ERR_UNKNOWN_FILE_EXTENSION")
    {
        println!("skipped: this Node does not run TypeScript (22.18 or later does)");
        return;
    }
    assert!(
        ran.status.success(),
        "stdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    for line in [
        "quickjs_greeter_hello (quickjs_greeter 1.0.0)",
        r#"greet: {"greeting":"hello, ana"}"#,
        "greet refused: BadValue: nobody to greet",
        r#"greetInto: the lent listener heard ["hello, dee"]"#,
        r#"conversation: 2 turns, transcript "hi again", listener heard ["hi","again"]"#,
        "quickjs_greeter_polite (quickjs_greeter 1.0.0)",
        "greet refused: BadValue: nobody to greet, politely",
    ] {
        assert!(stdout.contains(line), "{line} missing:\n{stdout}");
    }
}
