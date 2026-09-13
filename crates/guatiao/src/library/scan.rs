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
//! and answers for it, and [`scan_path`] takes the list a host assembled
//! from wherever its policy says.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use super::raw::{DECLARES_SYMBOL, ENTRY_SYMBOL};
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
    /// Entries of a search path that could not be read at all: a
    /// directory that does not exist, or one this process may not list.
    /// Reported rather than dropped, for the same reason a skip is — a
    /// person looking for a plugin that did not appear needs to see that
    /// the place was considered. Empty for a single-directory scan, which
    /// answers that with its `Err` instead.
    pub unreadable: Vec<(PathBuf, std::io::Error)>,
}

impl LoadReport {
    /// Whether anything at all loaded.
    pub fn is_empty(&self) -> bool {
        self.loaded.is_empty()
    }

    /// Takes everything `other` found onto the end of this report.
    pub fn absorb(&mut self, other: LoadReport) {
        self.loaded.extend(other.loaded);
        self.skipped.extend(other.skipped);
        self.failed.extend(other.failed);
        self.unreadable.extend(other.unreadable);
    }
}

/// Where a host looks: directories or files, in order, each once.
///
/// A host's search path is routinely the same place twice — an
/// environment variable pointing at the directory beside the executable,
/// which the host also adds by itself — so an entry is kept by the place
/// it names and the first spelling wins. [`SearchPath::parse`] splits on
/// the platform's own list separator (`;` on Windows, where a path carries
/// a `:` of its own; `:` elsewhere) and drops empty parts, so a trailing
/// or doubled separator costs nothing.
///
/// Accumulating is the host's choice: parse what the environment says,
/// then [`push`](SearchPath::push) the place adjacency finds, and the
/// override is added to the default rather than replacing it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchPath {
    entries: Vec<PathBuf>,
}

impl SearchPath {
    /// The character between entries: `;` on Windows, `:` elsewhere.
    pub const SEPARATOR: char = if cfg!(windows) { ';' } else { ':' };

    /// No entries.
    pub fn new() -> SearchPath {
        SearchPath::default()
    }

    /// Splits `spec` on [`SEPARATOR`](SearchPath::SEPARATOR), trimming
    /// each part and dropping the empty ones.
    pub fn parse(spec: &str) -> SearchPath {
        let mut path = SearchPath::new();
        for part in spec.split(Self::SEPARATOR) {
            let part = part.trim();
            if !part.is_empty() {
                path.push(part);
            }
        }
        path
    }

    /// Appends `entry`, unless an earlier entry names the same place.
    ///
    /// Sameness is by canonical path when the entry exists — so a
    /// relative spelling and an absolute one of one directory are one
    /// entry — and by the text as written when it does not.
    pub fn push(&mut self, entry: impl Into<PathBuf>) {
        let entry = entry.into();
        let place = same_place(&entry);
        if self.entries.iter().any(|e| same_place(e) == place) {
            return;
        }
        self.entries.push(entry);
    }

    /// [`push`](SearchPath::push), for building one in an expression.
    pub fn with(mut self, entry: impl Into<PathBuf>) -> SearchPath {
        self.push(entry);
        self
    }

    /// The entries, in the order given, each once.
    pub fn entries(&self) -> &[PathBuf] {
        &self.entries
    }

    /// Whether there is nowhere to look.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// A path resolved for comparison: canonical when it exists, as written
/// when it does not. A place that does not exist cannot be the same
/// place as one that does.
fn same_place(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The binary inside an Apple bundle, or `None` for anything else.
///
/// `Name.framework` is a DIRECTORY, and the library is the extension-less
/// file `Name` inside it — flat on iOS, and on macOS behind
/// `Versions/Current/`, which this follows when the flat file is absent.
/// A scan that took only files would walk straight past every framework,
/// and a search path naming one would have nothing to open.
///
/// Pure path logic, so it answers on every platform and is testable on
/// all of them; whether the binary it names can be mapped is the
/// loader's business.
pub fn bundle_binary(path: &Path) -> Option<PathBuf> {
    if !path.is_dir()
        || !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("framework"))
    {
        return None;
    }
    let name = path.file_stem()?;
    let flat = path.join(name);
    if flat.is_file() {
        return Some(flat);
    }
    let versioned = path.join("Versions").join("Current").join(name);
    versioned.is_file().then_some(versioned)
}

/// What in a directory is worth probing: a file carrying the platform's
/// own library extension, or a bundle whose binary is inside it.
///
/// The extension and nothing else. A file without it cannot be mapped on
/// this platform whatever it contains, so it is not a candidate rather
/// than a skip — and that one rule rejects a build's byproducts (`.pdb`,
/// `.lib`, `.exp`, `.d`) and a library somebody disabled by renaming it,
/// with no list to maintain.
fn candidate(path: PathBuf) -> Option<PathBuf> {
    if path.is_file() {
        return has_library_extension(&path).then_some(path);
    }
    bundle_binary(&path)
}

fn has_library_extension(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(std::env::consts::DLL_EXTENSION))
}

/// Whether a file on disk declares the entry symbol, decided without
/// executing it.
///
/// `Ok(true)` means the symbol is in the export table. `Ok(false)` means
/// the file parsed and the symbol is not there. `Err` means the bytes
/// could not be read or parsed at all, which is neither — a text file with
/// a `.so` extension is not a broken library, it is not a library.
///
/// [`probe`] answers this and what the library declares in one read.
pub fn declares_entry_symbol(path: &Path) -> Result<bool, std::io::Error> {
    probe(path).map(|p| p.entry)
}

/// The most bytes a declaration may span. A declaration is a handful of
/// short strings; a file claiming more is refused rather than read.
const DECLARES_WINDOW: usize = 4096;

/// What one file says about itself, read as data.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Probe {
    /// Whether it exports the entry symbol: whether it is a library.
    pub entry: bool,
    /// What it declares, empty when it declares nothing.
    pub declared: Declared,
}

/// A library's declarations: `key=value` pairs, in the order written.
///
/// Empty for a library that declares nothing, which a plain scan loads
/// as before. What a rule matches against before anything is mapped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Declared {
    pairs: Vec<(String, String)>,
}

impl Declared {
    /// Every pair, in the order written.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.pairs.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Whether `key=value` was declared.
    pub fn has(&self, key: &str, value: &str) -> bool {
        self.pairs.iter().any(|(k, v)| k == key && v == value)
    }

    /// Every value declared under `key`.
    pub fn values<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.pairs
            .iter()
            .filter(move |(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Every kind declared: the values of `kind`.
    pub fn kinds(&self) -> impl Iterator<Item = &str> {
        self.values("kind")
    }

    /// Whether nothing was declared.
    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    /// Parses the bytes of a declaration: `key=value` strings separated
    /// by NUL and ended by an empty string. `None` when the terminator is
    /// missing from the window, which is a malformed declaration.
    ///
    /// Public so a host can check its rules against a declaration it
    /// wrote by hand, with no file involved.
    pub fn parse(bytes: &[u8]) -> Option<Declared> {
        let mut pairs = Vec::new();
        let mut rest = bytes;
        loop {
            let end = rest.iter().position(|&b| b == 0)?;
            if end == 0 {
                return Some(Declared { pairs });
            }
            // A pair that is not UTF-8 is dropped; the rest still count.
            if let Ok(text) = std::str::from_utf8(&rest[..end]) {
                let (key, value) = text.split_once('=').unwrap_or((text, ""));
                let pair = (key.to_string(), value.to_string());
                if !pairs.contains(&pair) {
                    pairs.push(pair);
                }
            }
            rest = &rest[end + 1..];
        }
    }
}

/// Reads what a file says about itself, without executing it.
///
/// `Ok` means the file parsed as PE, ELF or Mach-O; `entry` says whether
/// it is a library and `declared` what it declares. `Err` means the bytes
/// could not be read or parsed, or a declaration was present and
/// malformed — a file that is not examinable.
///
/// # What it costs
///
/// The whole file is read into memory. A shared library is measured in
/// megabytes and this happens once per candidate at start-up, which is
/// cheaper than the mapping it replaces. The declaration is read from
/// the section it lives in, clamped to that section and to
/// 4 KiB: the bytes are an untrusted file's.
pub fn probe(path: &Path) -> Result<Probe, std::io::Error> {
    use object::{Object, ObjectSection, ObjectSymbol};

    let bytes = std::fs::read(path)?;
    let Ok(file) = object::File::parse(&*bytes) else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "not an object file this build can parse",
        ));
    };

    // The symbols as they are written in the export table, without the
    // trailing NUL the `dlsym` constants carry.
    let entry = &ENTRY_SYMBOL[..ENTRY_SYMBOL.len() - 1];
    let declares = &DECLARES_SYMBOL[..DECLARES_SYMBOL.len() - 1];

    // `exports()` is the direct question and is what PE answers well. ELF
    // and Mach-O also answer through the dynamic symbol table, and a
    // stripped-but-dynamic library lists it there, so both are consulted.
    // An export may be identified by ORDINAL rather than by name, in which
    // case there is nothing to compare.
    let mut has_entry = false;
    let mut declares_at: Option<u64> = None;
    if let Ok(exports) = file.exports() {
        for export in exports.flatten() {
            let Some(name) = export.name().into_name() else {
                continue;
            };
            if name == entry {
                has_entry = true;
            } else if name == declares
                && let object::ExportTarget::Address { address } = export.target()
            {
                declares_at = Some(address);
            }
        }
    }
    for symbol in file.dynamic_symbols() {
        if let Ok(name) = symbol.name_bytes() {
            if name == entry {
                has_entry = true;
            } else if name == declares && declares_at.is_none() && symbol.address() != 0 {
                declares_at = Some(symbol.address());
            }
        }
    }

    let mut declared = Declared::default();
    if let Some(address) = declares_at {
        // The section the address falls in, and the bytes from there to
        // the smaller of the section's end and the window.
        let section = file
            .sections()
            .find(|s| address >= s.address() && address < s.address().saturating_add(s.size()));
        let window = section.and_then(|s| {
            let offset = usize::try_from(address - s.address()).ok()?;
            let data = s.data().ok()?;
            let end = offset.saturating_add(DECLARES_WINDOW).min(data.len());
            data.get(offset..end)
        });
        match window.and_then(Declared::parse) {
            Some(found) => declared = found,
            None => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "the declaration is not readable as data or is not terminated",
                ));
            }
        }
    }
    Ok(Probe {
        entry: has_entry,
        declared,
    })
}

/// Rules a host applies to what a library declares, before mapping it.
///
/// Each rule is `KEY=VALUE` or `!KEY=VALUE`. A positive rule skips a file
/// that does NOT declare the pair; a negative one skips a file that does.
/// A file declaring nothing passes every negative rule and fails every
/// positive one. The rule that skipped a file is what
/// [`Skipped::Filtered`] names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanRules {
    rules: Vec<(bool, String, String, String)>,
}

/// A rule that is not `[!]KEY=VALUE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleError {
    /// The rule as written.
    pub rule: String,
}

impl std::fmt::Display for RuleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "`{}` is not a scan rule; a rule is `KEY=VALUE` to require a declaration or \
             `!KEY=VALUE` to skip one",
            self.rule
        )
    }
}

impl std::error::Error for RuleError {}

impl ScanRules {
    /// Parses each rule, refusing the first that is not `[!]KEY=VALUE`.
    pub fn parse<S: AsRef<str>>(rules: &[S]) -> Result<ScanRules, RuleError> {
        let mut parsed = Vec::with_capacity(rules.len());
        for rule in rules {
            let text = rule.as_ref().trim();
            if text.is_empty() {
                continue;
            }
            let (require, body) = match text.strip_prefix('!') {
                Some(body) => (false, body),
                None => (true, text),
            };
            let Some((key, value)) = body.split_once('=') else {
                return Err(RuleError {
                    rule: text.to_string(),
                });
            };
            if key.is_empty() || key.contains('!') {
                return Err(RuleError {
                    rule: text.to_string(),
                });
            }
            parsed.push((
                require,
                key.to_string(),
                value.to_string(),
                text.to_string(),
            ));
        }
        Ok(ScanRules { rules: parsed })
    }

    /// `Ok` when every rule allows `declared`, else the rule that did not.
    pub fn check(&self, declared: &Declared) -> Result<(), &str> {
        for (require, key, value, text) in &self.rules {
            if declared.has(key, value) != *require {
                return Err(text);
            }
        }
        Ok(())
    }

    /// Whether any rule was given.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
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
    scan_dir_with(registry, dir, order, |_| Ok(()))
}

/// The same, applying `rules` to what each library declares before it is
/// mapped. A file a rule refuses is reported as
/// [`Skipped::Filtered`] naming the rule.
pub fn scan_dir_rules(
    registry: &mut Registry,
    dir: &Path,
    order: Order,
    rules: &ScanRules,
) -> Result<LoadReport, std::io::Error> {
    scan_dir_with(registry, dir, order, |declared| {
        rules.check(declared).map_err(str::to_string)
    })
}

/// The same, with a filter of the host's own: called with what each
/// library declares, **before it is mapped**; `Err(why)` skips it as
/// [`Skipped::Filtered`] naming `why`.
pub fn scan_dir_with(
    registry: &mut Registry,
    dir: &Path,
    order: Order,
    mut filter: impl FnMut(&Declared) -> Result<(), String>,
) -> Result<LoadReport, std::io::Error> {
    let mut report = LoadReport::default();

    let mut candidates: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter_map(candidate)
        .collect();
    candidates.sort();
    if order == Order::Descending {
        candidates.reverse();
    }

    for path in candidates {
        consider(registry, path, &mut report, &mut filter);
    }
    Ok(report)
}

/// Probes one candidate, applies the filter, and loads it — recording the
/// outcome, whichever it is, on `report`.
fn consider(
    registry: &mut Registry,
    path: PathBuf,
    report: &mut LoadReport,
    filter: &mut impl FnMut(&Declared) -> Result<(), String>,
) {
    match probe(&path) {
        Ok(Probe {
            entry: true,
            declared,
        }) => {
            if let Err(by) = filter(&declared) {
                report.skipped.push((path, Skipped::Filtered { by }));
                return;
            }
        }
        Ok(Probe { entry: false, .. }) => {
            report.skipped.push((path, Skipped::NoEntrySymbol));
            return;
        }
        Err(_) => {
            // Unreadable, unparseable, or a malformed declaration.
            // Reported, because a person looking for a plugin that
            // did not appear needs to see the file was considered.
            report.skipped.push((path, Skipped::NotExaminable));
            return;
        }
    }

    match registry.load_file(&path) {
        Ok(Loading::Loaded(_)) => report.loaded.push(path),
        Ok(Loading::Skipped(why)) => report.skipped.push((path, why)),
        Err(e) => report.failed.push((path, e)),
    }
}

/// Loads every library along a search path, and reports the rest.
///
/// A directory entry is scanned as [`scan_dir_rules`] is, in `order`. A
/// file entry is probed and loaded on its own, under the same rules and
/// with the same reporting: naming a file is asking for it, so its
/// extension is not checked, but what it declares still is. A bundle is
/// its binary (see [`bundle_binary`]). An entry that cannot be read at
/// all — missing, or not listable — goes under [`LoadReport::unreadable`]
/// and the rest of the path is still walked, because a search path
/// routinely names a place that does not exist on this machine.
///
/// One report for the whole path, in the order the entries were given. A
/// library reachable twice loads once and is [`Skipped::AlreadyLoaded`]
/// the second time, naming the first — which is what a host wants to see
/// when an override points at the directory adjacency already found.
pub fn scan_path(
    registry: &mut Registry,
    path: &SearchPath,
    order: Order,
    rules: &ScanRules,
) -> LoadReport {
    let mut report = LoadReport::default();
    for entry in path.entries() {
        let entry = bundle_binary(entry).unwrap_or_else(|| entry.clone());
        if entry.is_dir() {
            match scan_dir_rules(registry, &entry, order, rules) {
                Ok(found) => report.absorb(found),
                Err(e) => report.unreadable.push((entry, e)),
            }
        } else if entry.is_file() {
            consider(registry, entry, &mut report, &mut |declared| {
                rules.check(declared).map_err(str::to_string)
            });
        } else {
            report
                .unreadable
                .push((entry, std::io::Error::from(std::io::ErrorKind::NotFound)));
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh directory under the target dir, for one test.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("guatiao-scan-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    #[test]
    fn a_search_path_splits_on_the_platform_separator_and_keeps_each_place_once() {
        let sep = SearchPath::SEPARATOR;
        let dir = scratch("split");
        let a = dir.join("a");
        let b = dir.join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        // A doubled separator, a blank part, and `a` spelled twice --
        // once through its own parent, which canonicalises to the same
        // place.
        let spec = format!(
            "{}{sep}{sep} {sep}{}{sep}{}",
            a.display(),
            b.display(),
            dir.join("b").join("..").join("a").display()
        );
        let path = SearchPath::parse(&spec);
        assert_eq!(path.entries(), [a.clone(), b.clone()]);

        // A place that does not exist is kept as written, and once.
        let path = path.with(dir.join("nowhere")).with(dir.join("nowhere"));
        assert_eq!(path.entries().len(), 3);
        assert!(SearchPath::parse("").is_empty());
        assert!(SearchPath::parse(&format!("{sep}{sep}")).is_empty());
    }

    #[test]
    fn a_bundle_resolves_to_the_binary_inside_it() {
        let dir = scratch("bundle");
        let flat = dir.join("Flat.framework");
        std::fs::create_dir_all(&flat).unwrap();
        std::fs::write(flat.join("Flat"), b"").unwrap();
        assert_eq!(bundle_binary(&flat), Some(flat.join("Flat")));

        let versioned = dir.join("Deep.framework");
        std::fs::create_dir_all(versioned.join("Versions").join("Current")).unwrap();
        std::fs::write(versioned.join("Versions").join("Current").join("Deep"), b"").unwrap();
        assert_eq!(
            bundle_binary(&versioned),
            Some(versioned.join("Versions").join("Current").join("Deep"))
        );

        // A bundle with no binary, a directory that is not a bundle, and
        // a plain file spelled like one: none is a bundle.
        let empty = dir.join("Empty.framework");
        std::fs::create_dir_all(&empty).unwrap();
        assert_eq!(bundle_binary(&empty), None);
        assert_eq!(bundle_binary(&dir), None);
        let file = dir.join("File.framework");
        std::fs::write(&file, b"").unwrap();
        assert_eq!(bundle_binary(&file), None);
        assert_eq!(bundle_binary(&dir.join("missing.framework")), None);
    }

    #[test]
    fn a_declaration_is_pairs_ended_by_an_empty_string() {
        let d = Declared::parse(b"kind=greeter\0kind=writer\0VIEWER=1\0\0trailing")
            .expect("terminated");
        assert_eq!(d.kinds().collect::<Vec<_>>(), ["greeter", "writer"]);
        assert!(d.has("VIEWER", "1"));
        assert!(!d.has("VIEWER", "0"));
        assert_eq!(d.iter().count(), 3, "nothing past the terminator counts");
    }

    #[test]
    fn a_declaration_without_its_terminator_is_refused() {
        assert_eq!(Declared::parse(b"kind=greeter\0kind=wr"), None);
        assert_eq!(Declared::parse(b""), None);
        assert!(Declared::parse(b"\0").expect("empty").is_empty());
    }

    #[test]
    fn a_pair_that_is_not_utf8_is_dropped_and_a_repeat_counted_once() {
        let d = Declared::parse(b"kind=a\0\xff\xfe=x\0kind=a\0\0").expect("terminated");
        assert_eq!(d.kinds().collect::<Vec<_>>(), ["a"]);
    }

    #[test]
    fn rules_are_key_value_with_an_optional_bang() {
        let rules =
            ScanRules::parse(&["!VIEWER=1", " kind=session-backend ", ""]).expect("two rules");
        let backend = Declared::parse(b"kind=session-backend\0\0").unwrap();
        let viewer = Declared::parse(b"kind=session-backend\0VIEWER=1\0\0").unwrap();
        let silent = Declared::default();
        assert_eq!(rules.check(&backend), Ok(()));
        assert_eq!(rules.check(&viewer), Err("!VIEWER=1"));
        assert_eq!(
            rules.check(&silent),
            Err("kind=session-backend"),
            "a library declaring nothing passes every `!` rule and fails every positive one"
        );
        assert!(ScanRules::parse::<&str>(&[]).unwrap().is_empty());
        for bad in ["nonsense", "=x", "!", "a!b=c"] {
            let e = ScanRules::parse(&[bad]).unwrap_err();
            assert_eq!(e.rule, bad);
            assert!(e.to_string().contains(bad), "{e}");
        }
    }
}
