// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The interpreter, and the three calls the source makes into it.
//!
//! One interpreter for the program: initialised the first time a source is
//! added, joined when the host is itself Python, never finalised (CPython
//! cannot be initialised again reliably). Every call takes the GIL for its
//! duration and releases it after.

#![forbid(unsafe_code)]

use alloc::ffi::CString;
use std::sync::OnceLock;

use pyo3::prelude::*;
use pyo3::types::PyModule;

use crate::manifest::Manifest;

/// The Python half, loaded once into the interpreter.
static BRIDGE: OnceLock<Py<PyModule>> = OnceLock::new();

const SOURCE: &str = include_str!("bridge.py");

fn bridge(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    if let Some(module) = BRIDGE.get() {
        return Ok(module.bind(py).clone());
    }
    let code = CString::new(SOURCE).expect("the bridge's source holds no NUL");
    let module = PyModule::from_code(py, &code, c"guatiao_py_bridge.py", c"_guatiao_py_bridge")?;
    // A second thread racing here built a module of its own; the first kept wins.
    let kept = BRIDGE.get_or_init(|| module.clone().unbind());
    Ok(kept.bind(py).clone())
}

/// Starts or joins the interpreter, puts `paths` on `sys.path` and points
/// the binding at `own` (this file) unless the host chose a library.
pub(crate) fn start(paths: &[String], own: Option<&str>) -> Result<(), String> {
    Python::initialize();
    Python::attach(|py| -> PyResult<()> {
        bridge(py)?.getattr("setup")?.call1((paths.to_vec(), own))?;
        Ok(())
    })
    .map_err(|e| format!("Python could not start the bridge: {e}"))
}

/// Imports the library `manifest` describes: the address of its
/// descriptor, and the handle [`close`] takes.
pub(crate) fn open(manifest: &Manifest) -> Result<(usize, i64), String> {
    Python::attach(|py| -> PyResult<(usize, i64)> {
        bridge(py)?
            .getattr("open_library")?
            .call1((
                manifest.id.as_str(),
                manifest.kinds.clone(),
                manifest.entry.as_str(),
                manifest.paths.clone(),
            ))?
            .extract()
    })
    .map_err(|e| e.to_string())
}

/// Lets one opened library go. What Python raises is dropped: the registry
/// has already let it go, and there is nobody left to tell.
pub(crate) fn close(handle: i64) {
    let _ = Python::attach(|py| -> PyResult<()> {
        bridge(py)?.getattr("close_library")?.call1((handle,))?;
        Ok(())
    });
}
