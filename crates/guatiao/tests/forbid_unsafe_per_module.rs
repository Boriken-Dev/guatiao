// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `unsafe` lives under `src/value/` and nowhere else, and every other
//! module is inside the scope of an attribute that says so.
//!
//! # Why per-module rather than crate-level
//!
//! `#![forbid(unsafe_code)]` at the crate root would be the strongest
//! statement, and this crate cannot make it: the value module writes raw
//! pointers by definition. The attribute is therefore applied per module,
//! so everything layered over the value model stays *provably* safe and an
//! auditor's scope is one directory rather than the whole crate.
//!
//! `deny` plus an `allow` on that directory was the alternative and is
//! weaker: `deny` is overridable from an enclosing scope in a way `forbid`
//! is not, so the property would hold only until somebody wrote an
//! `#[allow]`.
//!
//! # Every exempt path is named ONCE, here
//!
//! [`UNSAFE_PATHS`] is the list, and it is the only place any of those
//! paths is written down. A module that moves is a one-line edit here
//! rather than every file in it failing both halves at once, which is a
//! test going red for a rename instead of for a defect.
//!
//! # What this test actually checks
//!
//! Two halves, and both matter:
//!
//! 1. Every module outside `src/value/` is *covered* by
//!    `#![forbid(unsafe_code)]` — either its own file carries it, or the
//!    `mod.rs` beside it does, since the attribute binds child modules
//!    too. A new module that simply forgets the attribute and has no
//!    covering parent is the realistic failure, and nothing in a build
//!    would mention it.
//! 2. No module outside `src/value/` contains the token `unsafe` at all.
//!    That is redundant with (1) — `forbid` would reject it — but it
//!    catches the case where (1) has been silently removed in the same
//!    edit, which is exactly how a per-module attribute erodes.

use std::fs;
use std::path::{Path, PathBuf};

/// Every path under `src/` allowed to contain `unsafe`, relative to the
/// crate root. A directory covers everything beneath it; a file covers
/// only itself.
///
/// **A file by default; a directory only where every file in it is
/// boundary code by construction.**
///
/// Files, because `value/` as a whole is mostly provably safe: naming the
/// directory would exempt `value/merge/` and `value/read.rs`, which write
/// no raw pointers at all, purely for sitting next to code that does.
///
/// Two directories earn the coarser entry. `exports` is the `extern "C"`
/// surface, where every function takes pointers from a caller this crate
/// cannot see. `value/types` holds one file per type, each carrying that
/// type's impl — and in an FFI crate those impls write raw pointers, so
/// the exemption would have to be granted file by file anyway. Naming the
/// directory keeps an auditor's scope one directory, which is the whole
/// property this list is defending; seven file entries would be the list
/// nobody reads.
const UNSAFE_PATHS: &[&str] = &[
    // The value model.
    "value/alloc.rs",
    "value/convert.rs",
    "value/mutate.rs",
    "value/raw.rs",
    "value/types",
    "value/value.rs",
    // The loader, which maps a library and calls a symbol out of it.
    "library/raw.rs",
    // The `extern "C"` surface.
    "exports",
];

/// Modules exempt from **carrying** the attribute, though still checked
/// for the token.
///
/// `forbid` binds child modules, which is the whole reason it is the right
/// attribute — and it means a parent cannot carry it when one of its
/// children is on [`UNSAFE_PATHS`]. `library/mod.rs` declares `raw`, so
/// putting the attribute there would forbid the very code the exemption
/// exists for.
///
/// The waiver is narrow on purpose: these files are still scanned for the
/// token, so the module that declares an exempt child cannot quietly
/// become one itself.
const NO_FORBID: &[&str] = &[
    "library/mod.rs",
    "value/mod.rs",
    // Declares children that carry `unsafe`.
    "value/types/mod.rs",
];

#[test]
fn unsafe_is_confined_to_one_module() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let exempt: Vec<PathBuf> = UNSAFE_PATHS
        .iter()
        .map(|p| src.join(p.replace('/', std::path::MAIN_SEPARATOR_STR)))
        .collect();
    let no_forbid: Vec<PathBuf> = NO_FORBID
        .iter()
        .map(|p| src.join(p.replace('/', std::path::MAIN_SEPARATOR_STR)))
        .collect();

    let mut missing_forbid = Vec::new();
    let mut stray_unsafe = Vec::new();
    let mut safe_modules = 0usize;

    for file in rust_files(&src) {
        if exempt.iter().any(|e| file.starts_with(e)) {
            continue;
        }
        let text =
            fs::read_to_string(&file).expect("a source file this crate ships must be readable");

        // A module that DECLARES an exempt one cannot carry the
        // attribute, because `forbid` binds child modules -- which is the
        // whole reason it is the right attribute. `lib.rs` is one such
        // module and `library/mod.rs` is the other. Both are still checked
        // for the absence of the token.
        let is_root = file.file_name().is_some_and(|n| n == "lib.rs");
        let declares_an_exempt_child = no_forbid.contains(&file);
        if !is_root && !declares_an_exempt_child {
            safe_modules += 1;
            if !covered_by_forbid(&file, &text) {
                missing_forbid.push(file.display().to_string());
            }
        }

        for (n, line) in text.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("//") || trimmed.starts_with("*") {
                continue;
            }
            // The attribute itself contains the word.
            if trimmed.contains("forbid(unsafe_code)") {
                continue;
            }
            if line.contains("unsafe") {
                stray_unsafe.push(format!("{}:{}: {}", file.display(), n + 1, trimmed));
            }
        }
    }

    assert!(
        missing_forbid.is_empty(),
        "every module outside src/value/ must carry #![forbid(unsafe_code)] — the model half of \
         this crate being provably safe is one of the reasons it is adoptable, and the \
         attribute is per-module precisely so a new module cannot inherit an exemption. \
         Missing in:\n{}",
        missing_forbid.join("\n")
    );

    assert!(
        stray_unsafe.is_empty(),
        "`unsafe` appears outside src/value/. If a change seems to need it there, it belongs in \
         the value module instead. Found:\n{}",
        stray_unsafe.join("\n")
    );

    assert!(
        safe_modules >= 6,
        "expected at least the six ported model modules to be scanned, saw {safe_modules} — \
         the scan is looking in the wrong place, which would make this test pass by seeing \
         nothing"
    );
}

/// Whether `file` is inside the scope of a `#![forbid(unsafe_code)]`.
///
/// Its own file counts, and so does the `mod.rs` beside it: an inner
/// attribute binds the module it heads **and every module declared within
/// it**, so `merge/tests.rs` is covered by `merge/mod.rs` without carrying
/// the line itself. Requiring the attribute on every file regardless would
/// be a rule about text rather than about scope, and the first thing it
/// would reject is a correct file.
///
/// Only one level up is consulted, which is all this crate's layout needs
/// and is the shape a reader can check by eye.
fn covered_by_forbid(file: &Path, text: &str) -> bool {
    if declares_forbid(text) {
        return true;
    }
    let Some(dir) = file.parent() else {
        return false;
    };
    let parent_mod = dir.join("mod.rs");
    if parent_mod == file {
        return false;
    }
    fs::read_to_string(parent_mod).is_ok_and(|t| declares_forbid(&t))
}

/// Whether `text` carries the attribute as an ATTRIBUTE.
///
/// Line by line, skipping comments, because a substring search over the
/// whole file counts a mention in prose — and that is not hypothetical:
/// `value/mod.rs` explains the per-module arrangement in its own header
/// and names the attribute while doing it, which made every file in that
/// directory read as covered. The hole was invisible while the directory
/// was exempt anyway, and surfaced the moment the exemption was narrowed
/// to the four files that actually need it.
fn declares_forbid(text: &str) -> bool {
    const ATTR: &str = "#![forbid(unsafe_code)]";
    text.lines().any(|line| {
        let t = line.trim();
        !t.starts_with("//") && t.starts_with(ATTR)
    })
}

/// Every `.rs` file under `dir`, recursively.
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];

    while let Some(d) = stack.pop() {
        let entries = match fs::read_dir(&d) {
            Ok(e) => e,
            Err(_) if d != dir => continue,
            Err(e) => panic!("cannot read {}: {e}", d.display()),
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    assert!(
        !out.is_empty(),
        "found no .rs files under {}",
        dir.display()
    );
    out.sort();
    out
}
