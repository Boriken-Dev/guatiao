// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Child processes for the tests that build and run C or start a server:
//! one started at a time, read on threads, waited for within a deadline.

use core::time::Duration;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, channel};
use std::time::Instant;

/// One child started at a time: on Windows a child inherits the pipes of
/// another started at the same moment, which then never reach their end.
pub static SPAWN: Mutex<()> = Mutex::new(());

/// How long any child here may take.
pub const DEADLINE: Duration = Duration::from_secs(60);

/// Starts `cmd` with its output piped, read on threads: what it printed
/// arrives line by line whoever else holds the pipes.
pub fn start(cmd: &mut Command) -> (Child, Receiver<String>) {
    let mut child = {
        let _one = SPAWN.lock().unwrap_or_else(|e| e.into_inner());
        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the program starts")
    };
    let (tx, lines) = channel::<String>();
    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();
    let tx2 = tx.clone();
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    std::thread::spawn(move || {
        for line in BufReader::new(err).lines().map_while(Result::ok) {
            let _ = tx2.send(line);
        }
    });
    (child, lines)
}

/// Waits for `child` within the deadline, then answers whether it
/// succeeded and the lines it printed.
pub fn finish(mut child: Child, lines: &Receiver<String>) -> (bool, String) {
    let until = Instant::now() + DEADLINE;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > until {
            let _ = child.kill();
            panic!("a child did not end within {DEADLINE:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    // What it printed last may still be on its way.
    let mut text = String::new();
    while let Ok(line) = lines.recv_timeout(Duration::from_millis(200)) {
        text.push_str(&line);
        text.push('\n');
    }
    (status.success(), text)
}

/// Runs `cmd` to its end.
pub fn run(cmd: &mut Command) -> (bool, String) {
    let (child, lines) = start(cmd);
    finish(child, &lines)
}

/// A C compiler: `GUATIAO_TEST_CC`, else clang, cc or gcc.
pub fn compiler() -> Option<PathBuf> {
    if let Ok(cc) = std::env::var("GUATIAO_TEST_CC") {
        return Some(PathBuf::from(cc));
    }
    super::c_compiler::find()
}

/// The directory the workspace's build put its libraries in: the test
/// binary's own, or the one above it.
pub fn target_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    exe.parent().unwrap().parent().unwrap().to_path_buf()
}

/// A shared library the workspace builds, `base` named as Cargo names it.
pub fn library(base: &str) -> Option<PathBuf> {
    let name = format!(
        "{}{base}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let exe = std::env::current_exe().ok()?;
    let deps = exe.parent()?;
    [deps.to_path_buf(), target_dir()]
        .into_iter()
        .map(|d| d.join(&name))
        .find(|p| p.is_file())
}

/// Builds the C greeter of the core's tests into a shared library in
/// `dir`.
pub fn c_greeter(cc: &Path, dir: &Path) -> PathBuf {
    let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("../guatiao");
    let out = dir.join(format!(
        "{}c_greeter{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    let mut cmd = super::c_compiler::command(cc);
    cmd.args(["-shared", "-std=c11", "-Wall", "-Wextra", "-Werror"]);
    if !cfg!(windows) {
        cmd.arg("-fPIC");
    }
    cmd.arg("-I")
        .arg(core.join("include"))
        .arg("-I")
        .arg(core.join("../../examples/greeter_kind/include"))
        .arg("-o")
        .arg(&out)
        .arg(core.join("tests/c_consumer/greeter_in_c.c"));
    let (ok, said) = run(&mut cmd);
    assert!(ok, "the C greeter did not compile:\n{said}");
    out
}
