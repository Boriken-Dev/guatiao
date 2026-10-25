// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `guatiao_source`: this file as a source of JavaScript libraries.

use alloc::sync::Arc;

use guatiao::Map;
use guatiao::library::{Host, ProviderError};
use guatiao_bridge::BridgeSource;
use guatiao_libffi::Libffi;

use crate::engine::JsEngine;

/// Starts the QuickJS runtime on a thread of its own for the source's life.
fn start(_host: Host, config: &Map) -> Result<BridgeSource<JsEngine>, ProviderError> {
    Ok(BridgeSource::pinned(config, "guatiao-source-js")?.with_mechanism(Arc::new(Libffi)))
}

guatiao::source!(serves = "js", make = start);
