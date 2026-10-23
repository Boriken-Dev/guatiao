// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Why a bridge or its engine refused, and the status each crosses as.

use std::path::PathBuf;

use guatiao::Status;
use guatiao::library::ProviderError;

/// A limit an engine enforces on a library's code: see
/// [`Limits`](crate::Limits).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Limit {
    /// Operations (instructions, fuel, a counter the engine keeps) per call.
    Operations,
    /// Memory, in bytes.
    Memory,
    /// Time per call, in milliseconds.
    Time,
    /// How deep calls nest.
    Depth,
}

impl Limit {
    /// Its name in a configuration's `limits` and in a refusal.
    pub fn name(self) -> &'static str {
        match self {
            Limit::Operations => "operations",
            Limit::Memory => "memory",
            Limit::Time => "time_ms",
            Limit::Depth => "depth",
        }
    }
}

/// Why a bridge or its engine refused.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BridgeError {
    /// The source's configuration does not say what the bridge needs.
    Config {
        /// The key.
        key: String,
        /// What is wrong with it.
        why: String,
    },
    /// A manifest could not be read or does not describe a library.
    Manifest {
        /// The manifest.
        path: PathBuf,
        /// Why, in words that name the file.
        why: String,
    },
    /// No manifest lists the id now.
    NotListed {
        /// The id asked for.
        id: String,
    },
    /// The engine cannot run here: it did not start.
    Unavailable(String),
    /// The engine refused, in its own words: a library that does not load,
    /// a script error.
    Engine(String),
    /// A limit stopped the engine.
    Limit {
        /// Which limit.
        limit: Limit,
        /// Its value in the configuration.
        at: u64,
    },
    /// A kind a library serves has no description the bridge was given.
    UnknownKind(String),
    /// A kind table could not be built for a provider: which, and why.
    Table(String),
    /// The engine's thread is gone, or a call into it panicked.
    EngineGone,
}

impl BridgeError {
    /// A configuration key that is wrong.
    pub fn config(key: &str, why: impl Into<String>) -> BridgeError {
        BridgeError::Config {
            key: key.to_string(),
            why: why.into(),
        }
    }

    /// The engine's own words.
    pub fn engine(why: impl Into<String>) -> BridgeError {
        BridgeError::Engine(why.into())
    }

    /// The status this crosses a C boundary as.
    ///
    /// A memory limit is `GUATIAO_ERR_ALLOC`; any other limit, an engine
    /// gone or a panic is `GUATIAO_ERR_INTERNAL` (the call did not happen,
    /// the process carries on); a library that cannot be read or loaded is
    /// `GUATIAO_ERR_BAD_VALUE`, as any refusal of its content.
    pub fn status(&self) -> Status {
        match self {
            BridgeError::Config { .. }
            | BridgeError::Manifest { .. }
            | BridgeError::Engine(_)
            | BridgeError::Table(_) => Status::GUATIAO_ERR_BAD_VALUE,
            BridgeError::NotListed { .. } | BridgeError::UnknownKind(_) => {
                Status::GUATIAO_ERR_NOT_FOUND
            }
            BridgeError::Unavailable(_) => Status::GUATIAO_ERR_UNSUPPORTED,
            BridgeError::Limit {
                limit: Limit::Memory,
                ..
            } => Status::GUATIAO_ERR_ALLOC,
            BridgeError::Limit { .. } | BridgeError::EngineGone => Status::GUATIAO_ERR_INTERNAL,
        }
    }
}

impl core::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BridgeError::Config { key, why } => write!(f, "`{key}` {why}"),
            BridgeError::Manifest { why, .. } => f.write_str(why),
            BridgeError::NotListed { id } => write!(f, "no manifest describes `{id}` now"),
            BridgeError::Unavailable(why) | BridgeError::Engine(why) | BridgeError::Table(why) => {
                f.write_str(why)
            }
            BridgeError::Limit { limit, at } => {
                write!(f, "stopped at its {} limit ({at})", limit.name())
            }
            BridgeError::UnknownKind(kind) => {
                write!(f, "no description of the kind `{kind}` was given")
            }
            BridgeError::EngineGone => f.write_str("the engine is gone"),
        }
    }
}

impl core::error::Error for BridgeError {}

impl From<BridgeError> for ProviderError {
    fn from(e: BridgeError) -> ProviderError {
        ProviderError::new(e.status(), &e.to_string())
    }
}
