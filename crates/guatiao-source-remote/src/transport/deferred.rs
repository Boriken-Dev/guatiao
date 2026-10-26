// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A connection a listener accepted and set up later, on its first read:
//! on the thread serving it, given the read bound the link set for its
//! handshake, so a client that never finishes setting up holds that
//! thread alone and never the listener.

#![forbid(unsafe_code)]

use alloc::sync::Arc;
use core::time::Duration;
use std::io;
use std::sync::{Mutex, MutexGuard, PoisonError};

use super::{Connection, FrameRead, FrameWrite};

/// Sets the connection up, bounded by the read bound the link set (`None`:
/// the transport's own).
pub(crate) type Setup = Box<dyn FnOnce(Option<Duration>) -> io::Result<Connection> + Send>;

/// Ends what a set-up waits on, from another thread.
pub(crate) type Abort = Box<dyn Fn() + Send + Sync>;

enum State {
    /// Not set up yet.
    Pending { abort: Option<Abort> },
    /// Set up: its writer until the link's writer takes it, and its closer.
    Ready {
        writer: Option<Box<dyn FrameWrite>>,
        closer: Option<Arc<dyn Fn() + Send + Sync>>,
    },
    /// Closed before it was set up, or its set-up failed.
    Ended,
}

struct Shared {
    state: Mutex<State>,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn close(&self) {
        let mut state = self.state();
        match &mut *state {
            State::Pending { abort } => {
                let abort = abort.take();
                *state = State::Ended;
                drop(state);
                if let Some(abort) = abort {
                    abort();
                }
            }
            State::Ready { closer, .. } => {
                let closer = closer.clone();
                drop(state);
                if let Some(close) = closer {
                    close();
                }
            }
            State::Ended => {}
        }
    }
}

fn aborted() -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionAborted,
        "the connection ended before it was set up",
    )
}

/// A connection to `peer` that `setup` sets up on its first read; until
/// then a write is refused `NotConnected` and closing it runs `abort`.
pub(crate) fn deferred(peer: String, setup: Setup, abort: Option<Abort>) -> Connection {
    let shared = Arc::new(Shared {
        state: Mutex::new(State::Pending { abort }),
    });
    let closing = Arc::clone(&shared);
    Connection {
        reader: Box::new(Reader {
            shared: Arc::clone(&shared),
            setup: Some(setup),
            inner: None,
            timeout: None,
        }),
        writer: Box::new(Writer {
            shared,
            inner: None,
            timeout: None,
        }),
        peer,
        closer: Some(Box::new(move || closing.close())),
    }
}

struct Reader {
    shared: Arc<Shared>,
    setup: Option<Setup>,
    inner: Option<Box<dyn FrameRead>>,
    timeout: Option<Duration>,
}

impl Reader {
    fn set_up(&mut self) -> io::Result<&mut dyn FrameRead> {
        if self.inner.is_none() {
            let Some(setup) = self.setup.take() else {
                return Err(aborted());
            };
            if matches!(*self.shared.state(), State::Ended) {
                return Err(aborted());
            }
            let conn = match setup(self.timeout) {
                Ok(conn) => conn,
                Err(e) => {
                    *self.shared.state() = State::Ended;
                    return Err(e);
                }
            };
            let Connection {
                mut reader,
                mut writer,
                closer,
                ..
            } = conn;
            let bounded = reader.set_timeout(self.timeout);
            let mut state = self.shared.state();
            if bounded.is_err() || matches!(*state, State::Ended) {
                // Closed while it was being set up, or unusable.
                *state = State::Ended;
                drop(state);
                writer.shutdown();
                if let Some(close) = closer {
                    close();
                }
                return Err(bounded.err().unwrap_or_else(aborted));
            }
            *state = State::Ready {
                writer: Some(writer),
                closer: closer.map(Arc::from),
            };
            self.inner = Some(reader);
        }
        Ok(self.inner.as_deref_mut().expect("set up above"))
    }
}

impl FrameRead for Reader {
    fn read_frame(&mut self, buf: &mut Vec<u8>, max: usize) -> io::Result<bool> {
        self.set_up()?.read_frame(buf, max)
    }

    fn set_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.timeout = timeout;
        match &mut self.inner {
            Some(inner) => inner.set_timeout(timeout),
            None => Ok(()),
        }
    }
}

struct Writer {
    shared: Arc<Shared>,
    inner: Option<Box<dyn FrameWrite>>,
    timeout: Option<Duration>,
}

impl Writer {
    /// The set-up connection's writer, once there is one.
    fn inner(&mut self) -> io::Result<&mut dyn FrameWrite> {
        if self.inner.is_none() {
            let taken = match &mut *self.shared.state() {
                State::Ready { writer, .. } => writer.take(),
                _ => None,
            };
            let Some(mut writer) = taken else {
                return Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "the connection is not set up yet",
                ));
            };
            writer.set_timeout(self.timeout)?;
            self.inner = Some(writer);
        }
        Ok(self.inner.as_deref_mut().expect("taken above"))
    }
}

impl FrameWrite for Writer {
    fn write_frame(&mut self, frame: &[u8]) -> io::Result<()> {
        self.inner()?.write_frame(frame)
    }

    fn write_frame_parts(&mut self, head: &[u8], tail: &[u8]) -> io::Result<()> {
        self.inner()?.write_frame_parts(head, tail)
    }

    fn write_frames(&mut self, frames: &[&[u8]]) -> io::Result<()> {
        self.inner()?.write_frames(frames)
    }

    fn set_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.timeout = timeout;
        match &mut self.inner {
            Some(inner) => inner.set_timeout(timeout),
            None => Ok(()),
        }
    }

    fn shutdown(&mut self) {
        match self.inner() {
            Ok(inner) => inner.shutdown(),
            Err(_) => self.shared.close(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::memory;
    use std::sync::mpsc;

    /// A frame of `body`, behind its length.
    fn frame(body: &[u8]) -> Vec<u8> {
        let mut f = (body.len() as u32).to_le_bytes().to_vec();
        f.extend_from_slice(body);
        f
    }

    #[test]
    fn nothing_is_set_up_before_the_first_read_which_gets_the_links_bound() {
        let (ours, mut theirs) = memory::pair();
        let (ran_tx, ran_rx) = mpsc::channel();
        let conn = deferred(
            "deferred".to_string(),
            Box::new(move |bound| {
                ran_tx.send(bound).unwrap();
                Ok(ours)
            }),
            None,
        );
        let Connection {
            mut reader,
            mut writer,
            ..
        } = conn;
        let refused = writer.write_frame(&frame(b"early")).unwrap_err();
        assert_eq!(refused.kind(), io::ErrorKind::NotConnected);
        assert!(ran_rx.try_recv().is_err(), "not set up by a write");

        reader.set_timeout(Some(Duration::from_secs(3))).unwrap();
        theirs.writer.write_frame(&frame(b"hello")).unwrap();
        let mut buf = Vec::new();
        assert!(reader.read_frame(&mut buf, 64).unwrap());
        assert_eq!(buf, b"hello");
        assert_eq!(ran_rx.try_recv(), Ok(Some(Duration::from_secs(3))));

        writer.write_frame(&frame(b"welcome")).unwrap();
        assert!(theirs.reader.read_frame(&mut buf, 64).unwrap());
        assert_eq!(buf, b"welcome");
    }

    #[test]
    fn closing_before_the_set_up_aborts_it_and_nothing_is_set_up() {
        let (aborted_tx, aborted_rx) = mpsc::channel();
        let conn = deferred(
            "deferred".to_string(),
            Box::new(|_| panic!("never set up")),
            Some(Box::new(move || aborted_tx.send(()).unwrap())),
        );
        (conn.closer.as_ref().unwrap())();
        assert_eq!(aborted_rx.try_recv(), Ok(()));
        let mut reader = conn.reader;
        let e = reader.read_frame(&mut Vec::new(), 64).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::ConnectionAborted);
    }

    #[test]
    fn a_set_up_that_finishes_after_a_close_ends_its_connection() {
        let (ours, mut theirs) = memory::pair();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let (started_tx, started_rx) = mpsc::channel::<()>();
        let conn = deferred(
            "deferred".to_string(),
            Box::new(move |_| {
                started_tx.send(()).unwrap();
                // What the abort would end: here, a wait it releases.
                release_rx.recv().unwrap();
                Ok(ours)
            }),
            Some(Box::new(move || release_tx.send(()).unwrap())),
        );
        let Connection {
            mut reader, closer, ..
        } = conn;
        let reading = std::thread::spawn(move || reader.read_frame(&mut Vec::new(), 64));
        started_rx.recv().unwrap();
        (closer.unwrap())();
        let e = reading.join().unwrap().unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::ConnectionAborted);
        // The connection it set up was ended: the peer reads its end.
        theirs
            .reader
            .set_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        assert!(!theirs.reader.read_frame(&mut Vec::new(), 64).unwrap());
    }

    #[test]
    fn a_failed_set_up_is_the_reads_error_and_the_writer_stays_unconnected() {
        let conn = deferred(
            "deferred".to_string(),
            Box::new(|_| Err(io::Error::new(io::ErrorKind::InvalidData, "a bad path"))),
            None,
        );
        let Connection {
            mut reader,
            mut writer,
            ..
        } = conn;
        let e = reader.read_frame(&mut Vec::new(), 64).unwrap_err();
        assert_eq!(e.to_string(), "a bad path");
        let e = reader.read_frame(&mut Vec::new(), 64).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::ConnectionAborted);
        let e = writer.write_frame(&frame(b"x")).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::NotConnected);
    }
}
