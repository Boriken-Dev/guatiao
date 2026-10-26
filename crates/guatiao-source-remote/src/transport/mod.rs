// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a link runs over: something that reads and writes whole frames.
//!
//! A byte stream (TCP, a Unix socket, a pipe, a child's stdio, a shared
//! memory ring) carries each frame behind its `u32` length, as the link
//! builds it; a message transport (a WebSocket, a QUIC datagram) carries
//! the body alone, one frame per message.

#![deny(unsafe_code)]

pub mod child;
#[cfg(feature = "local")]
pub mod local;
pub mod memory;
#[cfg(feature = "rdma")]
pub mod rdma;
#[cfg(feature = "shm")]
#[allow(unsafe_code)]
pub mod shm;
pub mod stream;
#[cfg(feature = "tcp")]
pub mod tcp;

use core::time::Duration;
use std::io;

/// Reads whole frames.
pub trait FrameRead: Send {
    /// Reads the next frame's body into `buf` (replacing what it held).
    /// `Ok(false)` when the peer closed the link between frames; a body
    /// over `max` bytes is an error, read no further.
    fn read_frame(&mut self, buf: &mut Vec<u8>, max: usize) -> io::Result<bool>;

    /// Bounds how long a read waits, or lifts the bound. A transport that
    /// cannot bound a read ignores it.
    fn set_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        let _ = timeout;
        Ok(())
    }
}

/// Writes whole frames.
pub trait FrameWrite: Send {
    /// Writes one frame, its 4-byte length first, as the link built it.
    fn write_frame(&mut self, frame: &[u8]) -> io::Result<()>;

    /// Writes one frame whose last bytes are `tail`, a large buffer kept
    /// out of `head`: `head` holds the length (counting `tail`) and the
    /// body up to it. A stream writes both without joining them.
    fn write_frame_parts(&mut self, head: &[u8], tail: &[u8]) -> io::Result<()> {
        let mut joined = Vec::with_capacity(head.len() + tail.len());
        joined.extend_from_slice(head);
        joined.extend_from_slice(tail);
        self.write_frame(&joined)
    }

    /// Bounds how long a write waits while the peer takes nothing in, or
    /// lifts the bound: past it the write fails and the link ends. A
    /// transport that cannot bound a write ignores it.
    fn set_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        let _ = timeout;
        Ok(())
    }

    /// Ends the link: the peer's reads end, and so do this side's.
    fn shutdown(&mut self) {}
}

/// Both halves of a connected transport, and what it reaches.
pub struct Connection {
    /// Where frames come from.
    pub reader: Box<dyn FrameRead>,
    /// Where frames go.
    pub writer: Box<dyn FrameWrite>,
    /// The peer, in words: an address, a path, a command.
    pub peer: String,
    /// Ends the connection both ways without the writer, which a blocked
    /// write may hold: a socket's `shutdown`.
    pub closer: Option<Box<dyn Fn() + Send + Sync>>,
}

impl core::fmt::Debug for Connection {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Connection")
            .field("peer", &self.peer)
            .finish()
    }
}

/// Makes connections: what a client reconnects through.
pub trait Connect: Send + Sync {
    /// A new connection to the peer.
    fn connect(&self) -> io::Result<Connection>;

    /// The peer, in words.
    fn describe(&self) -> String;
}
