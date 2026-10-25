// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Frames over any byte stream: a `u32` length, then the body.

#![forbid(unsafe_code)]

use std::io::{self, BufReader, Read, Write};

use super::{FrameRead, FrameWrite};

/// The reader's buffer: one read takes many small frames at once.
const READ_BUFFER: usize = 64 << 10;

/// Bounds a stream's reads: called with the stream the reader reads, since
/// on some platforms (Windows) a bound set through another handle on the
/// same socket does not apply to this one.
type SetTimeout<R> = Box<dyn FnMut(&R, Option<core::time::Duration>) -> io::Result<()> + Send>;

/// Reads frames from a byte stream.
pub struct StreamReader<R: Read> {
    inner: BufReader<R>,
    timeout: SetTimeout<R>,
}

impl<R: Read + Send> StreamReader<R> {
    /// Reads from `inner`, which bounds no read.
    pub fn new(inner: R) -> StreamReader<R> {
        StreamReader {
            inner: BufReader::with_capacity(READ_BUFFER, inner),
            timeout: Box::new(|_, _| Ok(())),
        }
    }

    /// Reads from `inner`; `timeout` bounds the reads of the stream it is
    /// handed, which is `inner`, when asked.
    pub fn with_timeout(
        inner: R,
        timeout: impl FnMut(&R, Option<core::time::Duration>) -> io::Result<()> + Send + 'static,
    ) -> StreamReader<R> {
        StreamReader {
            inner: BufReader::with_capacity(READ_BUFFER, inner),
            timeout: Box::new(timeout),
        }
    }
}

impl<R: Read + Send> FrameRead for StreamReader<R> {
    fn read_frame(&mut self, buf: &mut Vec<u8>, max: usize) -> io::Result<bool> {
        let mut len = [0u8; 4];
        // A clean end is one between frames; inside one it is an error.
        let mut got = 0;
        while got < 4 {
            match self.inner.read(&mut len[got..]) {
                Ok(0) if got == 0 => return Ok(false),
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => got += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        let n = u32::from_le_bytes(len) as usize;
        if n > max {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("a frame of {n} bytes is over the {max} allowed"),
            ));
        }
        buf.clear();
        buf.resize(n, 0);
        self.inner.read_exact(buf)?;
        Ok(true)
    }

    fn set_timeout(&mut self, timeout: Option<core::time::Duration>) -> io::Result<()> {
        (self.timeout)(self.inner.get_ref(), timeout)
    }
}

/// Writes frames to a byte stream, each at once.
pub struct StreamWriter<W: Write> {
    inner: W,
    shutdown: Option<Box<dyn FnMut() + Send>>,
}

impl<W: Write + Send> StreamWriter<W> {
    /// Writes to `inner`.
    pub fn new(inner: W) -> StreamWriter<W> {
        StreamWriter {
            inner,
            shutdown: None,
        }
    }

    /// Writes to `inner`; `shutdown` ends the stream both ways.
    pub fn with_shutdown(inner: W, shutdown: impl FnMut() + Send + 'static) -> StreamWriter<W> {
        StreamWriter {
            inner,
            shutdown: Some(Box::new(shutdown)),
        }
    }
}

impl<W: Write + Send> FrameWrite for StreamWriter<W> {
    fn write_frame(&mut self, frame: &[u8]) -> io::Result<()> {
        self.inner.write_all(frame)?;
        self.inner.flush()
    }

    fn write_frame_parts(&mut self, head: &[u8], tail: &[u8]) -> io::Result<()> {
        self.inner.write_all(head)?;
        self.inner.write_all(tail)?;
        self.inner.flush()
    }

    fn shutdown(&mut self) {
        let _ = self.inner.flush();
        if let Some(f) = &mut self.shutdown {
            f();
        }
    }
}
