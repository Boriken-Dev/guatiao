// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A source's configuration as a host hands it over, the same in every
//! language:
//!
//! | key | holds |
//! | --- | --- |
//! | `connect` | `tcp://host:port`, `ws://host:port/path` (WebSocket, as a browser connects), `local://name` (a socket path on Unix, a pipe name on Windows), `shm://name` (shared memory beside that), `rdma://host:port` (RDMA, experimental, Linux only), or for a child `child` instead |
//! | `rdma_device` | the RDMA device `rdma://` goes through (default: the first) |
//! | `child` | a list: the program and its arguments, served on its stdio |
//! | `trust` | `local`, `trusted` or `secure`; `local` for a child |
//! | `key`, `key_file` | the trusted level's pre-shared key, as text or a file's bytes |
//! | `token` | what a secure client presents |
//! | `timeout_ms` | how long a call waits; `0` waits as long as the link lives |
//! | `max_frame`, `max_calls` | the link's bounds |
//! | `dirs`, `manifests` | instead of `connect` or `child`: directories of peer manifests (and their immediate subdirectories), and manifest files; the keys above are then every peer's unless its manifest names them |
//!
//! A peer manifest is a `<name>.guatiao.toml` of `type = "remote"`:
//!
//! ```toml
//! type = "remote"
//! name = "lab-printer"             # its libraries and providers are `lab-printer/<id>`
//! connect = "tcp://192.0.2.5:7000"
//! trust = "trusted"
//! key_file = "lab-printer.key"     # relative to this file
//! ```
//!
//! It may also name `timeout_ms`, `max_frame`, `max_calls` and
//! `rdma_device`, and nothing else: never `key` or `token` (a secret in a
//! search path), never `child` (a manifest names a peer that listens).

#![forbid(unsafe_code)]

use core::time::Duration;
use std::path::{Path, PathBuf};

use guatiao::library::ProviderError;
use guatiao::value::convert::{FromValue, TryAsRef};
use guatiao::{List, Map, Status};
use guatiao_bridge::manifest::ManifestFormat;

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

/// The secure level's refusal: its transports live outside this build.
fn no_secure_level() -> ProviderError {
    ProviderError::new(
        Status::GUATIAO_ERR_UNSUPPORTED,
        "the secure level is not in this build: reach a peer across an open network only through a link secured outside it",
    )
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
    if trust == Trust::Secure {
        return Err(no_secure_level());
    }
    let _ = (rest, trust);
    match scheme {
        #[cfg(feature = "tcp")]
        "tcp" => {
            if trust != Trust::Trusted {
                return Err(refused("`tcp://` runs the trusted level"));
            }
            Ok((
                Box::new(crate::transport::tcp::TcpConnect::new(rest)),
                options_of(map, trust)?,
            ))
        }
        #[cfg(feature = "ws")]
        "ws" => {
            if trust != Trust::Trusted {
                return Err(refused("`ws://` runs the trusted level"));
            }
            Ok((
                Box::new(crate::transport::ws::WsConnect::new(connect)),
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

/// Peer manifests: `type = "remote"`, the peer's `name` their id, their
/// stamp their content alone.
pub(crate) const PEER_MANIFESTS: ManifestFormat = ManifestFormat {
    serves: "remote",
    id_key: "name",
    code_extensions: &[],
    skip_dirs: &[],
};

/// What a peer manifest may name besides its `type` and `name`.
const PEER_KEYS: &[&str] = &[
    "connect",
    "trust",
    "key_file",
    "timeout_ms",
    "max_frame",
    "max_calls",
    "rdma_device",
];

/// The paths a configuration lists under `key`, or none.
pub(crate) fn paths(map: &Map, key: &str) -> Result<Vec<PathBuf>, ProviderError> {
    let Some(value) = map.get(key) else {
        return Ok(Vec::new());
    };
    let list = TryAsRef::<List>::try_as_ref(value)
        .ok_or_else(|| refused(format!("`{key}` is not a list of paths")))?;
    list.iter()
        .map(|v| {
            TryAsRef::<str>::try_as_ref(v)
                .map(PathBuf::from)
                .ok_or_else(|| refused(format!("`{key}` is not a list of paths")))
        })
        .collect()
}

/// The key file a peer manifest names, resolved against its directory.
fn key_file_of(manifest: &Map, path: &Path) -> Result<Option<PathBuf>, ProviderError> {
    let base = path.parent().unwrap_or(Path::new("."));
    Ok(text(manifest, "key_file")?.map(|file| base.join(file)))
}

/// What changes when the key file a peer uses does (its manifest's, else
/// the source's): its path and a hash of its bytes, so a replaced key is
/// a new peer.
pub(crate) fn key_stamp(defaults: &Map, manifest: &Map, path: &Path) -> String {
    let file = match key_file_of(manifest, path) {
        Ok(Some(file)) => file,
        Ok(None) => match text(defaults, "key_file") {
            Ok(Some(file)) => PathBuf::from(file),
            _ => return String::new(),
        },
        Err(_) => return String::new(),
    };
    let hash = std::fs::read(&file).map_or_else(
        |_| "absent".to_string(),
        |bytes| {
            let h = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
                (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
            });
            format!("{h:016x}")
        },
    );
    format!("{}={hash}", file.display())
}

/// Checks what a source of described peers gives every peer, once, so a
/// mistake there is the configuration's refusal rather than every
/// manifest's.
pub(crate) fn check_defaults(defaults: &Map) -> Result<(), ProviderError> {
    let trust = match text(defaults, "trust")? {
        Some(name) => trust_of(name)?,
        None => Trust::Trusted,
    };
    options_of(defaults, trust).map(|_| ())
}

/// The peer a manifest at `path` describes: its keys over the source's
/// `defaults`, its `key_file` read from beside it.
pub(crate) fn peer_of(
    defaults: &Map,
    manifest: &Map,
    path: &Path,
) -> Result<(Box<dyn Connect>, LinkOptions), ProviderError> {
    for entry in manifest.iter() {
        match entry.key() {
            "type" | "name" => {}
            key if PEER_KEYS.contains(&key) => {}
            secret @ ("key" | "token") => {
                return Err(refused(format!(
                    "`{secret}` is a secret, and a manifest in a search path holds none: name the file holding the key with `key_file`"
                )));
            }
            "child" => {
                return Err(refused(
                    "a peer manifest names a peer that listens, by `connect`; a child the host starts is the source's own configuration",
                ));
            }
            other => {
                return Err(refused(format!(
                    "`{other}` is not a key of a peer manifest: type, name, {}",
                    PEER_KEYS.join(", ")
                )));
            }
        }
    }
    let mut merged = Map::new();
    for entry in defaults.iter() {
        if !matches!(entry.key(), "dirs" | "manifests" | "connect" | "child") {
            merged.set(entry.key(), entry.value().clone());
        }
    }
    for entry in manifest.iter() {
        if PEER_KEYS.contains(&entry.key()) {
            merged.set(entry.key(), entry.value().clone());
        }
    }
    if let Some(file) = key_file_of(manifest, path)? {
        merged.remove("key");
        merged.set("key_file", file.display().to_string().as_str());
    }
    if !merged.contains_key("connect") {
        return Err(refused(
            "a peer manifest names where the peer is, `connect`",
        ));
    }
    client_of(&merged)
}
