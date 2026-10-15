// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The Rust snippets of the landing page and the root README, compiled and
//! checked to still be the text those pages show.
//!
//! Each test holds its own copy of a snippet between marker comments and
//! compares it, line for line, with the matching fenced `rust` block of the
//! page. A snippet that stops compiling fails the build; one that drifts from
//! its page fails the comparison. This crate holds them rather than `guatiao`
//! because the pages sit outside any published crate.

use std::path::Path;

const LANDING: &str = include_str!("../../../docs/index.md");
const README: &str = include_str!("../../../README.md");

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
