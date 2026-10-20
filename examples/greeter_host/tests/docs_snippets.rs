// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The Rust snippets of the landing page, the ownership guide and the root
//! README, compiled and checked to still be the text those pages show.
//!
//! Each test holds its own copy of a snippet between marker comments and
//! compares it, line for line, with the matching fenced `rust` block of the
//! page. A snippet that stops compiling fails the build; one that drifts from
//! its page fails the comparison. This crate holds them rather than `guatiao`
//! because the pages sit outside any published crate.

use std::path::Path;

const LANDING: &str = include_str!("../../../docs/index.md");
const README: &str = include_str!("../../../README.md");
const OWNERSHIP: &str = include_str!("../../../docs/ownership.md");

#[test]
fn the_landing_page_builds_and_reads_a_value() -> Result<(), Box<dyn core::error::Error>> {
    // TOUR-VALUE-BEGIN
    use guatiao::Map;

    let mut map = Map::new();
    map.set("host", "10.0.0.1");
    map.set("port", 5900);

    let host: &str = map.required("host")?.try_into()?;
    let port: u16 = map.required("port")?.try_into()?;
    // TOUR-VALUE-END
    assert_eq!((host, port), ("10.0.0.1", 5900));

    assert_eq!(rust_block(LANDING, 0), marked("TOUR-VALUE"));
    Ok(())
}

#[test]
fn the_landing_page_scans_a_directory_for_a_provider() {
    fn tour() -> Result<(), Box<dyn core::error::Error>> {
        // TOUR-REGISTRY-BEGIN
        use guatiao::library::{Registry, scan_dir};
        use std::path::Path;

        let mut registry = Registry::new("my-host", "1.0");
        scan_dir(&mut registry, Path::new("/opt/myapp/plugins"))?;
        let greeter = registry.best("greeter").expect("something serves it");
        // TOUR-REGISTRY-END
        let _ = greeter;
        Ok(())
    }

    // The directory does not exist, so the scan is refused before `best`.
    assert!(tour().is_err());
    assert!(!Path::new("/opt/myapp/plugins").exists());

    assert_eq!(rust_block(LANDING, 1), marked("TOUR-REGISTRY"));
}

#[test]
fn the_readme_quick_start_compiles_and_holds() -> Result<(), Box<dyn core::error::Error>> {
    // README-QUICK-START-BEGIN
    use guatiao::Map;

    let mut options = Map::new();
    options.set("compression", 6);

    let mut map = Map::new();
    map.set("host", "10.0.0.1");
    map.set("port", 5900);
    map.set("options", options);

    let host: &str = map.required("host")?.try_into()?;
    let port: u16 = map.required("port")?.try_into()?;
    assert_eq!((host, port), ("10.0.0.1", 5900));
    // README-QUICK-START-END

    assert_eq!(rust_block(README, 0), marked("README-QUICK-START"));
    Ok(())
}

#[test]
fn the_ownership_guide_passes_trees_by_value() -> Result<(), Box<dyn core::error::Error>> {
    // OWN-KIND-BEGIN
    use guatiao::library::ProviderError;
    use guatiao::{Alloc, Map, Text};

    #[guatiao::kind]
    pub trait Store: Send + Sync {
        /// Reads the configuration and keeps none of it: borrowed.
        fn check(&self, config: &Map) -> Result<bool, ProviderError>;
        /// Keeps the configuration: handed over, a move.
        fn save(&self, config: Map) -> Result<(), ProviderError>;
        /// An answer the caller owns, returned by value.
        fn load(&self) -> Result<Map, ProviderError>;
    }
    // OWN-KIND-END

    // OWN-PROVIDER-BEGIN
    use std::sync::Mutex;

    struct Files {
        saved: Mutex<Option<Map>>,
    }

    impl Store for Files {
        fn check(&self, config: &Map) -> Result<bool, ProviderError> {
            Ok(config.contains_key("path"))
        }

        fn save(&self, config: Map) -> Result<(), ProviderError> {
            // The caller's tree, now this provider's: kept as it arrived.
            *self.saved.lock().expect("not poisoned") = Some(config);
            Ok(())
        }

        fn load(&self) -> Result<Map, ProviderError> {
            self.load_in(Alloc::rust())
        }

        // What a call goes through: the answer is built in the allocator
        // the caller carried in, so it crosses back without a copy.
        fn load_in(&self, alloc: Alloc) -> Result<Map, ProviderError> {
            let mut map = Map::new_in(alloc);
            map.set_in("path", Text::new_in(alloc, "/etc/app.toml")?, alloc)?;
            Ok(map)
        }
    }
    // OWN-PROVIDER-END

    // OWN-HOST-BEGIN
    fn configure(store: &dyn Store) -> Result<Map, ProviderError> {
        let mut config = Map::new();
        config.set("path", "/etc/app.toml");
        // Lent to `check` for the call, then handed to `save`: nothing is
        // copied either way.
        if store.check(&config)? {
            store.save(config)?;
        }
        // Built in this side's allocator, by the provider.
        store.load()
    }
    // OWN-HOST-END

    let files = Files {
        saved: Mutex::new(None),
    };
    let loaded = configure(&files)?;
    assert!(loaded.is_local());
    assert!(files.saved.lock().expect("not poisoned").is_some());

    assert_eq!(rust_block(OWNERSHIP, 0), marked("OWN-KIND"));
    assert_eq!(rust_block(OWNERSHIP, 1), marked("OWN-PROVIDER"));
    assert_eq!(rust_block(OWNERSHIP, 2), marked("OWN-HOST"));
    Ok(())
}

#[test]
fn the_ownership_guide_builds_where_the_tree_will_live() -> Result<(), Box<dyn core::error::Error>>
{
    // OWN-ALLOC-BEGIN
    use guatiao::{Alloc, Map, Text, ValueError};

    /// What a library builds outside any call, in the allocator its host
    /// offered: in a library, `host.alloc().unwrap_or_else(Alloc::rust)`.
    fn describe(alloc: Alloc) -> Result<Map, ValueError> {
        let mut meta = Map::new_in(alloc);
        meta.set_in("vendor", Text::new_in(alloc, "acme")?, alloc)?;
        Ok(meta)
    }

    let meta = describe(Alloc::rust())?;
    // A tree in hand goes where it is wanted, copied only if it is not
    // already there.
    let kept = meta.into_alloc(Alloc::rust())?;
    // OWN-ALLOC-END
    assert!(kept.is_local());

    assert_eq!(rust_block(OWNERSHIP, 3), marked("OWN-ALLOC"));
    Ok(())
}

#[test]
fn the_ownership_guide_moves_instead_of_copying() -> Result<(), Box<dyn core::error::Error>> {
    // OWN-MOVES-BEGIN
    use guatiao::{List, Map, Value};

    // Build in place: a child goes into its parent by moving.
    let mut child = Map::new();
    child.set("depth", 2);
    let mut parent = Map::new();
    parent.set("child", child);

    // A container becomes a node by moving, and comes back out the same way.
    let node = Value::from(parent);
    let mut parent = Map::try_from(node).expect("a map");

    // Read by borrowing, not by converting into an owned copy.
    let child: &Map = parent.required("child")?.try_into()?;
    let depth: i64 = child.required("depth")?.try_into()?;

    // Take a value out instead of cloning it: `remove` hands it back owned,
    // and `mem::take` leaves a null behind.
    let taken: Value = parent.remove("child").expect("present");
    let mut list: List = ["a", "b"].into_iter().collect();
    let first = core::mem::take(&mut list[0]);

    // Own a tree you were handed: `into_local` copies only what is elsewhere.
    let owned = taken.into_local();
    // OWN-MOVES-END
    assert_eq!(depth, 2);
    assert_eq!(first, Value::from("a"));
    assert_eq!(list[0], Value::null());
    assert!(owned.is_local());

    assert_eq!(rust_block(OWNERSHIP, 4), marked("OWN-MOVES"));
    Ok(())
}

/// The `index`-th fenced `rust` block of `md`, minus doctest-hidden lines
/// (`#` alone or `# ` first).
fn rust_block(md: &str, index: usize) -> String {
    let mut found = 0;
    let mut inside = false;
    let mut out: Vec<&str> = Vec::new();

    for line in md.lines() {
        if !inside {
            if line.trim_end() == "```rust" {
                if found == index {
                    inside = true;
                }
                found += 1;
            }
            continue;
        }
        if line.trim_end() == "```" {
            return out.join("\n");
        }
        if line == "#" || line.starts_with("# ") {
            continue;
        }
        out.push(line);
    }
    panic!("no closed rust block number {index}");
}

/// The lines of this file between `// <name>-BEGIN` and `// <name>-END`,
/// dedented by the four spaces they carry for being inside a function; a
/// snippet nested one function deeper carries eight.
fn marked(name: &str) -> String {
    let begin = format!("// {name}-BEGIN");
    let end = format!("// {name}-END");
    let mut inside = false;
    let mut lines: Vec<&str> = Vec::new();

    for line in include_str!("docs_snippets.rs").lines() {
        let trimmed = line.trim_start();
        if !inside {
            inside = trimmed == begin;
            continue;
        }
        if trimmed == end {
            let indent = lines
                .iter()
                .filter(|l| !l.is_empty())
                .map(|l| l.len() - l.trim_start().len())
                .min()
                .unwrap_or(0);
            let body: Vec<&str> = lines
                .iter()
                .map(|l| if l.len() >= indent { &l[indent..] } else { *l })
                .collect();
            return body.join("\n");
        }
        lines.push(line);
    }
    panic!("the markers for {name} are missing");
}
