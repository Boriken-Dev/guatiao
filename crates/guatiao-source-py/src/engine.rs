// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! CPython as a guatiao-bridge engine: a library is a module, its manifest
//! a `*.guatiao.toml` of type `python`, its tables built by the Python binding itself.

#![forbid(unsafe_code)]

use guatiao_bridge::{
    BridgeError, Config, Engine, Manifest, ManifestFormat, Opened, ThreadModel, text_list,
};

/// The one interpreter this file keeps for the program: started (or
/// joined, in a Python host) when the source is added, never finalised.
pub(crate) struct Python;

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

    /// Puts the configuration's `paths` first on `sys.path` and points the
    /// binding at this file's own exports.
    fn start(config: &Config) -> Result<Python, BridgeError> {
        let paths = text_list(config.map(), "paths")?;
        crate::bridge::start(&paths, crate::exports::own_path().as_deref())
            .map_err(BridgeError::Unavailable)?;
        Ok(Python)
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
}
