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
}

#[cfg(windows)]
mod imp {
    use alloc::sync::Arc;
    use std::io::{self, Read, Write};

    use interprocess::local_socket::prelude::*;
    use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Name, Stream};

    use super::{Connection, StreamReader, StreamWriter};

    pub(super) type Listener = interprocess::local_socket::Listener;

    fn name_of(name: &str) -> io::Result<Name<'static>> {
        name.to_string().to_ns_name::<GenericNamespaced>()
    }

    /// One pipe for the reader and the writer. Nothing here can end a
    /// read blocked on it: ending the link sends GOODBYE, and the peer
    /// closing its end is what ends that read.
    struct Shared(Arc<Stream>);

    impl Read for Shared {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            (&*self.0).read(buf)
        }
    }

    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            (&*self.0).write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            (&*self.0).flush()
        }
    }

    fn connection(stream: Stream, peer: String) -> Connection {
        let stream = Arc::new(stream);
        Connection {
            reader: Box::new(StreamReader::new(Shared(Arc::clone(&stream)))),
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
