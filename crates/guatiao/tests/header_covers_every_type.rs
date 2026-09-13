// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Every `repr(C)` type this crate declares reaches the generated header.
//!
//! # The failure this exists for
//!
//! A header generator emits a type only when something it is already
//! emitting refers to it. A type that crosses as **data** rather than as a
//! function argument is referred to by nothing, so it is dropped —
//! silently, exit 0, no warning — and the header still compiles, because
//! what is missing is missing consistently.
//!
//! That is not hypothetical here. Every one of the schema types was
//! dropped exactly that way, and the header compiled cleanly without them:
//! a schema is data, so no signature names one. The hole would have
//! surfaced in somebody else's consumer, as a type that does not exist.
//!
//! Listing each type in the generator's config fixes it. This test is what
//! notices the next time somebody adds a type and forgets, which is the
//! same failure with a different name on it.
//!
//! # It also covers the rename table
//!
//! The types are named for Rust on the Rust side and for C on the C side,
//! and `cbindgen.toml`'s `[export.rename]` is the whole of that
//! translation. A type missing from the table reaches the header under its
//! Rust name, which is a name no C consumer expects and no prefix protects
//! — so this reads the table rather than hard-coding either spelling, and
//! a type that fell out of it fails here.

use std::collections::HashMap;
use std::path::PathBuf;

/// Every type the header introduces carries this crate's prefix.
///
/// The coverage test below asks whether a type is *mentioned*, which a
/// type cbindgen emitted under its bare Rust name satisfies — so it
/// passes while the header declares `Kinds` into a namespace it shares
/// with everything a consumer already includes. Both have happened:
/// `MaybeNull_Map` and `Kinds` each reached the committed header before
/// anything noticed.
///
/// This reads the header instead, so a missing `[export.rename]` entry
/// fails on the class rather than on the instance.
#[test]
fn every_type_the_header_declares_is_prefixed() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let header = std::fs::read_to_string(manifest.join("include/guatiao.h"))
        .expect("the header is committed");

    let mut names = Vec::new();
    for line in header.lines() {
        let line = line.trim();
        // `typedef struct X {`, and the `} X;` that closes an anonymous
        // one. Both spellings appear in what cbindgen emits.
        let named = line
            .strip_prefix("typedef struct ")
            .or_else(|| line.strip_prefix("typedef enum "))
            .or_else(|| line.strip_prefix("typedef union "))
            .and_then(|rest| rest.split_whitespace().next())
            .map(|n| n.trim_end_matches(';'))
            .or_else(|| {
                line.strip_prefix("} ")
                    .and_then(|rest| rest.strip_suffix(';'))
            });
        if let Some(name) = named
            && !name.is_empty()
            && name.chars().all(|c| c.is_alphanumeric() || c == '_')
        {
            names.push(name.to_string());
        }
    }

    assert!(
        names.len() >= 12,
        "found only {} declared types in the header, so this scan is not \
         reading it correctly",
        names.len()
    );

    let bare: Vec<_> = names
        .iter()
        .filter(|n| !n.starts_with("guatiao_") && !n.starts_with("GUATIAO_"))
        .collect();
    assert!(
        bare.is_empty(),
        "these types reach the header under a bare name and would collide \
         with whatever else a consumer includes: {bare:?}. Add each to \
         `[export.rename]` in cbindgen.toml and regenerate."
    );
}

#[test]
fn every_repr_c_type_is_in_the_generated_header() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let header = std::fs::read_to_string(manifest.join("include/guatiao.h"))
        .expect("the header is committed");

    // Every file under `src/`, RECURSIVELY, rather than a list to keep in
    // step. A fixed list rots in both directions: it misses a file
    // somebody adds, and it fails on one somebody removes — which is
    // exactly what happened the first time this ran.
    //
    // Recursive because a `repr(C)` type can live anywhere under `src/`:
    // the value model, the library envelope, and the type files under
    // `value/types/`. A type this walk cannot see is a type cbindgen
    // drops from the header with nothing red anywhere.
    let dir = manifest.join("src");
    let mut declared = Vec::new();
    for path in rust_files(&dir) {
        let text = std::fs::read_to_string(&path).expect("a source file is readable");
        declared.extend(repr_c_types(&text));
    }
    declared.sort();
    declared.dedup();

    // A floor, not a count of the API: it exists so a scan that finds
    // nothing fails loudly instead of passing by seeing nothing. Raise it
    // only if it stops being able to do that.
    assert!(
        declared.len() >= 12,
        "found only {} repr(C) types in {}, which means the scan is looking in the wrong place",
        declared.len(),
        dir.display()
    );

    let renames = rename_table(&manifest);
    let missing: Vec<_> = declared
        .iter()
        .map(|name| {
            (
                name.clone(),
                renames.get(name).cloned().unwrap_or_else(|| name.clone()),
            )
        })
        .filter(|(_, c_name)| !mentions(&header, c_name))
        .map(|(rust, c_name)| {
            if rust == c_name {
                format!(
                    "{rust} (no entry in [export.rename], so it would reach the header \
                         under its Rust name)"
                )
            } else {
                format!("{rust}, which the rename table spells {c_name}")
            }
        })
        .collect();

    assert!(
        missing.is_empty(),
        "these types are declared in Rust but absent from the generated header, which \
         means a consumer cannot name them. A type referred to by no exported function \
         is dropped silently, so each one has to be listed in cbindgen.toml's \
         `[export] include` AND given its C spelling in `[export.rename]`. Missing:\n  {}",
        missing.join("\n  ")
    );
}

/// The `[export.rename]` table, as `Rust name -> C name`.
///
/// Parsed rather than duplicated here: a second copy of the table would be
/// a second thing to keep in step, and this test exists precisely because
/// things kept in step by hand are not.
fn rename_table(manifest: &std::path::Path) -> HashMap<String, String> {
    let text = std::fs::read_to_string(manifest.join("cbindgen.toml"))
        .expect("the generator config is committed");
    let mut out = HashMap::new();
    let mut inside = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            inside = t == "[export.rename]";
            continue;
        }
        if !inside || t.is_empty() || t.starts_with('#') {
            continue;
        }
        if let Some((left, right)) = t.split_once('=') {
            let key = left.trim().trim_matches('"').to_string();
            let value = right.trim().trim_matches('"').to_string();
            out.insert(key, value);
        }
    }
    assert!(
        out.len() >= 10,
        "the rename table parsed as {} entries, so this test is not reading it",
        out.len()
    );
    out
}

/// Every `.rs` file under `dir`, at any depth.
fn rust_files(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(rust_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out
}

/// Every `pub` type carrying `#[repr(C)]` or `#[repr(u32)]`, by name.
fn repr_c_types(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut armed = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("#[repr(C)]") || t.starts_with("#[repr(u32)]") {
            armed = true;
            continue;
        }
        if !armed {
            continue;
        }
        // Attributes and doc comments may sit between the repr and the item.
        if t.starts_with('#') || t.starts_with("///") || t.starts_with("//") || t.is_empty() {
            continue;
        }
        armed = false;
        for kw in ["pub struct ", "pub union ", "pub enum "] {
            if let Some(rest) = t.strip_prefix(kw) {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    out.push(name);
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Whether the header declares `name` as a type of its own.
///
/// A bare substring search would pass on a mere mention inside a comment,
/// which is the one thing this test must not accept: the schema types were
/// all mentioned in prose while absent as declarations.
fn mentions(header: &str, name: &str) -> bool {
    header.lines().any(|l| {
        let t = l.trim();
        t == format!("}} {name};")
            || t.starts_with(&format!("struct {name} {{"))
            || t.starts_with(&format!("union {name} {{"))
            || t.starts_with(&format!("typedef enum {name}"))
            || t.starts_with(&format!("typedef struct {name} {name};"))
            || t.starts_with(&format!("typedef union {name} {name};"))
    })
}
