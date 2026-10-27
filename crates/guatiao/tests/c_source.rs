// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A source written in C, added by file and refreshed while the host runs.
//!
//! `tests/c_consumer/source_in_c.c` fills a `guatiao_source_info`, exports
//! `guatiao_source` and declares itself a source, and links nothing. This
//! compiles it, adds it to a registry -- from Rust and through the C
//! surface -- and drives its shelf through every change a source can make,
//! counting each open and close on the C side.

#![cfg(all(feature = "provider", feature = "load"))]

use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "support/clang_target.rs"]
mod clang_target;

use greeter_kind::Counter;
use guatiao::library::{Registry, ScanRules, Skipped, SourceError};
use guatiao::{Map, Value};

/// The C compilers worth trying, in order (as `c_consumer.rs`).
fn find_compiler() -> Option<PathBuf> {
    for name in ["clang", r"C:\Program Files\LLVM\bin\clang.exe", "cc", "gcc"] {
        let path = PathBuf::from(name);
        if path.is_absolute() && !path.exists() {
            continue;
        }
        if Command::new(&path).arg("--version").output().is_ok() {
            return Some(path);
        }
    }
    None
}

/// Builds the C source into a directory of its own, as `name`: each test
/// maps its own copy, whose statics are its own shelf.
fn build_shelf(cc: &Path, name: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("c-source");
    std::fs::create_dir_all(&dir).expect("a directory for the shelf");
    let out = dir.join(format!(
        "{}{name}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    let compile = Command::new(cc)
        .args(clang_target::args(cc))
        .args(["-shared", "-std=c11", "-Wall", "-Wextra", "-Werror"])
        .args(if cfg!(windows) {
            &[][..]
        } else {
            &["-fPIC"][..]
        })
        .arg("-I")
        .arg(manifest.join("include"))
        .arg("-o")
        .arg(&out)
        .arg(manifest.join("tests/c_consumer/source_in_c.c"))
        .output()
        .expect("the compiler answered --version, so it should run");
    assert!(
        compile.status.success(),
        "the C source did not compile against the header.\nstderr:\n{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    out
}

/// The shelf's controls, from the same image the registry mapped.
struct Shelf {
    image: libloading::Library,
}

impl Shelf {
    fn call(&self, name: &[u8], i: i32) {
        // SAFETY: the signature `source_in_c.c` declares.
        let f: libloading::Symbol<unsafe extern "C" fn(i32)> =
            unsafe { self.image.get(name) }.expect("exported");
        // SAFETY: an index into the shelf.
        unsafe { f(i) }
    }
    fn count(&self, name: &[u8], i: i32) -> i32 {
        // SAFETY: the signature `source_in_c.c` declares.
        let f: libloading::Symbol<unsafe extern "C" fn(i32) -> i32> =
            unsafe { self.image.get(name) }.expect("exported");
        // SAFETY: an index into the shelf.
        unsafe { f(i) }
    }
    fn refuse(&self, refuse: bool) {
        // SAFETY: the signature `source_in_c.c` declares.
        let f: libloading::Symbol<unsafe extern "C" fn(bool)> =
            unsafe { self.image.get(b"shelf_refuse\0") }.expect("exported");
        // SAFETY: a flag.
        unsafe { f(refuse) }
    }
    fn opened(&self, i: i32) -> i32 {
        self.count(b"shelf_opened\0", i)
    }
    fn closed(&self, i: i32) -> i32 {
        self.count(b"shelf_closed\0", i)
    }
}

fn config(ids: &[&str]) -> Map {
    let mut list = guatiao::List::new();
    for id in ids {
        list.push(*id);
    }
    let mut map = Map::new();
    map.set("ids", list);
    map
}

#[test]
fn a_source_written_in_c_serves_libraries_that_come_and_go() {
    let Some(cc) = find_compiler() else {
        println!("skipped: no C compiler found; this is the test that proves a source is a C ABI");
        return;
    };
    let path = build_shelf(&cc, "c_shelf");
    // SAFETY: the file the compiler just wrote; the registry maps the same
    // image, so these controls change what it reads.
    let shelf = Shelf {
        image: unsafe { libloading::Library::new(&path) }.expect("it loads"),
    };

    let mut registry = Registry::new("c-source-tests", "1.0");

    // A scan reports the file as a source and never loads it.
    let scanned = guatiao::library::scan_dir_rules(
        &mut registry,
        path.parent().unwrap(),
        guatiao::library::Order::Ascending,
        &ScanRules::default(),
    )
    .expect("a readable directory");
    assert!(scanned.loaded.is_empty());
    assert_eq!(
        scanned.skipped[0].1,
        Skipped::Source {
            serves: "c-shelf".into()
        }
    );

    // A configuration the source refuses, in its own words.
    let refused = registry
        .add_source_file(&path, &Map::new())
        .expect_err("no ids");
    assert!(
        matches!(&refused, SourceError::Refused { message, .. } if message.contains("`ids`")),
        "{refused:?}"
    );

    let name = registry
        .add_source_file(&path, &config(&["shelf_a", "shelf_b"]))
        .expect("the shelf configures");
    assert!(matches!(
        registry.add_source_file(&path, &config(&["shelf_a"])),
        Err(SourceError::Duplicate { .. })
    ));

    // 1. What is on the shelf loads, and is called as the Rust trait.
    let first = registry.refresh();
    let loaded: Vec<&str> = first.loaded.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(loaded, ["shelf_a", "shelf_b"]);
    assert_eq!(first.loaded[0].source, name);
    let counter = registry
        .offer::<dyn Counter>("shelf_a_counter")
        .expect("filed under its id")
        .expect("a counter table");
    let held = counter.default_instance().expect("its ctx is its instance");
    assert_eq!(held.count(), 1);
    assert!(registry.refresh().is_empty(), "nothing changed");

    // 2. A changed stamp serves a new build while the old is held.
    shelf.call(b"shelf_bump\0", 0);
    let changed = registry.refresh();
    assert_eq!(changed.loaded.len(), 1);
    assert_eq!(changed.retired.len(), 1);
    assert_eq!(shelf.opened(0), 2);
    registry.collect();
    assert_eq!(shelf.closed(0), 0, "the old build is held");
    assert_eq!(held.count(), 2, "and still answers");
    drop(counter);
    assert_eq!(registry.collect(), ["shelf_a"]);
    assert_eq!(shelf.closed(0), 1, "closed once nothing held it");

    // 3. A vanished one is retired and closed.
    shelf.call(b"shelf_take\0", 1);
    let gone = registry.refresh();
    assert_eq!(gone.retired[0].id, "shelf_b");
    registry.collect();
    assert_eq!(shelf.closed(1), 1);

    // 4. A refused open is reported, and nothing loads.
    shelf.refuse(true);
    shelf.call(b"shelf_put\0", 2);
    let refused = registry.refresh();
    assert!(refused.loaded.is_empty());
    assert_eq!(refused.refused[0].id.as_deref(), Some("shelf_c"));
    assert_eq!(refused.refused[0].reason, "the shelf refuses, says C");
    shelf.refuse(false);
    shelf.call(b"shelf_bump\0", 2);
    assert_eq!(registry.refresh().loaded[0].id, "shelf_c");

    // 5. Unloading a served library closes it; every close came once.
    registry.unload("shelf_c").expect("nothing holds it");
    assert_eq!(shelf.closed(2), 1);
    registry
        .unload("shelf_a")
        .expect("nothing holds the new build");
    assert_eq!(
        (shelf.closed(0), shelf.closed(1), shelf.closed(2)),
        (2, 1, 1)
    );
    assert_eq!(
        (shelf.opened(0), shelf.opened(1), shelf.opened(2)),
        (2, 1, 1)
    );
}

/// The same source, driven through the C surface a C or binding host
/// calls: add it by file, refresh, read the answer as a value.
#[test]
fn the_c_surface_adds_a_source_file_and_refreshes() {
    use guatiao::exports::library::{
        guatiao_registry_add_source_file, guatiao_registry_free, guatiao_registry_new,
        guatiao_registry_refresh,
    };
    use guatiao::library::ProviderError;
    use guatiao::value::convert::TryAsRef;
    use guatiao::{Alloc, Status, Str};

    let Some(cc) = find_compiler() else {
        println!("skipped: no C compiler found");
        return;
    };
    let path = build_shelf(&cc, "c_shelf_surface");
    let path_text = path.to_string_lossy().to_string();

    // SAFETY: two views of literal text, and the default allocator.
    let reg =
        unsafe { guatiao_registry_new(Str::new("c-surface"), Str::new("1.0"), core::ptr::null()) };
    assert!(!reg.is_null());
    let config = Value::from(config(&["shelf_a"]));
    let mut err = ProviderError::none();
    // SAFETY: a live registry, readable text, a well-formed value, a
    // writable error.
    let status =
        unsafe { guatiao_registry_add_source_file(reg, Str::new(&path_text), &config, &mut err) };
    assert_eq!(status, Status::GUATIAO_OK, "{}", err.message());

    let alloc = Alloc::rust();
    let mut out = Value::absent();
    // SAFETY: a live registry, the default allocator, a writable slot.
    let status = unsafe { guatiao_registry_refresh(reg, alloc.as_raw(), &mut out) };
    assert_eq!(status, Status::GUATIAO_OK);
    let answer: &Map = TryAsRef::<Map>::try_as_ref(&out).expect("a map");
    let loaded: &guatiao::List = answer
        .get("loaded")
        .and_then(TryAsRef::<guatiao::List>::try_as_ref)
        .expect("a list");
    assert_eq!(loaded.len(), 1);
    let first: &Map = TryAsRef::<Map>::try_as_ref(&loaded[0]).expect("a map");
    assert_eq!(
        first.get("id").and_then(TryAsRef::<str>::try_as_ref),
        Some("shelf_a")
    );
    assert_eq!(
        first.get("key").and_then(TryAsRef::<str>::try_as_ref),
        Some("shelf_a")
    );
    drop(out);

    // A file that is no source is refused with words.
    let mut err = ProviderError::none();
    let manifest = env!("CARGO_MANIFEST_DIR");
    let not_one = format!("{manifest}/Cargo.toml");
    // SAFETY: as above.
    let status =
        unsafe { guatiao_registry_add_source_file(reg, Str::new(&not_one), &config, &mut err) };
    assert_eq!(status, Status::GUATIAO_ERR_NOT_FOUND);
    assert!(!err.message().is_empty());

    // SAFETY: the handle made above, freed once.
    unsafe { guatiao_registry_free(reg) };
}
