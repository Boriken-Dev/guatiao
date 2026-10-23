// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `guatiao_source`: this file as a source of Lua libraries.

use guatiao::Map;
use guatiao::library::{Host, ProviderError};
use guatiao_bridge::BridgeSource;

use crate::engine::LuaEngine;

/// Starts the Lua state on a thread of its own for the source's life.
fn start(_host: Host, config: &Map) -> Result<BridgeSource<LuaEngine>, ProviderError> {
    Ok(BridgeSource::pinned(config, "guatiao-lua")?)
}

guatiao::source!(serves = "lua", make = start);
