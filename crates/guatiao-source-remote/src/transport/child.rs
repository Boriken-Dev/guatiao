// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A server in a child process, over its stdin and stdout: what keeps a
//! crashing or conflicting provider out of the host's process. The child's
//! stderr is the host's.

#![forbid(unsafe_code)]

use core::time::Duration;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::Instant;

use super::stream::{StreamReader, StreamWriter};
use super::{Connect, Connection, FrameRead, FrameWrite};

/// How long a child has to leave on its own once its stdin closes.
const GRACE: Duration = Duration::from_millis(500);

/// A command that serves a link on its stdio (`guatiao-remote-server
/// --stdio ...`), started again by every [`connect`](Connect::connect).
#[derive(Debug, Clone)]
pub struct ChildCommand {
    program: PathBuf,
    args: Vec<OsString>,
    envs: Vec<(OsString, OsString)>,
}

impl ChildCommand {
    /// Runs `program`.
    pub fn new(program: impl Into<PathBuf>) -> ChildCommand {
        ChildCommand {
            program: program.into(),
            args: Vec::new(),
            envs: Vec::new(),
        }
    }

    /// Adds an argument.
    pub fn arg(mut self, arg: impl Into<OsString>) -> ChildCommand {
        self.args.push(arg.into());
        self
    }

    /// Adds arguments.
    pub fn args<I: IntoIterator<Item = S>, S: Into<OsString>>(mut self, args: I) -> ChildCommand {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Sets a variable in the child's environment.
    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> ChildCommand {
        self.envs.push((key.into(), value.into()));
        self
    }
}

/// The child's stdin, and the child, ended together.
struct ChildWriter {
    stdin: Option<StreamWriter<ChildStdin>>,
    child: Child,
}

impl FrameWrite for ChildWriter {
    fn write_frame(&mut self, frame: &[u8]) -> io::Result<()> {
        self.stdin
            .as_mut()
            .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))?
            .write_frame(frame)
    }

    fn write_frame_parts(&mut self, head: &[u8], tail: &[u8]) -> io::Result<()> {
        self.stdin
            .as_mut()
            .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))?
            .write_frame_parts(head, tail)
    }

    fn shutdown(&mut self) {
        // Closing stdin asks the child to leave; one that does not is ended.
        self.stdin = None;
        let deadline = Instant::now() + GRACE;
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for ChildWriter {
    fn drop(&mut self) {
        if self.stdin.is_some() || matches!(self.child.try_wait(), Ok(None)) {
            self.shutdown();
        }
    }
}

impl Connect for ChildCommand {
    fn connect(&self) -> io::Result<Connection> {
        let mut child = Command::new(&self.program)
            .args(&self.args)
            .envs(self.envs.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdin = child.stdin.take().expect("piped");
        let stdout = child.stdout.take().expect("piped");
        let reader: Box<dyn FrameRead> = Box::new(StreamReader::new(stdout));
        Ok(Connection {
            reader,
            writer: Box::new(ChildWriter {
                stdin: Some(StreamWriter::new(stdin)),
                child,
            }),
            peer: format!("child {}", self.program.display()),
            closer: None,
        })
    }

    fn describe(&self) -> String {
        format!("child {}", self.program.display())
    }
}

/// This process's own stdin and stdout, for a server a host started as
/// its child. Nothing else may write to stdout.
pub fn stdio() -> Connection {
    Connection {
        reader: Box::new(StreamReader::new(io::stdin())),
        writer: Box::new(StreamWriter::new(io::stdout())),
        peer: "stdio".to_string(),
        closer: None,
    }
}
