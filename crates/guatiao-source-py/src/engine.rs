// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! CPython as a guatiao-bridge engine: a library is a module, its manifest
//! a `*.guatiao` of type `python`, its tables built by the Python binding itself.

#![forbid(unsafe_code)]

use guatiao::Map;
use guatiao::library::Note;
use guatiao::value::convert::TryAsRef;
use guatiao_bridge::{
    BridgeError, Config, Engine, Manifest, ManifestFormat, Opened, ThreadModel, text_list,
};

use crate::start::{Got, Isolation, Settings, StartError};

/// The one interpreter this file keeps for the program: started (or
/// joined, in a Python host) when the source is added, never finalised.
pub(crate) struct Python {
    /// What this source tells its host about how the interpreter runs.
    notes: Vec<Note>,
}

impl Engine for Python {
    /// The handle `bridge.py` answers for an open library.
    type Library = i64;

    /// Called from any thread; every call takes the GIL.
    const THREADS: ThreadModel = ThreadModel::Locked;

    const MANIFESTS: ManifestFormat = ManifestFormat {
        serves: "python",
        id_key: "module",
        code_extensions: &["py"],
        skip_dirs: &["__pycache__"],
    };

    /// Starts the interpreter under the configuration's `isolation` and
    /// `home` (or joins a Python host's), puts its `paths` first on
    /// `sys.path` and points the binding at this file's own exports.
    fn start(config: &Config) -> Result<Python, BridgeError> {
        let settings = settings(config.map())?;
        let paths = text_list(config.map(), "paths")?;
        let got = crate::start::start(&settings).map_err(|e| match e {
            StartError::Started(why) => BridgeError::Config {
                key: "isolation".to_string(),
                why,
            },
            StartError::Failed(why) => BridgeError::Unavailable(why),
        })?;
        crate::bridge::start(&paths, crate::exports::own_path().as_deref())
            .map_err(BridgeError::Unavailable)?;
        Ok(Python {
            notes: notes(&settings, got),
        })
    }

    /// The module's `__version__`, read as a literal from its source,
    /// which is not imported.
    fn version_of(&self, manifest: &Manifest) -> Result<String, BridgeError> {
        crate::bridge::literal_version(manifest.id(), manifest.paths()).map_err(BridgeError::Engine)
    }

    /// Imports the module afresh; `guatiao.provide` builds its library.
    fn open(&self, manifest: &Manifest, version: &str) -> Result<Opened<i64>, BridgeError> {
        let (address, library) =
            crate::bridge::open(manifest.id(), version, manifest.kinds(), manifest.paths())
                .map_err(BridgeError::Engine)?;
        let descriptor = crate::exports::descriptor(address).inspect_err(|_| {
            crate::bridge::close(library);
        })?;
        Ok(Opened::Built {
            library,
            descriptor,
        })
    }

    /// Lets the library's instances and modules go.
    fn close(&self, library: i64) {
        crate::bridge::close(library);
    }

    /// How the interpreter runs where that is looser than `shipped`, the
    /// default: what [`notes`] found when the source started.
    fn notes(&self) -> Vec<Note> {
        self.notes.clone()
    }
}

/// A note `isolation` while the environment reaches the interpreter
/// (`inherit` here, or one joined that does not ignore it), and a note
/// `home` when `home` was named and the interpreter was joined.
fn notes(settings: &Settings, got: Got) -> Vec<Note> {
    let mut notes = Vec::new();
    let joined = matches!(got, Got::Joined { .. });
    let environment = match got {
        Got::Started => settings.isolation == Isolation::Inherit,
        Got::Joined { environment } => environment,
    };
    if environment {
        let mut detail = Map::new();
        detail.set("isolation", "inherit");
        detail.set("joined", joined);
        let words = if joined {
            "the interpreter already running in this process is used as it is, and its \
             environment reaches it: PYTHONPATH, PYTHONHOME, the other PYTHON* variables \
             and user site-packages"
        } else {
            "the interpreter takes its environment (isolation `inherit`): PYTHONPATH, \
             PYTHONHOME, the other PYTHON* variables and user site-packages"
        };
        notes.push(Note::new("isolation", words).with_detail(detail));
    }
    if let (true, Some(home)) = (joined, &settings.home) {
        let mut detail = Map::new();
        detail.set("home", home.as_str());
        notes.push(
            Note::new(
                "home",
                format!(
                    "`home` ({home}) is not applied: the interpreter was already running in \
                     this process"
                ),
            )
            .with_detail(detail),
        );
    }
    notes
}

/// `isolation` (`shipped` unless given) and `home`, which `inherit` does
/// not take: there the environment's `PYTHONHOME` decides.
fn settings(map: &Map) -> Result<Settings, BridgeError> {
    let text = |key: &str| -> Result<Option<&str>, BridgeError> {
        map.get(key)
            .map(|v| {
                TryAsRef::<str>::try_as_ref(v)
                    .ok_or_else(|| BridgeError::config(key, "is not text"))
            })
            .transpose()
    };
    let asked = text("isolation")?;
    let isolation = match asked {
        None => Isolation::Shipped,
        Some(name) => Isolation::parse(name).ok_or_else(|| {
            BridgeError::config(
                "isolation",
                format!("is `{name}`, not `shipped`, `inherit` or `isolated`"),
            )
        })?,
    };
    // A relative `home` is this file's: where the program ships it.
    let beside = crate::exports::own_path()
        .and_then(|own| std::path::Path::new(&own).parent().map(|d| d.to_path_buf()));
    let home = text("home")?.map(|home| match &beside {
        Some(beside) if std::path::Path::new(home).is_relative() => {
            beside.join(home).display().to_string()
        }
        _ => home.to_string(),
    });
    if let Some(home) = &home {
        if isolation == Isolation::Inherit {
            return Err(BridgeError::config(
                "home",
                "is not taken under the isolation `inherit`, which leaves it to the environment",
            ));
        }
        if !std::path::Path::new(home).is_dir() {
            return Err(BridgeError::config(
                "home",
                format!("names no directory: {home}"),
            ));
        }
    }
    Ok(Settings {
        isolation,
        asked: asked.is_some(),
        home,
    })
}
