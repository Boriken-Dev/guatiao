// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Finding libraries in a directory **without running any of them**.
//!
//! # Why this reads bytes instead of loading
//!
//! Whether a file is a guatiao library is decided by one question: does it
//! export [`ENTRY_SYMBOL`]? The obvious way to ask is to map it and look
//! the symbol up — and that is the thing this module exists to avoid.
//!
//! **Mapping a shared library runs its static initialisers**, which is
//! arbitrary code executing before the caller gets control, and which can
//! and does abort the process. The directory beside an executable is full
//! of things that are not plugins: the application's own libraries, a UI
//! toolkit, a C runtime, whatever an installer dropped there. A scan that
//! maps all of them to find out runs all of their startup code.
//!
//! So the export table is read as **data**. The file is opened, its bytes
//! are parsed as PE, ELF or Mach-O, and the symbol names are compared.
//! Nothing in the file executes. Only a file that actually declares the
//! entry symbol is then mapped, by [`Registry::load_file`].
//!
//! That is what lets both rules hold at once: a candidate that is not a
//! library is **reported** rather than silently dropped, and nothing is
//! loaded speculatively.
//!
//! # What this deliberately does not do
//!
//! No name filter. Matching on a prefix once dropped every plugin on a
//! platform whose libraries are named `lib*`, and a filename is not
//! evidence of anything: the export table is.
//!
//! No recursion, and no environment variable. Which directories to search
//! is a host's policy, not a library's — [`scan_dir`] takes one directory
//! and answers for it.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use super::raw::ENTRY_SYMBOL;
use super::registry::{LoadError, Loading, Registry, Skipped};

/// What a scan found.
///
/// Three lists rather than one `Result`, because the three outcomes need
/// different handling: loaded is the answer, skipped is information a
/// person may want when a plugin they expected is missing, and failed is
/// something to report.
#[derive(Debug, Default)]
pub struct LoadReport {
    /// Files that loaded and registered at least one provider, in the
    /// order they were read.
    pub loaded: Vec<PathBuf>,
    /// Files passed over, and why. **Never an error**: a directory of
    /// libraries may hold anything.
    pub skipped: Vec<(PathBuf, Skipped)>,
    /// Files that looked like libraries and could not be used.
    pub failed: Vec<(PathBuf, LoadError)>,
}

/// Whether a file on disk declares the entry symbol, decided without
/// executing it.
///
/// `Ok(true)` means the symbol is in the export table. `Ok(false)` means
/// the file parsed and the symbol is not there. `Err` means the bytes
/// could not be read or parsed at all, which is neither — a text file with
/// a `.so` extension is not a broken library, it is not a library.
///
/// # What it costs
///
/// The whole file is read into memory. A shared library is measured in
/// megabytes and this happens once per candidate at start-up, which is
/// cheaper than the mapping it replaces.
pub fn declares_entry_symbol(path: &Path) -> Result<bool, std::io::Error> {
    use object::{Object, ObjectSymbol};

    let bytes = std::fs::read(path)?;
    let Ok(file) = object::File::parse(&*bytes) else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "not an object file this build can parse",
        ));
    };

    // The symbol as it is written in the export table, without the
    // trailing NUL the `dlsym` constant carries.
    let wanted = &ENTRY_SYMBOL[..ENTRY_SYMBOL.len() - 1];

    // `exports()` is the direct question and is what PE answers well. ELF
    // and Mach-O also answer through the dynamic symbol table, and a
    // stripped-but-dynamic library lists it there, so both are consulted.
    if let Ok(mut exports) = file.exports()
        && exports.any(|e| {
            // An export may be identified by ORDINAL rather than by name,
            // in which case there is nothing to compare and it is not the
            // symbol being looked for.
            e.is_ok_and(|e| e.name().into_name() == Some(wanted))
        })
    {
        return Ok(true);
    }
    Ok(file
        .dynamic_symbols()
        .any(|s| s.name_bytes().is_ok_and(|n| n == wanted)))
}

/// The order a scan visits file names in.
///
/// It decides **which build wins** when a host's library template gives
/// two files one key: the first to claim it keeps it, so the order is the
/// policy. Nothing here parses a version — ordering one is a host's job,
/// with the semver library it already has — but a directory whose names
/// carry versions sorts usefully on the bytes alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Order {
    /// By name, ascending. The default, and what two runs of a scan must
    /// agree on for a report to be reproducible.
    #[default]
    Ascending,
    /// By name, descending — so `libfoo-1.10.0` is offered before
    /// `libfoo-1.2.0` and, under a `%id` library key, is the one that
    /// loads.
    ///
    /// **Byte order, not version order.** It agrees with semver only while
    /// the names agree with it; `1.10.0` sorts above `1.2.0` here because
    /// `1` is above `2` at the fourth byte, which is luck rather than
    /// arithmetic. A host that needs real ordering scans with a library
    /// key of `%id@%version`, loads every build, and picks.
    Descending,
}

/// Loads every guatiao library in one directory, and reports the rest.
///
/// Visits names ascending. [`scan_dir_ordered`] takes the order.
pub fn scan_dir(registry: &mut Registry, dir: &Path) -> Result<LoadReport, std::io::Error> {
    scan_dir_ordered(registry, dir, Order::Ascending)
}

/// The same, in a stated order.
///
/// Not recursive, and it opens nothing that has not already been shown to
/// declare the entry symbol. See this module's header for why that matters
/// more than it sounds like it should.
///
/// Files are visited in a **sorted** order whichever is asked for, so two
/// runs on the same directory report the same thing and the same build
/// wins each time.
pub fn scan_dir_ordered(
    registry: &mut Registry,
    dir: &Path,
    order: Order,
) -> Result<LoadReport, std::io::Error> {
    let mut report = LoadReport::default();

    let mut candidates: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            // The platform's own extension and nothing else. A file
            // without it cannot be mapped on this platform whatever it
            // contains, so it is not a candidate rather than a skip.
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case(std::env::consts::DLL_EXTENSION))
        })
        .collect();
    candidates.sort();
    if order == Order::Descending {
        candidates.reverse();
    }

    for path in candidates {
        match declares_entry_symbol(&path) {
            Ok(true) => {}
            Ok(false) => {
                report.skipped.push((path, Skipped::NoEntrySymbol));
                continue;
            }
            Err(_) => {
                // Unreadable or unparseable. Reported, because a person
                // looking for a plugin that did not appear needs to see
                // the file was considered.
                report.skipped.push((path, Skipped::NotExaminable));
                continue;
            }
        }

        match registry.load_file(&path) {
            Ok(Loading::Loaded(_)) => report.loaded.push(path),
            Ok(Loading::Skipped(why)) => report.skipped.push((path, why)),
            Err(e) => report.failed.push((path, e)),
        }
    }
    Ok(report)
}
