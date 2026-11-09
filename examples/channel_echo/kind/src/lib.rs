// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A provider kind that starts a channel: the shape a kind of one's own
//! takes to hand a [`Channel`] back for a [`ChannelSink`] handed in. The
//! echo libraries in every language serve it, and the hosts drive the
//! channel rules through it.

pub use guatiao_channel::{Channel, ChannelSink};

use guatiao::Map;
use guatiao::library::{Object, ProviderError};

/// Starts a channel that sends back every message it is given.
#[guatiao::kind]
pub trait ChannelEcho: Send + Sync {
    /// A channel whose every message comes back through `sink`, in order,
    /// from a thread of the provider's own where it has one.
    ///
    /// `config` may say `close_after` (a count: after echoing that many
    /// messages the provider closes the stream itself) and `close_status`
    /// (the status it closes with then; `0` when left out) and
    /// `close_reason` (the text it closes with then; empty when left out).
    /// When the host closes, the echo passes the host's status and reason
    /// on to the sink.
    fn start(
        &self,
        config: &Map,
        sink: Object<dyn ChannelSink>,
    ) -> Result<Object<dyn Channel>, ProviderError>;
}
