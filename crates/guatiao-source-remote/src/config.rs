// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A source's configuration as a host hands it over, the same in every
//! language:
//!
//! | key | holds |
//! | --- | --- |
//! | `connect` | `tcp://host:port`, `local://name` (a socket path on Unix, a pipe name on Windows), `shm://name` (shared memory beside that), `rdma://host:port` (RDMA, experimental, Linux only), or for a child `child` instead |
//! | `rdma_device` | the RDMA device `rdma://` goes through (default: the first) |
//! | `child` | a list: the program and its arguments, served on its stdio |
//! | `trust` | `local`, `trusted` or `secure`; `local` for a child |
//! | `key`, `key_file` | the trusted level's pre-shared key, as text or a file's bytes |
//! | `token` | what a secure client presents |
//! | `timeout_ms` | how long a call waits; `0` waits as long as the link lives |
//! | `max_frame`, `max_calls` | the link's bounds |

#![forbid(unsafe_code)]

use core::time::Duration;

use guatiao::library::ProviderError;
use guatiao::value::convert::{FromValue, TryAsRef};
use guatiao::{List, Map, Status};

use crate::link::LinkOptions;
use crate::protocol::Trust;
use crate::transport::Connect;
use crate::transport::child::ChildCommand;

fn refused(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, why.as_ref())
}

fn text<'a>(map: &'a Map, key: &str) -> Result<Option<&'a str>, ProviderError> {
    match map.get(key) {
        None => Ok(None),
        Some(v) => TryAsRef::<str>::try_as_ref(v)
            .map(Some)
            .ok_or_else(|| refused(format!("`{key}` is not text"))),
    }
}

fn number(map: &Map, key: &str) -> Result<Option<u64>, ProviderError> {
    match map.get(key) {
        None => Ok(None),
        Some(v) => u64::from_value(v)
            .map(Some)
            .map_err(|_| refused(format!("`{key}` is not a whole number"))),
    }
}

/// Reads a trust level's name.
pub fn trust_of(name: &str) -> Result<Trust, ProviderError> {
    Ok(match name {
        "local" => Trust::Local,
        "trusted" => Trust::Trusted,
        "secure" => Trust::Secure,
        other => {
            return Err(refused(format!(
                "`trust` is `{other}`, not `local`, `trusted` or `secure`"
            )));
        }
    })
}

/// The link options a configuration names, at `trust`.
pub fn options_of(map: &Map, trust: Trust) -> Result<LinkOptions, ProviderError> {
    let mut options = LinkOptions::new(trust);
    if let Some(key) = text(map, "key")? {
        options = options.key(key.as_bytes());
    }
    if let Some(path) = text(map, "key_file")? {
        let key = std::fs::read(path).map_err(|e| refused(format!("`key_file` {path}: {e}")))?;
        options = options.key(&key);
    }
    if let Some(token) = text(map, "token")? {
        options = options.token(token);
    }
    if let Some(ms) = number(map, "timeout_ms")? {
        options = options.timeout((ms > 0).then(|| Duration::from_millis(ms)));
    }
    if let Some(n) = number(map, "max_frame")? {
        options = options.max_frame(usize::try_from(n).unwrap_or(usize::MAX));
    }
    if let Some(n) = number(map, "max_calls")? {
        options = options.max_calls(usize::try_from(n).unwrap_or(usize::MAX));
    }
    Ok(options)
}

/// Where a configuration says to connect, and how.
pub fn client_of(map: &Map) -> Result<(Box<dyn Connect>, LinkOptions), ProviderError> {
    if let Some(child) = map.get("child") {
        let list = TryAsRef::<List>::try_as_ref(child)
            .ok_or_else(|| refused("`child` is not a list of the program and its arguments"))?;
        let mut words = list.iter().map(|v| {
            TryAsRef::<str>::try_as_ref(v)
                .map(str::to_string)
                .ok_or_else(|| refused("`child` holds something that is not text"))
        });
        let program = words
            .next()
            .ok_or_else(|| refused("`child` names no program"))??;
        let mut command = ChildCommand::new(program);
        for w in words {
            command = command.arg(w?);
        }
        let trust = trust_of(text(map, "trust")?.unwrap_or("local"))?;
        return Ok((Box::new(command), options_of(map, trust)?));
    }
    let connect = text(map, "connect")?
        .ok_or_else(|| refused("the configuration names neither `connect` nor `child`"))?;
    let (scheme, rest) = connect
        .split_once("://")
        .ok_or_else(|| refused(format!("`connect` is `{connect}`, not `scheme://address`")))?;
    let trust =
        text(map, "trust")?.ok_or_else(|| refused("a network link names its `trust` level"))?;
    let trust = trust_of(trust)?;
    let _ = (rest, trust);
    match scheme {
        #[cfg(feature = "tcp")]
        "tcp" => {
            if trust != Trust::Trusted {
                return Err(refused(
                    "`tcp://` runs the trusted level; the secure level is `tls://` or `quic://`",
                ));
            }
            Ok((
                Box::new(crate::transport::tcp::TcpConnect::new(rest)),
                options_of(map, trust)?,
            ))
        }
        #[cfg(feature = "rdma")]
        "rdma" => {
            use crate::transport::rdma::{RdmaConnect, RdmaPort};
            if trust != Trust::Trusted {
                return Err(refused("`rdma://` runs the trusted level"));
            }
            let port = text(map, "rdma_device")?.map_or_else(RdmaPort::first, RdmaPort::named);
            Ok((
                Box::new(RdmaConnect::new(rest).port(port)),
                options_of(map, trust)?,
            ))
        }
        #[cfg(feature = "local")]
        "local" => {
            if trust != Trust::Local {
                return Err(refused("`local://` runs the local level"));
            }
            Ok((
                Box::new(crate::transport::local::LocalConnect::new(rest)),
                options_of(map, trust)?,
            ))
        }
        #[cfg(feature = "shm")]
        "shm" => {
            if trust != Trust::Local {
                return Err(refused("`shm://` runs the local level"));
            }
            Ok((
                Box::new(crate::transport::shm::ShmConnect::new(rest)),
                options_of(map, trust)?,
            ))
        }
        other => Err(ProviderError::new(
            Status::GUATIAO_ERR_UNSUPPORTED,
            &format!("`{other}://` is not a transport this build of guatiao-source-remote has"),
        )),
    }
}
