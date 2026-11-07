// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A two-way stream of bytes between a host and a provider, as two
//! [guatiao] object kinds.
//!
//! The host hands a [`ChannelSink`] in and gets a [`Channel`] back, from
//! a provider kind of its own:
//!
//! ```ignore
//! #[guatiao::kind]
//! pub trait Harness: Send + Sync {
//!     fn start(&self, config: &Map, sink: Object<dyn ChannelSink>)
//!         -> Result<Object<dyn Channel>, ProviderError>;
//! }
//! ```
//!
//! The host writes with [`Channel::send`] and ends with
//! [`Channel::close`]; the provider writes back with
//! [`ChannelSink::receive`], from a thread of its own if it likes, and
//! ends with [`ChannelSink::closed`]. Messages are bytes, lent for the
//! call: JSON text lines, a process's output, frames of a protocol.
//!
//! # The rules
//!
//! - **Back-pressure blocks.** `send` returns once the provider has taken
//!   the message, and `receive` once the host has; a side that cannot
//!   take more yet makes the other wait. Neither answers
//!   `GUATIAO_ERR_BUSY` and neither times out: a caller wanting a bound
//!   keeps its own. A host never waits inside `receive` for its own
//!   `send` to return: a provider whose output depends on its input, an
//!   echo, would deadlock once its queue is full.
//! - **Order is kept per direction.** Messages arrive in the order their
//!   calls were made. The provider makes every call on the sink, `receive`
//!   and `closed` alike, from one thread at a time, and none while
//!   another is running.
//! - **A status is a `guatiao_status` value.** In `close(status)` and
//!   `closed(status)`, `GUATIAO_OK` (`0`) is the normal end and anything
//!   else an error status, which an error from `receive` passes on.
//! - **The host closes once.** After [`Channel::close`] (or dropping the
//!   channel, which is `close(GUATIAO_OK)`), `send` answers
//!   `GUATIAO_ERR_GONE`, a `send` that was waiting for room included, and
//!   none is taken once `close` has returned. The provider calls
//!   [`ChannelSink::closed`] with the host's status exactly once, unless
//!   it already had, and never calls the sink again. What it took before
//!   the close it may still deliver first. `close` and dropping the
//!   channel return without waiting for a `receive` still running.
//! - **The provider closes once.** [`ChannelSink::closed`] ends the
//!   stream from the provider's side; from then on the host's `send`
//!   answers `GUATIAO_ERR_GONE`.
//! - **A sink that has gone says so.** A host that will take nothing more
//!   answers `GUATIAO_ERR_GONE` from `receive`. Any error from `receive`
//!   ends the stream: the provider calls `closed` with that status and
//!   stops, and a message still queued is not delivered.
//! - **A provider's library stays mapped while its threads run.** It
//!   refuses to unload (`GUATIAO_ERR_BUSY`) while a thread of its own is
//!   still inside a call on the sink.
//! - **`close` and `closed` cannot fail.** A C provider's `close` and a C
//!   host's `closed` answer `GUATIAO_OK`; any other answer is a bug the
//!   caller's proxy panics on.
//!
//! The kinds are `channel` and `channel_sink` in every language;
//! `include/guatiao_channel.kinds.json` describes them as data and
//! `include/guatiao_channel.h` declares their tables for C.

#![forbid(unsafe_code)]

/// The guatiao this crate's kinds are declared against: a kind crate
/// naming `Object<dyn ChannelSink>` names the same `Kind` trait through
/// it.
pub use guatiao;

use guatiao::library::ProviderError;

/// What the host writes on, handed back by the provider that started it.
///
/// The provider implements it and the host holds the channel object,
/// which may be shared between threads: `send` may be called from several
/// at once, each thread's messages kept in its own order. Dropping the
/// channel is `close(GUATIAO_OK)`.
#[guatiao::kind(object)]
pub trait Channel: Send + Sync {
    /// One message to the provider, lent for the call. Returns once the
    /// provider has taken it, which may wait on the provider; answers
    /// `GUATIAO_ERR_GONE` once either side has closed, and a `send` that
    /// was waiting when the host closed answers it too.
    fn send(&self, bytes: &[u8]) -> Result<(), ProviderError>;

    /// Ends the stream from the host's side, with a `guatiao_status` the
    /// provider hands to the sink's `closed`: `GUATIAO_OK` (0) for a plain
    /// end, an error status otherwise. Called once, and returns without
    /// waiting for a `receive` still running; `send` answers
    /// `GUATIAO_ERR_GONE` after it. Cannot fail.
    fn close(&self, status: u32);
}

/// What the provider writes back on, handed in by the host.
///
/// The host implements it and hands it over with the call that starts
/// the channel; the provider owns it from then on, makes every call on it
/// from one thread at a time, and destroys it when done.
#[guatiao::kind(object)]
pub trait ChannelSink: Send {
    /// One message to the host, lent for the call. Returns once the host
    /// has taken it, and a host never waits in it for its own `send`.
    /// `GUATIAO_ERR_GONE` says the host takes nothing more; any error
    /// ends the stream, and the provider then calls `closed` with that
    /// status and delivers nothing still queued.
    fn receive(&mut self, bytes: &[u8]) -> Result<(), ProviderError>;

    /// The stream has ended, from either side: with the host's status from
    /// the channel's `close`, or the provider's own, a `guatiao_status`
    /// (`GUATIAO_OK` is the normal end). Called exactly once, and nothing
    /// but `destroy` follows it. Cannot fail.
    fn closed(&mut self, status: u32);
}
