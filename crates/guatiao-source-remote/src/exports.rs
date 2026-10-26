// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The source file any host adds: `guatiao_source` over [`RemoteSource`],
//! configured as [`config`](crate::config) reads.

use guatiao::Map;
use guatiao::library::{Host, ProviderError};

use crate::client::RemoteSource;

fn start(_host: Host, config: &Map) -> Result<RemoteSource, ProviderError> {
    RemoteSource::from_config(config)
}

guatiao::source!(serves = "remote", make = start);
