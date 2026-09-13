// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! This crate holds no process-global mutable state, and that is what lets
//! a host and a library each link their own copy of it.
//!
//! # Why this is a test rather than a convention
//!
//! Every other rule in this crate about crossing a boundary is enforced by
//! a type. This one cannot be: a `static mut`, a `OnceLock` or a
//! `thread_local!` compiles perfectly well and breaks nothing until two
//! artifacts in one process each have their own copy of it. Then a value
//! interned by one is invisible to the other, and the symptom is not a
//! crash — it is a lookup that answers "not found" for something that was
//! definitely registered.
//!
//! The container this crate replaces had exactly that shape: an interning
//! table behind an opaque handle, which forced the whole "only one artifact
//! may export these symbols" rule, which in turn forced a second crate to
//! exist purely to bind those symbols. Deleting the state is what deletes
//! all three.
//!
//! # What is allowed
//!
//! `const` items, and `static` items whose type is immutable and whose
//! value is a compile-time constant — a `static NAME: &str = "..."` is a
//! constant in a different spelling and cannot make two linkages disagree.
//! What is forbidden is anything that can be *written* after start: `static
//! mut`, interior mutability behind a `static`, and the lazy-initialisation
//! wrappers.
//!
//! A library may of course hold whatever state it likes; the rule binds this
//! crate only. See the library module for why that asymmetry is sound.

use std::fs;
use std::path::{Path, PathBuf};

/// Spellings that introduce per-linkage mutable state.
///
/// Substring matching over source text is crude, and deliberately so: this
/// test's job is to make the rule *visible* at the moment somebody reaches
/// for one of these, not to prove a theorem about the crate. A false
/// positive costs one `#[allow]`-style exemption line below and a moment's
/// thought, which is the outcome this is for.
const FORBIDDEN: &[&str] = &[
    "static mut ",
    "OnceLock",
    "OnceCell",
    "LazyLock",
    "LazyCell",
    "lazy_static",
    "thread_local!",
    "AtomicUsize",
    "AtomicU32",
    "AtomicU64",
    "AtomicBool",
    "AtomicPtr",
];

/// Spellings that are interior mutability, and are forbidden only in a
/// `static`.
///
/// A `Mutex` inside a value the host owns is that value's own state and
/// is fine; the same `Mutex` behind a `static` is a table two linkages
/// hold two copies of. So these are checked on `static` declarations
/// alone, where the spellings in [`FORBIDDEN`] are checked everywhere
/// because they have no other use.
const FORBIDDEN_IN_STATICS: &[&str] = &[
    "Mutex",
    "RwLock",
    "RefCell",
    "Cell<",
    "UnsafeCell",
    "Lazy",
    "Atomic",
];

/// Lines exempt from the scan, by exact trimmed text.
///
/// Keyed on the whole line rather than on a file, so an exemption cannot
/// quietly widen to cover a second use that lands in the same file later.
/// Every entry needs a comment saying why.
const EXEMPT: &[&str] = &[
    // The body of `guatiao::providers!`: the `static` it names lands in the
    // LIBRARY AUTHOR's crate, which may hold whatever state it likes. The
    // scan reads the macro's text, not where it expands.
    "static REGISTERED: ::std::sync::OnceLock<::core::option::Option<$crate::library::kind::LibraryParts>> = ::std::sync::OnceLock::new();",
];

#[test]
fn the_crate_holds_no_process_global_mutable_state() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut hits = Vec::new();

    for file in rust_files(&src) {
        let text =
            fs::read_to_string(&file).expect("a source file this crate ships must be readable");
        for (n, line) in text.lines().enumerate() {
            let trimmed = line.trim();
            // A mention inside a comment is documentation, not state. This
            // is the one place the crude matching needs help: the rule is
            // worth explaining in prose, and explaining it must not break
            // the test that enforces it.
            if trimmed.starts_with("//") || trimmed.starts_with("*") {
                continue;
            }
            if EXEMPT.contains(&trimmed) {
                continue;
            }
            for bad in FORBIDDEN {
                if line.contains(bad) {
                    hits.push(format!(
                        "{}:{}: {} — {}",
                        file.display(),
                        n + 1,
                        bad.trim(),
                        trimmed
                    ));
                }
            }
            // A `static` whose declared type is interior-mutable. The
            // declaration line carries the type in every case this crate
            // has, so one line is what is examined.
            let is_static = trimmed.starts_with("static ")
                || trimmed.starts_with("pub static ")
                || trimmed.starts_with("pub(crate) static ");
            if is_static {
                for bad in FORBIDDEN_IN_STATICS {
                    if trimmed.contains(bad) {
                        hits.push(format!(
                            "{}:{}: a static holding {} — {}",
                            file.display(),
                            n + 1,
                            bad.trim_end_matches('<'),
                            trimmed
                        ));
                    }
                }
            }
        }
    }

    assert!(
        hits.is_empty(),
        "this crate must hold no process-global mutable state, because a host and a library \
         each link their own copy of it and two copies of a `static` are two different \
         values. Found:\n{}",
        hits.join("\n")
    );
}

/// Every `.rs` file under `dir`, recursively.
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];

    while let Some(d) = stack.pop() {
        let entries = match fs::read_dir(&d) {
            Ok(e) => e,
            // `src/` itself must exist; a subdirectory that vanished
            // mid-scan is not this test's problem to report.
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
        "found no .rs files under {} — the scan is looking in the wrong place, which would \
         make this test pass by seeing nothing",
        dir.display()
    );
    out.sort();
    out
}
