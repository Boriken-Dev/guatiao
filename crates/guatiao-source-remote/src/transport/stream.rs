// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Frames over any byte stream: a `u32` length, then the body.

#![forbid(unsafe_code)]

use std::io::{self, BufReader, IoSlice, Read, Write};

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

/// Bounds a stream's writes: called with the stream the writer writes.
type SetWriteTimeout<W> =
    Box<dyn FnMut(&mut W, Option<core::time::Duration>) -> io::Result<()> + Send>;

/// Writes frames to a byte stream, each at once.
pub struct StreamWriter<W: Write> {
    inner: W,
    shutdown: Option<Box<dyn FnMut() + Send>>,
    timeout: Option<SetWriteTimeout<W>>,
}

impl<W: Write + Send> StreamWriter<W> {
    /// Writes to `inner`.
    pub fn new(inner: W) -> StreamWriter<W> {
        StreamWriter {
            inner,
            shutdown: None,
            timeout: None,
        }
    }

    /// Writes to `inner`; `shutdown` ends the stream both ways.
    pub fn with_shutdown(inner: W, shutdown: impl FnMut() + Send + 'static) -> StreamWriter<W> {
        StreamWriter {
            inner,
            shutdown: Some(Box::new(shutdown)),
            timeout: None,
        }
    }

    /// Bounds the writes of the stream it is handed, which is `inner`,
    /// through `timeout` when asked.
    pub fn bounded_by(
        mut self,
        timeout: impl FnMut(&mut W, Option<core::time::Duration>) -> io::Result<()> + Send + 'static,
    ) -> StreamWriter<W> {
        self.timeout = Some(Box::new(timeout));
        self
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

    fn write_frames(&mut self, frames: &[&[u8]]) -> io::Result<()> {
        if let [frame] = frames {
            return self.write_frame(frame);
        }
        let mut slices: Vec<IoSlice<'_>> = frames.iter().map(|f| IoSlice::new(f)).collect();
        let mut left = &mut slices[..];
        IoSlice::advance_slices(&mut left, 0);
        while !left.is_empty() {
            match self.inner.write_vectored(left) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => IoSlice::advance_slices(&mut left, n),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        self.inner.flush()
    }

    fn set_timeout(&mut self, timeout: Option<core::time::Duration>) -> io::Result<()> {
        match &mut self.timeout {
            Some(set) => set(&mut self.inner, timeout),
            None => Ok(()),
        }
    }

    fn shutdown(&mut self) {
        let _ = self.inner.flush();
        if let Some(f) = &mut self.shutdown {
            f();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Takes at most three bytes a write, across buffers.
    struct Trickle(Vec<u8>);

    impl Write for Trickle {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let n = buf.len().min(3);
            self.0.extend_from_slice(&buf[..n]);
            Ok(n)
        }

        fn write_vectored(&mut self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
            let mut n = 0;
            for b in bufs {
                let take = b.len().min(3 - n);
                self.0.extend_from_slice(&b[..take]);
                n += take;
                if n == 3 {
                    break;
                }
            }
            Ok(n)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn frames_written_together_arrive_whole_and_in_order() {
        let mut w = StreamWriter::new(Trickle(Vec::new()));
        let frames: [&[u8]; 4] = [b"\x01\0\0\0a", b"", b"\x02\0\0\0bc", b"\x03\0\0\0def"];
        w.write_frames(&frames).unwrap();
        assert_eq!(w.inner.0, frames.concat());
    }
}
