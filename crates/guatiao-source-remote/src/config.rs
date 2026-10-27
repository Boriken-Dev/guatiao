// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A source's configuration as a host hands it over, the same in every
//! language:
//!
//! | key | holds |
//! | --- | --- |
//! | `connect` | `tcp://host:port`, `quic://host:port` (QUIC), `ws://host:port/path` and `wss://host:port/path` (WebSocket, as a browser connects, without and with TLS), `https://host:port/path` (WebTransport, HTTP/3 over QUIC, as a browser connects), `tls://host:port` (TCP with TLS), `local://name` (a socket path on Unix, a pipe name on Windows), `shm://name` (shared memory beside that), `rdma://host:port` (RDMA, experimental, Linux only), or for a child `child` instead |
//! | `rdma_device` | the RDMA device `rdma://` goes through (default: the first) |
//! | `child` | a list: the program and its arguments, served on its stdio |
//! | `security` | `none`, `server-tls` or `mutual-tls`: none on a child, `local://` and `shm://` (the only one they run), mutual TLS on every network transport unless told |
//! | `key`, `key_file` | the pre-shared key, as text or a file's bytes: proved both ways with `none`, and at server TLS when the server asks for it |
//! | `token`, `token_file` | what a client presents when its server requires a token, as text or a file's text (a byte order mark and one last line end left out); one or the other, and one empty or only white space is refused |
//! | `state_dir` | TLS: this side's identity (`identity.pem`, made when absent) and trust store (`peers/`) |
//! | `identity_file` | TLS: the identity file instead (made when absent) |
//! | `pins` | TLS: the fingerprints (`sha256:<hex>`) of the servers trusted; without any, the trust store's accepted peers |
//! | `roots` | TLS: also trust a server whose certificate an authority issued for the host dialled: `"platform"` (the operating system's, feature `platform-roots`) or a CA bundle's path (PEM) |
//! | `policy` | TLS: the TLS policy, a map in coqui-peer-tls's text form (default strict) |
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
//! security = "none"
//! key_file = "lab-printer.key"     # relative to this file
//! ```
//!
//! It may also name `timeout_ms`, `max_frame`, `max_calls`,
//! `rdma_device`, and for TLS `token_file`, `pins`, `roots`, `policy`,
//! `identity_file` and `state_dir` (paths relative to it), and nothing
//! else: never `key` or `token` (a secret in a search path), never `child`
//! (a manifest names a peer that listens).

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

/// A link's security: one choice, the same on every transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Security {
    /// No TLS. On the same machine (a child, `local://`, `shm://`) the
    /// operating system's permissions are the gate; across a network peers
    /// are admitted by an access control list and prove a pre-shared key
    /// both ways, and nothing is encrypted (over QUIC, which always runs
    /// TLS, no certificate is checked).
    None,
    /// TLS that proves the server by its certificate; the client proves
    /// itself by the pre-shared key (when the server asks for it) or a
    /// token, inside the encrypted channel.
    ServerTls,
    /// TLS that proves both sides by certificate.
    MutualTls,
}

impl Security {
    /// Its name in a configuration: `none`, `server-tls`, `mutual-tls`.
    pub fn name(self) -> &'static str {
        match self {
            Security::None => "none",
            Security::ServerTls => "server-tls",
            Security::MutualTls => "mutual-tls",
        }
    }

    /// What a link runs when nothing names its security: none on a child
    /// and on `local://` and `shm://`, mutual TLS on every other scheme.
    pub fn default_for(scheme: &str) -> Security {
        match scheme {
            "child" | "stdio" | "local" | "shm" => Security::None,
            _ => Security::MutualTls,
        }
    }

    /// The trust level a HELLO names for it: none is `local` on the same
    /// machine and `trusted` across a network.
    pub fn trust(self, local: bool) -> Trust {
        match self {
            Security::None if local => Trust::Local,
            Security::None => Trust::Trusted,
            Security::ServerTls => Trust::ServerTls,
            Security::MutualTls => Trust::Secure,
        }
    }
}

/// Reads a security's name.
pub fn security_of(name: &str) -> Result<Security, ProviderError> {
    Ok(match name {
        "none" => Security::None,
        "server-tls" => Security::ServerTls,
        "mutual-tls" => Security::MutualTls,
        other => {
            return Err(refused(format!(
                "`security` is `{other}`, not `none`, `server-tls` or `mutual-tls`"
            )));
        }
    })
}

/// The security a configuration names, else `scheme`'s default.
fn security_in(map: &Map, scheme: &str) -> Result<Security, ProviderError> {
    text(map, "security")?.map_or(Ok(Security::default_for(scheme)), security_of)
}

/// The token a file holds: its text, a byte order mark and one last line
/// end left out. One empty or only white space is refused.
pub fn token_in(path: &Path) -> Result<String, ProviderError> {
    let text =
        std::fs::read_to_string(path).map_err(|e| refused(format!("{}: {e}", path.display())))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let token = text
        .strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(text);
    if token.trim().is_empty() {
        return Err(refused(format!(
            "{}: the token is empty or only white space, and proves nothing",
            path.display()
        )));
    }
    Ok(token.to_string())
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
    match (text(map, "token")?, text(map, "token_file")?) {
        (Some(_), Some(_)) => return Err(refused("name `token` or `token_file`, not both")),
        (Some(token), None) => {
            if token.trim().is_empty() {
                return Err(refused(
                    "`token` is empty or only white space, and proves nothing: name the token the server requires, or leave `token` out",
                ));
            }
            options = options.token(token);
        }
        (None, Some(path)) => {
            let token = token_in(Path::new(path))
                .map_err(|e| refused(format!("`token_file` {}", e.message())))?;
            options = options.token(&token);
        }
        (None, None) => {}
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

/// TLS's refusal in a build without `tls`.
#[cfg(not(feature = "tls"))]
fn no_tls(security: Security) -> ProviderError {
    ProviderError::new(
        Status::GUATIAO_ERR_UNSUPPORTED,
        &format!(
            "{} is not in this build (guatiao-source-remote's `tls` feature): name `security = \"none\"` only where the network itself is the boundary",
            security.name()
        ),
    )
}

/// A scheme that runs no TLS, asked for it.
#[cfg(any(feature = "ws", feature = "local", feature = "rdma"))]
fn plain_only(scheme: &str, security: Security) -> ProviderError {
    refused(format!(
        "`{scheme}://` carries no TLS, and the link is {}: name `security = \"none\"`{}",
        security.name(),
        match scheme {
            "ws" => ", or connect to `wss://`",
            "local" | "shm" => " (the operating system's permissions are the gate)",
            _ => "",
        }
    ))
}

/// Reads an access control list from a value in coqui-net-acl's text
/// form: `default` (`deny` unless `permit`) and `rule`, a list of maps
/// with `action`, `source`, `destination`, `port`, `fingerprint`, `log`.
#[cfg(feature = "tcp")]
pub fn acl_of(value: &guatiao::Value) -> Result<coqui_net_acl::Acl, ProviderError> {
    guatiao_serde::adapt::from_value(value)
        .map_err(|e| refused(format!("not an access control list: {e}")))
}

/// Reads a TLS policy from a value in coqui-peer-tls's text form, version
/// 1: `profile` (`strict`, the default, `compatible`, `legacy`) and any
/// field over it (`tls_versions`, `cipher_suites`, `kx_groups`,
/// `pin_digests`, `check_validity`, `allow_chain_when_pinned`,
/// `allow_resumption`, the waiting caps, `acl`).
#[cfg(feature = "tls")]
pub fn policy_of(value: &guatiao::Value) -> Result<crate::transport::tls::Policy, ProviderError> {
    guatiao_serde::adapt::from_value(value).map_err(|e| refused(format!("not a TLS policy: {e}")))
}

/// Every setting looser than strict in the TLS policy a configuration
/// names (its access control list's default included), when its link runs
/// TLS; empty under the strict policy, and at no security, where no TLS
/// policy applies.
#[cfg(feature = "tls")]
pub(crate) fn weakened_in(map: &Map) -> Vec<crate::transport::tls::Weakening> {
    let scheme = match text(map, "connect") {
        Ok(Some(connect)) => connect.split_once("://").map_or("", |(scheme, _)| scheme),
        _ => "child",
    };
    match (security_in(map, scheme), map.get("policy")) {
        (Ok(Security::None) | Err(_), _) | (_, None) => Vec::new(),
        (Ok(_), Some(policy)) => policy_of(policy).map_or_else(|_| Vec::new(), |p| p.weakened()),
    }
}

/// The authorities `roots` names: `platform`, or a CA bundle's path.
#[cfg(feature = "tls")]
pub fn roots_of(text: &str) -> Result<crate::transport::tls::Roots, ProviderError> {
    use crate::transport::tls::Roots;
    if text == "platform" {
        #[cfg(feature = "platform-roots")]
        return Ok(Roots::platform());
        #[cfg(not(feature = "platform-roots"))]
        return Err(ProviderError::new(
            Status::GUATIAO_ERR_UNSUPPORTED,
            "`roots = \"platform\"` is not in this build (guatiao-source-remote's `platform-roots` feature): name a CA bundle file instead",
        ));
    }
    let bundle =
        std::fs::read_to_string(text).map_err(|e| refused(format!("`roots` {text}: {e}")))?;
    if !bundle.contains("-----BEGIN CERTIFICATE-----") {
        return Err(refused(format!(
            "`roots` {text} holds no certificate: name a PEM bundle of the authorities' certificates, or `platform`"
        )));
    }
    Ok(Roots::bundle_pem(bundle))
}

/// What a client's TLS runs as, from a configuration's `state_dir` or
/// `identity_file`, `pins`, `roots` and `policy`, at `security`: mutual
/// TLS names this side's identity, server TLS needs none (and presents
/// none).
#[cfg(feature = "tls")]
pub fn tls_config_of(
    map: &Map,
    security: Security,
) -> Result<crate::transport::tls::TlsConfig, ProviderError> {
    use crate::transport::tls::{PeerId, TlsConfig};
    let mut config = match (text(map, "state_dir")?, text(map, "identity_file")?) {
        (None, None) if security == Security::ServerTls => {
            TlsConfig::without_identity().map_err(|e| refused(e.to_string()))?
        }
        (Some(dir), None) => {
            TlsConfig::open(Path::new(dir)).map_err(|e| refused(format!("`state_dir`: {e}")))?
        }
        (None, Some(file)) => TlsConfig::new(
            crate::transport::tls::identity_at(Path::new(file)).map_err(|e| {
                refused(format!(
                    "`identity_file` {file}: {}",
                    crate::transport::tls::words(&e)
                ))
            })?,
        ),
        (Some(_), Some(_)) => {
            return Err(refused("name `state_dir` or `identity_file`, not both"));
        }
        (None, None) => {
            return Err(refused(
                "mutual TLS names this side's identity: `state_dir` or `identity_file`",
            ));
        }
    };
    if security == Security::ServerTls {
        config = config.server_tls();
    }
    if let Some(pins) = map.get("pins") {
        let list = TryAsRef::<List>::try_as_ref(pins)
            .ok_or_else(|| refused("`pins` is not a list of fingerprints"))?;
        let pins = list
            .iter()
            .map(|v| {
                TryAsRef::<str>::try_as_ref(v)
                    .and_then(|t| t.parse::<PeerId>().ok())
                    .ok_or_else(|| refused("`pins` holds something that is not a fingerprint"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        config = config.pins(pins);
    }
    if let Some(roots) = text(map, "roots")? {
        config = config.roots(roots_of(roots)?);
    }
    if let Some(policy) = map.get("policy") {
        config = config.policy(policy_of(policy)?);
    }
    Ok(config)
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
        let security = security_in(map, "child")?;
        if security != Security::None {
            return Err(refused(format!(
                "a child runs over its stdio with no TLS, and the link is {}: leave `security` out",
                security.name()
            )));
        }
        return Ok((Box::new(command), options_of(map, Trust::Local)?));
    }
    let connect = text(map, "connect")?
        .ok_or_else(|| refused("the configuration names neither `connect` nor `child`"))?;
    let (scheme, rest) = connect
        .split_once("://")
        .ok_or_else(|| refused(format!("`connect` is `{connect}`, not `scheme://address`")))?;
    let security = security_in(map, scheme)?;
    let local = matches!(scheme, "local" | "shm");
    let trust = security.trust(local);
    #[cfg(not(feature = "tls"))]
    if security != Security::None && !local {
        return Err(no_tls(security));
    }
    let _ = (rest, trust);
    match scheme {
        #[cfg(feature = "tcp")]
        "tcp" | "tls" => match security {
            Security::None if scheme == "tls" => Err(refused(
                "`tls://` runs TLS: name `security` `server-tls` or `mutual-tls`, or connect to `tcp://`",
            )),
            Security::None => Ok((
                Box::new(crate::transport::tcp::TcpConnect::new(rest)),
                options_of(map, trust)?,
            )),
            #[cfg(feature = "tls")]
            _ => {
                let config = tls_config_of(map, security)?;
                let connect = crate::transport::tls::TlsConnect::new(rest, &config)
                    .map_err(|e| refused(e.to_string()))?;
                Ok((Box::new(connect), options_of(map, trust)?))
            }
            #[cfg(not(feature = "tls"))]
            _ => Err(no_tls(security)),
        },
        #[cfg(feature = "quic")]
        "quic" => {
            if security == Security::None {
                let connect = crate::transport::quic::QuicConnect::trusted(rest)
                    .map_err(|e| refused(e.to_string()))?;
                return Ok((Box::new(connect), options_of(map, trust)?));
            }
            let config = tls_config_of(map, security)?;
            let connect = crate::transport::quic::QuicConnect::new(rest, &config)
                .map_err(|e| refused(e.to_string()))?;
            Ok((Box::new(connect), options_of(map, trust)?))
        }
        #[cfg(not(feature = "quic"))]
        "quic" => Err(ProviderError::new(
            Status::GUATIAO_ERR_UNSUPPORTED,
            "`quic://` is not in this build (guatiao-source-remote's `quic` feature)",
        )),
        // WebTransport runs QUIC's TLS at every security, as `quic://`.
        #[cfg(feature = "webtransport")]
        "https" => {
            use crate::transport::webtransport::WtConnect;
            let connect = if security == Security::None {
                WtConnect::trusted(connect)
            } else {
                WtConnect::new(connect, &tls_config_of(map, security)?)
            }
            .map_err(|e| refused(e.to_string()))?;
            Ok((Box::new(connect), options_of(map, trust)?))
        }
        #[cfg(not(feature = "webtransport"))]
        "https" => Err(ProviderError::new(
            Status::GUATIAO_ERR_UNSUPPORTED,
            "`https://` (WebTransport) is not in this build (guatiao-source-remote's `webtransport` feature)",
        )),
        #[cfg(feature = "ws")]
        "ws" => {
            if security != Security::None {
                return Err(plain_only(scheme, security));
            }
            Ok((
                Box::new(crate::transport::ws::WsConnect::new(connect)),
                options_of(map, trust)?,
            ))
        }
        #[cfg(feature = "ws")]
        "wss" => match security {
            Security::None => Err(refused(
                "`wss://` runs TLS: name `security` `server-tls` or `mutual-tls`, or connect to `ws://`",
            )),
            #[cfg(feature = "tls")]
            _ => {
                let config = tls_config_of(map, security)?;
                let connect = crate::transport::ws::WsConnect::new(connect)
                    .tls(&config)
                    .map_err(|e| refused(e.to_string()))?;
                Ok((Box::new(connect), options_of(map, trust)?))
            }
            #[cfg(not(feature = "tls"))]
            _ => Err(no_tls(security)),
        },
        #[cfg(feature = "rdma")]
        "rdma" => {
            use crate::transport::rdma::{RdmaConnect, RdmaPort};
            if security != Security::None {
                return Err(plain_only(scheme, security));
            }
            let port = text(map, "rdma_device")?.map_or_else(RdmaPort::first, RdmaPort::named);
            Ok((
                Box::new(RdmaConnect::new(rest).port(port)),
                options_of(map, trust)?,
            ))
        }
        #[cfg(feature = "local")]
        "local" => {
            if security != Security::None {
                return Err(plain_only(scheme, security));
            }
            Ok((
                Box::new(crate::transport::local::LocalConnect::new(rest)),
                options_of(map, trust)?,
            ))
        }
        #[cfg(feature = "shm")]
        "shm" => {
            if security != Security::None {
                return Err(plain_only(scheme, security));
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
    "security",
    "key_file",
    "token_file",
    "timeout_ms",
    "max_frame",
    "max_calls",
    "rdma_device",
    "pins",
    "roots",
    "policy",
    "identity_file",
    "state_dir",
];

/// What a peer manifest names by a path, resolved against its directory.
const PEER_PATHS: &[&str] = &["key_file", "token_file", "identity_file", "state_dir"];

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

/// A path a peer manifest names under `key`, resolved against its
/// directory.
fn manifest_path(manifest: &Map, path: &Path, key: &str) -> Result<Option<PathBuf>, ProviderError> {
    let base = path.parent().unwrap_or(Path::new("."));
    Ok(text(manifest, key)?.map(|file| base.join(file)))
}

/// The file under `key` a peer uses: its manifest's, else the source's.
fn peer_file(defaults: &Map, manifest: &Map, path: &Path, key: &str) -> Option<PathBuf> {
    match manifest_path(manifest, path, key) {
        Ok(Some(file)) => Some(file),
        Ok(None) => text(defaults, key).ok().flatten().map(PathBuf::from),
        Err(_) => None,
    }
}

/// What changes when a file a peer's trust rests on does (the key file,
/// the token file, the identity file or the one in its state directory,
/// the CA bundle): each one's path and a hash of its bytes, so a replaced
/// key, token, identity or bundle is a new peer.
pub(crate) fn key_stamp(defaults: &Map, manifest: &Map, path: &Path) -> String {
    let mut files = Vec::new();
    files.extend(peer_file(defaults, manifest, path, "key_file"));
    files.extend(peer_file(defaults, manifest, path, "token_file"));
    files.extend(peer_file(defaults, manifest, path, "identity_file"));
    let roots = [manifest, defaults]
        .into_iter()
        .find_map(|map| text(map, "roots").ok().flatten());
    if roots != Some("platform") {
        files.extend(peer_file(defaults, manifest, path, "roots"));
    }
    #[cfg(feature = "tls")]
    files.extend(
        peer_file(defaults, manifest, path, "state_dir")
            .map(|dir| dir.join(crate::transport::tls::IDENTITY_FILE)),
    );
    files
        .iter()
        .map(|file| {
            let hash = std::fs::read(file).map_or_else(
                |_| "absent".to_string(),
                |bytes| {
                    let h = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
                        (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
                    });
                    format!("{h:016x}")
                },
            );
            format!("{}={hash}", file.display())
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// Checks what a source of described peers gives every peer, once, so a
/// mistake there is the configuration's refusal rather than every
/// manifest's.
pub(crate) fn check_defaults(defaults: &Map) -> Result<(), ProviderError> {
    security_in(defaults, "tcp")?;
    options_of(defaults, Trust::Trusted).map(|_| ())
}

/// The peer a manifest at `path` describes: its keys over the source's
/// `defaults`, its `key_file` and `token_file` read from beside it; and
/// that configuration.
pub(crate) fn peer_of(
    defaults: &Map,
    manifest: &Map,
    path: &Path,
) -> Result<(Box<dyn Connect>, LinkOptions, Map), ProviderError> {
    for entry in manifest.iter() {
        match entry.key() {
            "type" | "name" => {}
            key if PEER_KEYS.contains(&key) => {}
            secret @ ("key" | "token") => {
                return Err(refused(format!(
                    "`{secret}` is a secret, and a manifest in a search path holds none: name the file holding it with `{secret}_file`"
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
    for key in PEER_PATHS {
        if let Some(file) = manifest_path(manifest, path, key)? {
            merged.set(key, file.display().to_string().as_str());
        }
    }
    // A CA bundle is a path beside the manifest; `platform` is no file.
    if text(manifest, "roots")?.is_some_and(|roots| roots != "platform")
        && let Some(file) = manifest_path(manifest, path, "roots")?
    {
        merged.set("roots", file.display().to_string().as_str());
    }
    // The manifest's own secret replaces the source's in any form.
    let named = |key: &str| manifest.contains_key(key);
    if named("key_file") {
        merged.remove("key");
    }
    if named("token_file") {
        merged.remove("token");
    }
    if named("identity_file") && !named("state_dir") {
        merged.remove("state_dir");
    }
    if named("state_dir") && !named("identity_file") {
        merged.remove("identity_file");
    }
    if !merged.contains_key("connect") {
        return Err(refused(
            "a peer manifest names where the peer is, `connect`",
        ));
    }
    let (connect, options) = client_of(&merged)?;
    Ok((connect, options, merged))
}
