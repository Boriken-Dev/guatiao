// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Two ends of a link in one process, for tests: frames move through
//! bounded queues, so a slow reader slows its writer.

use core::time::Duration;
use std::io;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};

use super::{Connection, FrameRead, FrameWrite};

/// How many frames each direction holds before its writer waits.
const DEPTH: usize = 64;

struct Reader {
    from: Receiver<Vec<u8>>,
    timeout: Option<Duration>,
}

struct Writer {
    to: Option<SyncSender<Vec<u8>>>,
}

impl FrameRead for Reader {
    fn read_frame(&mut self, buf: &mut Vec<u8>, max: usize) -> io::Result<bool> {
        let frame = match self.timeout {
            None => match self.from.recv() {
                Ok(f) => f,
                Err(_) => return Ok(false),
            },
            Some(t) => match self.from.recv_timeout(t) {
                Ok(f) => f,
                Err(RecvTimeoutError::Timeout) => return Err(io::ErrorKind::TimedOut.into()),
                Err(RecvTimeoutError::Disconnected) => return Ok(false),
            },
        };
        if frame.len() - 4 > max {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "a frame over the limit",
            ));
        }
        buf.clear();
        buf.extend_from_slice(&frame[4..]);
        Ok(true)
    }

    fn set_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.timeout = timeout;
        Ok(())
    }
}

impl FrameWrite for Writer {
    fn write_frame(&mut self, frame: &[u8]) -> io::Result<()> {
        self.to
            .as_ref()
            .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))?
            .send(frame.to_vec())
            .map_err(|_| io::ErrorKind::BrokenPipe.into())
    }

    fn shutdown(&mut self) {
        self.to = None;
    }
}

/// Two connected ends: what one writes, the other reads.
pub fn pair() -> (Connection, Connection) {
    let (a_to_b, b_from_a) = sync_channel(DEPTH);
    let (b_to_a, a_from_b) = sync_channel(DEPTH);
    (
        Connection {
            reader: Box::new(Reader {
                from: a_from_b,
                timeout: None,
            }),
            writer: Box::new(Writer { to: Some(a_to_b) }),
            peer: "memory:b".to_string(),
            closer: None,
        },
        Connection {
            reader: Box::new(Reader {
                from: b_from_a,
                timeout: None,
            }),
            writer: Box::new(Writer { to: Some(b_to_a) }),
            peer: "memory:a".to_string(),
            closer: None,
        },
    )
}
