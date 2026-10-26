// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The same machine, by name: a Unix socket at a path (created `0600`) on
//! Unix, a named pipe on Windows (its default access: the creating user,
//! administrators and the system). The `local` trust level: the operating
//! system's permissions are the gate.

#![forbid(unsafe_code)]

use std::io;

use super::stream::{StreamReader, StreamWriter};
use super::{Connect, Connection};

/// Connects to a local endpoint: a socket path on Unix, a pipe name on
/// Windows.
#[derive(Debug, Clone)]
pub struct LocalConnect {
    name: String,
}

impl LocalConnect {
    /// To the endpoint `name`.
    pub fn new(name: &str) -> LocalConnect {
        LocalConnect {
            name: name.to_string(),
        }
    }
}

impl Connect for LocalConnect {
    fn connect(&self) -> io::Result<Connection> {
        imp::connect(&self.name)
    }

    fn describe(&self) -> String {
        format!("local {}", self.name)
    }
}

/// Listens on a local endpoint.
pub struct LocalListen {
    inner: imp::Listener,
    name: String,
}

impl core::fmt::Debug for LocalListen {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LocalListen")
            .field("name", &self.name)
            .finish()
    }
}

impl LocalListen {
    /// Listens on `name`. On Unix a socket left at that path by a
    /// listener that is gone is replaced; anything else there is refused.
    pub fn bind(name: &str) -> io::Result<LocalListen> {
        Ok(LocalListen {
            inner: imp::bind(name)?,
            name: name.to_string(),
        })
    }

    /// The next connection.
    pub fn accept(&self) -> io::Result<Connection> {
        imp::accept(&self.inner, &self.name)
    }
}

#[cfg(unix)]
mod imp {
    use alloc::sync::Arc;
    use std::io::{self, Read, Write};
    use std::net::Shutdown;
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};

    use super::{Connection, StreamReader, StreamWriter};

    pub(super) type Listener = UnixListener;

    /// One socket for the reader, the writer and the closer, as for TCP.
    struct Shared(Arc<UnixStream>);

    impl Read for Shared {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            (&*self.0).read(buf)
        }
    }

    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            (&*self.0).write(buf)
        }

        fn write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize> {
            (&*self.0).write_vectored(bufs)
        }

        fn flush(&mut self) -> io::Result<()> {
            (&*self.0).flush()
        }
    }

    fn connection(stream: UnixStream, peer: String) -> Connection {
        let stream = Arc::new(stream);
        let (shutting, closing) = (Arc::clone(&stream), Arc::clone(&stream));
        Connection {
            reader: Box::new(StreamReader::with_timeout(
                Shared(Arc::clone(&stream)),
                |s: &Shared, t| s.0.set_read_timeout(t),
            )),
            writer: Box::new(
                StreamWriter::with_shutdown(Shared(stream), move || {
                    let _ = shutting.shutdown(Shutdown::Both);
                })
                .bounded_by(|s: &mut Shared, t| s.0.set_write_timeout(t)),
            ),
            peer,
            closer: Some(Box::new(move || {
                let _ = closing.shutdown(Shutdown::Both);
            })),
        }
    }

    pub(super) fn connect(name: &str) -> io::Result<Connection> {
        Ok(connection(
            UnixStream::connect(name)?,
            format!("local {name}"),
        ))
    }

    pub(super) fn bind(name: &str) -> io::Result<Listener> {
        if let Ok(meta) = std::fs::symlink_metadata(name) {
            if !meta.file_type().is_socket() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{name} exists and is not a socket"),
                ));
            }
            if UnixStream::connect(name).is_ok() {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!("a listener already answers at {name}"),
                ));
            }
            std::fs::remove_file(name)?;
        }
        let listener = UnixListener::bind(name)?;
        std::fs::set_permissions(name, std::fs::Permissions::from_mode(0o600))?;
        Ok(listener)
    }

    pub(super) fn accept(listener: &Listener, name: &str) -> io::Result<Connection> {
        let (stream, _) = listener.accept()?;
        Ok(connection(stream, format!("local {name}")))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_vectored_write_reaches_the_socket_whole() {
            let (ours, mut theirs) = UnixStream::pair().unwrap();
            let mut shared = Shared(Arc::new(ours));
            let parts = [io::IoSlice::new(b"head"), io::IoSlice::new(b"tail")];
            // Not the default, which writes only the first part.
            assert_eq!(shared.write_vectored(&parts).unwrap(), 8);
            let mut got = [0u8; 8];
            theirs.read_exact(&mut got).unwrap();
            assert_eq!(&got, b"headtail");
        }
    }
}

#[cfg(windows)]
mod imp {
    use alloc::sync::Arc;
    use core::time::Duration;
    use std::io::{self, Read, Write};
    use std::os::windows::io::AsHandle;
    use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
    use std::time::Instant;

    use interprocess::local_socket::prelude::*;
    use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Name, Stream};

    use super::{Connection, StreamReader, StreamWriter};
    use crate::transport::cancel::cancel_io;

    pub(super) type Listener = interprocess::local_socket::Listener;

    /// How soon a read that outlived its bound is cancelled again: a read
    /// started just after a cancel is not ended by it.
    const AGAIN: Duration = Duration::from_millis(1);

    fn name_of(name: &str) -> io::Result<Name<'static>> {
        name.to_string().to_ns_name::<GenericNamespaced>()
    }

    /// A pipe's reads and their bound. A pipe takes no read timeout, so
    /// while a bound is set (a link's handshake) a thread watches each read
    /// and cancels one that outlives it; with none, a read waits for data
    /// or the peer's end, and nothing here ends it: ending the link sends
    /// GOODBYE, and the peer closing its end is what ends that read.
    struct Watch {
        pipe: Arc<Stream>,
        state: Mutex<Watched>,
        changed: Condvar,
    }

    #[derive(Default)]
    struct Watched {
        bound: Option<Duration>,
        /// The deadline of the bounded read in progress.
        reading: Option<Instant>,
        /// Whether that read was cancelled at its deadline.
        expired: bool,
        /// Whether a thread watches.
        watching: bool,
    }

    impl Watch {
        fn state(&self) -> MutexGuard<'_, Watched> {
            self.state.lock().unwrap_or_else(PoisonError::into_inner)
        }

        fn cancel(&self) {
            match &*self.pipe {
                Stream::NamedPipe(p) => cancel_io(p.inner().as_handle()),
            }
        }

        /// Cancels each bounded read past its deadline, until no bound is
        /// set and no read is in progress.
        fn run(&self) {
            let mut state = self.state();
            loop {
                let Some(deadline) = state.reading else {
                    if state.bound.is_none() {
                        state.watching = false;
                        return;
                    }
                    state = self
                        .changed
                        .wait(state)
                        .unwrap_or_else(PoisonError::into_inner);
                    continue;
                };
                let now = Instant::now();
                let wait = if now >= deadline {
                    state.expired = true;
                    // Under the lock: the read cancelled is the one still
                    // registered, never I/O begun after it returned.
                    self.cancel();
                    AGAIN
                } else {
                    deadline - now
                };
                state = self
                    .changed
                    .wait_timeout(state, wait)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            }
        }
    }

    /// The reading half.
    struct Reading(Arc<Watch>);

    impl Read for Reading {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let bounded = {
                let mut state = self.0.state();
                // A bound past what a clock holds is none.
                match state.bound.and_then(|b| Instant::now().checked_add(b)) {
                    None => false,
                    Some(deadline) => {
                        state.reading = Some(deadline);
                        state.expired = false;
                        if !state.watching {
                            let watch = Arc::clone(&self.0);
                            std::thread::Builder::new()
                                .name("guatiao-remote-pipe-watch".to_string())
                                .spawn(move || watch.run())
                                .inspect_err(|_| state.reading = None)?;
                            state.watching = true;
                        }
                        self.0.changed.notify_all();
                        true
                    }
                }
            };
            let read = (&*self.0.pipe).read(buf);
            if !bounded {
                return read;
            }
            let expired = {
                let mut state = self.0.state();
                state.reading = None;
                state.expired
            };
            match read {
                Err(_) if expired => Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "nothing arrived on the pipe within the read's bound",
                )),
                read => read,
            }
        }
    }

    impl Drop for Reading {
        fn drop(&mut self) {
            let mut state = self.0.state();
            state.bound = None;
            state.reading = None;
            self.0.changed.notify_all();
        }
    }

    /// The writing half: the pipe itself.
    struct Shared(Arc<Stream>);

    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            (&*self.0).write(buf)
        }

        fn write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize> {
            (&*self.0).write_vectored(bufs)
        }

        fn flush(&mut self) -> io::Result<()> {
            (&*self.0).flush()
        }
    }

    fn connection(stream: Stream, peer: String) -> Connection {
        let stream = Arc::new(stream);
        let watch = Arc::new(Watch {
            pipe: Arc::clone(&stream),
            state: Mutex::new(Watched::default()),
            changed: Condvar::new(),
        });
        Connection {
            reader: Box::new(StreamReader::with_timeout(
                Reading(watch),
                |r: &Reading, bound| {
                    r.0.state().bound = bound;
                    r.0.changed.notify_all();
                    Ok(())
                },
            )),
            writer: Box::new(StreamWriter::new(Shared(stream))),
            peer,
            closer: None,
        }
    }

    pub(super) fn connect(name: &str) -> io::Result<Connection> {
        Ok(connection(
            Stream::connect(name_of(name)?)?,
            format!("local {name}"),
        ))
    }

    pub(super) fn bind(name: &str) -> io::Result<Listener> {
        ListenerOptions::new().name(name_of(name)?).create_sync()
    }

    pub(super) fn accept(listener: &Listener, name: &str) -> io::Result<Connection> {
        Ok(connection(listener.accept()?, format!("local {name}")))
    }
}
