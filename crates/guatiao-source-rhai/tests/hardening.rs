// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a script must not be able to do to its host or to the sandbox: reach
//! a file through `import`, write to the host's standard output, or call
//! back into an object that is already running.

mod common;

extern crate alloc;

use alloc::sync::Arc;
use std::sync::Mutex;

use common::*;
use greeter_kind::{Greeter, Listener};
use guatiao::value::convert::TryAsRef;

/// A name no other test uses, so a file by it in the working directory is
/// this test's alone.
fn module_name() -> String {
    format!("rhai_elsewhere_{}", std::process::id())
}

#[test]
fn import_reaches_no_file_wherever_one_is() {
    let dir = scratch("import");
    let name = module_name();
    let module = "export const X = 42;";
    // Beside the script, and in the working directory, where Rhai's own
    // file resolver would look.
    std::fs::write(dir.join(format!("{name}.rhai")), module).expect("written");
    let in_cwd = std::path::PathBuf::from(format!("{name}.rhai"));
    std::fs::write(&in_cwd, module).expect("written");
    put(
        &dir,
        "rhai_importing",
        &format!(
            r#"
const VERSION = "1.0.0";
let p = #{{ kinds: ["greeter"], greet: |name| {{
    import "{name}" as e;
    #{{ greeting: `${{e::X}}` }}
}} }};
#{{ providers: #{{ p: p }} }}
"#
        ),
    );
    let mut registry = serving(&config(&dir, None));
    assert!(registry.refresh().refused.is_empty());
    let refused = greeter(&registry, "rhai_importing_p").greet("go");
    let _ = std::fs::remove_file(&in_cwd);
    let refused = refused.unwrap_err();
    assert!(
        refused.message().contains(&name),
        "the module is not found: {}",
        refused.message()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

const LOUD: &str = r#"
const VERSION = "1.0.0";
let p = #{ kinds: ["greeter"], greet: |name| {
    print("SCRIPT-PRINTED-THIS");
    debug("SCRIPT-DEBUGGED-THIS");
    #{ greeting: name }
} };
#{ providers: #{ p: p } }
"#;

/// The child half of [`a_script_writes_nothing_to_the_hosts_output`]: it
/// runs the loud script and prints what the script did not.
#[test]
fn loud_script_child() {
    if std::env::var_os("RHAI_LOUD_CHILD").is_none() {
        return;
    }
    let dir = scratch("loud");
    put(&dir, "rhai_loud", LOUD);
    let mut registry = serving(&config(&dir, None));
    assert!(registry.refresh().refused.is_empty());
    assert!(greeter(&registry, "rhai_loud_p").greet("go").is_ok());
    println!("CHILD-REACHED-THE-END");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_script_writes_nothing_to_the_hosts_output() {
    let out = std::process::Command::new(std::env::current_exe().expect("this test"))
        .args(["--exact", "loud_script_child", "--nocapture"])
        .env("RHAI_LOUD_CHILD", "1")
        .output()
        .expect("ran");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stdout.contains("CHILD-REACHED-THE-END"), "{stdout}{stderr}");
    for said in ["SCRIPT-PRINTED-THIS", "SCRIPT-DEBUGGED-THIS"] {
        assert!(
            !stdout.contains(said) && !stderr.contains(said),
            "{stdout}{stderr}"
        );
    }
}

#[test]
fn a_char_answers_a_text_slot() {
    let dir = scratch("char");
    put(
        &dir,
        "rhai_char",
        r#"
const VERSION = "1.0.0";
let p = #{ kinds: ["greeter"], greet: |name| #{ greeting: name }, shout: |name| 'x' };
#{ providers: #{ p: p } }
"#,
    );
    let mut registry = serving(&config(&dir, None));
    assert!(registry.refresh().refused.is_empty());
    assert_eq!(greeter(&registry, "rhai_char_p").shout("a"), "x");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_depth_stop_by_default_names_the_depth_it_stopped_at() {
    let dir = scratch("default-depth");
    put(
        &dir,
        "rhai_recursive",
        r#"
const VERSION = "1.0.0";
fn down(n) { 1 + down(n + 1) }
let p = #{ kinds: ["greeter"], greet: |name| { down(0); #{ greeting: name } } };
#{ providers: #{ p: p } }
"#,
    );
    let mut registry = serving(&config(&dir, None));
    assert!(registry.refresh().refused.is_empty());
    let stopped = greeter(&registry, "rhai_recursive_p")
        .greet("go")
        .unwrap_err();
    assert_eq!(stopped.message(), "stopped at its depth limit (32)");
    let _ = std::fs::remove_dir_all(&dir);
}

struct Calls(Arc<dyn Greeter>, Arc<Mutex<Vec<String>>>);

impl Listener for Calls {
    fn heard(&mut self, _what: &str) {
        let answer = self.0.greet("again").expect("answered");
        let said = answer
            .get("greeting")
            .and_then(TryAsRef::<str>::try_as_ref)
            .unwrap_or("")
            .to_string();
        self.1.lock().unwrap().push(said);
    }
}

#[test]
fn a_handle_called_while_it_runs_is_busy_and_the_host_carries_on() {
    let dir = scratch("handle-busy");
    // `p` keeps the listener it is handed in a variable `q` also sees,
    // and calls it; the host's listener calls `q`, which calls it again.
    put(
        &dir,
        "rhai_keep",
        r#"
const VERSION = "1.0.0";
let keep = ();
let p = #{ kinds: ["greeter"],
    greet: |name| #{ greeting: name },
    start: |listener| {
        keep = listener;
        let local = keep;
        local.heard("first");
        #{ say: |what| (), read: |dst| 0, turns: || 0 }
    },
};
let q = #{ kinds: ["greeter"], greet: |name| {
    // Copies of the handle: the same object, not the shared variable
    // Rhai itself guards.
    let again = keep;
    try {
        again.heard("second");
        return #{ greeting: "reached the object" };
    } catch (e) {
        return #{ greeting: `${e.status}: ${e.message}` };
    }
} };
#{ providers: #{ p: p, q: q } }
"#,
    );
    let mut registry = serving(&config(&dir, None));
    assert!(registry.refresh().refused.is_empty());
    let p = greeter(&registry, "rhai_keep_p");
    let q = greeter(&registry, "rhai_keep_q");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let conversation = p
        .start(Calls(q, Arc::clone(&seen)).into_object())
        .expect("the host's listener ran, and the engine carried on");
    drop(conversation);
    let said = seen.lock().unwrap().pop().expect("heard");
    assert!(
        said.starts_with("busy: the object is already in use"),
        "{said}"
    );
    assert!(p.greet("after").is_ok(), "the engine answers the next call");
    let _ = std::fs::remove_dir_all(&dir);
}
