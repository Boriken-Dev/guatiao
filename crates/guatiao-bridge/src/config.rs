// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The configuration a host hands a bridge's source, and the limits in it.

use core::time::Duration;
use std::path::PathBuf;
use std::time::Instant;

use guatiao::Map;
use guatiao::value::convert::{FromValue, TryAsRef};

use crate::error::{BridgeError, Limit};

/// A list of text under `key`, or none when it is absent.
pub fn text_list(map: &Map, key: &str) -> Result<Vec<String>, BridgeError> {
    let Some(value) = map.get(key) else {
        return Ok(Vec::new());
    };
    let list: &guatiao::List = TryAsRef::try_as_ref(value)
        .ok_or_else(|| BridgeError::config(key, "is not a list of paths"))?;
    list.iter()
        .map(|v| {
            TryAsRef::<str>::try_as_ref(v)
                .map(str::to_string)
                .ok_or_else(|| BridgeError::config(key, "holds something that is not a path"))
        })
        .collect()
}

/// What a bridge's source is configured with: where its libraries'
/// manifests are, which kind descriptions it serves through, its limits,
/// and the whole map, for the keys only its engine reads.
///
/// | key | holds |
/// | --- | --- |
/// | `dirs` | directories whose manifests, and those of their immediate subdirectories, are libraries |
/// | `manifests` | manifest files, listed as well |
/// | `kinds` | kind description documents (`<crate>.kinds.json`) |
/// | `limits` | [`Limits`] |
#[derive(Debug, Clone)]
pub struct Config {
    map: Map,
    dirs: Vec<PathBuf>,
    manifests: Vec<PathBuf>,
    kinds: Vec<PathBuf>,
    limits: Limits,
}

impl Config {
    /// Reads the keys every bridge shares; any other key is the engine's.
    pub fn from_map(map: &Map) -> Result<Config, BridgeError> {
        let paths = |key| -> Result<Vec<PathBuf>, BridgeError> {
            Ok(text_list(map, key)?
                .into_iter()
                .map(PathBuf::from)
                .collect())
        };
        Ok(Config {
            dirs: paths("dirs")?,
            manifests: paths("manifests")?,
            kinds: paths("kinds")?,
            limits: Limits::from_config(map)?,
            map: map.clone(),
        })
    }

    /// The whole configuration.
    pub fn map(&self) -> &Map {
        &self.map
    }

    /// Directories of manifests.
    pub fn dirs(&self) -> &[PathBuf] {
        &self.dirs
    }

    /// Manifest files.
    pub fn manifests(&self) -> &[PathBuf] {
        &self.manifests
    }

    /// Kind description documents.
    pub fn kinds(&self) -> &[PathBuf] {
        &self.kinds
    }

    /// The limits a library's code runs under.
    pub fn limits(&self) -> &Limits {
        &self.limits
    }
}

/// What a library's code may use, from the configuration's `limits` map:
/// `{"operations": n, "memory": bytes, "time_ms": n, "depth": n}`, each
/// optional. The engine enforces what it can and says which it cannot;
/// a limit hit is a [`BridgeError::Limit`], `GUATIAO_ERR_ALLOC` for memory
/// and `GUATIAO_ERR_INTERNAL` for the rest, named in the error's words.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Limits {
    /// Operations per call.
    pub operations: Option<u64>,
    /// Memory, in bytes.
    pub memory: Option<u64>,
    /// Time per call.
    pub time: Option<Duration>,
    /// How deep calls nest.
    pub depth: Option<u64>,
}

impl Limits {
    /// The key a configuration holds them under.
    pub const KEY: &'static str = "limits";

    /// Reads `limits` from a configuration; none when it is absent. A key
    /// that is not a limit is refused, so a misspelled one does not go
    /// unenforced.
    pub fn from_config(config: &Map) -> Result<Limits, BridgeError> {
        let Some(value) = config.get(Self::KEY) else {
            return Ok(Limits::default());
        };
        let map: &Map = TryAsRef::try_as_ref(value)
            .ok_or_else(|| BridgeError::config(Self::KEY, "is not a map"))?;
        let mut limits = Limits::default();
        for entry in map.iter() {
            let limit = match entry.key() {
                "operations" => Limit::Operations,
                "memory" => Limit::Memory,
                "time_ms" => Limit::Time,
                "depth" => Limit::Depth,
                other => {
                    return Err(BridgeError::config(
                        Self::KEY,
                        format!(
                            "names `{other}`, which is not a limit (operations, memory, time_ms, depth)"
                        ),
                    ));
                }
            };
            let n = u64::from_value(entry.value()).map_err(|_| {
                BridgeError::config(
                    Self::KEY,
                    format!("`{}` is not a whole number", limit.name()),
                )
            })?;
            match limit {
                Limit::Operations => limits.operations = Some(n),
                Limit::Memory => limits.memory = Some(n),
                Limit::Time => limits.time = Some(Duration::from_millis(n)),
                Limit::Depth => limits.depth = Some(n),
            }
        }
        Ok(limits)
    }

    /// A budget for one call: counts operations and the time spent against
    /// these limits, for an engine that keeps its own count.
    pub fn budget(&self) -> Budget {
        Budget {
            limits: *self,
            spent: 0,
            started: Instant::now(),
        }
    }
}

/// One call's operations and time, against its [`Limits`].
#[derive(Debug, Clone)]
pub struct Budget {
    limits: Limits,
    spent: u64,
    started: Instant,
}

impl Budget {
    /// Counts `n` operations; refuses once the operations or time limit is
    /// passed.
    pub fn spend(&mut self, n: u64) -> Result<(), BridgeError> {
        self.spent = self.spent.saturating_add(n);
        if let Some(at) = self.limits.operations
            && self.spent > at
        {
            return Err(BridgeError::Limit {
                limit: Limit::Operations,
                at,
            });
        }
        if let Some(at) = self.limits.time
            && self.started.elapsed() > at
        {
            return Err(BridgeError::Limit {
                limit: Limit::Time,
                at: u64::try_from(at.as_millis()).unwrap_or(u64::MAX),
            });
        }
        Ok(())
    }

    /// The operations counted so far.
    pub fn spent(&self) -> u64 {
        self.spent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shared_keys_are_read_and_a_wrong_one_named() {
        let mut map = Map::new();
        let mut dirs = guatiao::List::new();
        dirs.push("libs");
        map.set("dirs", dirs);
        let mut limits = Map::new();
        limits.set("operations", 10u64);
        limits.set("time_ms", 50u64);
        map.set("limits", limits);
        let config = Config::from_map(&map).expect("read");
        assert_eq!(config.dirs(), [PathBuf::from("libs")]);
        assert_eq!(config.limits().operations, Some(10));
        assert_eq!(config.limits().time, Some(Duration::from_millis(50)));

        let mut wrong = Map::new();
        wrong.set("dirs", "libs");
        let e = Config::from_map(&wrong).unwrap_err();
        assert_eq!(e.to_string(), "`dirs` is not a list of paths");

        let mut misspelled = Map::new();
        let mut limits = Map::new();
        limits.set("operation", 10u64);
        misspelled.set("limits", limits);
        assert!(
            Config::from_map(&misspelled)
                .unwrap_err()
                .to_string()
                .contains("`operation`")
        );
    }

    #[test]
    fn a_budget_refuses_past_its_limit() {
        let limits = Limits {
            operations: Some(3),
            ..Limits::default()
        };
        let mut budget = limits.budget();
        assert!(budget.spend(3).is_ok());
        let e = budget.spend(1).unwrap_err();
        assert_eq!(
            e,
            BridgeError::Limit {
                limit: Limit::Operations,
                at: 3
            }
        );
        assert_eq!(e.status(), guatiao::Status::GUATIAO_ERR_INTERNAL);
    }
}
